//! [`TypedLogicalScalar<DT, T>`] — the typed logical category (scaffolding).

use yggdryl_dtype::DataType;

use crate::{LogicalScalar, TypedScalar};

/// A [`LogicalScalar`] that also exposes its typed value
/// ([`TypedScalar<DT, T>`](crate::TypedScalar)).
///
/// Blanket-implemented, and **scaffolding** for now (no concrete logical scalars yet).
/// Generic, so Rust-only.
///
/// ```
/// use yggdryl_scalar::TypedLogicalScalar;
/// fn _accepts<DT: yggdryl_dtype::DataType, T, S: TypedLogicalScalar<DT, T>>(_: &S) {}
/// ```
pub trait TypedLogicalScalar<DT: DataType, T>: LogicalScalar + TypedScalar<DT, T> {}

impl<DT: DataType, T, S> TypedLogicalScalar<DT, T> for S where S: LogicalScalar + TypedScalar<DT, T> {}
