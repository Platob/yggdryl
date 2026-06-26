//! The [`Field`] type — a named, nullable [`DataType`] with metadata: the header
//! of a column, and the child element of every [`NestedType`](crate::NestedType).

use std::collections::BTreeMap;
use std::fmt;

use crate::cast::CastError;
#[allow(unused_imports)]
use crate::log_event;
use crate::parse::find_top_level;
use crate::{DataType, DataTypeError};

/// The suffix that marks a non-nullable field in the string grammar.
const NOT_NULL: &str = "not null";

/// Error returned when a [`Field`] cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    /// The input was empty.
    Empty,
    /// No `name: type` separator was found.
    MissingSeparator(String),
    /// The data type after the separator was invalid.
    DataType(DataTypeError),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldError::Empty => write!(f, "field is empty"),
            FieldError::MissingSeparator(value) => {
                write!(f, "field '{value}' is not 'name: type' (missing ':')")
            }
            FieldError::DataType(err) => write!(f, "field data type: {err}"),
        }
    }
}

impl std::error::Error for FieldError {}

impl From<DataTypeError> for FieldError {
    fn from(err: DataTypeError) -> FieldError {
        FieldError::DataType(err)
    }
}

/// A named, nullable [`DataType`] carrying optional string key/value metadata —
/// the column header, mirroring `arrow_schema::Field`.
///
/// Metadata is kept in a [`BTreeMap`] for deterministic ordering (so rendering
/// and serialisation are stable); it is dropped from the string form (use
/// `serde`, or the Arrow bridge, to preserve it).
///
/// ```
/// use yggdryl_saga::{DataType, Field, PrimitiveType};
///
/// let f = Field::new("price", DataType::from(PrimitiveType::Float64), false);
/// assert_eq!(f.name(), "price");
/// assert!(!f.is_nullable());
/// assert_eq!(f.to_str(), "price: float64 not null");
/// assert_eq!(Field::from_str("price: float64 not null").unwrap(), f);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field {
    name: String,
    data_type: DataType,
    nullable: bool,
    metadata: BTreeMap<String, String>,
}

impl Field {
    /// Creates a field with no metadata.
    pub fn new(name: impl Into<String>, data_type: DataType, nullable: bool) -> Field {
        Field {
            name: name.into(),
            data_type,
            nullable,
            metadata: BTreeMap::new(),
        }
    }

    /// Creates a **nullable** field (the common case when inferring a schema from a
    /// CSV/Parquet source).
    pub fn nullable(name: impl Into<String>, data_type: DataType) -> Field {
        Field::new(name, data_type, true)
    }

    /// Creates a **non-nullable** (required) field.
    pub fn required(name: impl Into<String>, data_type: DataType) -> Field {
        Field::new(name, data_type, false)
    }

    /// The field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's [`DataType`].
    pub fn data_type(&self) -> &DataType {
        &self.data_type
    }

    /// Whether the field admits nulls.
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// The field's metadata map (possibly empty).
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// The metadata value for `key`, if present.
    pub fn metadata_value(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Returns a copy with the name replaced.
    pub fn with_name(mut self, name: impl Into<String>) -> Field {
        self.name = name.into();
        self
    }

    /// Returns a copy with the data type replaced (unchecked — see
    /// [`cast`](Field::cast) for a validated change).
    pub fn with_data_type(mut self, data_type: DataType) -> Field {
        self.data_type = data_type;
        self
    }

    /// Returns a copy with the nullability replaced.
    pub fn with_nullable(mut self, nullable: bool) -> Field {
        self.nullable = nullable;
        self
    }

    /// Returns a copy with the metadata replaced.
    pub fn with_metadata(mut self, metadata: BTreeMap<String, String>) -> Field {
        self.metadata = metadata;
        self
    }

    /// Returns a copy with one metadata entry inserted (or overwritten).
    pub fn with_metadata_entry(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Field {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Returns the field re-typed to `data_type`, keeping the name, nullability and
    /// metadata. Errors with [`CastError`] when the current type cannot cast to the
    /// target (see [`DataType::can_cast_to`]). This is the field-level mirror of
    /// [`Column::cast`](crate::Column::cast).
    pub fn cast(&self, data_type: DataType) -> Result<Field, CastError> {
        if self.data_type.can_cast_to(&data_type) {
            Ok(self.clone().with_data_type(data_type))
        } else {
            Err(CastError::Unsupported {
                from: self.data_type.clone(),
                to: data_type,
            })
        }
    }

    /// Parses a `name: type` string, with an optional trailing `not null` marking
    /// the field non-nullable (the default is nullable). The type is parsed by
    /// [`DataType::from_str`].
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Field, FieldError> {
        log_event!(trace, "Field::from_str {input:?}");
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(FieldError::Empty);
        }
        let sep = find_top_level(trimmed, ':')
            .ok_or_else(|| FieldError::MissingSeparator(trimmed.to_string()))?;
        let name = trimmed[..sep].trim().to_string();
        let mut rest = trimmed[sep + 1..].trim();

        let nullable = if let Some(stripped) = rest.strip_suffix(NOT_NULL) {
            rest = stripped.trim_end();
            false
        } else {
            true
        };
        let data_type = DataType::from_str(rest)?;
        Ok(Field {
            name,
            data_type,
            nullable,
            metadata: BTreeMap::new(),
        })
    }

    /// Renders to `name: type` (plus a trailing ` not null` when non-nullable) —
    /// the inverse of [`from_str`](Field::from_str). Metadata is not rendered.
    pub fn to_str(&self) -> String {
        if self.nullable {
            format!("{}: {}", self.name, self.data_type.to_str())
        } else {
            format!("{}: {} {NOT_NULL}", self.name, self.data_type.to_str())
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

/// Conversion to `arrow_schema::Field` (infallible), carrying name, type,
/// nullability and metadata across the boundary.
#[cfg(feature = "arrow")]
impl From<&Field> for arrow_schema::Field {
    fn from(f: &Field) -> arrow_schema::Field {
        let metadata: std::collections::HashMap<String, String> = f
            .metadata
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        arrow_schema::Field::new(f.name.clone(), (&f.data_type).into(), f.nullable)
            .with_metadata(metadata)
    }
}

/// Conversion from `arrow_schema::Field` (infallible).
#[cfg(feature = "arrow")]
impl From<&arrow_schema::Field> for Field {
    fn from(f: &arrow_schema::Field) -> Field {
        Field {
            name: f.name().clone(),
            data_type: f.data_type().into(),
            nullable: f.is_nullable(),
            metadata: f
                .metadata()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

impl Field {
    /// Converts to an `arrow_schema::Field` (infallible).
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        self.into()
    }

    /// Builds a [`Field`] from an `arrow_schema::Field` (infallible).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Field {
        field.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrimitiveType;

    #[test]
    fn accessors_and_builders() {
        let f = Field::new("id", PrimitiveType::Int64.into(), false);
        assert_eq!(f.name(), "id");
        assert_eq!(f.data_type(), &DataType::from(PrimitiveType::Int64));
        assert!(!f.is_nullable());
        assert!(f.metadata().is_empty());

        let g = f.clone().with_name("key").with_nullable(true);
        assert_eq!(g.name(), "key");
        assert!(g.is_nullable());
        // The original is untouched (builders are non-mutating).
        assert_eq!(f.name(), "id");
    }

    #[test]
    fn constructors_and_metadata_helpers() {
        assert!(Field::nullable("a", PrimitiveType::Int64.into()).is_nullable());
        assert!(!Field::required("a", PrimitiveType::Int64.into()).is_nullable());
        let f = Field::nullable("a", PrimitiveType::Int64.into()).with_metadata_entry("unit", "ms");
        assert_eq!(f.metadata_value("unit"), Some("ms"));
        assert_eq!(f.metadata_value("nope"), None);
    }

    #[test]
    fn cast_keeps_identity_and_validates() {
        let f =
            Field::new("ts", PrimitiveType::Utf8.into(), true).with_metadata_entry("source", "csv");
        // utf8 -> timestamp is a valid cast; name/nullability/metadata survive.
        let ts = DataType::from_str("timestamp(ns, UTC)").unwrap();
        let casted = f.cast(ts.clone()).unwrap();
        assert_eq!(casted.name(), "ts");
        assert!(casted.is_nullable());
        assert_eq!(casted.data_type(), &ts);
        assert_eq!(casted.metadata_value("source"), Some("csv"));
        // A struct target is not castable from utf8.
        let struct_ty = DataType::from_str("struct<a: int64>").unwrap();
        assert!(f.cast(struct_ty).is_err());
    }

    #[test]
    fn string_round_trips() {
        for (s, name, nullable) in [
            ("id: int64", "id", true),
            ("id: int64 not null", "id", false),
            ("ts: timestamp(ns, UTC) not null", "ts", false),
            ("tags: list<item: utf8>", "tags", true),
        ] {
            let f = Field::from_str(s).unwrap();
            assert_eq!(f.name(), name, "{s}");
            assert_eq!(f.is_nullable(), nullable, "{s}");
            assert_eq!(f.to_str(), s, "{s}");
        }
    }

    #[test]
    fn nested_field_name_is_top_level() {
        // The ':' inside the nested type is not the field separator.
        let f = Field::from_str("col: struct<a: int64, b: utf8 not null>").unwrap();
        assert_eq!(f.name(), "col");
        assert!(f.data_type().is_nested());
    }

    #[test]
    fn errors() {
        assert_eq!(Field::from_str(""), Err(FieldError::Empty));
        assert!(matches!(
            Field::from_str("noseparator"),
            Err(FieldError::MissingSeparator(_))
        ));
        assert!(matches!(
            Field::from_str("x: notatype"),
            Err(FieldError::DataType(_))
        ));
    }

    #[cfg(feature = "arrow")]
    #[test]
    fn arrow_round_trips_with_metadata() {
        let mut meta = BTreeMap::new();
        meta.insert("unit".to_string(), "bps".to_string());
        let f = Field::new("spread", PrimitiveType::Float64.into(), false).with_metadata(meta);

        let arrow = f.to_arrow();
        assert_eq!(arrow.name(), "spread");
        assert!(!arrow.is_nullable());
        assert_eq!(arrow.metadata().get("unit"), Some(&"bps".to_string()));
        assert_eq!(Field::from_arrow(&arrow), f);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_preserves_metadata() {
        let mut meta = BTreeMap::new();
        meta.insert("k".to_string(), "v".to_string());
        let f = Field::new("c", PrimitiveType::Int64.into(), true).with_metadata(meta);
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<Field>(&json).unwrap(), f);
    }
}
