//! The [`RawDataType`] base trait: a physical, FFI-facing data type descriptor.

use super::DataError;

/// The base trait every data type implements: a *physical* type descriptor built for
/// Apache Arrow interop and zero-copy FFI.
///
/// It answers three questions about a type — what it is called ([`name`](RawDataType::name)),
/// how Arrow describes it over the C Data Interface
/// ([`arrow_format`](RawDataType::arrow_format)), and how wide one value is
/// ([`byte_width`](RawDataType::byte_width) / [`bit_width`](RawDataType::bit_width)) —
/// and converts to and from the Apache Arrow type it mirrors
/// ([`to_arrow`](RawDataType::to_arrow) / [`from_arrow`](RawDataType::from_arrow)).
/// Concrete types (`Int32`, `Utf8`, `Boolean`, …) implement it, and the parameterised
/// [`RawField`](super::RawField) and [`RawScalar`](super::RawScalar) build on it.
///
/// Following the FFI rules, it carries no lifetime parameters; the one borrow —
/// [`name`](RawDataType::name) — is a `&self` accessor that never escapes. It is
/// `Debug` (schema printing and diagnostics), `Send + Sync` (types are shared
/// metadata handed across threads and over FFI), and object-safe, so a heterogeneous
/// schema can hold `Box<dyn RawDataType>` ([`from_arrow`](RawDataType::from_arrow),
/// which returns `Self`, is `Self: Sized` and stays off the vtable). Type *equality*
/// is intentionally not a supertrait — comparing
/// [`arrow_format`](RawDataType::arrow_format) keeps the trait object-safe — so a
/// `PartialEq` bound is avoided.
///
/// ```
/// use yggdryl_data::{arrow_schema, DataError, RawDataType};
///
/// // A minimal fixed-width primitive.
/// #[derive(Debug)]
/// struct Int32;
///
/// impl RawDataType for Int32 {
///     fn name(&self) -> &str {
///         "int32"
///     }
///     fn arrow_format(&self) -> String {
///         "i".to_string() // Arrow C Data Interface format for int32
///     }
///     fn byte_width(&self) -> Option<usize> {
///         Some(4)
///     }
///     fn to_arrow(&self) -> arrow_schema::DataType {
///         arrow_schema::DataType::Int32
///     }
///     fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
///         match data_type {
///             arrow_schema::DataType::Int32 => Ok(Int32),
///             other => Err(DataError::IncompatibleArrowType {
///                 expected: "Int32".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// assert_eq!(Int32.name(), "int32");
/// assert_eq!(Int32.arrow_format(), "i");
/// assert_eq!(Int32.byte_width(), Some(4));
/// assert_eq!(Int32.bit_width(), Some(32)); // default: eight times the byte width
///
/// // Arrow interop round-trips through the arrow-schema type.
/// assert_eq!(Int32.to_arrow(), arrow_schema::DataType::Int32);
/// assert!(Int32::from_arrow(&Int32.to_arrow()).is_ok());
/// assert!(Int32::from_arrow(&arrow_schema::DataType::Utf8).is_err());
/// ```
pub trait RawDataType: std::fmt::Debug + Send + Sync {
    /// A stable, lowercase name identifying this type, e.g. `"int32"`, `"utf8"`,
    /// `"boolean"`.
    fn name(&self) -> &str;

    /// The Apache Arrow C Data Interface format string for this type — the compact,
    /// zero-copy descriptor exported over FFI (e.g. `"i"` for int32, `"g"` for
    /// float64, `"u"` for utf8).
    fn arrow_format(&self) -> String;

    /// The fixed size of one value, in bytes, or `None` for a variable-width type
    /// (e.g. utf8) or a sub-byte type (e.g. boolean, which reports a
    /// [`bit_width`](RawDataType::bit_width) instead).
    fn byte_width(&self) -> Option<usize>;

    /// The fixed size of one value, in bits, or `None` when the type has no fixed
    /// width. Defaults to eight times [`byte_width`](RawDataType::byte_width); a
    /// sub-byte type overrides it directly.
    fn bit_width(&self) -> Option<usize> {
        self.byte_width().map(|width| width * 8)
    }

    /// The [`arrow_schema::DataType`] this type mirrors.
    fn to_arrow(&self) -> arrow_schema::DataType;

    /// Build this type from the [`arrow_schema::DataType`] it mirrors — the exact
    /// inverse of [`to_arrow`](RawDataType::to_arrow). A different Arrow type errors
    /// with [`DataError::IncompatibleArrowType`].
    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError>
    where
        Self: Sized;
}
