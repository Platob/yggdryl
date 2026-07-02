//! The base trait every yggdryl data type implements.

use core::fmt::{Debug, Display};
use core::hash::Hash;
use std::collections::BTreeMap;

use arrow_schema::DataType as ArrowDataType;

use crate::{metadata, DataTypeError, DataTypeId};

/// A yggdryl data type: the typed description of a value's physical layout
/// and semantics.
///
/// Every concrete type — one per file, grouped one module per category —
/// implements this trait; category subtraits ([`PrimitiveType`],
/// [`LogicalType`], [`NestedType`]) refine it. The Arrow mapping is total and
/// reversible for the supported subset: `from_arrow` is the only inbound
/// conversion and validates fully, and `to_arrow` always round-trips back.
///
/// The trait is deliberately not object safe (`from_arrow` and `from_bytes`
/// are constructors); heterogeneous collections hold the erased
/// [`AnyDataType`](crate::AnyDataType) instead.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeError, Int8};
///
/// let arrow = Int8.to_arrow();
/// assert_eq!(Int8::from_arrow(&arrow), Ok(Int8));
/// assert!(matches!(
///     Int8::from_arrow(&arrow_schema::DataType::Utf8),
///     Err(DataTypeError::ArrowTypeMismatch { .. })
/// ));
/// ```
///
/// [`PrimitiveType`]: crate::PrimitiveType
/// [`LogicalType`]: crate::LogicalType
/// [`NestedType`]: crate::NestedType
pub trait DataType: Clone + Debug + Display + Eq + Hash + Send + Sync + Sized + 'static {
    /// The identifier of this value's type constructor, shared by every
    /// parameterization (a method rather than a constant so erased types like
    /// [`AnyDataType`](crate::AnyDataType) can implement it per value).
    fn type_id(&self) -> DataTypeId;

    /// The Arrow data type this type maps to.
    fn to_arrow(&self) -> ArrowDataType;

    /// The `ygg.*` field metadata restoring semantics Arrow's type system
    /// cannot express (see [`metadata`]); empty when the Arrow type alone is
    /// lossless. [`Field::to_arrow`](crate::Field::to_arrow) merges it into
    /// the Arrow field's metadata.
    fn arrow_metadata(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// Validates and converts an Arrow data type back into this type — the
    /// only inbound conversion. Types that anchor on a physical type reject
    /// the anchor here and are restored through
    /// [`from_arrow_parts`](DataType::from_arrow_parts), which sees the
    /// field metadata.
    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError>;

    /// Validates and converts an Arrow data type plus its field metadata.
    /// The default rejects any unknown `ygg.*` key and delegates to
    /// [`from_arrow`](DataType::from_arrow); anchored types override it to
    /// consume their keys.
    fn from_arrow_parts(
        data_type: &ArrowDataType,
        metadata_map: &BTreeMap<String, String>,
    ) -> Result<Self, DataTypeError> {
        if let Some(key) = metadata_map
            .keys()
            .find(|key| key.starts_with(metadata::PREFIX))
        {
            return Err(DataTypeError::UnknownMetadata { key: key.clone() });
        }
        Self::from_arrow(data_type)
    }

    /// Serializes the type to its canonical byte encoding.
    fn to_bytes(&self) -> Vec<u8>;

    /// Deserializes the type from the encoding produced by
    /// [`to_bytes`](DataType::to_bytes), validating fully.
    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError>;
}
