//! [`TypedPrimitiveField<DT, T>`] — the typed primitive category.

use yggdryl_dtype::DataType;

use crate::{PrimitiveField, TypedField};

/// A [`PrimitiveField`] that also exposes its concrete typed data type
/// ([`TypedField<DT, T>`](crate::TypedField)).
///
/// Blanket-implemented, so every concrete primitive field is automatically a
/// `TypedPrimitiveField` for its `(DT, T)` — mirroring
/// [`TypedPrimitiveType`](yggdryl_dtype::TypedPrimitiveType). Generic, so Rust-only.
///
/// ```
/// use yggdryl_dtype::I64Type;
/// use yggdryl_field::{I64Field, TypedPrimitiveField};
/// fn _accepts<DT: yggdryl_dtype::DataType, T, F: TypedPrimitiveField<DT, T>>(_: &F) {}
/// _accepts::<I64Type, i64, _>(&I64Field::new("id", false));
/// ```
pub trait TypedPrimitiveField<DT: DataType, T>: PrimitiveField + TypedField<DT, T> {}

impl<DT: DataType, T, F> TypedPrimitiveField<DT, T> for F where F: PrimitiveField + TypedField<DT, T>
{}
