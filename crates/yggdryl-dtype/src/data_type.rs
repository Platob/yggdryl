//! [`DataType`] — the base Arrow data-type contract.

use arrow_schema::DataType as ArrowDataType;

/// A data type in the Apache Arrow model — the FFI-opaque base of the type hierarchy.
///
/// Every data type — primitive, logical, or nested — reports its canonical
/// [`name`](DataType::name), its fixed [`byte_width`](DataType::byte_width) when it has
/// one, converts to the equivalent Arrow [`DataType`](arrow_schema::DataType) via
/// [`to_arrow`](DataType::to_arrow), and serialises its parameters to bytes via
/// [`serialize_bytes`](DataType::serialize_bytes) (rule 5). The trait is object-safe and
/// carries no generics or lifetimes, so the bindings can hold a data type behind it;
/// [`TypedDataType<T>`](crate::TypedDataType) adds the value-typed surface and is
/// Rust-only.
///
/// ```
/// use yggdryl_dtype::{DataType, I64Type};
/// use arrow_schema::DataType as ArrowDataType;
///
/// let dt = I64Type::new();
/// assert_eq!(dt.name(), "int64");
/// assert_eq!(dt.byte_width(), Some(8));
/// assert_eq!(dt.to_arrow(), ArrowDataType::Int64);
/// // A primitive type is a value-free marker, so its payload is empty.
/// assert!(dt.serialize_bytes().is_empty());
/// ```
pub trait DataType {
    /// The canonical lower-snake type name, e.g. `"int64"`.
    fn name(&self) -> &'static str;

    /// The fixed width of one value in bytes, or `None` for a variable-width or
    /// sub-byte type (e.g. `Boolean`, which is bit-packed).
    fn byte_width(&self) -> Option<usize>;

    /// The equivalent Arrow [`DataType`](arrow_schema::DataType).
    fn to_arrow(&self) -> ArrowDataType;

    /// This type's parameters serialised to little-endian bytes (rule 5). A primitive
    /// type is a value-free marker, so this is empty; parameterised types (future
    /// timestamps / decimals) carry their parameters here.
    fn serialize_bytes(&self) -> Vec<u8>;
}
