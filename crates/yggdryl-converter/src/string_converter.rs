//! [`StringConverter<T>`] — flexible string parsing to/from a primitive.

use core::marker::PhantomData;
use std::borrow::Cow;
use std::num::IntErrorKind;

use yggdryl_buffer::IoPrimitive;

use crate::{ConvertError, Converter, TypedConverter};

/// Parses a string into a numeric primitive `T`, and renders `T` back to a string.
///
/// [`encode`](TypedConverter::encode) parses flexibly and cheaply, trying formats
/// **most-common first** and allocating only when a value actually needs it:
///
/// * integers — decimal (optional `+`/`-`), then the radix prefixes `0x` / `0o` /
///   `0b` (any case); `_` and `,` are accepted as digit separators;
/// * floats — decimal and scientific (`1e9`), `inf` / `nan`; `_` and `,` separators.
///
/// [`decode`](TypedConverter::decode) renders with the value's [`Display`] form. A
/// string no format accepts yields a [`ConvertError::ParseFailed`]; a well-formed
/// value that overflows the target yields a [`ConvertError::OutOfRange`] naming the
/// value and the accepted range. The offending value is truncated in the message when
/// long, so errors stay human-readable.
///
/// ```
/// use yggdryl_converter::{StringConverter, TypedConverter};
///
/// let ints = StringConverter::<i32>::new();
/// assert_eq!(ints.encode("42".to_string()).unwrap(), 42);
/// assert_eq!(ints.encode("0x2A".to_string()).unwrap(), 42);
/// assert_eq!(ints.encode(" -1_000 ".to_string()).unwrap(), -1000);
/// assert_eq!(ints.encode("1,000,000".to_string()).unwrap(), 1_000_000);
/// assert_eq!(ints.decode(42).unwrap(), "42");
///
/// // A well-formed but too-big value reports the value and the range.
/// let err = ints.encode("99999999999".to_string()).unwrap_err();
/// assert!(err.to_string().contains("out of range"));
///
/// let floats = StringConverter::<f64>::new();
/// assert_eq!(floats.encode("-1.5e3".to_string()).unwrap(), -1500.0);
/// assert!(ints.encode("not a number".to_string()).is_err());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct StringConverter<T> {
    _marker: PhantomData<T>,
}

impl<T> StringConverter<T> {
    /// Creates the string converter.
    pub const fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Parses `text` into `T`, mapping a failure to a guided [`ConvertError`] that carries
/// the (truncated) offending value.
fn parse<T: ParseNumber>(text: &str) -> Result<T, ConvertError> {
    T::parse_flexible(text).map_err(|failure| match failure {
        ParseFailure::Format => ConvertError::ParseFailed {
            input: truncate(text),
            target: T::TYPE_NAME,
            expected: T::EXPECTED,
        },
        ParseFailure::OutOfRange { min, max } => ConvertError::OutOfRange {
            input: truncate(text),
            target: T::TYPE_NAME,
            min,
            max,
        },
    })
}

/// Caps an offending input at 64 chars (on a char boundary) so the error message
/// stays human-readable when the value is large.
fn truncate(text: &str) -> String {
    if text.len() <= 64 {
        return text.to_string();
    }
    let mut end = 61;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

impl<T: IoPrimitive + ParseNumber + core::fmt::Display> Converter for StringConverter<T> {
    fn convert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        let text = core::str::from_utf8(bytes).map_err(|error| ConvertError::InvalidUtf8 {
            valid_up_to: error.valid_up_to(),
        })?;
        Ok(parse::<T>(text)?.to_le_vec())
    }

    fn invert_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
        if bytes.len() != T::WIDTH {
            return Err(ConvertError::InvalidByteLength {
                len: bytes.len(),
                width: T::WIDTH,
            });
        }
        Ok(T::from_le_slice(bytes).to_string().into_bytes())
    }
}

impl<T: IoPrimitive + ParseNumber + core::fmt::Display> TypedConverter<String, T>
    for StringConverter<T>
{
    fn encode(&self, value: String) -> Result<T, ConvertError> {
        parse::<T>(&value)
    }

    fn decode(&self, value: T) -> Result<String, ConvertError> {
        Ok(value.to_string())
    }
}

/// Why a flexible parse failed — a bad format, or a good format whose value did not
/// fit the target's range.
pub(crate) enum ParseFailure {
    /// No accepted format matched.
    Format,
    /// The format was valid but the value overflowed the target's `min..=max`.
    OutOfRange {
        /// The lowest value the target accepts.
        min: String,
        /// The highest value the target accepts.
        max: String,
    },
}

/// Flexibly parses one numeric primitive from a string — the internal machinery
/// behind [`StringConverter`]. Implemented for the ten native primitives.
pub(crate) trait ParseNumber: Sized {
    /// The target type name for error messages, e.g. `"i32"`.
    const TYPE_NAME: &'static str;
    /// The accepted formats, for error messages.
    const EXPECTED: &'static str;
    /// Parses `input`, distinguishing a bad format from an out-of-range value.
    fn parse_flexible(input: &str) -> Result<Self, ParseFailure>;
}

/// Whether `s` carries any digit separator (`_` or `,`).
fn has_separators(s: &str) -> bool {
    s.as_bytes().iter().any(|&b| b == b'_' || b == b',')
}

/// Strips `_` and `,` digit separators.
fn strip_separators(s: &str) -> String {
    s.chars().filter(|&c| c != '_' && c != ',').collect()
}

/// Whether a [`ParseIntError`](std::num::ParseIntError) is an overflow (vs a bad
/// digit / empty string).
fn is_int_overflow(error: &std::num::ParseIntError) -> bool {
    matches!(
        error.kind(),
        IntErrorKind::PosOverflow | IntErrorKind::NegOverflow
    )
}

/// Splits a flexible integer string into `(radix, sign+digits)` with any `0x`/`0o`/
/// `0b` prefix removed and `_` / `,` separators stripped. Allocates only when the
/// value carries a sign together with a prefix, or contains separators.
fn split_radix(s: &str) -> Option<(u32, Cow<'_, str>)> {
    let (sign, rest): (&str, &str) = match s.as_bytes().first()? {
        b'+' => ("", &s[1..]),
        b'-' => ("-", &s[1..]),
        _ => ("", s),
    };
    let (radix, digits) = if let Some(hex) = strip_ci(rest, "0x") {
        (16, hex)
    } else if let Some(oct) = strip_ci(rest, "0o") {
        (8, oct)
    } else if let Some(bin) = strip_ci(rest, "0b") {
        (2, bin)
    } else {
        (10, rest)
    };
    if digits.is_empty() {
        return None;
    }
    let body: Cow<'_, str> = if has_separators(digits) {
        Cow::Owned(strip_separators(digits))
    } else {
        Cow::Borrowed(digits)
    };
    let normalized = if sign.is_empty() {
        body
    } else {
        Cow::Owned(format!("{sign}{body}"))
    };
    Some((radix, normalized))
}

/// Strips a two-byte ASCII prefix case-insensitively (e.g. `0x` / `0X`).
fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let bytes = s.as_bytes();
    let pre = prefix.as_bytes();
    if bytes.len() >= 2 && bytes[0] == pre[0] && bytes[1].eq_ignore_ascii_case(&pre[1]) {
        Some(&s[2..])
    } else {
        None
    }
}

/// Stamps out [`ParseNumber`] for the native integers.
macro_rules! parse_int {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ParseNumber for $ty {
                const TYPE_NAME: &'static str = stringify!($ty);
                const EXPECTED: &'static str =
                    "a decimal, 0x-hex, 0o-octal or 0b-binary integer \
                     (optional +/- sign, _ or , separators allowed)";

                fn parse_flexible(input: &str) -> Result<Self, ParseFailure> {
                    let s = input.trim();
                    if s.is_empty() {
                        return Err(ParseFailure::Format);
                    }
                    // Fast path (most common): plain signed decimal without separators.
                    if !has_separators(s) {
                        match s.parse::<$ty>() {
                            Ok(value) => return Ok(value),
                            Err(error) if is_int_overflow(&error) => {
                                return Err(ParseFailure::OutOfRange {
                                    min: <$ty>::MIN.to_string(),
                                    max: <$ty>::MAX.to_string(),
                                });
                            }
                            Err(_) => {} // fall through to the flexible radix path
                        }
                    }
                    // Less common: signs, radix prefixes, and separators.
                    let (radix, digits) = match split_radix(s) {
                        Some(parts) => parts,
                        None => return Err(ParseFailure::Format),
                    };
                    match <$ty>::from_str_radix(&digits, radix) {
                        Ok(value) => Ok(value),
                        Err(error) if is_int_overflow(&error) => Err(ParseFailure::OutOfRange {
                            min: <$ty>::MIN.to_string(),
                            max: <$ty>::MAX.to_string(),
                        }),
                        Err(_) => Err(ParseFailure::Format),
                    }
                }
            }
        )+
    };
}

/// Stamps out [`ParseNumber`] for the native floats (overflow parses to `inf`, so a
/// float parse only ever fails on format).
macro_rules! parse_float {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ParseNumber for $ty {
                const TYPE_NAME: &'static str = stringify!($ty);
                const EXPECTED: &'static str =
                    "a decimal or scientific float (e.g. 3.14, -1e9, inf, nan; \
                     _ or , separators allowed)";

                fn parse_flexible(input: &str) -> Result<Self, ParseFailure> {
                    let s = input.trim();
                    if s.is_empty() {
                        return Err(ParseFailure::Format);
                    }
                    let cleaned: Cow<'_, str> = if has_separators(s) {
                        Cow::Owned(strip_separators(s))
                    } else {
                        Cow::Borrowed(s)
                    };
                    cleaned.parse::<$ty>().map_err(|_| ParseFailure::Format)
                }
            }
        )+
    };
}

parse_int!(i8, i16, i32, i64, u8, u16, u32, u64);
parse_float!(f32, f64);
