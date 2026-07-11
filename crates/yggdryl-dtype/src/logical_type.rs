//! [`LogicalType`] — the logical category of [`DataType`] (scaffolding).

use crate::DataType;

/// A logical data type — a semantic type layered over an underlying physical
/// representation (a timestamp stored as `int64`, a decimal as a fixed-width integer,
/// and so on).
///
/// This category trait is **scaffolding**: it establishes the logical layer of the
/// hierarchy so future concrete types (timestamps, durations, decimals) slot in beside
/// the primitives without reshaping the API. No concrete logical types exist yet.
///
/// ```
/// // A logical type is a `DataType`, so generic code can bound on it today.
/// use yggdryl_dtype::{DataType, LogicalType};
/// fn arrow_of<L: LogicalType>(logical: &L) -> arrow_schema::DataType {
///     logical.to_arrow()
/// }
/// ```
pub trait LogicalType: DataType {}
