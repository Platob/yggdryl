//! The [`MapScalar`] scalar of the [`MapType`](yggdryl_dtype::MapType) data type.

use crate::scalar::{concat_scalar_arrays, scalars_from_elements};
use crate::{Scalar, ScalarFactory, TypedScalar};
use yggdryl_dtype::{DataError, DataType, Map, MapType};

/// A single, possibly-null `map` value: a sequence of key–value entries of inner
/// scalars `SK` / `SV` over the key and value types `K` / `V`.
///
/// Its [`Value`](Scalar::Value) is the borrowed slice `[(SK, SV)]`, so
/// [`value`](Scalar::value) yields `Option<&[(SK, SV)]>`. The Arrow form is a
/// one-element `MapArray` whose entries struct concatenates the key and value
/// scalars' own Arrow forms; [`from_arrow`](Scalar::from_arrow) redirects every key
/// and value back through the inner scalars' `from_arrow`.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type, UInt8Type};
/// use yggdryl_scalar::{Int64Scalar, MapScalar, Scalar, UInt8Scalar};
///
/// let ranks = MapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]).unwrap();
/// assert!(!ranks.is_null());
/// assert_eq!(ranks.value().map(<[_]>::len), Some(1));
/// assert_eq!(ranks.data_type().name(), "map");
///
/// // The Arrow round trip preserves the entries.
/// let arrow = ranks.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(MapScalar::from_arrow(arrow.as_ref()).unwrap(), ranks);
///
/// let missing: MapScalar<UInt8Type, Int64Type, UInt8Scalar, Int64Scalar> = MapScalar::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapScalar<K, V, SK, SV> {
    data_type: MapType<K, V>,
    entries: Option<Vec<(SK, SV)>>,
}

impl<K, V, SK, SV> MapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<K>,
    SV: Scalar<V>,
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
            data_type: MapType::default(),
            entries: Some(entries),
        })
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
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<K>,
    SV: Scalar<V>,
{
    /// The default map scalar: the empty map.
    fn default() -> Self {
        Self {
            data_type: MapType::default(),
            entries: Some(Vec::new()),
        }
    }
}

impl<K, V, SK, SV> Scalar<MapType<K, V>> for MapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<K>,
    SV: Scalar<V>,
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
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        let entry_fields = self.data_type.entry_fields();
        let keys = concat_scalar_arrays(
            entries.iter().map(|(key, _)| key.to_arrow()).collect(),
            || entry_fields[0].data_type().clone(),
        );
        let values = concat_scalar_arrays(
            entries.iter().map(|(_, value)| value.to_arrow()).collect(),
            || entry_fields[1].data_type().clone(),
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

impl<K, V, SK, SV> TypedScalar<MapType<K, V>, [(SK, SV)]> for MapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<K>,
    SV: Scalar<V>,
{
}

impl<TK, TV, K, V> ScalarFactory<Vec<(TK, TV)>> for MapType<K, V>
where
    K: ScalarFactory<TK> + Default,
    V: ScalarFactory<TV> + Default,
    K::Scalar: Scalar<K>,
    V::Scalar: Scalar<V>,
{
    type Scalar = MapScalar<K, V, K::Scalar, V::Scalar>;

    /// A map scalar holding the native `entries`, each key and value converted
    /// through its own scalar factory (map keys are never null).
    fn scalar(&self, entries: Vec<(TK, TV)>) -> Self::Scalar {
        MapScalar::new(
            entries
                .into_iter()
                .map(|(key, value)| (self.key_type().scalar(key), self.value_type().scalar(value)))
                .collect(),
        )
        .expect("factory-built key scalars are never null")
    }

    /// The null map scalar.
    fn null_scalar(&self) -> Self::Scalar {
        MapScalar::null()
    }

    /// The default map scalar: the empty map.
    fn default_scalar(&self) -> Self::Scalar {
        MapScalar::default()
    }
}
