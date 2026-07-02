//! The [`RawLogical`] base trait: a type layered over a physical storage type.

use super::RawDataType;

/// The untyped surface every logical type carries: a type layered over a physical
/// storage type `S` — e.g. a timestamp stored as an `int64`, or an optional stored
/// as a union.
///
/// [`storage`](RawLogical::storage) returns the physical type the values are
/// actually laid out as; the logical type reinterprets those bytes. It is
/// parameterised by the storage type (rather than boxing it) so the concrete type
/// is preserved for zero-cost access, mirroring [`RawField`](crate::RawField) and
/// [`RawScalar`](crate::RawScalar); a logical type whose values also have a native
/// representation implements the typed [`Logical`](crate::Logical).
///
/// ```
/// use yggdryl_data::{arrow_schema, DataError, Int64, RawDataType, RawLogical};
///
/// // A timestamp in microseconds, physically an int64.
/// #[derive(Debug)]
/// struct TimestampMicros {
///     storage: Int64,
/// }
///
/// impl RawDataType for TimestampMicros {
///     fn name(&self) -> &str { "timestamp[us]" }
///     fn arrow_format(&self) -> String { "tsu:".to_string() }
///     fn byte_width(&self) -> Option<usize> { Some(8) }
///     fn to_arrow(&self) -> arrow_schema::DataType {
///         arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None)
///     }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         match data_type {
///             arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None) => {
///                 Ok(TimestampMicros { storage: Int64 })
///             }
///             other => Err(DataError::IncompatibleArrowType {
///                 expected: "Timestamp(Microsecond, None)".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// impl RawLogical<Int64> for TimestampMicros {
///     fn storage(&self) -> &Int64 {
///         &self.storage
///     }
/// }
///
/// let ts = TimestampMicros { storage: Int64 };
/// assert_eq!(ts.name(), "timestamp[us]");
/// assert_eq!(ts.storage().name(), "int64"); // reinterprets int64 bytes
/// ```
pub trait RawLogical<S: RawDataType>: RawDataType {
    /// The physical type this logical type's values are stored as.
    fn storage(&self) -> &S;
}
