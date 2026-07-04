//! The [`DataType`] base trait: a physical, FFI-facing data type descriptor.

use super::DataError;

/// The base trait every data type implements: a *physical* type descriptor built for
/// Apache Arrow interop and zero-copy FFI.
///
/// It answers three questions about a type — what it is called ([`name`](DataType::name)),
/// how Arrow describes it over the C Data Interface
/// ([`arrow_format`](DataType::arrow_format)), and how wide one value is
/// ([`byte_width`](DataType::byte_width) / [`bit_width`](DataType::bit_width)) —
/// and converts to and from the Apache Arrow type it mirrors
/// ([`to_arrow`](DataType::to_arrow) / [`from_arrow`](DataType::from_arrow), the
/// Arrow *factory*). Concrete types ([`Int32Type`](crate::Int32Type), `Utf8Type`,
/// `BooleanType`, …) implement it, and the parameterised `Field` (in
/// `yggdryl-field`) and `Scalar` (in `yggdryl-scalar`) build on it.
///
/// Following the FFI rules, it carries no lifetime parameters; the one borrow —
/// [`name`](DataType::name) — is a `&self` accessor that never escapes. It is
/// `Debug` (schema printing and diagnostics), `Send + Sync` (types are shared
/// metadata handed across threads and over FFI), and object-safe, so a heterogeneous
/// schema can hold `Box<dyn DataType>` ([`from_arrow`](DataType::from_arrow),
/// which returns `Self`, is `Self: Sized` and stays off the vtable). Type *equality*
/// is intentionally not a supertrait — comparing
/// [`arrow_format`](DataType::arrow_format) keeps the trait object-safe — so a
/// `PartialEq` bound is avoided.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataError, DataType};
///
/// // A minimal fixed-width primitive.
/// #[derive(Debug)]
/// struct Int32Type;
///
/// impl DataType for Int32Type {
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
///             arrow_schema::DataType::Int32 => Ok(Int32Type),
///             other => Err(DataError::IncompatibleArrowType {
///                 expected: "Int32Type".to_string(),
///                 got: other.to_string(),
///             }),
///         }
///     }
/// }
///
/// assert_eq!(Int32Type.name(), "int32");
/// assert_eq!(Int32Type.arrow_format(), "i");
/// assert_eq!(Int32Type.byte_width(), Some(4));
/// assert_eq!(Int32Type.bit_width(), Some(32)); // default: eight times the byte width
///
/// // Arrow interop round-trips through the arrow-schema type.
/// assert_eq!(Int32Type.to_arrow(), arrow_schema::DataType::Int32);
/// assert!(Int32Type::from_arrow(&Int32Type.to_arrow()).is_ok());
/// assert!(Int32Type::from_arrow(&arrow_schema::DataType::Utf8).is_err());
/// ```
pub trait DataType: std::fmt::Debug + Send + Sync {
    /// A stable, lowercase name identifying this type, e.g. `"int32"`, `"utf8"`,
    /// `"boolean"`.
    fn name(&self) -> &str;

    /// The Apache Arrow C Data Interface format string for this type — the compact,
    /// zero-copy descriptor exported over FFI (e.g. `"i"` for int32, `"g"` for
    /// float64, `"u"` for utf8).
    fn arrow_format(&self) -> String;

    /// The fixed size of one value, in bytes, or `None` for a variable-width type
    /// (e.g. utf8) or a sub-byte type (e.g. boolean, which reports a
    /// [`bit_width`](DataType::bit_width) instead).
    fn byte_width(&self) -> Option<usize>;

    /// The fixed size of one value, in bits, or `None` when the type has no fixed
    /// width. Defaults to eight times [`byte_width`](DataType::byte_width); a
    /// sub-byte type overrides it directly.
    fn bit_width(&self) -> Option<usize> {
        self.byte_width().map(|width| width * 8)
    }

    /// The [`arrow_schema::DataType`] this type mirrors.
    fn to_arrow(&self) -> arrow_schema::DataType;

    /// Build this type from the [`arrow_schema::DataType`] it mirrors — the exact
    /// inverse of [`to_arrow`](DataType::to_arrow), and the Arrow *factory*. A
    /// different Arrow type errors with [`DataError::IncompatibleArrowType`].
    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError>
    where
        Self: Sized;

    /// A compact, human-readable **signature** for fast debugging — our lowercase
    /// name plus, for a container, its children in angle brackets (`int64`,
    /// `list<int64>`, `struct<x: int64, y: float64>`, `map<utf8, int64>`,
    /// `optional<int64>`). Built from [`to_arrow`](DataType::to_arrow) through the
    /// shared [`signature`](crate::signature) walker, so every nesting level renders.
    ///
    /// ```
    /// use yggdryl_dtype::{DataType, Int64Type};
    /// assert_eq!(Int64Type.display(), "int64");
    /// ```
    fn display(&self) -> String {
        crate::signature(&self.to_arrow())
    }
}
