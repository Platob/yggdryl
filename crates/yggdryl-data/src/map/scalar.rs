//! The [`MapScalar`] scalar of the [`MapType`](super::MapType) data type.

use super::MapType;
use crate::raw_scalar::{concat_scalar_arrays, scalars_from_elements};
use crate::{DataError, RawDataType, RawScalar, Scalar};

/// A single, possibly-null `map` value: a sequence of key–value entries of inner
/// scalars `SK` / `SV` over the key and value types `K` / `V`.
///
/// Its [`Value`](RawScalar::Value) is the borrowed slice `[(SK, SV)]`, so
/// [`value`](RawScalar::value) yields `Option<&[(SK, SV)]>`. The Arrow form is a
/// one-element `MapArray` whose entries struct concatenates the key and value
/// scalars' own Arrow forms; [`from_arrow`](RawScalar::from_arrow) redirects every
/// key and value back through the inner scalars' `from_arrow`.
///
/// ```
/// use yggdryl_data::{Int64, Int64Scalar, MapScalar, RawDataType, RawScalar, UInt8, UInt8Scalar};
///
/// let ranks = MapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]);
/// assert!(!ranks.is_null());
/// assert_eq!(ranks.value().map(<[_]>::len), Some(1));
/// assert_eq!(ranks.data_type().name(), "map");
///
/// // The Arrow round trip preserves the entries.
/// let arrow = ranks.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(MapScalar::from_arrow(arrow.as_ref()).unwrap(), ranks);
///
/// let missing: MapScalar<UInt8, Int64, UInt8Scalar, Int64Scalar> = MapScalar::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapScalar<K, V, SK, SV> {
    data_type: MapType<K, V>,
    entries: Option<Vec<(SK, SV)>>,
}

impl<K, V, SK, SV> MapScalar<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    /// A scalar holding the `entries` (an empty sequence is the empty map, not
    /// null).
    pub fn new(entries: Vec<(SK, SV)>) -> Self {
        Self {
            data_type: MapType::default(),
            entries: Some(entries),
        }
    }

    /// The null map scalar.
    pub fn null() -> Self {
        Self {
            data_type: MapType::default(),
            entries: None,
        }
    }
}

impl<K, V, SK, SV> Default for MapScalar<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    /// The default map scalar: the empty map.
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<K, V, SK, SV> From<Vec<(SK, SV)>> for MapScalar<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    /// A scalar holding the `entries`.
    fn from(entries: Vec<(SK, SV)>) -> Self {
        Self::new(entries)
    }
}

impl<K, V, SK, SV> RawScalar<MapType<K, V>> for MapScalar<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    type Value = [(SK, SV)];

    fn data_type(&self) -> &MapType<K, V> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.entries.is_none()
    }

    fn value(&self) -> Option<&[(SK, SV)]> {
        self.entries.as_deref()
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        let Some(entries) = &self.entries else {
            return arrow_array::new_null_array(&crate::RawDataType::to_arrow(&self.data_type), 1);
        };
        let entry_fields = self.data_type.entry_fields();
        let keys = concat_scalar_arrays(
            entries.iter().map(|(key, _)| key.to_arrow()).collect(),
            entry_fields[0].data_type(),
        );
        let values = concat_scalar_arrays(
            entries.iter().map(|(_, value)| value.to_arrow()).collect(),
            entry_fields[1].data_type(),
        );
        let entry_struct = arrow_array::StructArray::try_new_with_length(
            entry_fields,
            vec![keys, values],
            None,
            entries.len(),
        )
        .expect("per-entry one-element arrays assemble into the entries struct");
        let array = arrow_array::MapArray::try_new(
            self.data_type.entries_field(),
            arrow_buffer::OffsetBuffer::from_lengths([entries.len()]),
            entry_struct,
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
        // The data type validates the layout and redirects the key and value
        // children to `K` / `V`; then every entry redirects to the inner scalars.
        let data_type = MapType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::MapArray>()
            .expect("a value with a map data type is a map array");
        let entries = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            let entry_struct = array.value(0);
            let keys: Vec<SK> = scalars_from_elements(entry_struct.column(0).as_ref())?;
            let values: Vec<SV> = scalars_from_elements(entry_struct.column(1).as_ref())?;
            Some(keys.into_iter().zip(values).collect())
        };
        Ok(Self { data_type, entries })
    }
}

impl<K, V, SK, SV> Scalar<[(SK, SV)]> for MapScalar<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    type Type = MapType<K, V>;
}
