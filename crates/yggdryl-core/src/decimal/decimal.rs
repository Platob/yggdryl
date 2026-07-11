//! [`Decimal`] — the base, FFI-opaque trait over the fixed-width decimals.

/// A fixed-width decimal number: an integer **mantissa** scaled by a power of ten, so the
/// represented value is `mantissa × 10^(−scale)`.
///
/// This is the base of the decimal hierarchy over the four widths — [`Decimal32`],
/// [`Decimal64`], [`Decimal128`], [`Decimal256`] — matching Apache Arrow's decimal
/// widths. It is object-safe (no generics, no lifetimes), so the bindings can hold it.
/// Each width is byte-based (mantissa little-endian bytes + a scale byte), has value
/// semantics (equal **iff** its [`serialize_bytes`](Decimal::serialize_bytes) are equal),
/// and converts to an [`f64`] / integer and between the widths.
///
/// [`Decimal32`]: crate::Decimal32
/// [`Decimal64`]: crate::Decimal64
/// [`Decimal128`]: crate::Decimal128
/// [`Decimal256`]: crate::Decimal256
pub trait Decimal {
    /// The number of fractional decimal digits: `value = mantissa × 10^(−scale)`.
    fn scale(&self) -> i8;

    /// The mantissa's width in bits (`32` / `64` / `128` / `256`).
    fn bits(&self) -> u32;

    /// The unscaled mantissa's little-endian two's-complement bytes (`bits / 8` of them).
    fn mantissa_le_bytes(&self) -> Vec<u8>;

    /// The value as an [`f64`] — `mantissa / 10^scale` (lossy for large mantissas).
    fn to_f64(&self) -> f64;

    /// The integer part as an [`i128`], truncated toward zero, or `None` if it overflows
    /// `i128` (possible only for [`Decimal256`](crate::Decimal256)).
    fn to_i128(&self) -> Option<i128>;

    /// The value's bytes: the mantissa's little-endian bytes followed by the `scale` byte.
    /// Two decimals are equal iff these are equal (`CLAUDE.md` rule 7).
    fn serialize_bytes(&self) -> Vec<u8>;
}
