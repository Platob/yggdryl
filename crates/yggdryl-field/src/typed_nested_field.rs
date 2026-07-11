//! [`TypedNestedField<DT, T>`] — the typed nested category (scaffolding).

use yggdryl_dtype::DataType;

use crate::{NestedField, TypedField};

/// A [`NestedField`] that also exposes its concrete typed data type
/// ([`TypedField<DT, T>`](crate::TypedField)).
///
/// Blanket-implemented, and **scaffolding** for now (no concrete nested fields yet).
/// Kept for structural parallel with the primitive and logical categories. Rust-only.
///
/// ```
/// use yggdryl_field::TypedNestedField;
/// fn _accepts<DT: yggdryl_dtype::DataType, T, F: TypedNestedField<DT, T>>(_: &F) {}
/// ```
pub trait TypedNestedField<DT: DataType, T>: NestedField + TypedField<DT, T> {}

impl<DT: DataType, T, F> TypedNestedField<DT, T> for F where F: NestedField + TypedField<DT, T> {}
