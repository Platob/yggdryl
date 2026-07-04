//! The dynamic [`MapScalar`] scalar of the [`MapType`](yggdryl_dtype::MapType) data
//! type.

use crate::Scalar;
use arrow_array::ArrayRef;
use yggdryl_dtype::{DataError, DataType, MapType};

/// A single, possibly-null `map` value with its key and value types erased: a
/// sequence of key–value entries backed by one zero-copy Arrow `"entries"` struct
/// array, carrying a dynamic [`MapType`](yggdryl_dtype::MapType).
///
/// It is the untyped base of the statically-typed
/// [`TypedMapScalar<K, V, SK, SV>`](crate::TypedMapScalar): it implements only the
/// base [`Scalar`] surface ([`to_arrow_scalar`](Scalar::to_arrow_scalar) /
/// [`from_arrow`](Scalar::from_arrow), all reference-count bumps) plus
/// [`len`](MapScalar::len) / [`is_empty`](MapScalar::is_empty), since the entry
/// scalar types are erased — the per-entry native accessors and the
/// [`TypedScalar`](crate::TypedScalar) surface live on `TypedMapScalar<K, V, SK, SV>`,
/// which [`erase`](crate::TypedMapScalar::erase)s back to this type.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::DataType;
/// use yggdryl_scalar::{Int64Scalar, Scalar, TypedMapScalar, UInt8Scalar};
///
/// // A dynamic map is reached by erasing a typed one, or from Arrow.
/// let ranks =
///     TypedMapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]).unwrap().erase();
/// assert!(!ranks.is_null());
/// assert_eq!(ranks.len(), 1);
/// assert_eq!(ranks.data_type().name(), "map");
/// assert_eq!(
///     yggdryl_scalar::MapScalar::from_arrow(ranks.to_arrow_scalar().as_ref()).unwrap(),
///     ranks
/// );
/// ```
#[derive(Debug, Clone)]
pub struct MapScalar {
    data_type: MapType,
    entries: Option<ArrayRef>,
}

impl MapScalar {
    /// A dynamic map over an already-built Arrow `entries` struct array (shared
    /// zero-copy) of the given dynamic `data_type`, or the null map for `None`.
    pub(crate) fn from_parts(data_type: MapType, entries: Option<ArrayRef>) -> Self {
        Self { data_type, entries }
    }

    /// The number of entries, `0` when null or empty ([`is_null`](Scalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.entries
            .as_ref()
            .map_or(0, |entries| arrow_array::Array::len(entries.as_ref()))
    }

    /// Whether the map holds no entries (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl PartialEq for MapScalar {
    // The backing entries arrays compare by value through `dyn Array` equality, so
    // two maps are equal when their entries (and nulls) are; null is distinct from
    // every present map.
    fn eq(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (None, None) => true,
            (Some(left), Some(right)) => left.as_ref() == right.as_ref(),
            _ => false,
        }
    }
}

impl Eq for MapScalar {}

impl Scalar for MapScalar {
    type DataType = MapType;
    type Value = dyn arrow_array::Array;

    fn data_type(&self) -> &MapType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.entries.is_none()
    }

    fn value(&self) -> Option<&(dyn arrow_array::Array + 'static)> {
        self.entries.as_deref()
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let Some(entries) = &self.entries else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The entries struct is shared into the one-element map — a reference-count
        // bump, not a copy.
        let entry_struct = entries
            .as_ref()
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .expect("a dynamic map's entries are the entries struct array");
        let array = arrow_array::MapArray::try_new(
            yggdryl_dtype::Map::entries_field(&self.data_type),
            arrow_buffer::OffsetBuffer::from_lengths([arrow_array::Array::len(entry_struct)]),
            entry_struct.clone(),
            None,
            false,
        )
        .expect("a one-element map of the declared entries struct is valid");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout; the entries struct is shared zero-copy.
        let data_type = MapType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::MapArray>()
            .expect("a value with a map data type is a map array");
        let entries = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(std::sync::Arc::new(array.value(0)) as ArrayRef)
        };
        Ok(Self { data_type, entries })
    }
}
