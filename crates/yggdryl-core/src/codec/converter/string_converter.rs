//! [`StringConverter<T>`] — flexible string parsing to/from a primitive.

use core::marker::PhantomData;
use std::borrow::Cow;

use crate::{ConvertError, Converter, IoPrimitive, TypedConverter};

/// Parses a string into a numeric primitive `T`, and renders `T` back to a string.
///
/// [`encode`](TypedConverter::encode) parses flexibly and cheaply — it tries the
/// fastest format first and only falls back when needed, allocating **only** when a
/// value actually uses underscores or a radix prefix:
///
/// * integers — decimal (with optional `+`/`-`), the radix prefixes `0x` / `0o` /
///   `0b` (any case), and `_` digit separators;
/// * floats — decimal and scientific (`1e9`), `inf` / `nan`, and `_` separators.
///
/// [`decode`](TypedConverter::decode) renders with the value's [`Display`] form. A
/// string no format accepts yields a [`ConvertError::ParseFailed`] listing the
/// accepted formats.
///
/// ```
/// use yggdryl_core::{StringConverter, TypedConverter};
///
/// let ints = StringConverter::<i32>::new();
/// assert_eq!(ints.encode("42".to_string()).unwrap(), 42);
/// assert_eq!(ints.encode("0x2A".to_string()).unwrap(), 42);
/// assert_eq!(ints.encode(" -1_000 ".to_string()).unwrap(), -1000);
/// assert_eq!(ints.decode(42).unwrap(), "42");
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

/// Parses `text` into `T` or builds a guided [`ConvertError::ParseFailed`].
fn parse<T: ParseNumber>(text: &str) -> Result<T, ConvertError> {
    T::parse_flexible(text).ok_or_else(|| ConvertError::ParseFailed {
        input: truncate(text),
        target: T::TYPE_NAME,
        expected: T::EXPECTED,
    })
}

/// Caps an offending input at 64 chars so the error message stays readable.
fn truncate(text: &str) -> String {
    if text.len() <= 64 {
        text.to_string()
    } else {
        let mut end = 61;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &text[..end])
    }
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

/// Flexibly parses one numeric primitive from a string — the internal machinery
/// behind [`StringConverter`]. Implemented for the ten native primitives.
pub(crate) trait ParseNumber: Sized {
    /// The target type name for error messages, e.g. `"i32"`.
    const TYPE_NAME: &'static str;
    /// The accepted formats, for error messages.
    const EXPECTED: &'static str;
    /// Parses `input`, returning `None` if no accepted format matches.
    fn parse_flexible(input: &str) -> Option<Self>;
}

/// Splits a flexible integer string into `(radix, sign+digits)` with any `0x`/`0o`/
/// `0b` prefix removed and `_` separators stripped. Allocates only when the value
/// carries a sign together with a prefix, or contains underscores.
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
    let body: Cow<'_, str> = if digits.as_bytes().contains(&b'_') {
        Cow::Owned(digits.replace('_', ""))
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
                     (optional +/- sign, _ separators allowed)";

                fn parse_flexible(input: &str) -> Option<Self> {
                    let s = input.trim();
                    if s.is_empty() {
                        return None;
                    }
                    // Fast path: plain signed decimal without separators.
                    if !s.as_bytes().contains(&b'_') {
                        if let Ok(value) = s.parse::<$ty>() {
                            return Some(value);
                        }
                    }
                    let (radix, digits) = split_radix(s)?;
                    <$ty>::from_str_radix(&digits, radix).ok()
                }
            }
        )+
    };
}

/// Stamps out [`ParseNumber`] for the native floats.
macro_rules! parse_float {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl ParseNumber for $ty {
                const TYPE_NAME: &'static str = stringify!($ty);
                const EXPECTED: &'static str =
                    "a decimal or scientific float (e.g. 3.14, -1e9, inf, nan; \
                     _ separators allowed)";

                fn parse_flexible(input: &str) -> Option<Self> {
                    let s = input.trim();
                    if s.is_empty() {
                        return None;
                    }
                    if s.as_bytes().contains(&b'_') {
                        return s.replace('_', "").parse::<$ty>().ok();
                    }
                    s.parse::<$ty>().ok()
                }
            }
        )+
    };
}

parse_int!(i8, i16, i32, i64, u8, u16, u32, u64);
parse_float!(f32, f64);
