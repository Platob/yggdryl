//! [`TypedNestedScalar<DT, T>`] — the typed nested category (scaffolding).

use yggdryl_dtype::DataType;

use crate::{NestedScalar, TypedScalar};

/// A [`NestedScalar`] that also exposes its typed value
/// ([`TypedScalar<DT, T>`](crate::TypedScalar)).
///
/// Blanket-implemented, and **scaffolding** for now (no concrete nested scalars yet).
/// Kept for structural parallel with the primitive and logical categories. Rust-only.
///
/// ```
/// use yggdryl_scalar::TypedNestedScalar;
/// fn _accepts<DT: yggdryl_dtype::DataType, T, S: TypedNestedScalar<DT, T>>(_: &S) {}
/// ```
pub trait TypedNestedScalar<DT: DataType, T>: NestedScalar + TypedScalar<DT, T> {}

impl<DT: DataType, T, S> TypedNestedScalar<DT, T> for S where S: NestedScalar + TypedScalar<DT, T> {}
