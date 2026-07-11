//! [`TypedLogicalType<T>`] — the value-typed logical category (scaffolding).

use crate::{LogicalType, TypedDataType};

/// A [`LogicalType`] that also exposes its value↔bytes codec
/// ([`TypedDataType<T>`](crate::TypedDataType)) over its physical native `T`.
///
/// Blanket-implemented like [`TypedPrimitiveType`](crate::TypedPrimitiveType), and
/// **scaffolding** for now (no concrete logical types yet). Generic, so Rust-only.
///
/// ```
/// // Establishes the value-typed logical bound for future concrete types.
/// use yggdryl_dtype::TypedLogicalType;
/// fn _accepts<T, L: TypedLogicalType<T>>(_: &L) {}
/// ```
pub trait TypedLogicalType<T>: LogicalType + TypedDataType<T> {}

impl<T, L> TypedLogicalType<T> for L where L: LogicalType + TypedDataType<T> {}
