//! [`NestedScalar`] — the nested category of [`Scalar`] (scaffolding).

use crate::Scalar;

/// A scalar whose data type is nested ([`NestedType`](yggdryl_dtype::NestedType)).
///
/// The scalar-layer parallel of [`yggdryl_dtype::NestedType`], and **scaffolding** for
/// now — it establishes the category so future nested scalars (list, struct, map values)
/// slot in without reshaping the API. No concrete nested scalars exist yet.
///
/// ```
/// use yggdryl_scalar::{NestedScalar, Scalar};
/// fn is_null<S: NestedScalar>(scalar: &S) -> bool {
///     scalar.is_null()
/// }
/// ```
pub trait NestedScalar: Scalar {}
