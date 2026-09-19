//! UTF-16 in both byte orders: the one charset family here that does not
//! agree with US-ASCII below `0x80`, so nothing in it borrows.

use std::borrow::Cow;

use super::sink::Utf8Sink;
use super::{REPLACEMENT, REPLACEMENT_UTF8, truncated, unpaired};
use crate::Result;

/// The canonical name little-endian UTF-16 reports itself by.
pub(super) const UTF16LE: &str = "utf-16le";
/// The canonical name big-endian UTF-16 reports itself by.
pub(super) const UTF16BE: &str = "utf-16be";

/// Read one UTF-16 code unit in the order `BIG` selects.
const fn unit<const BIG: bool>(low: u8, high: u8) -> u16 {
    if BIG {
        u16::from_be_bytes([low, high])
    } else {
        u16::from_le_bytes([low, high])
    }
}

/// The canonical name of the UTF-16 order `BIG` selects.
const fn name<const BIG: bool>() -> &'static str {
    if BIG { UTF16BE } else { UTF16LE }
}

/// Decode UTF-16 into a target, replacing broken sequences when `LOSSY`.
///
/// Pairing is the standard library's: this walks `char::decode_utf16` and
/// tracks the byte position itself, because a surrogate pair is two code units
/// and an error has to name where in the input it sits.
pub(super) fn decode_into<const BIG: bool, const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    let name = name::<BIG>();
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
pub(super) fn decode<const BIG: bool>(input: &[u8]) -> Result<Cow<'static, str>> {
    let mut target = String::new();
    decode_into::<BIG, false>(input, &mut target)?;
    Ok(Cow::Owned(target))
}

/// Decode a complete UTF-16 buffer, replacing broken sequences.
pub(super) fn decode_lossy<const BIG: bool>(input: &[u8]) -> Cow<'static, str> {
    let mut target = String::new();
    match decode_into::<BIG, true>(input, &mut target) {
        // A lossy UTF-16 decode replaces every fault it can meet.
        Ok(()) => Cow::Owned(target),
        Err(_) => Cow::Owned(String::from(REPLACEMENT)),
    }
}

/// How many trailing bytes of `input` begin a UTF-16 sequence it cuts short.
///
/// Two things can be cut short: a code unit split across the boundary, and a
/// high surrogate whose low half has not arrived.
pub(super) fn pending<const BIG: bool>(input: &[u8]) -> usize {
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
pub(super) fn encode_into<const BIG: bool>(input: &str, target: &mut Vec<u8>) {
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
