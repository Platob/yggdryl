//! windows-1252: the code page, and the six string leaves that carry it. One
//! module because the leaf is a declaration that a column holds legacy bytes,
//! and the codec is what those bytes mean: the generated table read through
//! the single-byte machinery in [`crate::charset`], which is also the page
//! every stray byte offered as UTF-8 reads as - so the datatype that stores
//! it, the door that transcribes it and the reading that recovers it all name
//! one table from one place.

use std::borrow::Cow;

use smol_str::SmolStr;

use crate::charset::sink::Utf8Sink;
use crate::charset::tables::CP1252;
use crate::{Charset, DataType, Result, StringType};

/// The canonical name this charset reports itself by.
pub(crate) const fn name() -> &'static str {
    CP1252.name
}

/// The scalar one byte decodes to, or `None` for one of the five bytes the
/// page leaves unassigned.
pub(crate) fn scalar_of(byte: u8) -> Option<char> {
    CP1252.scalar_of(byte)
}

/// The byte one scalar encodes to, or `None` where the page has none.
pub(crate) fn byte_of(scalar: char) -> Option<u8> {
    CP1252.byte_of(scalar)
}

/// Decode a complete buffer, borrowing it when it is already US-ASCII.
pub(crate) fn decode(input: &[u8]) -> Result<Cow<'_, str>> {
    CP1252.decode(input)
}

/// Decode a complete buffer, replacing an unassigned byte with `U+FFFD`.
pub(crate) fn decode_lossy(input: &[u8]) -> Cow<'_, str> {
    CP1252.decode_lossy(input)
}

/// Decode into a target, replacing an unassigned byte when `LOSSY`.
pub(crate) fn decode_into<const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    CP1252.decode_into::<LOSSY>(input, target)
}

/// Decode a complete buffer, reading an unassigned byte as its ISO 8859-1
/// scalar - the C1 control of its number - rather than replacing it.
pub(crate) fn transcribe(input: &[u8]) -> Cow<'_, str> {
    CP1252.transcribe(input)
}

/// [`transcribe`] over whichever target a caller brought.
///
/// This is the walk every stray byte offered as UTF-8 takes: the transcribing
/// UTF-8 door hands each invalid run here, byte for byte.
pub(crate) fn transcribe_sink(input: &[u8], target: &mut impl Utf8Sink) -> Result<()> {
    CP1252.transcribe_sink(input, target)
}

/// [`transcribe`] into the compact string a value holds.
///
/// The door a `windows-1252` column's bytes take into a value: the column is
/// a declaration that it holds legacy bytes, so those are transcribed rather
/// than refused, through [`Charset::transcribe_smol`] under this one page.
pub(crate) fn transcribe_smol(input: &[u8]) -> SmolStr {
    Charset::Cp1252.transcribe_smol(input)
}

/// Encode complete text, borrowing it when every scalar is US-ASCII.
pub(crate) fn encode(input: &str) -> Result<Cow<'_, [u8]>> {
    CP1252.encode(input)
}

/// Encode text into a byte target.
pub(crate) fn encode_into(input: &str, target: &mut Vec<u8>) -> Result<()> {
    CP1252.encode_into(input, target)
}

/// The six leaves that carry this charset, in shape order: plain, large,
/// view, large view, fixed, sized - with the placeholder `1` where a leaf
/// carries a number.
pub const LEAVES: [StringType; 6] = [
    StringType::Cp1252String,
    StringType::LargeCp1252String,
    StringType::Cp1252StringView,
    StringType::LargeCp1252StringView,
    StringType::FixedCp1252String(1),
    StringType::SizedCp1252String(1),
];

/// The fixed leaf of this charset, stating `width`.
pub(crate) const fn fixed_leaf(width: u32) -> StringType {
    StringType::FixedCp1252String(width)
}

/// The sized leaf of this charset, stating `max`.
pub(crate) const fn sized_leaf(max: u32) -> StringType {
    StringType::SizedCp1252String(max)
}

impl DataType {
    /// Unbounded windows-1252 with 32-bit offsets.
    #[must_use]
    pub const fn cp1252() -> Self {
        Self::String(StringType::Cp1252String)
    }

    /// Unbounded windows-1252 with 64-bit offsets.
    #[must_use]
    pub const fn large_cp1252() -> Self {
        Self::String(StringType::LargeCp1252String)
    }

    /// Unbounded windows-1252 in the view layout.
    #[must_use]
    pub const fn cp1252_view() -> Self {
        Self::String(StringType::Cp1252StringView)
    }

    /// Unbounded windows-1252 in the view layout over 64-bit offsets.
    #[must_use]
    pub const fn large_cp1252_view() -> Self {
        Self::String(StringType::LargeCp1252StringView)
    }

    /// Windows-1252 of exactly `width` stored bytes, padded with trailing NUL.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_cp1252(width: u32) -> Result<Self> {
        Self::string(StringType::FixedCp1252String(width))
    }

    /// Windows-1252 of at most `max` stored bytes, over 32-bit offsets.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a maximum of zero.
    pub fn sized_cp1252(max: u32) -> Result<Self> {
        Self::string(StringType::SizedCp1252String(max))
    }
}
