//! The [`PrimitiveField`] category marker.

use crate::field::Field;

/// Marks a field whose type is primitive — no child fields (mirrors
/// [`PrimitiveType`](crate::PrimitiveType)).
///
/// ```
/// use yggdryl_schema::{Int32Field, PrimitiveField};
///
/// fn takes_primitive<F: PrimitiveField>(_f: &F) {}
/// takes_primitive(&Int32Field::new("x"));
/// ```
pub trait PrimitiveField: Field {}
