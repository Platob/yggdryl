//! The [`List`] data type.

use crate::{DataError, DataType, RawDataType, RawNested};

/// The Apache Arrow `list` data type: a variable-length sequence of one value type
/// `D` (32-bit offsets).
///
/// Its single child is the nullable `"item"` field of the value type. The typed
/// [`DataType<Vec<T>>`] byte codec concatenates the value type's per-element bytes;
/// splitting them back requires the value type's fixed
/// [`byte_width`](RawDataType::byte_width) (a variable-width element errors with
/// [`DataError::IndeterminateElementWidth`] — decode such lists from Arrow).
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataType, Int64, List, RawDataType, RawList};
///
/// let list = List::new(Int64);
/// assert_eq!(list.name(), "list");
/// assert_eq!(list.arrow_format(), "+l");
/// assert_eq!(list.byte_width(), None);
/// assert_eq!(list.value_type().name(), "int64");
///
/// // The byte codec is per-element concatenation of the value type's codec.
/// let bytes = list.native_to_bytes(&vec![1, 2]);
/// assert_eq!(bytes.len(), 16);
/// assert_eq!(list.native_from_bytes(&bytes).unwrap(), vec![1, 2]);
///
/// // The type knows its default: the empty list.
/// assert_eq!(list.default_value(), Vec::<i64>::new());
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert!(matches!(list.to_arrow(), arrow_schema::DataType::List(..)));
/// assert_eq!(List::from_arrow(&list.to_arrow()).unwrap(), list);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct List<D> {
    value_type: D,
}

impl<D: RawDataType> List<D> {
    /// The list of `value_type`.
    pub fn new(value_type: D) -> Self {
        Self { value_type }
    }

    /// The list's single Arrow child: the nullable `"item"` field of the value
    /// type — the exact child [`to_arrow`](RawDataType::to_arrow) wraps (the
    /// scalar layer assembles its one-element `ListArray` around it).
    pub fn item_field(&self) -> arrow_schema::FieldRef {
        std::sync::Arc::new(arrow_schema::Field::new(
            "item",
            self.value_type.to_arrow(),
            true,
        ))
    }
}

impl<D: RawDataType> super::RawList<D> for List<D> {
    fn value_type(&self) -> &D {
        &self.value_type
    }
}

impl<D: RawDataType> RawDataType for List<D> {
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
        arrow_schema::DataType::List(self.item_field())
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        let incompatible = || DataError::IncompatibleArrowType {
            expected: "a list of a nullable \"item\" child".to_string(),
            got: data_type.to_string(),
        };
        let arrow_schema::DataType::List(item) = data_type else {
            return Err(incompatible());
        };
        if item.name() != "item" || !item.is_nullable() || !item.metadata().is_empty() {
            return Err(incompatible());
        }
        // The item child redirects to the value type's own from_arrow.
        Ok(Self::new(D::from_arrow(item.data_type())?))
    }
}

impl<D: RawDataType> RawNested for List<D> {
    fn child_count(&self) -> usize {
        1
    }
}

impl<T, D: DataType<T>> DataType<Vec<T>> for List<D> {
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

impl<T, D: DataType<T>> crate::Nested<Vec<T>> for List<D> {}

impl<T, D: DataType<T>> super::TypedList<T> for List<D> {
    type ValueType = D;
}
