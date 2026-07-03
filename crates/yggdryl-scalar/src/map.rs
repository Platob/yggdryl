//! The [`Map`] scalar of the [`map`](yggdryl_dtype::Map) data type.

use crate::raw_scalar::{concat_scalar_arrays, scalars_from_elements};
use crate::{DefaultScalar, RawScalar, Scalar};
use yggdryl_dtype::{DataError, RawDataType};

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
/// use yggdryl_scalar::yggdryl_dtype::{Int64 as Int64Type, RawDataType, UInt8 as UInt8Type};
/// use yggdryl_scalar::{Int64, Map, RawScalar, UInt8};
///
/// let ranks = Map::new(vec![(UInt8::new(7), Int64::new(42))]).unwrap();
/// assert!(!ranks.is_null());
/// assert_eq!(ranks.value().map(<[_]>::len), Some(1));
/// assert_eq!(ranks.data_type().name(), "map");
///
/// // The Arrow round trip preserves the entries.
/// let arrow = ranks.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Map::from_arrow(arrow.as_ref()).unwrap(), ranks);
///
/// let missing: Map<UInt8Type, Int64Type, UInt8, Int64> = Map::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Map<K, V, SK, SV> {
    data_type: yggdryl_dtype::Map<K, V>,
    entries: Option<Vec<(SK, SV)>>,
}

impl<K, V, SK, SV> Map<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    /// A scalar holding the `entries` (an empty sequence is the empty map, not
    /// null). A null key errors: Arrow map keys are non-nullable.
    pub fn new(entries: Vec<(SK, SV)>) -> Result<Self, DataError> {
        if entries.iter().any(|(key, _)| key.is_null()) {
            return Err(DataError::IncompatibleArrowType {
                expected: "non-null map keys".to_string(),
                got: "a null key scalar".to_string(),
            });
        }
        Ok(Self {
            data_type: yggdryl_dtype::Map::default(),
            entries: Some(entries),
        })
    }

    /// The null map scalar.
    pub fn null() -> Self {
        Self {
            data_type: yggdryl_dtype::Map::default(),
            entries: None,
        }
    }
}

impl<K, V, SK, SV> Default for Map<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    /// The default map scalar: the empty map.
    fn default() -> Self {
        Self {
            data_type: yggdryl_dtype::Map::default(),
            entries: Some(Vec::new()),
        }
    }
}

impl<K, V, SK, SV> RawScalar<yggdryl_dtype::Map<K, V>> for Map<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    type Value = [(SK, SV)];

    fn data_type(&self) -> &yggdryl_dtype::Map<K, V> {
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
            return arrow_array::new_null_array(&RawDataType::to_arrow(&self.data_type), 1);
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
        let data_type = yggdryl_dtype::Map::from_arrow(arrow_array::Array::data_type(array))?;
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

impl<K, V, SK, SV> Scalar<[(SK, SV)]> for Map<K, V, SK, SV>
where
    K: RawDataType + Default,
    V: RawDataType + Default,
    SK: RawScalar<K>,
    SV: RawScalar<V>,
{
    type Type = yggdryl_dtype::Map<K, V>;
}

impl<TK, TV, K, V> DefaultScalar<Vec<(TK, TV)>> for yggdryl_dtype::Map<K, V>
where
    K: DefaultScalar<TK> + Default,
    V: DefaultScalar<TV> + Default,
    K::Scalar: RawScalar<K>,
    V::Scalar: RawScalar<V>,
{
    type Scalar = Map<K, V, K::Scalar, V::Scalar>;

    /// The default map scalar: the empty map.
    fn default_scalar(&self) -> Self::Scalar {
        Map::default()
    }
}
