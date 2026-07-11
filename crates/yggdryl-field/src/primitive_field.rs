//! [`PrimitiveField`] — the primitive category of [`Field`].

use crate::Field;

/// A field whose data type is a primitive
/// ([`PrimitiveType`](yggdryl_dtype::PrimitiveType)) — the category of the ten native
/// numeric fields plus `boolean`.
///
/// The field-layer parallel of [`yggdryl_dtype::PrimitiveType`]; marker for now, so
/// generic code can bound on "a primitive field" independently of its concrete type.
///
/// ```
/// use yggdryl_field::{I64Field, PrimitiveField};
/// fn is_primitive<F: PrimitiveField>(_: &F) {}
/// is_primitive(&I64Field::new("id", false));
/// ```
pub trait PrimitiveField: Field {}
