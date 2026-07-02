//! The [`RawIOCursor`] adapter: a moving cursor over any [`RawIOBase`] resource.

use std::cell::Cell;

use super::bits;
use super::{IOError, RawIOBase, Seekable, Whence};

/// A moving cursor over a [`RawIOBase`] resource: it owns the resource and a
/// position, and every positioned read or write advances the position past the
/// bytes (or bits) it touched — turning random-access I/O into a sequential
/// stream.
///
/// The wrapped resource is reached again with [`get_ref`](RawIOCursor::get_ref),
/// [`get_mut`](RawIOCursor::get_mut) or [`into_inner`](RawIOCursor::into_inner).
/// [`Whence::Current`] is measured from the cursor; [`seek`](Seekable::seek) moves
/// it without touching the data, and [`tell`](Seekable::tell) reports it in bytes.
///
/// The position is tracked in bits so byte- and bit-granular access share one
/// cursor; [`tell`](Seekable::tell) floors it to the enclosing byte and
/// [`seek`](Seekable::seek) lands on a byte boundary. Because a read must advance
/// the cursor through a `&self` method, the position lives behind a [`Cell`].
///
/// The inherited [`pread_io`](RawIOBase::pread_io) / [`pwrite_io`](RawIOBase::pwrite_io)
/// streams address each chunk absolutely, so when a cursor is a stream endpoint use
/// [`Whence::Start`] or [`Whence::End`]; [`Whence::Current`] is resolved per chunk
/// and is not meaningful there.
///
/// ```
/// use yggdryl_core::{ByteBuffer, RawIOBase, RawIOCursor, Seekable, Whence};
///
/// let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40]));
/// // Each read starts where the last one stopped.
/// assert_eq!(cursor.pread_byte_array(0, Whence::Current, 2)?, vec![10, 20]);
/// assert_eq!(cursor.tell(), 2);
/// assert_eq!(cursor.pread_byte_array(0, Whence::Current, 2)?, vec![30, 40]);
/// assert_eq!(cursor.tell(), 4);
///
/// // Writes advance it too; the wrapped buffer is still reachable.
/// cursor.seek(0, Whence::Start)?;
/// cursor.pwrite_byte_array(0, Whence::Current, &[1, 2])?;
/// assert_eq!(cursor.tell(), 2);
/// assert_eq!(cursor.get_ref().as_bytes(), &[1, 2, 30, 40]);
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawIOCursor<I> {
    inner: I,
    // The cursor position, in bits from the start, so byte and bit access share it.
    cursor: Cell<usize>,
}

impl<I> RawIOCursor<I> {
    /// Wrap `inner`, with the cursor at the start.
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            cursor: Cell::new(0),
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

    /// Consume the cursor, returning the wrapped resource.
    pub fn into_inner(self) -> I {
        self.inner
    }
}

impl<I: RawIOBase> RawIOCursor<I> {
    /// Resolve a byte `position`/`whence` to an absolute byte offset, measuring
    /// [`Whence::Current`] from the cursor (floored to the enclosing byte).
    fn byte_start(&self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let base = match whence {
            Whence::Current => self.cursor.get() / 8,
            Whence::End => self.inner.byte_size(),
            _ => 0,
        };
        bits::offset(base, position)
    }

    /// Resolve a bit `position`/`whence` to an absolute bit offset, measuring
    /// [`Whence::Current`] from the cursor.
    fn bit_start(&self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let base = match whence {
            Whence::Current => self.cursor.get(),
            Whence::End => self.inner.bit_size(),
            _ => 0,
        };
        bits::offset(base, position)
    }

    /// Move the cursor to just past `count` bytes starting at byte `start`.
    fn advance_bytes(&self, start: usize, count: usize) {
        self.cursor
            .set(start.saturating_add(count).saturating_mul(8));
        crate::log_event!(trace, "RawIOCursor -> bit {}", self.cursor.get());
    }

    /// Move the cursor to just past `count` bits starting at bit `start`.
    fn advance_bits(&self, start: usize, count: usize) {
        self.cursor.set(start.saturating_add(count));
        crate::log_event!(trace, "RawIOCursor -> bit {}", self.cursor.get());
    }
}

impl<I: RawIOBase> Seekable for RawIOCursor<I> {
    fn tell(&self) -> usize {
        self.cursor.get() / 8
    }

    fn seek(&mut self, position: usize, whence: Whence) -> Result<usize, IOError> {
        let target = self.byte_start(position, whence)?;
        self.cursor.set(target.saturating_mul(8));
        crate::log_event!(debug, "RawIOCursor::seek -> {target}");
        Ok(target)
    }
}

impl<I: RawIOBase> RawIOBase for RawIOCursor<I> {
    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }

    fn bit_size(&self) -> usize {
        self.inner.bit_size()
    }

    fn byte_capacity(&self) -> usize {
        self.inner.byte_capacity()
    }

    fn bit_capacity(&self) -> usize {
        self.inner.bit_capacity()
    }

    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.inner.resize_byte_capacity(capacity)
    }

    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.inner.resize_bit_capacity(capacity)
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.inner.resize_bytes(size)
    }

    fn resize_bits(&mut self, size: usize) -> Result<(), IOError> {
        self.inner.resize_bits(size)
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.byte_start(position, whence)?;
        let out = self.inner.pread_byte_array(start, Whence::Start, size)?;
        if !out.is_empty() {
            self.advance_bytes(start, out.len());
        }
        Ok(out)
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
        let start = self.byte_start(position, whence)?;
        self.inner.pwrite_byte_array(start, Whence::Start, values)?;
        self.advance_bytes(start, values.len());
        Ok(())
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        let start = self.bit_start(position, whence)?;
        let out = self.inner.pread_bit_array(start, Whence::Start, size)?;
        if !out.is_empty() {
            self.advance_bits(start, out.len());
        }
        Ok(out)
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
        let start = self.bit_start(position, whence)?;
        self.inner.pwrite_bit_array(start, Whence::Start, values)?;
        self.advance_bits(start, values.len());
        Ok(())
    }
}
