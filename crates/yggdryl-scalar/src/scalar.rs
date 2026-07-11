//! [`Scalar`] — the base Arrow scalar contract.

use arrow_schema::DataType as ArrowDataType;

/// A **scalar** in the Apache Arrow model — a single value of a data type; the FFI-opaque
/// base of the scalar hierarchy.
///
/// A scalar is **always present** — it carries a value, never a null. Nullability is a
/// property of a column/union, not of a scalar, and is modelled separately (a `NullType`
/// value and, later, union types), so a scalar stays a plain value that always serialises.
///
/// Every scalar reports the Arrow [`data type`](Scalar::arrow_data_type) of its value and
/// serialises to bytes via [`serialize_bytes`](Scalar::serialize_bytes) (rule 5) — just the
/// value's little-endian bytes. The trait is object-safe and carries no generics or
/// lifetimes; [`TypedScalar<DT, T>`](crate::TypedScalar) adds the typed value and is
/// Rust-only.
///
/// ```
/// use yggdryl_scalar::{I64Scalar, Scalar};
/// use arrow_schema::DataType as ArrowDataType;
///
/// let value = I64Scalar::new(7);
/// assert_eq!(value.arrow_data_type(), ArrowDataType::Int64);
/// assert_eq!(I64Scalar::deserialize_bytes(&value.serialize_bytes()).unwrap(), value);
/// // The default scalar of this scalar's type, behind a `dyn Scalar`.
/// assert!(value.default_any_scalar().serialize_bytes().iter().all(|&b| b == 0));
/// ```
pub trait Scalar {
    /// The Arrow [`DataType`](arrow_schema::DataType) of the scalar's value.
    fn arrow_data_type(&self) -> ArrowDataType;

    /// The scalar serialised to its value's little-endian bytes (rule 5).
    fn serialize_bytes(&self) -> Vec<u8>;

    /// The default scalar of this scalar's type behind an object-safe [`Box<dyn Scalar>`] —
    /// the FFI-opaque counterpart of
    /// [`default_scalar`](crate::TypedScalar::default_scalar), usable from a `dyn Scalar`.
    fn default_any_scalar(&self) -> Box<dyn Scalar>;
}
