//! The [`RawDataType`] base trait: a physical, FFI-facing data type descriptor.

/// The base trait every data type implements: a *physical* type descriptor built for
/// Apache Arrow interop and zero-copy FFI.
///
/// It answers three questions about a type — what it is called ([`name`](RawDataType::name)),
/// how Arrow describes it over the C Data Interface
/// ([`arrow_format`](RawDataType::arrow_format)), and how wide one value is
/// ([`byte_width`](RawDataType::byte_width) / [`bit_width`](RawDataType::bit_width)).
/// Concrete types (`Int32`, `Utf8`, `Boolean`, …) implement it, and the parameterised
/// [`RawField`](super::RawField) and [`RawScalar`](super::RawScalar) build on it.
///
/// Following the FFI rules, it carries no lifetime parameters; the one borrow —
/// [`name`](RawDataType::name) — is a `&self` accessor that never escapes.
///
/// ```
/// use yggdryl_data::RawDataType;
///
/// // A minimal fixed-width primitive.
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
/// }
///
/// assert_eq!(Int32.name(), "int32");
/// assert_eq!(Int32.arrow_format(), "i");
/// assert_eq!(Int32.byte_width(), Some(4));
/// assert_eq!(Int32.bit_width(), Some(32)); // default: eight times the byte width
/// ```
pub trait RawDataType {
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
}
