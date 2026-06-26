//! The [`NestedType`] family: Arrow types that carry child [`Field`]s or child
//! [`DataType`]s — lists, structs, maps, unions, dictionaries and run-end
//! encoding — plus the [`UnionMode`] enumeration.

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::parse::{split_head, split_top_level, Head};
use crate::{DataType, Field};

use super::DataTypeError;

/// How a [`Union`](NestedType::Union)'s child arrays are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum UnionMode {
    /// Every child holds a slot for every row (`sparse`).
    Sparse,
    /// Children are packed, addressed by an offsets buffer (`dense`).
    Dense,
}

impl UnionMode {
    /// Parses a mode name (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<UnionMode, DataTypeError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "sparse" => Ok(UnionMode::Sparse),
            "dense" => Ok(UnionMode::Dense),
            _ => Err(DataTypeError::Invalid(format!(
                "unknown union mode '{input}', expected 'sparse' or 'dense'"
            ))),
        }
    }

    /// The lowercase name (`sparse` / `dense`).
    pub fn as_str(&self) -> &'static str {
        match self {
            UnionMode::Sparse => "sparse",
            UnionMode::Dense => "dense",
        }
    }
}

impl fmt::Display for UnionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An Arrow type that contains other types: a list of one child, a struct of many
/// named children, a sorted-key map, a tagged union, a dictionary-encoded column,
/// or run-end encoding. Children are [`Field`]s (named, nullable) except a
/// [`Dictionary`](NestedType::Dictionary)'s key/value, which are bare
/// [`DataType`]s.
///
/// The string grammar renders each child [`Field`] with [`Field::to_str`], so a
/// nested type round-trips losslessly through [`from_str`](NestedType::from_str):
///
/// ```
/// use yggdryl_saga::{DataType, NestedType};
///
/// let dt = DataType::from_str("struct<id: int64, name: utf8 not null>").unwrap();
/// assert!(matches!(dt, DataType::Nested(NestedType::Struct(_))));
/// assert_eq!(dt.to_str(), "struct<id: int64, name: utf8 not null>");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NestedType {
    /// A variable-length list with 32-bit offsets (`list<child>`).
    List(Box<Field>),
    /// A variable-length list with 64-bit offsets (`large_list<child>`).
    LargeList(Box<Field>),
    /// A list in the view layout with 32-bit offsets (`list_view<child>`).
    ListView(Box<Field>),
    /// A list in the view layout with 64-bit offsets (`large_list_view<child>`).
    LargeListView(Box<Field>),
    /// A fixed-length list of the given element count (`fixed_size_list(n)<child>`).
    FixedSizeList(Box<Field>, i32),
    /// A struct of named child fields (`struct<a: …, b: …>`).
    Struct(Vec<Field>),
    /// A map of key→value entries; the child is the `entries` struct, and the flag
    /// records whether keys are sorted (`map<entries: …>` / `map(sorted)<…>`).
    Map(Box<Field>, bool),
    /// A tagged union of child fields with the given layout (`union(mode)<…>`).
    /// Type ids are assigned `0..n` in field order.
    Union(Vec<Field>, UnionMode),
    /// A dictionary-encoded column: an index type and a value type
    /// (`dictionary<key, value>`).
    Dictionary(Box<DataType>, Box<DataType>),
    /// Run-end encoding: a run-ends field and a values field
    /// (`run_end_encoded<run_ends: …, values: …>`).
    RunEndEncoded(Box<Field>, Box<Field>),
}

impl NestedType {
    /// Parses a canonical nested name (e.g. `list<item: int64>`,
    /// `struct<a: int64>`, `dictionary<int32, utf8>`). Returns
    /// [`DataTypeError::Unknown`] for a name that is not a nested type.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<NestedType, DataTypeError> {
        log_event!(trace, "NestedType::from_str {input:?}");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(DataTypeError::Empty);
        }
        let head =
            split_head(trimmed).ok_or_else(|| DataTypeError::Invalid(trimmed.to_string()))?;
        NestedType::from_head(&head)
    }

    /// Builds a nested type from a parsed [`Head`]. Unowned names return
    /// [`DataTypeError::Unknown`]; an owned name with a bad body returns
    /// [`DataTypeError::Invalid`].
    pub(crate) fn from_head(head: &Head) -> Result<NestedType, DataTypeError> {
        match head.name {
            "list" => Ok(NestedType::List(Self::one_field(head)?)),
            "large_list" => Ok(NestedType::LargeList(Self::one_field(head)?)),
            "list_view" => Ok(NestedType::ListView(Self::one_field(head)?)),
            "large_list_view" => Ok(NestedType::LargeListView(Self::one_field(head)?)),
            "fixed_size_list" => {
                let size = head
                    .params
                    .ok_or_else(|| {
                        DataTypeError::Invalid(
                            "'fixed_size_list' needs a length, e.g. fixed_size_list(3)<…>"
                                .to_string(),
                        )
                    })?
                    .trim()
                    .parse::<i32>()
                    .map_err(|_| {
                        DataTypeError::Invalid(
                            "fixed_size_list length must be an integer".to_string(),
                        )
                    })?;
                Ok(NestedType::FixedSizeList(Self::one_field(head)?, size))
            }
            "struct" => {
                let fields = Self::fields(head)?;
                Ok(NestedType::Struct(fields))
            }
            "map" => {
                let sorted = match head.params {
                    None => false,
                    Some("sorted") => true,
                    Some(other) => {
                        return Err(DataTypeError::Invalid(format!(
                            "unknown map flag '{other}', expected 'sorted'"
                        )))
                    }
                };
                Ok(NestedType::Map(Self::one_field(head)?, sorted))
            }
            "union" => {
                let mode = match head.params {
                    Some(m) => UnionMode::from_str(m)?,
                    None => {
                        return Err(DataTypeError::Invalid(
                            "'union' needs a mode, e.g. union(sparse)<…>".to_string(),
                        ))
                    }
                };
                Ok(NestedType::Union(Self::fields(head)?, mode))
            }
            "dictionary" => {
                let body = head.body.ok_or_else(|| {
                    DataTypeError::Invalid("'dictionary' needs <key, value>".to_string())
                })?;
                let parts = split_top_level(body, ',');
                if parts.len() != 2 {
                    return Err(DataTypeError::Invalid(
                        "'dictionary' needs exactly <key, value>".to_string(),
                    ));
                }
                let key = DataType::from_str(parts[0])?;
                let value = DataType::from_str(parts[1])?;
                Ok(NestedType::Dictionary(Box::new(key), Box::new(value)))
            }
            "run_end_encoded" => {
                let body = head.body.ok_or_else(|| {
                    DataTypeError::Invalid("'run_end_encoded' needs <run_ends, values>".to_string())
                })?;
                let parts = split_top_level(body, ',');
                if parts.len() != 2 {
                    return Err(DataTypeError::Invalid(
                        "'run_end_encoded' needs exactly <run_ends, values>".to_string(),
                    ));
                }
                let run_ends = Field::from_str(parts[0])?;
                let values = Field::from_str(parts[1])?;
                Ok(NestedType::RunEndEncoded(
                    Box::new(run_ends),
                    Box::new(values),
                ))
            }
            _ => Err(DataTypeError::Unknown(head.name.to_string())),
        }
    }

    /// Reads the single child [`Field`] from a nested type's `<body>`.
    fn one_field(head: &Head) -> Result<Box<Field>, DataTypeError> {
        let body = head.body.ok_or_else(|| {
            DataTypeError::Invalid(format!(
                "'{}' needs a child, e.g. {}<item: …>",
                head.name, head.name
            ))
        })?;
        let parts = split_top_level(body, ',');
        if parts.len() != 1 || parts[0].is_empty() {
            return Err(DataTypeError::Invalid(format!(
                "'{}' needs exactly one child field",
                head.name
            )));
        }
        Ok(Box::new(Field::from_str(parts[0])?))
    }

    /// Reads the comma-separated child [`Field`]s from a struct/union `<body>`
    /// (an empty body, `struct<>`, yields no fields).
    fn fields(head: &Head) -> Result<Vec<Field>, DataTypeError> {
        let body = head
            .body
            .ok_or_else(|| DataTypeError::Invalid(format!("'{}' needs a <body>", head.name)))?;
        if body.is_empty() {
            return Ok(Vec::new());
        }
        split_top_level(body, ',')
            .into_iter()
            .map(|s| Field::from_str(s).map_err(DataTypeError::from))
            .collect()
    }

    /// Renders the canonical name — the inverse of [`from_str`](NestedType::from_str).
    pub fn to_str(&self) -> String {
        use NestedType::*;
        let join = |fields: &[Field]| -> String {
            fields
                .iter()
                .map(Field::to_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            List(f) => format!("list<{}>", f.to_str()),
            LargeList(f) => format!("large_list<{}>", f.to_str()),
            ListView(f) => format!("list_view<{}>", f.to_str()),
            LargeListView(f) => format!("large_list_view<{}>", f.to_str()),
            FixedSizeList(f, n) => format!("fixed_size_list({n})<{}>", f.to_str()),
            Struct(fields) => format!("struct<{}>", join(fields)),
            Map(f, false) => format!("map<{}>", f.to_str()),
            Map(f, true) => format!("map(sorted)<{}>", f.to_str()),
            Union(fields, mode) => format!("union({mode})<{}>", join(fields)),
            Dictionary(key, value) => format!("dictionary<{}, {}>", key.to_str(), value.to_str()),
            RunEndEncoded(run_ends, values) => {
                format!(
                    "run_end_encoded<{}, {}>",
                    run_ends.to_str(),
                    values.to_str()
                )
            }
        }
    }
}

impl fmt::Display for NestedType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

/// Conversion to the matching `arrow_schema::DataType` (infallible). Children are
/// converted recursively through [`Field`]'s own Arrow conversion.
#[cfg(feature = "arrow")]
impl From<&NestedType> for arrow_schema::DataType {
    fn from(n: &NestedType) -> arrow_schema::DataType {
        use arrow_schema::DataType as A;
        use std::sync::Arc;
        use NestedType::*;
        let field_ref =
            |f: &Field| -> arrow_schema::FieldRef { Arc::new(arrow_schema::Field::from(f)) };
        match n {
            List(f) => A::List(field_ref(f)),
            LargeList(f) => A::LargeList(field_ref(f)),
            ListView(f) => A::ListView(field_ref(f)),
            LargeListView(f) => A::LargeListView(field_ref(f)),
            FixedSizeList(f, size) => A::FixedSizeList(field_ref(f), *size),
            Struct(fields) => A::Struct(
                fields
                    .iter()
                    .map(arrow_schema::Field::from)
                    .collect::<Vec<_>>()
                    .into(),
            ),
            Map(f, sorted) => A::Map(field_ref(f), *sorted),
            Union(fields, mode) => {
                let union_fields = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (i as i8, field_ref(f)))
                    .collect::<arrow_schema::UnionFields>();
                A::Union(union_fields, (*mode).into())
            }
            Dictionary(key, value) => A::Dictionary(
                Box::new(key.as_ref().into()),
                Box::new(value.as_ref().into()),
            ),
            RunEndEncoded(run_ends, values) => {
                A::RunEndEncoded(field_ref(run_ends), field_ref(values))
            }
        }
    }
}

#[cfg(feature = "arrow")]
impl From<UnionMode> for arrow_schema::UnionMode {
    fn from(m: UnionMode) -> arrow_schema::UnionMode {
        match m {
            UnionMode::Sparse => arrow_schema::UnionMode::Sparse,
            UnionMode::Dense => arrow_schema::UnionMode::Dense,
        }
    }
}

#[cfg(feature = "arrow")]
impl From<arrow_schema::UnionMode> for UnionMode {
    fn from(m: arrow_schema::UnionMode) -> UnionMode {
        match m {
            arrow_schema::UnionMode::Sparse => UnionMode::Sparse,
            arrow_schema::UnionMode::Dense => UnionMode::Dense,
        }
    }
}
