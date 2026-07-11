//! [`Field`] — the base Arrow field contract.

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl_http::HeadersBased;

/// A **field** in the Apache Arrow model — a named, nullable data type with optional
/// headers; the FFI-opaque base of the field hierarchy.
///
/// Every field reports its [`name`](Field::name) and [`is_nullable`](Field::is_nullable)
/// flag, its optional [`headers`](yggdryl_http::HeadersBased::headers) (via the
/// [`HeadersBased`](yggdryl_http::HeadersBased) supertrait), the Arrow
/// [`data type`](Field::arrow_data_type) of its values, converts to an Arrow
/// [`Field`](arrow_schema::Field) via [`to_arrow`](Field::to_arrow), and serialises to
/// bytes via [`serialize_bytes`](Field::serialize_bytes) (rule 5). The trait is
/// object-safe and carries no generics or lifetimes;
/// [`TypedField<DT, T>`](crate::TypedField) adds the concrete typed data type and is
/// Rust-only.
///
/// ```
/// use yggdryl_field::{Field, I64Field};
/// use arrow_schema::DataType as ArrowDataType;
///
/// let field = I64Field::new("id", false);
/// assert_eq!(field.name(), "id");
/// assert!(!field.is_nullable());
/// assert_eq!(field.arrow_data_type(), ArrowDataType::Int64);
/// assert_eq!(field.to_arrow().name(), "id");
/// ```
pub trait Field: HeadersBased {
    /// The field's name.
    fn name(&self) -> &str;

    /// Whether the field's values may be null.
    fn is_nullable(&self) -> bool;

    /// The Arrow [`DataType`](arrow_schema::DataType) of the field's values.
    fn arrow_data_type(&self) -> ArrowDataType;

    /// The equivalent Arrow [`Field`](arrow_schema::Field) (name + data type + nullable).
    fn to_arrow(&self) -> ArrowField {
        ArrowField::new(self.name(), self.arrow_data_type(), self.is_nullable())
    }

    /// The field serialised to little-endian bytes (rule 5): a 1-byte nullable flag, the
    /// length-prefixed UTF-8 name, then the headers bytes when present.
    fn serialize_bytes(&self) -> Vec<u8>;
}
