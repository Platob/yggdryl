//! The typed [`TypedStruct`] trait: a statically-shaped [`Struct`](super::Struct)
//! with a native row type.

use super::Struct;
use crate::TypedDataType;

/// A statically-shaped struct whose rows have the native type `T` — the typed
/// layer over [`Struct`].
///
/// The dynamic [`StructType`](crate::StructType), whose children are only known at
/// runtime, stays untyped; a struct with a fixed shape carries its row as a plain
/// Rust value (typically a tuple) and implements [`TypedDataType<T>`] over it — the
/// codec concatenates the child codecs, and the default row is the children's
/// defaults.
///
/// ```
/// use yggdryl_dtype::{
///     arrow_schema, DataError, DataType, Int64Type, Nested, Struct, StructType, TypedDataType,
///     TypedStruct,
/// };
///
/// // A static point struct: two non-null int64 children, row type (i64, i64).
/// #[derive(Debug, Default)]
/// struct PointType {
///     coordinate: Int64Type,
/// }
///
/// impl PointType {
///     fn shape() -> StructType {
///         StructType::new(arrow_schema::Fields::from(vec![
///             arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
///             arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
///         ]))
///     }
/// }
///
/// impl DataType for PointType {
///     fn name(&self) -> &str { "struct" }
///     fn arrow_format(&self) -> String { "+s".to_string() }
///     fn byte_width(&self) -> Option<usize> { Some(16) } // two fixed-width children
///     fn to_arrow(&self) -> arrow_schema::DataType { Self::shape().to_arrow() }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         (data_type == &Self::shape().to_arrow())
///             .then(Self::default)
///             .ok_or_else(|| DataError::IncompatibleArrowType {
///                 expected: "a struct of int64 x and y".to_string(),
///                 got: data_type.to_string(),
///             })
///     }
/// }
///
/// impl Nested for PointType {
///     fn child_count(&self) -> usize { 2 }
/// }
///
/// impl Struct for PointType {
///     fn fields(&self) -> &arrow_schema::Fields {
///         static FIELDS: std::sync::OnceLock<arrow_schema::Fields> = std::sync::OnceLock::new();
///         FIELDS.get_or_init(|| match PointType::shape().to_arrow() {
///             arrow_schema::DataType::Struct(fields) => fields,
///             _ => unreachable!(),
///         })
///     }
/// }
///
/// // The typed layer: the row is (x, y), the codec concatenates the children.
/// impl TypedDataType<(i64, i64)> for PointType {
///     fn native_to_bytes(&self, (x, y): &(i64, i64)) -> Vec<u8> {
///         let mut bytes = self.coordinate.native_to_bytes(x);
///         bytes.extend(self.coordinate.native_to_bytes(y));
///         bytes
///     }
///     fn native_from_bytes(&self, bytes: &[u8]) -> Result<(i64, i64), DataError> {
///         if bytes.len() != 16 {
///             return Err(DataError::InvalidByteLength { expected: 16, got: bytes.len() });
///         }
///         Ok((
///             self.coordinate.native_from_bytes(&bytes[..8])?,
///             self.coordinate.native_from_bytes(&bytes[8..])?,
///         ))
///     }
///     fn default_value(&self) -> (i64, i64) {
///         (self.coordinate.default_value(), self.coordinate.default_value())
///     }
/// }
///
/// impl TypedStruct<(i64, i64)> for PointType {}
///
/// let point = PointType::default();
/// assert_eq!(point.default_value(), (0, 0));
/// assert_eq!(point.native_from_bytes(&point.native_to_bytes(&(1, 2))).unwrap(), (1, 2));
/// ```
pub trait TypedStruct<T>: Struct + TypedDataType<T> {}
