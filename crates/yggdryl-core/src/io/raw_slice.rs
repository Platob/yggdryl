//! The [`RawIOSlice`] adapter: a bounded `[start, end)` byte window over a
//! [`RawIOBase`] resource.

use super::bits;
use super::{IOError, RawIOBase, Whence};

/// A bounded byte window `[start, end)` over a [`RawIOBase`] resource: it owns the
/// resource and restricts every access to that byte range, so code handed the slice
/// sees a smaller resource and cannot read or write outside it.
///
/// The wrapped resource is reached again with [`get_ref`](RawIOSlice::get_ref),
/// [`get_mut`](RawIOSlice::get_mut) or [`into_inner`](RawIOSlice::into_inner).
/// Positions are window-relative: [`Whence::Start`] is the window's first byte,
/// [`Whence::End`] its last backed byte, and — the slice keeps no cursor —
/// [`Whence::Current`] is measured from the start. `start` and `end` are byte
/// offsets, so bit access is offset by `start * 8`.
///
/// [`byte_size`](RawIOBase::byte_size) reports the backed length within the window
/// (`min(inner size, end) - start`), so reads within it always succeed; writes may
/// grow the inner up to `end` but never past it, and
/// [`resize_bytes`](RawIOBase::resize_bytes) moves the `end` bound (growing the
/// inner to back it, never truncating data outside the window).
///
/// ```
/// use yggdryl_core::{ByteBuffer, RawIOBase, RawIOSlice, Whence};
///
/// fn main() {
///     let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40, 50]), 1, 4);
///     // The window is bytes [1, 4): 20, 30, 40.
///     assert_eq!(slice.byte_size(), 3);
///     assert_eq!(slice.pread_byte_array(0, Whence::Start, 3).unwrap(), vec![20, 30, 40]);
///     // Positions are window-relative; reading past the window fails.
///     assert!(slice.pread_byte_one(3, Whence::Start).is_err());
///     // Writes stay within the window and reach the underlying buffer.
///     slice.pwrite_byte_one(0, Whence::Start, 99).unwrap();
///     assert_eq!(slice.get_ref().as_bytes(), &[10, 99, 30, 40, 50]);
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawIOSlice<I> {
    inner: I,
    start: usize,
    end: usize,
}

impl<I> RawIOSlice<I> {
    /// Wrap `inner`, restricting access to the byte window `[start, end)`. An `end`
    /// below `start` is clamped up to `start` (an empty window).
    pub fn new(inner: I, start: usize, end: usize) -> Self {
        Self {
            inner,
            start,
            end: end.max(start),
        }
    }

    /// A shared reference to the wrapped resource.
    pub fn get_ref(&self) -> &I {
        &self.inner
    }

    /// A mutable reference to the wrapped resource.
    pub fn get_mut(&mut self) -> &mut I {
        &mut self.inner
    }

    /// Consume the slice, returning the wrapped resource.
    pub fn into_inner(self) -> I {
        self.inner
    }

    /// The window's start byte offset into the wrapped resource.
    pub fn start(&self) -> usize {
        self.start
    }

    /// The window's end byte offset (exclusive) into the wrapped resource.
    pub fn end(&self) -> usize {
        self.end
    }
}

impl<I: RawIOBase> RawIOSlice<I> {
    /// The absolute byte offset one past the window's backed data, clamped to the
    /// window: `min(inner size, end)` but never below `start`. Clamping up to `start`
    /// keeps the [`Whence::End`] base inside `[start, end]` even when the window
    /// begins past the inner's current data, so an `End`-relative access can never
    /// escape below `start`.
    fn backed_end(&self) -> usize {
        self.inner.byte_size().min(self.end).max(self.start)
    }

    /// Resolve a byte `position`/`whence` to an absolute offset in the inner,
    /// measuring [`Whence::End`] from the backed end and everything else (the slice
    /// has no cursor) from `start`.
    fn byte_abs(&self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let base = match whence {
            Whence::End => self.backed_end(),
            Whence::Current => {
                crate::log_event!(
                    warn,
                    "RawIOSlice has no cursor; Whence::Current measured from the window start"
                );
                self.start
            }
            _ => self.start,
        };
        bits::offset(base, position)
    }

    /// Resolve a bit `position`/`whence` to an absolute bit offset in the inner.
    fn bit_abs(&self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let base = match whence {
            Whence::End => self.backed_end().saturating_mul(8),
            Whence::Current => {
                crate::log_event!(
                    warn,
                    "RawIOSlice has no cursor; Whence::Current measured from the window start"
                );
                self.start.saturating_mul(8)
            }
            _ => self.start.saturating_mul(8),
        };
        bits::offset(base, position)
    }
}

impl<I: RawIOBase> RawIOBase for RawIOSlice<I> {
    fn byte_size(&self) -> usize {
        self.backed_end().saturating_sub(self.start)
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.end = bits::offset(self.start, size)?;
        if self.inner.byte_size() < self.end {
            self.inner.resize_bytes(self.end)?;
        }
        crate::log_event!(debug, "RawIOSlice::resize_bytes -> {size}");
        Ok(())
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.byte_abs(position, whence)?;
        bits::checked_end(start, size, self.backed_end())?;
        self.inner.pread_byte_array(start, Whence::Start, size)
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        if values.is_empty() {
            return Ok(());
        }
        let start = self.byte_abs(position, whence)?;
        let write_end = bits::offset(start, values.len())?;
        if write_end > self.end {
            return Err(IOError::OutOfBounds {
                offset: write_end,
                len: self.end,
            });
        }
        self.inner.pwrite_byte_array(start, Whence::Start, values)
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        let start = self.bit_abs(position, whence)?;
        bits::checked_end(start, size, self.backed_end().saturating_mul(8))?;
        self.inner.pread_bit_array(start, Whence::Start, size)
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        if values.is_empty() {
            return Ok(());
        }
        let start = self.bit_abs(position, whence)?;
        let write_end = bits::offset(start, values.len())?;
        let limit = self.end.saturating_mul(8);
        if write_end > limit {
            return Err(IOError::OutOfBounds {
                offset: write_end,
                len: limit,
            });
        }
        self.inner.pwrite_bit_array(start, Whence::Start, values)
    }
}
