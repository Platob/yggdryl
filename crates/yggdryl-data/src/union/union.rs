//! The typed [`Union`] trait: a statically-shaped [`RawUnion`](super::RawUnion)
//! with a first data type.

use super::RawUnion;
use crate::DataType;

/// A statically-shaped union whose *first* variant has the native type `T` — the
/// typed layer over [`RawUnion`].
///
/// The first data type anchors the union's typed surface: per convention, a typed
/// union's [`default_value`](DataType::default_value) and
/// [`default_scalar`](DataType::default_scalar) are *the first data type's
/// defaults*, and its byte codec is the first data type's codec. The dynamic
/// [`UnionType`](crate::UnionType), whose children are only known at runtime, stays
/// raw-only.
///
/// ```
/// use yggdryl_data::{
///     arrow_schema, DataError, DataType, Int64, Int64Scalar, Nested, RawDataType, RawUnion,
///     Union, UnionType,
/// };
///
/// // A static two-variant union: an int64 (first), or a uint8 tag.
/// #[derive(Debug, Default)]
/// struct NumberOrTag {
///     first: Int64,
/// }
///
/// impl NumberOrTag {
///     fn storage() -> UnionType {
///         UnionType::new(
///             arrow_schema::UnionFields::try_new(
///                 [0, 1],
///                 [
///                     arrow_schema::Field::new("number", arrow_schema::DataType::Int64, false),
///                     arrow_schema::Field::new("tag", arrow_schema::DataType::UInt8, false),
///                 ],
///             )
///             .unwrap(),
///             arrow_schema::UnionMode::Sparse,
///         )
///     }
/// }
///
/// impl RawDataType for NumberOrTag {
///     fn name(&self) -> &str { "union" }
///     fn arrow_format(&self) -> String { Self::storage().arrow_format() }
///     fn byte_width(&self) -> Option<usize> { None }
///     fn to_arrow(&self) -> arrow_schema::DataType { Self::storage().to_arrow() }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         (data_type == &Self::storage().to_arrow())
///             .then(Self::default)
///             .ok_or_else(|| DataError::IncompatibleArrowType {
///                 expected: "a sparse union of int64 and uint8".to_string(),
///                 got: data_type.to_string(),
///             })
///     }
/// }
///
/// impl Nested for NumberOrTag {
///     fn child_count(&self) -> usize { 2 }
/// }
///
/// impl RawUnion for NumberOrTag {
///     fn fields(&self) -> &arrow_schema::UnionFields {
///         static FIELDS: std::sync::OnceLock<arrow_schema::UnionFields> =
///             std::sync::OnceLock::new();
///         FIELDS.get_or_init(|| Self::storage().fields().clone())
///     }
///     fn mode(&self) -> arrow_schema::UnionMode { arrow_schema::UnionMode::Sparse }
/// }
///
/// // The typed layer: codec and defaults come from the FIRST data type.
/// impl DataType<i64> for NumberOrTag {
///     type Scalar = Int64Scalar;
///     fn native_to_bytes(&self, value: &i64) -> Vec<u8> { self.first.native_to_bytes(value) }
///     fn native_from_bytes(&self, bytes: &[u8]) -> Result<i64, DataError> {
///         self.first.native_from_bytes(bytes)
///     }
///     fn default_value(&self) -> i64 { self.first.default_value() }
///     fn default_scalar(&self) -> Int64Scalar { self.first.default_scalar() }
/// }
///
/// impl Union<i64> for NumberOrTag {
///     type First = Int64;
///     fn first_type(&self) -> &Int64 { &self.first }
/// }
///
/// let union = NumberOrTag::default();
/// assert_eq!(union.first_type().name(), "int64");
/// assert_eq!(union.default_value(), 0); // the first data type's default
/// ```
pub trait Union<T>: RawUnion + DataType<T> {
    /// The union's first data type, whose native type is `T`.
    type First: DataType<T>;

    /// The first variant's data type — the union's defaults are its defaults.
    fn first_type(&self) -> &Self::First;
}
