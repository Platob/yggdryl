//! [`TypedLogicalField<DT, T>`] — the typed logical category (scaffolding).

use yggdryl_dtype::DataType;

use crate::{LogicalField, TypedField};

/// A [`LogicalField`] that also exposes its concrete typed data type
/// ([`TypedField<DT, T>`](crate::TypedField)).
///
/// Blanket-implemented, and **scaffolding** for now (no concrete logical fields yet).
/// Generic, so Rust-only.
///
/// ```
/// use yggdryl_field::TypedLogicalField;
/// fn _accepts<DT: yggdryl_dtype::DataType, T, F: TypedLogicalField<DT, T>>(_: &F) {}
/// ```
pub trait TypedLogicalField<DT: DataType, T>: LogicalField + TypedField<DT, T> {}

impl<DT: DataType, T, F> TypedLogicalField<DT, T> for F where F: LogicalField + TypedField<DT, T> {}
