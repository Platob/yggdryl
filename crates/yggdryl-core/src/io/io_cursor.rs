//! [`IOCursor`] — a moving read/write position layered over [`IOBase`].

use super::{IOBase, IoError, Whence};

/// An [`IOBase`] with a **cursor**: a current position that [`read`](IOCursor::read) /
/// [`write`](IOCursor::write) advance, and [`seek`](IOCursor::seek) moves by a signed offset
/// from a [`Whence`]. The stream-style methods are the cursor counterparts of `IOBase`'s
/// positioned ones — `read` is `pread` at the cursor, `write` is `pwrite` at the cursor —
/// each stepping the position forward by the number of bytes moved.
///
/// Only [`position`](IOCursor::position) and [`set_position`](IOCursor::set_position) are
/// required; everything else has a default built on them and the `IOBase` primitives.
pub trait IOCursor: IOBase {
    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    fn position(&self) -> u64;

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    fn set_position(&mut self, position: u64);

    /// **Seeks** to `whence + offset` and returns the new position. A position past the end
    /// is allowed; seeking before the start is an [`IoError::InvalidSeek`].
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOCursor, Whence};
    ///
    /// let mut data = Bytes::from_slice(b"hello world");
    /// assert_eq!(data.seek(Whence::Start, 6).unwrap(), 6);
    /// assert_eq!(data.seek(Whence::End, -5).unwrap(), 6);
    /// assert!(data.seek(Whence::Start, -1).is_err());
    /// ```
    fn seek(&mut self, whence: Whence, offset: i64) -> Result<u64, IoError> {
        let position = whence.resolve(offset, self.position(), self.len())?;
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
    /// use yggdryl_core::io::{Bytes, IOCursor, Whence};
    ///
    /// let mut data = Bytes::from_slice(b"hello world");
    /// data.seek(Whence::Start, 6).unwrap();
    /// let mut buf = [0u8; 5];
    /// assert_eq!(data.read(&mut buf), 5);
    /// assert_eq!(&buf, b"world");
    /// assert_eq!(data.position(), 11);
    /// ```
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let read = self.pread(self.position(), buf);
        self.set_position(self.position() + read as u64);
        read
    }

    /// **Cursor write.** Writes `data` at the current position, advancing it by the number
    /// written (growing the storage as needed); returns that count (always `data.len()`).
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOCursor};
    ///
    /// let mut data = Bytes::new();
    /// assert_eq!(data.write(b"hello"), 5);
    /// assert_eq!(data.write(b" world"), 6);
    /// assert_eq!(data.as_slice(), b"hello world");
    /// ```
    fn write(&mut self, data: &[u8]) -> usize {
        let position = self.position();
        let written = self.pwrite(position, data);
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

    /// Reads up to `len` bytes from the current position into a fresh `Vec` (short near the
    /// end), advancing the cursor by the number read — the owning counterpart of
    /// [`read`](IOCursor::read) for callers that want the bytes returned rather than filled
    /// into a buffer they supply.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOCursor};
    ///
    /// let mut data = Bytes::from_slice(b"hello world");
    /// assert_eq!(data.read_vec(5), b"hello");
    /// assert_eq!(data.read_vec(100), b" world"); // clamped to what remains
    /// ```
    fn read_vec(&mut self, len: usize) -> Vec<u8> {
        let out = self.pread_vec(self.position(), len);
        self.set_position(self.position() + out.len() as u64);
        out
    }

    /// Reads from the current position **to the end** into a fresh `Vec`, advancing the
    /// cursor to the end. One pre-sized allocation.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOCursor, Whence};
    ///
    /// let mut data = Bytes::from_slice(b"hello world");
    /// data.seek(Whence::Start, 6).unwrap();
    /// assert_eq!(data.read_to_end(), b"world");
    /// assert_eq!(data.position(), 11);
    /// ```
    fn read_to_end(&mut self) -> Vec<u8> {
        let position = self.position();
        let remaining = self.len().saturating_sub(position);
        let out = self.pread_vec(position, remaining as usize);
        self.set_position(self.len());
        out
    }
}
