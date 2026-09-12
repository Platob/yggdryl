//! Byte-order marks: what a payload says about itself before its first scalar.

use super::Charset;

/// `U+FEFF` in UTF-8.
pub(super) const UTF8: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// `U+FEFF` in little-endian UTF-16.
pub(super) const UTF16LE: [u8; 2] = [0xFF, 0xFE];
/// `U+FEFF` in big-endian UTF-16.
pub(super) const UTF16BE: [u8; 2] = [0xFE, 0xFF];

/// The charset a leading mark names, and how many bytes the mark occupies.
///
/// Big-endian UTF-16 is tested before little-endian because the two marks are
/// each other's reverse, and a payload beginning `FE FF` is big-endian whatever
/// follows it.
pub(super) fn detect(input: &[u8]) -> Option<(Charset, usize)> {
    if input.starts_with(&UTF8) {
        return Some((Charset::Utf8, UTF8.len()));
    }
    if input.starts_with(&UTF16BE) {
        return Some((Charset::Utf16Be, UTF16BE.len()));
    }
    if input.starts_with(&UTF16LE) {
        return Some((Charset::Utf16Le, UTF16LE.len()));
    }
    None
}

/// The mark a charset is written with, where it has one.
///
/// Only the Unicode encoding forms have one: a single-byte charset has no
/// order to declare, and `U+FEFF` is not in any of their repertoires.
pub(super) const fn mark(charset: Charset) -> Option<&'static [u8]> {
    match charset {
        Charset::Utf8 => Some(&UTF8),
        Charset::Utf16Le => Some(&UTF16LE),
        Charset::Utf16Be => Some(&UTF16BE),
        Charset::Ascii
        | Charset::Latin1
        | Charset::Latin2
        | Charset::Latin9
        | Charset::Cp1250
        | Charset::Cp1251
        | Charset::Cp1252
        | Charset::Cp437
        | Charset::Cp850
        | Charset::MacRoman => None,
    }
}
