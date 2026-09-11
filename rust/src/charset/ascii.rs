//! US-ASCII, and the scan every other charset in this module starts with.

use std::borrow::Cow;

use super::sink::Utf8Sink;
use super::{REPLACEMENT, undecodable, unencodable};
use crate::Result;

/// The canonical name this charset reports itself by.
pub(super) const NAME: &str = "us-ascii";

/// The bit that leaves US-ASCII, in every byte of a machine word.
///
/// `usize::MAX / 0xFF` is one in the low bit of every byte, so multiplying it
/// by `0x80` raises the high bit of every byte whatever the word width is.
const HIGH_BITS: usize = usize::MAX / 0xFF * 0x80;

/// How many leading bytes of `input` are US-ASCII.
///
/// The scan reads a machine word at a time while a whole one remains, so plain
/// text costs one masked load per eight bytes instead of one comparison per
/// byte. Every decode in this module's siblings begins here: bytes below
/// `0x80` are their own UTF-8, so an ASCII run is copied whole - or, when it
/// is the whole input, borrowed rather than transcoded at all.
pub(crate) fn ascii_len(input: &[u8]) -> usize {
    const LANE: usize = size_of::<usize>();

    let mut index = 0;
    while index + LANE <= input.len() {
        let Ok(lane) = <[u8; LANE]>::try_from(&input[index..index + LANE]) else {
            break;
        };
        if usize::from_ne_bytes(lane) & HIGH_BITS != 0 {
            break;
        }
        index += LANE;
    }
    while index < input.len() && input[index] < 0x80 {
        index += 1;
    }
    index
}

/// Borrow a run [`ascii_len`] proved to be US-ASCII as text.
///
/// Every byte below `0x80` is its own one-byte UTF-8 encoding, so this
/// conversion always succeeds. It is spelled as a checked one because the
/// crate denies `unsafe`, and over an all-ASCII run the standard library's
/// check is the same word-at-a-time scan written above.
pub(super) fn text(run: &[u8]) -> Result<&str> {
    std::str::from_utf8(run).map_err(|error| {
        let position = error.valid_up_to();
        undecodable(
            NAME,
            position,
            run.get(position).copied().unwrap_or_default(),
        )
    })
}

/// Decode a complete buffer, borrowing it when it is already US-ASCII.
pub(super) fn decode(input: &[u8]) -> Result<Cow<'_, str>> {
    let boundary = ascii_len(input);
    if boundary < input.len() {
        return Err(undecodable(NAME, boundary, input[boundary]));
    }
    Ok(Cow::Borrowed(text(input)?))
}

/// Decode into a sink, replacing bytes above `0x7F` when `LOSSY`.
pub(super) fn decode_into<const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    target.reserve(input.len());
    let mut index = 0;
    while index < input.len() {
        let run = ascii_len(&input[index..]);
        if run > 0 {
            target.push_utf8(&input[index..index + run])?;
            index += run;
            continue;
        }
        if !LOSSY {
            return Err(undecodable(NAME, index, input[index]));
        }
        target.push_scalar(REPLACEMENT, super::REPLACEMENT_UTF8);
        index += 1;
    }
    Ok(())
}

/// Encode complete text, borrowing it when every scalar is US-ASCII.
pub(super) fn encode(input: &str) -> Result<Cow<'_, [u8]>> {
    let bytes = input.as_bytes();
    let boundary = ascii_len(bytes);
    if boundary < bytes.len() {
        let scalar = input
            .get(boundary..)
            .and_then(|rest| rest.chars().next())
            .unwrap_or(REPLACEMENT);
        return Err(unencodable(NAME, boundary, scalar));
    }
    Ok(Cow::Borrowed(bytes))
}

/// Encode text into a byte target.
pub(super) fn encode_into(input: &str, target: &mut Vec<u8>) -> Result<()> {
    match encode(input)? {
        Cow::Borrowed(bytes) => target.extend_from_slice(bytes),
        Cow::Owned(bytes) => target.extend_from_slice(&bytes),
    }
    Ok(())
}
