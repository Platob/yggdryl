//! Private helpers shared by the buffer types: MSB-first bit access and a checked
//! bounds computation.

use super::IOError;

/// Read bit `index` (MSB-first, so bit `0` of a byte is its most significant bit).
/// The caller guarantees `index / 8` is in bounds.
pub(super) fn get_bit(data: &[u8], index: usize) -> bool {
    (data[index / 8] >> (7 - index % 8)) & 1 == 1
}

/// Set bit `index` (MSB-first) to `value`. The caller guarantees `index / 8` exists.
pub(super) fn set_bit(data: &mut [u8], index: usize, value: bool) {
    let mask = 1u8 << (7 - index % 8);
    if value {
        data[index / 8] |= mask;
    } else {
        data[index / 8] &= !mask;
    }
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
