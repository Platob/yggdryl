//! The [`Logical`] base trait: a type layered over a physical storage type.

use super::DataType;

/// The untyped surface every logical type carries: a type layered over a physical
/// storage type `S` — e.g. a timestamp stored as an `int64`, or an optional stored
/// as a union.
///
/// [`storage`](Logical::storage) returns the physical type the values are actually
/// laid out as; the logical type reinterprets those bytes. It is parameterised by
/// the storage type (rather than boxing it) so the concrete type is preserved for
/// zero-cost access, mirroring `yggdryl-field`'s `Field` and `yggdryl-scalar`'s
/// `Scalar`; a logical type whose values also have a native representation
/// implements the typed [`TypedLogical`](crate::TypedLogical).
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataError, DataType, Int64Type, Logical};
///
/// // A timestamp in microseconds, physically an int64.
/// #[derive(Debug)]
/// struct TimestampMicrosType {
///     storage: Int64Type,
/// }
///
/// impl DataType for TimestampMicrosType {
///     fn name(&self) -> &str { "timestamp[us]" }
///     fn arrow_format(&self) -> String { "tsu:".to_string() }
///     fn byte_width(&self) -> Option<usize> { Some(8) }
///     fn to_arrow(&self) -> arrow_schema::DataType {
///         arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None)
///     }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         match data_type {
///             arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None) => {
///                 Ok(TimestampMicrosType { storage: Int64Type })
///             }
///             other => Err(DataError::IncompatibleArrowType {
///                 expected: "Timestamp(Microsecond, None)".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// impl Logical<Int64Type> for TimestampMicrosType {
///     fn storage(&self) -> &Int64Type {
///         &self.storage
///     }
/// }
///
/// let ts = TimestampMicrosType { storage: Int64Type };
/// assert_eq!(ts.name(), "timestamp[us]");
/// assert_eq!(ts.storage().name(), "int64"); // reinterprets int64 bytes
/// ```
pub trait Logical<S: DataType>: DataType {
    /// The physical type this logical type's values are stored as.
    fn storage(&self) -> &S;
}
