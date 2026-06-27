//! Nested-category types: the [`UnionMode`], the container checks, the child-field
//! accessors and the nested constructors (`list`, `struct_`, `map`, …).

use std::fmt;

use super::{DataType, SchemaError};
use crate::Field;

/// The physical layout of a [`Union`](DataType::Union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum UnionMode {
    /// Every child array is full-length; a value occupies its slot in each.
    Sparse,
    /// Child arrays are packed; an offsets buffer points into them.
    Dense,
}

impl UnionMode {
    /// Parses a union-mode token (case-insensitive), `"sparse"` or `"dense"`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<UnionMode, SchemaError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sparse" => Ok(UnionMode::Sparse),
            "dense" => Ok(UnionMode::Dense),
            _ => Err(SchemaError::UnknownUnit(value.to_string())),
        }
    }

    /// The lowercase name (`"sparse"` / `"dense"`).
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

impl DataType {
    // ---- constructors ----

    /// A variable-length [`List`](DataType::List) of `item`.
    pub fn list(item: Field) -> DataType {
        DataType::List {
            item: Box::new(item),
            large: false,
            view: false,
            size: None,
        }
    }

    /// A 64-bit-offset [`LargeList`](DataType::List) of `item`.
    pub fn large_list(item: Field) -> DataType {
        DataType::List {
            item: Box::new(item),
            large: true,
            view: false,
            size: None,
        }
    }

    /// A fixed-length list of `item`, `size` elements long.
    pub fn fixed_size_list(item: Field, size: i32) -> DataType {
        DataType::List {
            item: Box::new(item),
            large: false,
            view: false,
            size: Some(size),
        }
    }

    /// A [`Struct`](DataType::Struct) of the given fields.
    pub fn struct_(fields: Vec<Field>) -> DataType {
        DataType::Struct(fields)
    }

    /// A [`Map`](DataType::Map) from `key` to `value`.
    pub fn map(key: DataType, value: DataType, sorted: bool) -> DataType {
        DataType::Map {
            key: Box::new(key),
            value: Box::new(value),
            sorted,
        }
    }

    /// A [`Union`](DataType::Union) of the given alternatives.
    pub fn union(fields: Vec<Field>, mode: UnionMode) -> DataType {
        DataType::Union { fields, mode }
    }

    /// A [`RunEndEncoded`](DataType::RunEndEncoded) of `run_ends` (an integer) and `values`.
    pub fn run_end_encoded(run_ends: DataType, values: DataType) -> DataType {
        DataType::RunEndEncoded {
            run_ends: Box::new(run_ends),
            values: Box::new(values),
        }
    }

    // ---- checks / accessors ----

    /// Whether this is a [nested](super::TypeCategory::Nested) container.
    pub fn is_nested(&self) -> bool {
        use DataType::*;
        matches!(
            self,
            List { .. } | Struct(_) | Map { .. } | Union { .. } | RunEndEncoded { .. }
        )
    }

    /// Whether this is a [`List`](DataType::List) (any list kind).
    pub fn is_list(&self) -> bool {
        matches!(self, DataType::List { .. })
    }

    /// Whether this is a [`Struct`](DataType::Struct).
    pub fn is_struct(&self) -> bool {
        matches!(self, DataType::Struct(_))
    }

    /// Whether this is a [`Union`](DataType::Union).
    pub fn is_union(&self) -> bool {
        matches!(self, DataType::Union { .. })
    }

    /// Whether this is a [`Map`](DataType::Map).
    pub fn is_map(&self) -> bool {
        matches!(self, DataType::Map { .. })
    }

    /// The immediate child [`Field`]s of a nested type — the list item, the struct
    /// members, or the union alternatives. Maps / run-end / dictionary types hold
    /// child *types* (not fields) and report an empty slice here.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Field};
    /// let s = DataType::struct_(vec![Field::new("a", DataType::int(32, true), true)]);
    /// assert_eq!(s.children().len(), 1);
    /// assert!(DataType::int(32, true).children().is_empty());
    /// ```
    pub fn children(&self) -> Vec<&Field> {
        match self {
            DataType::List { item, .. } => vec![item],
            DataType::Struct(fields) | DataType::Union { fields, .. } => fields.iter().collect(),
            _ => Vec::new(),
        }
    }
}
