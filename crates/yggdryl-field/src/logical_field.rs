//! [`LogicalField`] — the logical category of [`Field`] (scaffolding).

use crate::Field;

/// A field whose data type is logical ([`LogicalType`](yggdryl_dtype::LogicalType)).
///
/// The field-layer parallel of [`yggdryl_dtype::LogicalType`], and **scaffolding** for
/// now — it establishes the category so future logical fields (timestamp, decimal) slot
/// in without reshaping the API. No concrete logical fields exist yet.
///
/// ```
/// use yggdryl_field::{Field, LogicalField};
/// fn name_of<F: LogicalField>(field: &F) -> &str {
///     field.name()
/// }
/// ```
pub trait LogicalField: Field {}
