//! [`Bitmap`] — a bit-packed validity mask, shared by the fixed [`Serie`](super::fixed::Serie)
//! and the variable-length columns. Crate-internal: it is the *shape* of a column's nulls, not
//! a public type.

/// A bit-packed validity mask (LSB-first within each byte): bit `i` set = element `i` is
/// present. Byte-identical to Arrow's null buffer (`1 = valid`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bitmap {
    bits: Vec<u8>,
    len: usize,
}

impl Bitmap {
    /// A mask of `len` bits all set (all present).
    pub(crate) fn all_present(len: usize) -> Self {
        let mut bits = vec![0xffu8; len.div_ceil(8)];
        // Clear the unused high bits of the last byte so `null_count` / round-trips are exact.
        if !len.is_multiple_of(8) {
            if let Some(last) = bits.last_mut() {
                *last = (1u16 << (len % 8)).wrapping_sub(1) as u8;
            }
        }
        Self { bits, len }
    }

    /// Whether element `index` is present.
    pub(crate) fn get(&self, index: usize) -> bool {
        index < self.len && (self.bits[index / 8] >> (index % 8)) & 1 == 1
    }

    /// Appends one presence bit.
    pub(crate) fn push(&mut self, present: bool) {
        if self.len.is_multiple_of(8) {
            self.bits.push(0);
        }
        if present {
            self.bits[self.len / 8] |= 1 << (self.len % 8);
        }
        self.len += 1;
    }

    /// Sets element `index`'s presence bit (in range; a no-op mask stays canonical). Used by the
    /// in-place column `set` operations.
    pub(crate) fn set(&mut self, index: usize, present: bool) {
        if index >= self.len {
            return;
        }
        let (byte, bit) = (index / 8, index % 8);
        if present {
            self.bits[byte] |= 1 << bit;
        } else {
            self.bits[byte] &= !(1 << bit);
        }
    }

    /// The number of null (unset) bits in `[0, len)`.
    pub(crate) fn null_count(&self) -> usize {
        (0..self.len).filter(|&i| !self.get(i)).count()
    }

    /// The packed bytes.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    /// Rebuilds a mask of `len` bits from packed `bytes` (padded / truncated to fit).
    pub(crate) fn from_bytes(bytes: &[u8], len: usize) -> Self {
        let mut bits = bytes.to_vec();
        bits.resize(len.div_ceil(8), 0);
        // Clear the unused high bits of the last byte (a foreign/corrupt writer may leave them
        // set), so the canonical form stays canonical and equality/round-trips are exact — the
        // padding bits are logically invisible to `get` / `null_count`.
        if !len.is_multiple_of(8) {
            if let Some(last) = bits.last_mut() {
                *last &= (1u16 << (len % 8)).wrapping_sub(1) as u8;
            }
        }
        Self { bits, len }
    }
}

/// Appends `added` presence bits to a column's optional validity mask that currently describes
/// `current_len` present-or-null elements. `present(offset)` gives the presence of the `offset`-th
/// appended element. The mask is materialized **lazily** — created only when an appended element is
/// null (or it already exists) — so a fully-present append onto a null-free column leaves it
/// mask-free (canonical, matching `from_options`). Shared by every leaf serie's `extend_*` / `concat`
/// grow path, so the validity grows in lock-step with the values in one pass.
pub(crate) fn extend_validity(
    validity: &mut Option<Bitmap>,
    current_len: usize,
    added: usize,
    mut present: impl FnMut(usize) -> bool,
) {
    for offset in 0..added {
        if present(offset) {
            if let Some(bitmap) = validity {
                bitmap.push(true);
            }
        } else {
            validity
                .get_or_insert_with(|| Bitmap::all_present(current_len + offset))
                .push(false);
        }
    }
}
