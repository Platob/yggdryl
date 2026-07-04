//! The [`TypedSerieType`] data type.

use crate::{DataError, DataType, Nested, SerieType, TypedDataType};

/// The statically-typed [`SerieType`](crate::SerieType): a serie of a value type `D`
/// known at compile time.
///
/// Where the dynamic [`SerieType`](crate::SerieType) carries its child as an Arrow
/// field, `TypedSerieType<D>` keeps the concrete value type `D`, so it adds the
/// [`TypedSerie`](crate::TypedSerie) surface — the value-type accessor and the
/// [`TypedDataType<Vec<T>>`] byte codec. The codec concatenates the value type's
/// per-element bytes; splitting them back requires the value type's fixed
/// [`byte_width`](DataType::byte_width) (a variable-width element errors with
/// [`DataError::IndeterminateElementWidth`] — decode such lists from Arrow).
/// [`erase`](TypedSerieType::erase) drops the static type back to a dynamic
/// [`SerieType`](crate::SerieType).
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Serie, SerieType, TypedDataType, TypedSerie, TypedSerieType};
///
/// let serie = TypedSerieType::new(Int64Type);
/// assert_eq!(serie.name(), "list");
/// assert_eq!(serie.arrow_format(), "+l");
/// assert_eq!(serie.value_type().name(), "int64");
/// assert_eq!(serie.item_field().name(), "item");
///
/// // The byte codec is per-element concatenation of the value type's codec.
/// let bytes = serie.native_to_bytes(&vec![1, 2]);
/// assert_eq!(bytes.len(), 16);
/// assert_eq!(serie.native_from_bytes(&bytes).unwrap(), vec![1, 2]);
/// assert_eq!(serie.default_value(), Vec::<i64>::new());
///
/// // Erase to the dynamic serie; from_arrow is the exact inverse of to_arrow.
/// assert_eq!(serie.erase(), SerieType::from_arrow(&serie.to_arrow()).unwrap());
/// assert_eq!(TypedSerieType::from_arrow(&serie.to_arrow()).unwrap(), serie);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TypedSerieType<D> {
    value_type: D,
}

impl<D: DataType> TypedSerieType<D> {
    /// The serie of `value_type`.
    pub fn new(value_type: D) -> Self {
        Self { value_type }
    }

    /// Drop the static value type, returning the dynamic [`SerieType`].
    pub fn erase(&self) -> SerieType {
        SerieType::new(self.value_type.to_arrow())
    }
}

impl<D: DataType> super::Serie for TypedSerieType<D> {
    fn item_field(&self) -> arrow_schema::FieldRef {
        std::sync::Arc::new(arrow_schema::Field::new(
            "item",
            self.value_type.to_arrow(),
            true,
        ))
    }
}

impl<D: DataType> DataType for TypedSerieType<D> {
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
        arrow_schema::DataType::List(super::Serie::item_field(self))
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        // Reuse the dynamic serie's structural validation, then decode the child.
        let dynamic = SerieType::from_arrow(data_type)?;
        Ok(Self::new(D::from_arrow(
            super::Serie::item_field(&dynamic).data_type(),
        )?))
    }
}

impl<D: DataType> Nested for TypedSerieType<D> {
    fn child_count(&self) -> usize {
        1
    }
}

impl<T, D: TypedDataType<T>> TypedDataType<Vec<T>> for TypedSerieType<D> {
    fn native_to_bytes(&self, values: &Vec<T>) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| self.value_type.native_to_bytes(value))
            .collect()
    }

    fn native_from_bytes(&self, bytes: &[u8]) -> Result<Vec<T>, DataError> {
        let width = self
            .value_type
            .codec_byte_width()
            .filter(|width| *width > 0)
            .ok_or_else(|| DataError::IndeterminateElementWidth {
                data_type: self.value_type.name().to_string(),
            })?;
        if !bytes.len().is_multiple_of(width) {
            return Err(DataError::InvalidByteLength {
                // The nearest valid length: a whole number of elements, rounded up.
                expected: bytes.len().div_ceil(width) * width,
                got: bytes.len(),
            });
        }
        bytes
            .chunks(width)
            .map(|chunk| self.value_type.native_from_bytes(chunk))
            .collect()
    }

    fn default_value(&self) -> Vec<T> {
        Vec::new()
    }
}

impl<T, D: TypedDataType<T>> crate::TypedNested<Vec<T>> for TypedSerieType<D> {}

impl<T, D: TypedDataType<T>> super::TypedSerie<T> for TypedSerieType<D> {
    type ValueType = D;

    fn value_type(&self) -> &D {
        &self.value_type
    }
}
