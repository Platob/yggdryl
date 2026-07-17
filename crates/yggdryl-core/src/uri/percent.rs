//! RFC 3986 percent-encoding — the codec [`Uri`](super::Uri) uses to store its components
//! in encoded form and to hand them back decoded.
//!
//! Both directions are **zero-copy on the clean path**: [`encode`] returns the input
//! borrowed when every byte is already safe, and [`decode`] returns it borrowed when there
//! is no `%`. Each URI component has its own safe set (which bytes are left as-is); anything
//! else becomes `%XX` (UTF-8 bytes, uppercase hex).

use std::borrow::Cow;

/// The always-safe *unreserved* set: `ALPHA / DIGIT / "-" / "." / "_" / "~"`.
const fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// The RFC 3986 `sub-delims`: `! $ & ' ( ) * + , ; =`.
const fn is_sub_delim(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

/// Path safe set — `pchar` plus the `/` separator (`pchar = unreserved / sub-delims / : / @`).
pub(crate) const fn is_path_safe(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delim(byte) || matches!(byte, b':' | b'@' | b'/')
}

/// Query / fragment safe set — `pchar` plus `/` and `?`.
pub(crate) const fn is_query_safe(byte: u8) -> bool {
    is_path_safe(byte) || byte == b'?'
}

/// Userinfo safe set — `unreserved / sub-delims` (a value's own `:` `@` are encoded).
pub(crate) const fn is_userinfo_safe(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delim(byte)
}

/// Query-parameter key/value safe set — `unreserved` only, so the structural `&` `=` `+`
/// (and spaces) of a value never break the surrounding query.
pub(crate) const fn is_param_safe(byte: u8) -> bool {
    is_unreserved(byte)
}

const fn hex_digit(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'A' + (nibble - 10),
    }
}

const fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-encodes every byte for which `is_safe` is false. Returns the input **borrowed**
/// when it is already clean — no allocation.
pub(crate) fn encode(input: &str, is_safe: fn(u8) -> bool) -> Cow<'_, str> {
    if input.bytes().all(is_safe) {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len() + 8);
    for &byte in input.as_bytes() {
        if is_safe(byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(hex_digit(byte >> 4) as char);
            out.push(hex_digit(byte & 0x0f) as char);
        }
    }
    Cow::Owned(out)
}

/// Encodes `input` (an owned `String`) in place-ish: reuses the buffer when it is already
/// clean, so a clean value costs no extra allocation.
pub(crate) fn encode_owned(input: String, is_safe: fn(u8) -> bool) -> String {
    match encode(&input, is_safe) {
        Cow::Borrowed(_) => input,
        Cow::Owned(encoded) => encoded,
    }
}

/// Percent-decodes `%XX` escapes. Returns the input **borrowed** when there is no `%` — no
/// allocation. A malformed escape (not two hex digits) is left verbatim; decoded bytes are
/// read as UTF-8 (lossily, so decoding never fails).
pub(crate) fn decode(input: &str) -> Cow<'_, str> {
    if !input.contains('%') {
        return Cow::Borrowed(input);
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Cow::Owned(
        String::from_utf8(out)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()),
    )
}
