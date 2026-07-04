//! The [`SerieType`] data type.

use crate::{DataError, DataType, Nested};
use arrow_schema::FieldRef;

/// The Apache Arrow `list` data type: a variable-length sequence of one value type
/// (32-bit offsets).
///
/// It carries its single Arrow child — the nullable `"item"` field of the value type
/// — exactly as Arrow models it, so [`to_arrow`](DataType::to_arrow) /
/// [`from_arrow`](DataType::from_arrow) round-trip losslessly, like the dynamic
/// [`StructType`](crate::StructType) / [`UnionType`](crate::UnionType). It stays
/// *untyped* (the value's native Rust type is erased); a statically-typed serie
/// carrying the value type's byte codec is [`TypedSerieType<D>`](crate::TypedSerieType),
/// whose [`erase`](crate::TypedSerieType::erase) drops back to this dynamic type.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataType, Nested, Serie, SerieType};
///
/// let serie = SerieType::new(arrow_schema::DataType::Int64);
/// assert_eq!(serie.name(), "list");
/// assert_eq!(serie.arrow_format(), "+l");
/// assert_eq!(serie.byte_width(), None);
/// assert_eq!(serie.child_count(), 1);
/// assert_eq!(serie.item_field().name(), "item");
///
/// // to_arrow / from_arrow are lossless.
/// assert!(matches!(serie.to_arrow(), arrow_schema::DataType::List(..)));
/// assert_eq!(SerieType::from_arrow(&serie.to_arrow()).unwrap(), serie);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerieType {
    item: FieldRef,
}

impl SerieType {
    /// A serie of the given `value_type`, wrapping it in the nullable `"item"` child
    /// Arrow models a list around.
    pub fn new(value_type: arrow_schema::DataType) -> Self {
        Self {
            item: std::sync::Arc::new(arrow_schema::Field::new("item", value_type, true)),
        }
    }
}

impl super::Serie for SerieType {
    fn item_field(&self) -> FieldRef {
        self.item.clone()
    }
}

impl DataType for SerieType {
    fn name(&self) -> &str {
        "list"
    }

    fn arrow_format(&self) -> String {
        "+l".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::List(self.item.clone())
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        let incompatible = || DataError::IncompatibleArrowType {
            expected: "a serie of a nullable \"item\" child".to_string(),
            got: data_type.to_string(),
        };
        let arrow_schema::DataType::List(item) = data_type else {
            return Err(incompatible());
        };
        if item.name() != "item" || !item.is_nullable() || !item.metadata().is_empty() {
            return Err(incompatible());
        }
        Ok(Self { item: item.clone() })
    }
}

impl Nested for SerieType {
    fn child_count(&self) -> usize {
        1
    }
}
