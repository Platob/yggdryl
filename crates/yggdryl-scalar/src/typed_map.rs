//! The statically-typed [`TypedMapScalar`] scalar of the
//! [`TypedMapType`](yggdryl_dtype::TypedMapType) data type.

use crate::scalar::{concat_scalar_arrays, scalars_from_elements};
use crate::{Scalar, ScalarFactory, TypedScalar};
use yggdryl_dtype::{DataError, DataType, Map, TypedMap, TypedMapType};

/// A single, possibly-null `map` value: a sequence of key–value entries of inner
/// scalars `SK` / `SV` over the key and value types `K` / `V`.
///
/// It is the statically-typed counterpart of the dynamic
/// [`MapScalar`](crate::MapScalar): its [`Value`](Scalar::Value) is the borrowed
/// slice `[(SK, SV)]`, so [`value`](Scalar::value) yields `Option<&[(SK, SV)]>`. The
/// Arrow form is a one-element `MapArray` whose entries struct concatenates the key
/// and value scalars' own Arrow forms; [`from_arrow`](Scalar::from_arrow) redirects
/// every key and value back through the inner scalars' `from_arrow`.
/// [`erase`](TypedMapScalar::erase) drops the static key and value types to a dynamic
/// [`MapScalar`](crate::MapScalar).
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type, UInt8Type};
/// use yggdryl_scalar::{Int64Scalar, Scalar, TypedMapScalar, UInt8Scalar};
///
/// let ranks = TypedMapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]).unwrap();
/// assert!(!ranks.is_null());
/// assert_eq!(ranks.value().map(<[_]>::len), Some(1));
/// assert_eq!(ranks.data_type().name(), "map");
///
/// // The Arrow round trip preserves the entries.
/// let arrow = ranks.to_arrow_scalar().into_inner();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(TypedMapScalar::from_arrow(arrow.as_ref()).unwrap(), ranks);
///
/// // Erase drops the static key and value types to a dynamic map.
/// assert_eq!(ranks.erase().len(), 1);
///
/// let missing: TypedMapScalar<UInt8Type, Int64Type, UInt8Scalar, Int64Scalar> =
///     TypedMapScalar::null();
/// assert!(missing.is_null());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedMapScalar<K, V, SK, SV> {
    data_type: TypedMapType<K, V>,
    entries: Option<Vec<(SK, SV)>>,
}

impl<K, V, SK, SV> TypedMapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<DataType = K>,
    SV: Scalar<DataType = V>,
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
            data_type: TypedMapType::default(),
            entries: Some(entries),
        })
    }

    /// The null map scalar.
    pub fn null() -> Self {
        Self {
            data_type: TypedMapType::default(),
            entries: None,
        }
    }

    /// Drop the static key and value types, returning the dynamic
    /// [`MapScalar`](crate::MapScalar) over the same entries assembled into one Arrow
    /// struct array (a null map erases to the null map).
    pub fn erase(&self) -> crate::MapScalar {
        crate::MapScalar::from_parts(
            self.data_type.erase(),
            self.entries_struct().map(crate::AnySerie::from_arrow),
        )
    }

    /// The entries assembled into their Arrow `"entries"` struct array (the `"key"`
    /// and `"value"` columns), or `None` when null — the shared assembly behind
    /// [`to_arrow_scalar`](Scalar::to_arrow_scalar) and [`erase`](TypedMapScalar::erase).
    fn entries_struct(&self) -> Option<arrow_array::ArrayRef> {
        self.entries_struct_limited(usize::MAX)
    }

    /// The first `limit` entries assembled into their `"entries"` struct array (or all
    /// of them when `limit` exceeds the count), or `None` when null. Display assembles
    /// only the first [`max_rows`](crate::DisplayOptions::max_rows) rows through this,
    /// so printing a huge typed map stays cheap rather than materializing every entry.
    fn entries_struct_limited(&self, limit: usize) -> Option<arrow_array::ArrayRef> {
        let entries = self.entries.as_ref()?;
        let shown = limit.min(entries.len());
        let entry_fields = self.data_type.entry_fields();
        let keys = concat_scalar_arrays(
            entries
                .iter()
                .take(shown)
                .map(|(key, _)| key.to_arrow_scalar().into_inner())
                .collect(),
            || entry_fields[0].data_type().clone(),
        );
        let values = concat_scalar_arrays(
            entries
                .iter()
                .take(shown)
                .map(|(_, value)| value.to_arrow_scalar().into_inner())
                .collect(),
            || entry_fields[1].data_type().clone(),
        );
        let entry_struct = arrow_array::StructArray::try_new_with_length(
            entry_fields,
            vec![keys, values],
            None,
            shown,
        )
        .expect("per-entry one-element arrays assemble into the entries struct");
        Some(std::sync::Arc::new(entry_struct))
    }
}

impl<K, V, SK, SV> Default for TypedMapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<DataType = K>,
    SV: Scalar<DataType = V>,
{
    /// The default map scalar: the empty map.
    fn default() -> Self {
        Self {
            data_type: TypedMapType::default(),
            entries: Some(Vec::new()),
        }
    }
}

impl<K, V, SK, SV> Scalar for TypedMapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<DataType = K>,
    SV: Scalar<DataType = V>,
{
    type DataType = TypedMapType<K, V>;
    type Value = [(SK, SV)];

    fn data_type(&self) -> &TypedMapType<K, V> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.entries.is_none()
    }

    fn value(&self) -> Option<&[(SK, SV)]> {
        self.entries.as_deref()
    }

    // A `key | value` table. Only the first `max_rows` entries are assembled — never
    // the whole map (which `erase` would do) — so printing a huge typed map stays
    // cheap; the true entry count still drives the `… (N more)` footer.
    fn display_with(&self, options: crate::DisplayOptions) -> String {
        match self.entries_struct_limited(options.max_rows) {
            None => "null".to_string(),
            Some(head) => crate::display::render_serie_with_total(
                &crate::AnySerie::from_arrow(head),
                "entries",
                self.entries.as_ref().map_or(0, Vec::len),
                options,
            ),
        }
    }

    fn to_arrow_scalar(&self) -> arrow_array::Scalar<arrow_array::ArrayRef> {
        let Some(entries) = self.entries_struct() else {
            return arrow_array::Scalar::new(arrow_array::new_null_array(
                &DataType::to_arrow(&self.data_type),
                1,
            ));
        };
        // The assembled entries struct is shared into the one-element map — a
        // reference-count bump, not a copy.
        let entry_struct = entries
            .as_ref()
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .expect("entries_struct builds the entries struct array");
        let array = arrow_array::MapArray::try_new(
            self.data_type.entries_field(),
            arrow_buffer::OffsetBuffer::from_lengths([arrow_array::Array::len(entry_struct)]),
            entry_struct.clone(),
            None,
            false,
        )
        .expect("a one-element map of the declared entries struct is valid");
        arrow_array::Scalar::new(std::sync::Arc::new(array))
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout and redirects the key and value
        // children to `K` / `V`; then every entry redirects to the inner scalars.
        let data_type = TypedMapType::from_arrow(arrow_array::Array::data_type(array))?;
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

    fn as_map(&self) -> Result<crate::MapScalar, DataError> {
        Ok(self.erase())
    }
}

impl<K, V, SK, SV> crate::NestedSerie for TypedMapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<DataType = K>,
    SV: Scalar<DataType = V>,
{
    fn child_serie_count(&self) -> usize {
        1
    }

    // The entries struct is assembled on demand from the native entries (the typed
    // map stores the scalars themselves — the most decomposed form).
    fn child_serie_at(&self, index: usize) -> Option<crate::AnySerie> {
        (index == 0)
            .then(|| self.entries_struct().map(crate::AnySerie::from_arrow))
            .flatten()
    }

    fn child_serie_name_at(&self, index: usize) -> Option<String> {
        (index == 0).then(|| "entries".to_string())
    }

    fn child_serie_by(&self, name: &str) -> Option<crate::AnySerie> {
        match name {
            "entries" => self.child_serie_at(0),
            // The key / value projections redirect through the erased map.
            "key" | "value" => self.erase().child_serie_by(name),
            _ => None,
        }
    }
}

impl<K, V, SK, SV> TypedScalar<TypedMapType<K, V>, [(SK, SV)], arrow_array::MapArray>
    for TypedMapScalar<K, V, SK, SV>
where
    K: DataType + Default,
    V: DataType + Default,
    SK: Scalar<DataType = K>,
    SV: Scalar<DataType = V>,
{
}

impl<TK, TV, K, V> ScalarFactory<Vec<(TK, TV)>> for TypedMapType<K, V>
where
    K: ScalarFactory<TK> + Default,
    V: ScalarFactory<TV> + Default,
    K::Scalar: Scalar<DataType = K>,
    V::Scalar: Scalar<DataType = V>,
{
    type Scalar = TypedMapScalar<K, V, K::Scalar, V::Scalar>;

    /// A map scalar holding the native `entries`, each key and value converted
    /// through its own scalar factory (map keys are never null).
    fn scalar(&self, entries: Vec<(TK, TV)>) -> Self::Scalar {
        TypedMapScalar::new(
            entries
                .into_iter()
                .map(|(key, value)| (self.key_type().scalar(key), self.value_type().scalar(value)))
                .collect(),
        )
        .expect("factory-built key scalars are never null")
    }

    /// The null map scalar.
    fn null_scalar(&self) -> Self::Scalar {
        TypedMapScalar::null()
    }

    /// The default map scalar: the empty map.
    fn default_scalar(&self) -> Self::Scalar {
        TypedMapScalar::default()
    }
}
