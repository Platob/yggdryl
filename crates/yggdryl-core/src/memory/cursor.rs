//! [`IOCursor`] — a moving read/write position layered over [`IOBase`].

use super::IoError;
use super::{IOBase, Whence};

/// An [`IOBase`] with a **cursor**: a current position that [`read`](IOCursor::read) /
/// [`write`](IOCursor::write) advance, and [`seek`](IOCursor::seek) moves by a signed offset
/// from a [`Whence`]. The stream-style methods are the cursor counterparts of `IOBase`'s
/// positioned ones — `read` is `pread_byte_array` at the cursor, `write` is `pwrite_byte_array`
/// at the cursor — each stepping the position forward by the number of bytes moved. The typed
/// accessors ([`read_byte`](IOCursor::read_byte) / [`read_i32`](IOCursor::read_i32) /
/// [`read_i64`](IOCursor::read_i64) and their `write_*` twins) read/write a little-endian value
/// at the cursor and advance past it.
///
/// DESIGN: the cursor is **byte-addressed**, so it has no `read_bit` — bit access is positioned
/// only, via [`IOBase::pread_bit`] / [`IOBase::pwrite_bit`] with an absolute bit offset.
///
/// Only [`position`](IOCursor::position) and [`set_position`](IOCursor::set_position) are
/// required; everything else has a default built on them and the `IOBase` methods.
pub trait IOCursor: IOBase {
    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    fn position(&self) -> u64;

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    fn set_position(&mut self, position: u64);

    /// **Seeks** to `whence + offset` and returns the new position. A position past the end
    /// is allowed; seeking before the start is an [`IoError::InvalidSeek`].
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor, Whence};
    ///
    /// let mut data = Heap::from_slice(b"hello world");
    /// assert_eq!(data.seek(Whence::Start, 6).unwrap(), 6);
    /// assert_eq!(data.seek(Whence::End, -5).unwrap(), 6);
    /// assert!(data.seek(Whence::Start, -1).is_err());
    /// ```
    fn seek(&mut self, whence: Whence, offset: i64) -> Result<u64, IoError> {
        let position = whence.resolve(offset, self.position(), self.byte_size())?;
        self.set_position(position);
        Ok(position)
    }

    /// Resets the cursor to the start (`seek(Start, 0)` without the error path).
    fn rewind(&mut self) {
        self.set_position(0);
    }

    /// **Cursor read.** Reads up to `buf.len()` bytes from the current position, advancing it
    /// by the number read; returns that count (`0` at the end).
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor, Whence};
    ///
    /// let mut data = Heap::from_slice(b"hello world");
    /// data.seek(Whence::Start, 6).unwrap();
    /// let mut buf = [0u8; 5];
    /// assert_eq!(data.read(&mut buf), 5);
    /// assert_eq!(&buf, b"world");
    /// assert_eq!(data.position(), 11);
    /// ```
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let read = self.pread_byte_array(self.position(), buf);
        self.set_position(self.position() + read as u64);
        read
    }

    /// **Cursor write.** Writes `data` at the current position, advancing it by the number
    /// written (growing the storage as needed); returns that count (always `data.len()`).
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor};
    ///
    /// let mut data = Heap::new();
    /// assert_eq!(data.write(b"hello"), 5);
    /// assert_eq!(data.write(b" world"), 6);
    /// assert_eq!(data.as_slice(), b"hello world");
    /// ```
    fn write(&mut self, data: &[u8]) -> usize {
        let position = self.position();
        let written = self.pwrite_byte_array(position, data);
        self.set_position(position + written as u64);
        written
    }

    /// **Full cursor read** — fills all of `buf` from the cursor, advancing it, or errors
    /// with [`IoError::UnexpectedEof`] (leaving the cursor put).
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), IoError> {
        let position = self.position();
        self.pread_exact(position, buf)?;
        self.set_position(position + buf.len() as u64);
        Ok(())
    }

    /// **Full cursor write** of all of `data` from the cursor, advancing it.
    fn write_all(&mut self, data: &[u8]) -> Result<(), IoError> {
        let position = self.position();
        self.pwrite_all(position, data)?;
        self.set_position(position + data.len() as u64);
        Ok(())
    }

    /// Reads the next byte at the cursor, advancing it by 1, or errors with
    /// [`IoError::UnexpectedEof`] at the end.
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor};
    ///
    /// let mut data = Heap::from_slice(b"ab");
    /// assert_eq!(data.read_byte().unwrap(), b'a');
    /// assert_eq!(data.read_byte().unwrap(), b'b');
    /// assert!(data.read_byte().is_err());
    /// ```
    fn read_byte(&mut self) -> Result<u8, IoError> {
        let position = self.position();
        let value = self.pread_byte(position)?;
        self.set_position(position + 1);
        Ok(value)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    fn write_byte(&mut self, value: u8) -> Result<(), IoError> {
        let position = self.position();
        self.pwrite_byte(position, value)?;
        self.set_position(position + 1);
        Ok(())
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor};
    ///
    /// let mut data = Heap::new();
    /// data.write_i32(-7).unwrap();
    /// data.rewind();
    /// assert_eq!(data.read_i32().unwrap(), -7);
    /// ```
    fn read_i32(&mut self) -> Result<i32, IoError> {
        let position = self.position();
        let value = self.pread_i32(position)?;
        self.set_position(position + 4);
        Ok(value)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    fn write_i32(&mut self, value: i32) -> Result<(), IoError> {
        let position = self.position();
        self.pwrite_i32(position, value)?;
        self.set_position(position + 4);
        Ok(())
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor};
    ///
    /// let mut data = Heap::new();
    /// data.write_i64(1 << 40).unwrap();
    /// data.rewind();
    /// assert_eq!(data.read_i64().unwrap(), 1 << 40);
    /// ```
    fn read_i64(&mut self) -> Result<i64, IoError> {
        let position = self.position();
        let value = self.pread_i64(position)?;
        self.set_position(position + 8);
        Ok(value)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    fn write_i64(&mut self, value: i64) -> Result<(), IoError> {
        let position = self.position();
        self.pwrite_i64(position, value)?;
        self.set_position(position + 8);
        Ok(())
    }

    /// Reads up to `len` bytes from the current position into a fresh `Vec` (short near the
    /// end), advancing the cursor by the number read — the owning counterpart of
    /// [`read`](IOCursor::read) for callers that want the bytes returned rather than filled
    /// into a buffer they supply.
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor};
    ///
    /// let mut data = Heap::from_slice(b"hello world");
    /// assert_eq!(data.read_vec(5), b"hello");
    /// assert_eq!(data.read_vec(100), b" world"); // clamped to what remains
    /// ```
    fn read_vec(&mut self, len: usize) -> Vec<u8> {
        let out = self.pread_vec(self.position(), len);
        self.set_position(self.position() + out.len() as u64);
        out
    }

    /// Reads **exactly** `len` bytes into a fresh `Vec`, advancing the cursor, or errors with
    /// [`IoError::UnexpectedEof`]. Unlike `vec![0u8; len]` + [`read_exact`](IOCursor::read_exact),
    /// it **caps the working allocation** (64 KiB) and grows only as bytes are actually delivered —
    /// so a corrupt/hostile declared length errors once the (short) source is exhausted instead of
    /// aborting the process on a giant up-front allocation. The bounded counterpart of
    /// [`read_vec`](IOCursor::read_vec), which clamps to what remains rather than requiring `len`.
    fn read_exact_vec(&mut self, len: usize) -> Result<Vec<u8>, IoError> {
        const CHUNK: usize = 64 * 1024;
        let mut out = Vec::with_capacity(len.min(CHUNK));
        let mut buf = vec![0u8; len.clamp(1, CHUNK)];
        let mut remaining = len;
        while remaining > 0 {
            let take = remaining.min(buf.len());
            self.read_exact(&mut buf[..take])?;
            out.extend_from_slice(&buf[..take]);
            remaining -= take;
        }
        Ok(out)
    }

    /// Reads from the current position **to the end** into a fresh `Vec`, advancing the
    /// cursor to the end. One pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::memory::{Heap, IOCursor, Whence};
    ///
    /// let mut data = Heap::from_slice(b"hello world");
    /// data.seek(Whence::Start, 6).unwrap();
    /// assert_eq!(data.read_to_end(), b"world");
    /// assert_eq!(data.position(), 11);
    /// ```
    fn read_to_end(&mut self) -> Vec<u8> {
        let position = self.position();
        let remaining = self.byte_size().saturating_sub(position);
        let out = self.pread_vec(position, remaining as usize);
        self.set_position(self.byte_size());
        out
    }
}
