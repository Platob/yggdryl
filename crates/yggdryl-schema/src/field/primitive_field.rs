//! The [`PrimitiveField`] category marker.

use crate::field::Field;

/// Marks a field whose type is primitive — no child fields (mirrors
/// [`PrimitiveType`](crate::PrimitiveType)).
///
/// ```
/// use yggdryl_schema::{BinaryField, PrimitiveField};
///
/// fn takes_primitive<F: PrimitiveField>(_f: &F) {}
/// takes_primitive(&BinaryField::new("x"));
/// ```
pub trait PrimitiveField: Field {}
