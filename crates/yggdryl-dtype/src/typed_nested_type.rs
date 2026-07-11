//! [`TypedNestedType<T>`] — the value-typed nested category (scaffolding).

use crate::{NestedType, TypedDataType};

/// A [`NestedType`] that also exposes a value↔bytes codec
/// ([`TypedDataType<T>`](crate::TypedDataType)) — where `T` is the nested type's Rust
/// value shape (e.g. a `Vec` of the child's native type).
///
/// Blanket-implemented, and **scaffolding** for now (no concrete nested types yet).
/// Generic, so Rust-only. Kept for structural parallel with the primitive and logical
/// categories so all three read identically.
///
/// ```
/// use yggdryl_dtype::TypedNestedType;
/// fn _accepts<T, N: TypedNestedType<T>>(_: &N) {}
/// ```
pub trait TypedNestedType<T>: NestedType + TypedDataType<T> {}

impl<T, N> TypedNestedType<T> for N where N: NestedType + TypedDataType<T> {}
