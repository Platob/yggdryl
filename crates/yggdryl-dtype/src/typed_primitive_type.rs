//! [`TypedPrimitiveType<T>`] — the value-typed primitive category.

use crate::{PrimitiveType, TypedDataType};

/// A [`PrimitiveType`] that also exposes its value↔bytes codec
/// ([`TypedDataType<T>`](crate::TypedDataType)).
///
/// Blanket-implemented, so every concrete primitive type is automatically a
/// `TypedPrimitiveType` for its native `T` — the category and the value codec are two
/// facets of the same type, joined here without extra per-type code (mirroring the
/// core's `TypedCompressionEncoder`). Generic, so Rust-only.
///
/// ```
/// use yggdryl_dtype::{F64Type, TypedPrimitiveType};
///
/// fn round_trip<T: PartialEq + core::fmt::Debug, D: TypedPrimitiveType<T>>(dt: &D, value: T)
/// where
///     T: Copy,
/// {
///     let bytes = dt.value_to_bytes(value);
///     assert_eq!(dt.value_from_bytes(&bytes).unwrap(), value);
/// }
/// round_trip(&F64Type::new(), 1.5_f64);
/// ```
pub trait TypedPrimitiveType<T>: PrimitiveType + TypedDataType<T> {}

impl<T, D> TypedPrimitiveType<T> for D where D: PrimitiveType + TypedDataType<T> {}
