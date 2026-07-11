//! [`LogicalScalar`] — the logical category of [`Scalar`] (scaffolding).

use crate::Scalar;

/// A scalar whose data type is logical ([`LogicalType`](yggdryl_dtype::LogicalType)).
///
/// The scalar-layer parallel of [`yggdryl_dtype::LogicalType`], and **scaffolding** for
/// now — it establishes the category so future logical scalars (timestamp, decimal) slot
/// in without reshaping the API. No concrete logical scalars exist yet.
///
/// ```
/// use yggdryl_scalar::{LogicalScalar, Scalar};
/// fn to_bytes<S: LogicalScalar>(scalar: &S) -> Vec<u8> {
///     scalar.serialize_bytes()
/// }
/// ```
pub trait LogicalScalar: Scalar {}
