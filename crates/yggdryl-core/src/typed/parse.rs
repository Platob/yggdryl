//! [`FlexibleFromStr`] / [`FlexibleToStr`] — tolerant string ↔ value parsing for the native scalars
//! behind every [`DataType`](super::DataType).
//!
//! [`FlexibleFromStr::parse_flexible`] reads the **mainstream** textual forms a human or a CSV/JSON
//! feed produces — surrounding whitespace, a leading `+`, thousands separators (`,` and `_`), the
//! `0x`/`0b`/`0o` radix prefixes, and `1e3`-style scientific notation — by *normalizing* the string
//! and delegating to the fast native parse ([`str::parse`] / [`from_str_radix`](i64::from_str_radix)),
//! never a per-char accumulation loop. [`FlexibleFromStr::parse_exact`] is the strict counterpart
//! (`str::parse`, no coercion). [`FlexibleToStr::to_flexible_string`] is the inverse rendering
//! (plain [`Display`](std::fmt::Display)).
//!
//! Both are implemented for every numeric native (`i8`…`u128`, `f32`, `f64`) and `bool`, so the
//! [`Encoder::encode_str`](super::Encoder) / [`Decoder::decode_str`](super::Decoder) families can
//! parse and render columns generically.

use std::borrow::Cow;

use crate::io::memory::IoError;

/// The accepted-forms hint shown when an integer parse fails.
const INT_EXPECTED: &str =
    "expected an integer like 1, +1, 1_000, 1,000, 0xFF, 0b1010, 0o17, or 1e3";
/// The accepted-forms hint shown when a float parse fails.
const FLOAT_EXPECTED: &str = "expected a number like 1.5, -2, 1,234.5, 1.5e3, inf, or nan";
/// The accepted-forms hint shown when a boolean parse fails.
const BOOL_EXPECTED: &str = "expected a boolean like true/false, 1/0, yes/no, or t/f";
/// The hint shown when a fractional value is handed to an integer target.
const FRACTIONAL_HINT: &str =
    "the value has a fractional part; provide a whole number or use a float type instead";

/// Build the guided [`IoError::ParseError`] naming the offending string, target type, and fix.
fn parse_err(kind: &'static str, input: &str, expected: &'static str) -> IoError {
    IoError::ParseError {
        kind,
        input: input.to_string(),
        expected,
    }
}

/// Drop the mainstream thousands separators (`,` and `_`), **borrowing** the input when it has none
/// (the common case — no allocation).
fn strip_separators(s: &str) -> Cow<'_, str> {
    if s.as_bytes().iter().any(|&b| b == b',' || b == b'_') {
        Cow::Owned(s.chars().filter(|&c| c != ',' && c != '_').collect())
    } else {
        Cow::Borrowed(s)
    }
}

/// Split a single leading `+`/`-`; returns `(is_negative, rest_without_sign)`.
fn split_sign(s: &str) -> (bool, &str) {
    match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    }
}

/// Detect a `0x`/`0b`/`0o` radix prefix (case-insensitive) on a **sign-stripped** body; returns
/// `(radix, digits)` when present.
fn detect_radix(body: &str) -> Option<(u32, &str)> {
    for (lower, upper, radix) in [("0x", "0X", 16u32), ("0b", "0B", 2), ("0o", "0O", 8)] {
        if let Some(digits) = body
            .strip_prefix(lower)
            .or_else(|| body.strip_prefix(upper))
        {
            return Some((radix, digits));
        }
    }
    None
}

/// A native scalar that can be parsed from a string — the **tolerant** [`parse_flexible`](Self::parse_flexible)
/// front door and its strict [`parse_exact`](Self::parse_exact) twin.
///
/// ```
/// use yggdryl_core::typed::FlexibleFromStr;
///
/// assert_eq!(i64::parse_flexible("1,000,000").unwrap(), 1_000_000);
/// assert_eq!(i64::parse_flexible(" +42 ").unwrap(), 42);
/// assert_eq!(i64::parse_flexible("0xFF").unwrap(), 255);
/// assert_eq!(u8::parse_flexible("0b1010").unwrap(), 10);
/// assert_eq!(i64::parse_flexible("1e3").unwrap(), 1000);
/// assert!(bool::parse_flexible("YES").unwrap());
///
/// // `parse_exact` is strict: it refuses the thousands separators `parse_flexible` accepts.
/// assert!(i64::parse_exact("1,000").is_err());
/// ```
pub trait FlexibleFromStr: Sized {
    /// Parse the **tolerant, mainstream** forms (whitespace, leading `+`, `,`/`_` thousands
    /// separators, `0x`/`0b`/`0o` radices, `1e3` scientific). A value the target type cannot
    /// represent surfaces a guided [`IoError::ParseError`].
    fn parse_flexible(s: &str) -> Result<Self, IoError>;

    /// The **strict** parse — plain [`str::parse`], no coercion (the `_exact` counterpart).
    fn parse_exact(s: &str) -> Result<Self, IoError>;
}

/// Implements [`FlexibleFromStr`] for the integer natives: normalize, then delegate to
/// [`from_str_radix`](i64::from_str_radix) / [`str::parse`], with an `f64` fallback for
/// `1e3`-style scientific input that must be **integral and in range**.
macro_rules! impl_int_flexible {
    ($($t:ty),+ $(,)?) => {$(
        impl FlexibleFromStr for $t {
            fn parse_flexible(s: &str) -> Result<Self, IoError> {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return Err(parse_err(stringify!($t), s, INT_EXPECTED));
                }
                let cleaned = strip_separators(trimmed);
                let cs: &str = cleaned.as_ref();
                let (neg, body) = split_sign(cs);
                // Radix-prefixed (`0x`/`0b`/`0o`): re-attach the sign for `from_str_radix`, which
                // parses `-` correctly (so `i8` "-0x80" reaches its MIN) and rejects a negative
                // magnitude for an unsigned target.
                if let Some((radix, digits)) = detect_radix(body) {
                    let parsed = if neg {
                        let mut buf = String::with_capacity(digits.len() + 1);
                        buf.push('-');
                        buf.push_str(digits);
                        <$t>::from_str_radix(&buf, radix)
                    } else {
                        <$t>::from_str_radix(digits, radix)
                    };
                    return parsed.map_err(|_| parse_err(stringify!($t), s, INT_EXPECTED));
                }
                // Plain decimal (fast path — `str::parse` accepts a leading `+`/`-`).
                if let Ok(v) = cs.parse::<$t>() {
                    return Ok(v);
                }
                // Scientific / decimal-point form: accept only an integral, in-range value.
                if let Ok(fv) = cs.parse::<f64>() {
                    if !fv.is_finite() {
                        return Err(parse_err(stringify!($t), s, INT_EXPECTED));
                    }
                    if fv.fract() != 0.0 {
                        return Err(parse_err(stringify!($t), s, FRACTIONAL_HINT));
                    }
                    if fv >= <$t>::MIN as f64 && fv <= <$t>::MAX as f64 {
                        return Ok(fv as $t);
                    }
                    return Err(parse_err(stringify!($t), s, INT_EXPECTED));
                }
                Err(parse_err(stringify!($t), s, INT_EXPECTED))
            }

            fn parse_exact(s: &str) -> Result<Self, IoError> {
                s.parse::<$t>()
                    .map_err(|_| parse_err(stringify!($t), s, INT_EXPECTED))
            }
        }
    )+};
}

impl_int_flexible!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128);

/// Implements [`FlexibleFromStr`] for the float natives: normalize, then delegate to
/// [`str::parse`], which already handles scientific notation and `inf`/`infinity`/`nan`
/// (case-insensitive).
macro_rules! impl_float_flexible {
    ($($t:ty),+ $(,)?) => {$(
        impl FlexibleFromStr for $t {
            fn parse_flexible(s: &str) -> Result<Self, IoError> {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    return Err(parse_err(stringify!($t), s, FLOAT_EXPECTED));
                }
                let cleaned = strip_separators(trimmed);
                cleaned
                    .as_ref()
                    .parse::<$t>()
                    .map_err(|_| parse_err(stringify!($t), s, FLOAT_EXPECTED))
            }

            fn parse_exact(s: &str) -> Result<Self, IoError> {
                s.parse::<$t>()
                    .map_err(|_| parse_err(stringify!($t), s, FLOAT_EXPECTED))
            }
        }
    )+};
}

impl_float_flexible!(f32, f64);

impl FlexibleFromStr for bool {
    fn parse_flexible(s: &str) -> Result<Self, IoError> {
        let t = s.trim();
        if t.eq_ignore_ascii_case("true")
            || t == "1"
            || t.eq_ignore_ascii_case("yes")
            || t.eq_ignore_ascii_case("t")
        {
            Ok(true)
        } else if t.eq_ignore_ascii_case("false")
            || t == "0"
            || t.eq_ignore_ascii_case("no")
            || t.eq_ignore_ascii_case("f")
        {
            Ok(false)
        } else {
            Err(parse_err("bool", s, BOOL_EXPECTED))
        }
    }

    fn parse_exact(s: &str) -> Result<Self, IoError> {
        s.parse::<bool>()
            .map_err(|_| parse_err("bool", s, BOOL_EXPECTED))
    }
}

/// A native scalar that renders to a string — the inverse of [`FlexibleFromStr`], used by
/// [`Decoder::decode_str`](super::Decoder). The default rendering is plain
/// [`Display`](std::fmt::Display).
pub trait FlexibleToStr {
    /// Render `self` as its canonical decimal string.
    fn to_flexible_string(&self) -> String;
}

/// Implements [`FlexibleToStr`] via [`ToString`] ([`Display`](std::fmt::Display)) for each native.
macro_rules! impl_to_str {
    ($($t:ty),+ $(,)?) => {$(
        impl FlexibleToStr for $t {
            fn to_flexible_string(&self) -> String {
                self.to_string()
            }
        }
    )+};
}

impl_to_str!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64, bool);
