//! The dynamic [`MapScalar`] scalar of the [`MapType`](yggdryl_dtype::MapType) data
//! type.

use crate::{AnySerie, Scalar};
use arrow_array::ArrayRef;
use yggdryl_dtype::{DataError, DataType, MapType};

/// A single, possibly-null `map` value with its key and value types erased: a
/// sequence of key–value entries held as the crate's own [`AnySerie`] over the
/// `"entries"` struct — a serie of struct entries — carrying a dynamic
/// [`MapType`](yggdryl_dtype::MapType).
///
/// It is the untyped base of the statically-typed
/// [`TypedMapScalar<K, V, SK, SV>`](crate::TypedMapScalar): it implements only the
/// base [`Scalar`] surface ([`to_arrow_scalar`](Scalar::to_arrow_scalar) /
/// [`from_arrow`](Scalar::from_arrow), all reference-count bumps — the Arrow map is
/// reconstituted on demand and decomposed on the way in) plus
/// [`len`](MapScalar::len) / [`is_empty`](MapScalar::is_empty) and the
/// [`NestedSerie`](crate::NestedSerie) child access (the `"entries"` child, with
/// `"key"` / `"value"` projections by name), since the entry scalar types are
/// erased — the per-entry native accessors and the [`TypedScalar`](crate::TypedScalar)
/// surface live on `TypedMapScalar<K, V, SK, SV>`, which
/// [`erase`](crate::TypedMapScalar::erase)s back to this type.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::DataType;
/// use yggdryl_scalar::{Int64Scalar, NestedSerie, Scalar, TypedMapScalar, UInt8Scalar};
///
/// // A dynamic map is reached by erasing a typed one, or from Arrow.
/// let ranks =
///     TypedMapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]).unwrap().erase();
/// assert!(!ranks.is_null());
/// assert_eq!(ranks.len(), 1);
/// assert_eq!(ranks.data_type().name(), "map");
/// assert_eq!(ranks.child_serie_by("key").unwrap().len(), 1); // the keys projection
/// assert_eq!(
///     yggdryl_scalar::MapScalar::from_arrow(ranks.to_arrow_scalar().as_ref()).unwrap(),
///     ranks
/// );
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MapScalar {
    data_type: MapType,
    entries: Option<AnySerie>,
}

impl Eq for MapScalar {}

impl MapScalar {
    /// A dynamic map over an already-built entries serie (shared zero-copy) of the
    /// given dynamic `data_type`, or the null map for `None`.
    pub(crate) fn from_parts(data_type: MapType, entries: Option<AnySerie>) -> Self {
        Self { data_type, entries }
    }

    /// The number of entries, `0` when null or empty ([`is_null`](Scalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.entries.as_ref().map_or(0, AnySerie::len)
    }

    /// Whether the map holds no entries (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `"key"` (column 0) or `"value"` (column 1) projection of the entries
    /// struct, decomposed into its own serie — behind the by-name child access.
    fn project(&self, column: usize) -> Option<AnySerie> {
        let entries = self.entries.as_ref()?.to_arrow();
        let entries = entries
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .expect("a dynamic map's entries are the entries struct array");
        Some(AnySerie::from_arrow(entries.column(column).clone()))
    }
}

impl crate::NestedSerie for MapScalar {
    fn child_serie_count(&self) -> usize {
        1
    }

    fn child_serie_at(&self, index: usize) -> Option<AnySerie> {
        (index == 0).then(|| self.entries.clone()).flatten()
    }

    fn child_serie_name_at(&self, index: usize) -> Option<String> {
        (index == 0).then(|| "entries".to_string())
    }

    // Beyond the single "entries" child, the key and value columns are reachable
    // as named projections.
    fn child_serie_by(&self, name: &str) -> Option<AnySerie> {
        match name {
            "entries" => self.entries.clone(),
            "key" => self.project(0),
            "value" => self.project(1),
            _ => None,
        }
    }
}

impl Scalar for MapScalar {
    type DataType = MapType;
    type Value = AnySerie;

    fn data_type(&self) -> &MapType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.entries.is_none()
    }

    fn value(&self) -> Option<&AnySerie> {
        self.entries.as_ref()
    }

    // The entries as a `key | value` table (the entries column is a struct), or `null`.
    fn display_with(&self, options: crate::DisplayOptions) -> String {
        match &self.entries {
            None => "null".to_string(),
            Some(entries) => crate::display::render_serie(entries, "entries", options),
        }
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let Some(entries) = &self.entries else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The entries serie is reconstituted into the one-element map — a
        // reference-count bump, not a copy.
        let entries = entries.to_arrow();
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
        // The data type validates the layout; the entries struct is decomposed into
        // the crate's own serie, shared zero-copy.
        let data_type = MapType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::MapArray>()
            .expect("a value with a map data type is a map array");
        let entries = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(AnySerie::from_arrow(
                std::sync::Arc::new(array.value(0)) as ArrayRef
            ))
        };
        Ok(Self { data_type, entries })
    }

    fn as_map(&self) -> Result<MapScalar, DataError> {
        Ok(self.clone())
    }
}
