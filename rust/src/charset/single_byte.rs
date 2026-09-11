//! One byte per scalar, driven by a generated 128-entry table.
//!
//! Every charset here agrees with US-ASCII below `0x80`, so a decode is an
//! ASCII scan that copies runs whole and looks one table entry up per byte
//! above it. That is what makes a mostly-ASCII payload - which nearly every
//! legacy export is - cost a borrow or a `memcpy` rather than a transcode.

use std::borrow::Cow;

use super::ascii::{ascii_len, text};
use super::sink::Utf8Sink;
use super::{REPLACEMENT, REPLACEMENT_UTF8, undecodable, unencodable};
use crate::Result;

/// One single-byte charset's whole mapping, in the four shapes it is read in.
///
/// The four arrays are projections of the same 128 facts rather than four
/// decisions: [`crate::charset`] never lets them disagree because
/// `scripts/generate_charset_tables.py` writes all four from one codec.
pub(super) struct SingleByte {
    /// The canonical charset name, which every error from it reports.
    pub(super) name: &'static str,
    /// The scalar each byte `0x80..=0xFF` decodes to; `U+FFFD` marks a byte
    /// the charset leaves unassigned.
    pub(super) scalars: [char; 128],
    /// Those scalars pre-encoded as UTF-8, padded to the widest of them, so a
    /// byte target copies bytes instead of encoding a scalar.
    pub(super) encoded: [[u8; 3]; 128],
    /// How many bytes of `encoded` each scalar fills; zero is the branch-free
    /// spelling of the `U+FFFD` in `scalars`.
    pub(super) widths: [u8; 128],
    /// `(scalar, byte)` above US-ASCII ordered by scalar, which encoding
    /// binary-searches.
    pub(super) reverse: &'static [(char, u8)],
}

impl SingleByte {
    /// The scalar one byte decodes to, or `None` where the charset assigns it
    /// nothing.
    pub(super) fn scalar_of(&self, byte: u8) -> Option<char> {
        let Some(slot) = usize::from(byte).checked_sub(0x80) else {
            return Some(char::from(byte));
        };
        (self.widths[slot] != 0).then(|| self.scalars[slot])
    }

    /// The byte one scalar encodes to, or `None` where the charset has none.
    pub(super) fn byte_of(&self, scalar: char) -> Option<u8> {
        if scalar.is_ascii() {
            return Some(scalar as u8);
        }
        self.reverse
            .binary_search_by_key(&scalar, |(point, _)| *point)
            .ok()
            .map(|index| self.reverse[index].1)
    }

    /// Decode a complete buffer, borrowing it when it is already US-ASCII.
    pub(super) fn decode<'input>(&self, input: &'input [u8]) -> Result<Cow<'input, str>> {
        if ascii_len(input) == input.len() {
            return Ok(Cow::Borrowed(text(input)?));
        }
        let mut target = String::new();
        self.decode_into::<false>(input, &mut target)?;
        Ok(Cow::Owned(target))
    }

    /// Decode a complete buffer, replacing unassigned bytes rather than
    /// refusing them.
    pub(super) fn decode_lossy<'input>(&self, input: &'input [u8]) -> Cow<'input, str> {
        if ascii_len(input) == input.len() {
            if let Ok(borrowed) = text(input) {
                return Cow::Borrowed(borrowed);
            }
        }
        let mut target = String::new();
        match self.decode_into::<true>(input, &mut target) {
            // A lossy decode assigns every byte, so the only refusal left is
            // the impossible one a proven-ASCII run can raise.
            Ok(()) => Cow::Owned(target),
            Err(_) => Cow::Owned(String::from(REPLACEMENT)),
        }
    }

    /// Decode into a target, replacing unassigned bytes when `LOSSY`.
    pub(super) fn decode_into<const LOSSY: bool>(
        &self,
        input: &[u8],
        target: &mut impl Utf8Sink,
    ) -> Result<()> {
        // A high byte is at most three UTF-8 bytes, and nearly all of a real
        // payload is ASCII; reserving its length is the one allocation a
        // transcode of plain text needs.
        target.reserve(input.len());
        let mut index = 0;
        while index < input.len() {
            let run = ascii_len(&input[index..]);
            if run > 0 {
                target.push_utf8(&input[index..index + run])?;
                index += run;
                continue;
            }
            let byte = input[index];
            let slot = usize::from(byte) - 0x80;
            let width = usize::from(self.widths[slot]);
            if width == 0 {
                if !LOSSY {
                    return Err(undecodable(self.name, index, byte));
                }
                target.push_scalar(REPLACEMENT, REPLACEMENT_UTF8);
            } else {
                target.push_scalar(self.scalars[slot], &self.encoded[slot][..width]);
            }
            index += 1;
        }
        Ok(())
    }

    /// Encode complete text, borrowing it when every scalar is US-ASCII.
    pub(super) fn encode<'input>(&self, input: &'input str) -> Result<Cow<'input, [u8]>> {
        let bytes = input.as_bytes();
        if ascii_len(bytes) == bytes.len() {
            return Ok(Cow::Borrowed(bytes));
        }
        let mut target = Vec::new();
        self.encode_into(input, &mut target)?;
        Ok(Cow::Owned(target))
    }

    /// Encode text into a byte target.
    pub(super) fn encode_into(&self, input: &str, target: &mut Vec<u8>) -> Result<()> {
        // One byte per scalar is the whole charset, so the output is never
        // longer than the input and is usually shorter.
        target.reserve(input.len());
        let bytes = input.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            let run = ascii_len(&bytes[index..]);
            if run > 0 {
                target.extend_from_slice(&bytes[index..index + run]);
                index += run;
                continue;
            }
            // The run stopped at a lead byte, so `index` is a scalar boundary.
            let Some(scalar) = input.get(index..).and_then(|rest| rest.chars().next()) else {
                return Err(undecodable(self.name, index, bytes[index]));
            };
            let Some(byte) = self.byte_of(scalar) else {
                return Err(unencodable(self.name, index, scalar));
            };
            target.push(byte);
            index += scalar.len_utf8();
        }
        Ok(())
    }
}
