//! The [`Logical`] category trait: a type layered over a physical storage type.

use super::RawDataType;

/// A logical type layered over a physical storage type — e.g. a timestamp stored as
/// an `int64`, or a date stored as an `int32`.
///
/// [`storage`](Logical::storage) returns the physical [`RawDataType`] the values are
/// actually laid out as; the logical type reinterprets those bytes. The physical type
/// is the associated [`Storage`](Logical::Storage), so it is preserved concretely.
///
/// ```
/// use yggdryl_data::{Int64, Logical, RawDataType};
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
/// }
///
/// impl Logical for TimestampMicros {
///     type Storage = Int64;
///     fn storage(&self) -> &Int64 {
///         &self.storage
///     }
/// }
///
/// let ts = TimestampMicros { storage: Int64 };
/// assert_eq!(ts.name(), "timestamp[us]");
/// assert_eq!(ts.storage().name(), "int64"); // reinterprets int64 bytes
/// ```
pub trait Logical: RawDataType {
    /// The physical storage type backing this logical type.
    type Storage: RawDataType;

    /// The physical type this logical type's values are stored as.
    fn storage(&self) -> &Self::Storage;
}
