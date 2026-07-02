//! Private helpers shared by the buffer types: MSB-first bit access and a checked
//! bounds computation.

use super::IOError;

/// Read bit `index` (MSB-first, so bit `0` of a byte is its most significant bit).
/// The caller guarantees `index / 8` is in bounds.
fn get_bit(data: &[u8], index: usize) -> bool {
    (data[index / 8] >> (7 - index % 8)) & 1 == 1
}

/// Set bit `index` (MSB-first) to `value`. The caller guarantees `index / 8` exists.
fn set_bit(data: &mut [u8], index: usize, value: bool) {
    let mask = 1u8 << (7 - index % 8);
    if value {
        data[index / 8] |= mask;
    } else {
        data[index / 8] &= !mask;
    }
}

/// Read `size` bits starting at bit `start` (MSB-first). Byte-aligned runs unpack a
/// whole byte at a time instead of re-deriving the byte index per bit.
pub(super) fn read_bits(data: &[u8], start: usize, size: usize) -> Vec<bool> {
    let mut out = Vec::with_capacity(size);
    let end = start + size;
    let mut i = start;
    while i < end && !i.is_multiple_of(8) {
        out.push(get_bit(data, i));
        i += 1;
    }
    while i + 8 <= end {
        let byte = data[i / 8];
        // Eight independent mask tests and one extend: no per-bit capacity check
        // and no serial shift chain, so the unpack pipelines.
        out.extend_from_slice(&[
            byte & 0x80 != 0,
            byte & 0x40 != 0,
            byte & 0x20 != 0,
            byte & 0x10 != 0,
            byte & 0x08 != 0,
            byte & 0x04 != 0,
            byte & 0x02 != 0,
            byte & 0x01 != 0,
        ]);
        i += 8;
    }
    while i < end {
        out.push(get_bit(data, i));
        i += 1;
    }
    out
}

/// Write `values` starting at bit `start` (MSB-first). Byte-aligned runs pack eight
/// bits and store the whole byte, instead of read-modify-writing each bit.
pub(super) fn write_bits(data: &mut [u8], start: usize, values: &[bool]) {
    let mut i = 0;
    while i < values.len() && !(start + i).is_multiple_of(8) {
        set_bit(data, start + i, values[i]);
        i += 1;
    }
    while i + 8 <= values.len() {
        let mut byte = 0u8;
        for bit in 0..8 {
            byte = (byte << 1) | values[i + bit] as u8;
        }
        data[(start + i) / 8] = byte;
        i += 8;
    }
    while i < values.len() {
        set_bit(data, start + i, values[i]);
        i += 1;
    }
}

/// `base + position`, guarded against overflow; an [`IOError::OutOfBounds`] on wrap.
/// Used to resolve a `Whence`-relative offset without silently wrapping.
pub(super) fn offset(base: usize, position: usize) -> Result<usize, IOError> {
    base.checked_add(position).ok_or(IOError::OutOfBounds {
        offset: base.saturating_add(position),
        len: base,
    })
}

/// `start + size`, guarded against overflow and required to be `<= limit`; otherwise
/// an [`IOError::OutOfBounds`].
pub(super) fn checked_end(start: usize, size: usize, limit: usize) -> Result<usize, IOError> {
    match start.checked_add(size) {
        Some(end) if end <= limit => Ok(end),
        _ => Err(IOError::OutOfBounds {
            offset: start.saturating_add(size),
            len: limit,
        }),
    }
}
