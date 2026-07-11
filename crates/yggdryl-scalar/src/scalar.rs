//! [`Scalar`] — the base Arrow scalar contract.

use arrow_schema::DataType as ArrowDataType;

/// A **scalar** in the Apache Arrow model — a single, possibly-null value of a data
/// type; the FFI-opaque base of the scalar hierarchy.
///
/// Every scalar reports whether it [`is_null`](Scalar::is_null), the Arrow
/// [`data type`](Scalar::arrow_data_type) of its value, and serialises to bytes via
/// [`serialize_bytes`](Scalar::serialize_bytes) (rule 5) — a 1-byte null flag followed
/// by the value's little-endian bytes when present. The trait is object-safe and
/// carries no generics or lifetimes; [`TypedScalar<DT, T>`](crate::TypedScalar) adds the
/// typed value and is Rust-only.
///
/// ```
/// use yggdryl_scalar::{I64Scalar, Scalar};
/// use arrow_schema::DataType as ArrowDataType;
///
/// let value = I64Scalar::new(7);
/// assert!(!value.is_null());
/// assert_eq!(value.arrow_data_type(), ArrowDataType::Int64);
/// assert!(I64Scalar::null().is_null());
/// ```
pub trait Scalar {
    /// Whether the scalar holds no value (SQL `NULL`).
    fn is_null(&self) -> bool;

    /// The Arrow [`DataType`](arrow_schema::DataType) of the scalar's value.
    fn arrow_data_type(&self) -> ArrowDataType;

    /// The scalar serialised to little-endian bytes (rule 5): a 1-byte null flag, then
    /// the value's bytes when present.
    fn serialize_bytes(&self) -> Vec<u8>;
}
