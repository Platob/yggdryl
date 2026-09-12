//! The Unicode encoding forms: UTF-8, and UTF-16 in both byte orders.

use std::borrow::Cow;

use super::sink::Utf8Sink;
use super::tables::CP1252;
use super::{REPLACEMENT, REPLACEMENT_UTF8, truncated, unpaired};
use crate::{Error, Result};

/// The canonical name UTF-8 reports itself by.
pub(super) const UTF8: &str = "utf-8";
/// The canonical name little-endian UTF-16 reports itself by.
pub(super) const UTF16LE: &str = "utf-16le";
/// The canonical name big-endian UTF-16 reports itself by.
pub(super) const UTF16BE: &str = "utf-16be";

/// Decode UTF-8, which is a validation and a borrow rather than a transcode.
pub(super) fn utf8_decode(input: &[u8]) -> Result<Cow<'_, str>> {
    std::str::from_utf8(input)
        .map(Cow::Borrowed)
        .map_err(|error| utf8_fault(input, error))
}

/// Decode UTF-8 into a target.
pub(super) fn utf8_decode_into<const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    target.reserve(input.len());
    if LOSSY {
        return match String::from_utf8_lossy(input) {
            Cow::Borrowed(text) => target.push_utf8(text.as_bytes()),
            Cow::Owned(text) => target.push_utf8(text.as_bytes()),
        };
    }
    match std::str::from_utf8(input) {
        Ok(text) => target.push_utf8(text.as_bytes()),
        Err(error) => Err(utf8_fault(input, error)),
    }
}

/// How many trailing bytes of `input` begin a sequence it cuts short.
///
/// A genuinely invalid byte is not pending: it answers zero so the decode that
/// follows reports it with its position rather than waiting forever for a
/// continuation that would not fix it. Only the tail is judged, because only
/// the tail can be cut short: a fault earlier in the input is the decode's
/// to refuse or transcribe, and must not hide a sequence the boundary split
/// after it - a transcribing decode reads on past the fault, and would
/// otherwise read the held lead byte as a stray one.
pub(super) fn utf8_pending(input: &[u8]) -> usize {
    // A sequence is at most four bytes, so one cut short is a lead byte
    // among the last three followed only by continuation bytes.
    let tail = &input[input.len().saturating_sub(3)..];
    let Some(lead) = tail.iter().rposition(|byte| byte & 0xC0 != 0x80) else {
        return 0;
    };
    match std::str::from_utf8(&tail[lead..]) {
        Err(error) if error.error_len().is_none() => tail.len() - lead,
        _ => 0,
    }
}

/// Read bytes offered as UTF-8 that may not be, appending the text to
/// `target` and answering how many bytes were not UTF-8.
///
/// This is the one reading of a stray byte, and every door that reads bytes
/// offered as UTF-8 or as US-ASCII without refusing them is this function:
/// [`Charset::transcribe`](super::Charset::transcribe) under both charsets,
/// and the text line where it is made. Every valid UTF-8 run is kept as it
/// is, and every other byte reads as the character Windows-1252 gives it. Per
/// invalid run rather than per buffer, because a buffer is mostly UTF-8 with
/// a stray byte far more often than it is wholly Windows-1252: reading a valid
/// `é` (`C3 A9`) as `Ã©` because a lone `0xE9` stands elsewhere would destroy
/// what was right to repair what was wrong, and a wholly Windows-1252 buffer
/// has no valid multi-byte run to keep and reads byte for byte either way.
///
/// The table and the rule for its five holes are not restated here because
/// they are the layer's: the table is the generated `windows-1252`, checked
/// against Python's codec registry in both directions, and the holes rule is
/// the one `SingleByte::transcribe_sink` already applies to an unassigned
/// byte of any Windows page. `utf8_chunks()` never puts a byte below `0x80`
/// in an invalid run, so that walk is one table entry per byte. The walk is
/// byte-wise over the whole buffer and never the chunked
/// [`Decoder`](super::Decoder), which would hold a sequence the buffer cuts
/// short as pending: `E2 82` at the end is the two bytes that are left, and
/// they read `â‚`.
pub(crate) fn utf8_transcribe_into(input: &[u8], target: &mut String) -> usize {
    // A `String` re-checks each run it is handed, and every run here is one
    // `utf8_chunks` proved or one the table walked scalar by scalar, so the
    // refusal the sink trait lets a target spell cannot arrive. This door
    // exists because the trait is the module's and the reading is the crate's.
    utf8_transcribe_sink(input, target).expect("a run utf8_chunks proved is text by construction")
}

/// [`utf8_transcribe_into`] over whichever target a caller brought.
///
/// # Errors
///
/// Returns the refusal of a target that re-checks a run, which a run
/// `utf8_chunks` proved never trips.
pub(super) fn utf8_transcribe_sink(input: &[u8], target: &mut impl Utf8Sink) -> Result<usize> {
    // Every byte answers at least one UTF-8 byte and a stray byte at most
    // three, so the input length is a floor: reserved once here, and the
    // table's walk reserves its exact answer for each run it reads.
    target.reserve(input.len());
    let mut read = 0;
    for chunk in input.utf8_chunks() {
        target.push_utf8(chunk.valid().as_bytes())?;
        let invalid = chunk.invalid();
        if !invalid.is_empty() {
            CP1252.transcribe_sink(invalid, target)?;
            read += invalid.len();
        }
    }
    Ok(read)
}

/// Turn a UTF-8 validation failure into this crate's located refusal.
fn utf8_fault(input: &[u8], error: std::str::Utf8Error) -> Error {
    let position = input.len().min(error.valid_up_to());
    match error.error_len() {
        Some(_) => super::undecodable(
            UTF8,
            position,
            input.get(position).copied().unwrap_or_default(),
        ),
        None => truncated(UTF8, position, "a whole UTF-8 sequence"),
    }
}

/// Read one UTF-16 code unit in the order `BIG` selects.
const fn unit<const BIG: bool>(low: u8, high: u8) -> u16 {
    if BIG {
        u16::from_be_bytes([low, high])
    } else {
        u16::from_le_bytes([low, high])
    }
}

/// The canonical name of the UTF-16 order `BIG` selects.
const fn utf16_name<const BIG: bool>() -> &'static str {
    if BIG { UTF16BE } else { UTF16LE }
}

/// Decode UTF-16 into a target, replacing broken sequences when `LOSSY`.
///
/// Pairing is the standard library's: this walks `char::decode_utf16` and
/// tracks the byte position itself, because a surrogate pair is two code units
/// and an error has to name where in the input it sits.
pub(super) fn utf16_decode_into<const BIG: bool, const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    let name = utf16_name::<BIG>();
    // One code unit is at most three UTF-8 bytes, and a surrogate pair is four
    // bytes in and four out, so half again the input bounds every case.
    target.reserve(input.len() + input.len() / 2);

    let units = input.chunks_exact(2);
    let remainder = units.remainder();
    let mut buffer = [0_u8; 4];
    let mut position = 0;
    for scalar in char::decode_utf16(units.map(|pair| unit::<BIG>(pair[0], pair[1]))) {
        match scalar {
            Ok(scalar) => {
                target.push_scalar(scalar, scalar.encode_utf8(&mut buffer).as_bytes());
                position += scalar.len_utf16() * 2;
            }
            Err(error) => {
                if !LOSSY {
                    return Err(unpaired(name, position, error.unpaired_surrogate()));
                }
                target.push_scalar(REPLACEMENT, REPLACEMENT_UTF8);
                position += 2;
            }
        }
    }

    if !remainder.is_empty() {
        if !LOSSY {
            return Err(truncated(name, position, "a whole 16-bit code unit"));
        }
        target.push_scalar(REPLACEMENT, REPLACEMENT_UTF8);
    }
    Ok(())
}

/// Decode a complete UTF-16 buffer.
pub(super) fn utf16_decode<const BIG: bool>(input: &[u8]) -> Result<Cow<'static, str>> {
    let mut target = String::new();
    utf16_decode_into::<BIG, false>(input, &mut target)?;
    Ok(Cow::Owned(target))
}

/// Decode a complete UTF-16 buffer, replacing broken sequences.
pub(super) fn utf16_decode_lossy<const BIG: bool>(input: &[u8]) -> Cow<'static, str> {
    let mut target = String::new();
    match utf16_decode_into::<BIG, true>(input, &mut target) {
        // A lossy UTF-16 decode replaces every fault it can meet.
        Ok(()) => Cow::Owned(target),
        Err(_) => Cow::Owned(String::from(REPLACEMENT)),
    }
}

/// How many trailing bytes of `input` begin a UTF-16 sequence it cuts short.
///
/// Two things can be cut short: a code unit split across the boundary, and a
/// high surrogate whose low half has not arrived.
pub(super) fn utf16_pending<const BIG: bool>(input: &[u8]) -> usize {
    let odd = input.len() % 2;
    let units = input.len() - odd;
    let trailing_high = units >= 2
        && input
            .get(units - 2..units)
            .is_some_and(|pair| (0xD800..0xDC00).contains(&unit::<BIG>(pair[0], pair[1])));
    odd + usize::from(trailing_high) * 2
}

/// Encode text as UTF-16 in the order `BIG` selects.
///
/// Every Unicode scalar has a UTF-16 encoding, so this never refuses input.
pub(super) fn utf16_encode_into<const BIG: bool>(input: &str, target: &mut Vec<u8>) {
    target.reserve(input.len() * 2);
    let mut buffer = [0_u16; 2];
    for scalar in input.chars() {
        for code in scalar.encode_utf16(&mut buffer) {
            target.extend_from_slice(&if BIG {
                code.to_be_bytes()
            } else {
                code.to_le_bytes()
            });
        }
    }
}
