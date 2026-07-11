//! [`NestedField`] — the nested category of [`Field`] (scaffolding).

use crate::Field;

/// A field whose data type is nested ([`NestedType`](yggdryl_dtype::NestedType)).
///
/// The field-layer parallel of [`yggdryl_dtype::NestedType`], and **scaffolding** for
/// now — it establishes the category so future nested fields (list, struct, map) slot in
/// without reshaping the API. No concrete nested fields exist yet.
///
/// ```
/// use yggdryl_field::{Field, NestedField};
/// fn is_nullable<F: NestedField>(field: &F) -> bool {
///     field.is_nullable()
/// }
/// ```
pub trait NestedField: Field {}
