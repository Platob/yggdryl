//! [`IOCursor`] — a concrete moving read/write position over any [`IOBase`].

use super::{IOBase, IoError, Whence};

/// Generates the cursor read/write/seek surface as **inherent** methods, given a `self` that is
/// an [`IOBase`] carrying a `position: u64` field. Applied to both [`IOCursor`] (which adds a
/// cursor to *any* source) and [`Heap`](super::Heap) (which has a built-in one), so the two share
/// exactly one implementation of the stream operations.
macro_rules! cursor_methods {
    () => {
        /// The current cursor position (bytes from the start). May sit past the end after a seek.
        pub fn position(&self) -> u64 {
            self.position
        }

        /// Moves the cursor to an absolute `position` (past the end is allowed).
        pub fn set_position(&mut self, position: u64) {
            self.position = position;
        }

        /// **Seeks** to `whence + offset` and returns the new position. A position past the end is
        /// allowed; seeking before the start is an [`IoError::InvalidSeek`].
        pub fn seek(&mut self, whence: Whence, offset: i64) -> Result<u64, IoError> {
            let position = whence.resolve(offset, self.position, self.byte_size())?;
            self.position = position;
            Ok(position)
        }

        /// Resets the cursor to the start.
        pub fn rewind(&mut self) {
            self.position = 0;
        }

        /// **Cursor read.** Reads up to `buf.len()` bytes from the current position, advancing it
        /// by the number read; returns that count (`0` at the end).
        pub fn read(&mut self, buf: &mut [u8]) -> usize {
            let read = self.pread_byte_array(self.position, buf);
            self.position += read as u64;
            read
        }

        /// **Cursor write.** Writes `data` at the current position, advancing it by the number
        /// written (growing the storage as needed); returns that count (always `data.len()`).
        pub fn write(&mut self, data: &[u8]) -> usize {
            let written = self.pwrite_byte_array(self.position, data);
            self.position += written as u64;
            written
        }

        /// **Full cursor read** — fills all of `buf` from the cursor, advancing it, or errors with
        /// [`IoError::UnexpectedEof`] (leaving the cursor put).
        pub fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), IoError> {
            self.pread_exact(self.position, buf)?;
            self.position += buf.len() as u64;
            Ok(())
        }

        /// **Full cursor write** of all of `data` from the cursor, advancing it.
        pub fn write_all(&mut self, data: &[u8]) -> Result<(), IoError> {
            self.pwrite_all(self.position, data)?;
            self.position += data.len() as u64;
            Ok(())
        }

        /// Reads the next byte at the cursor, advancing it by 1, or errors with
        /// [`IoError::UnexpectedEof`] at the end.
        pub fn read_byte(&mut self) -> Result<u8, IoError> {
            let value = self.pread_byte(self.position)?;
            self.position += 1;
            Ok(value)
        }

        /// Writes the byte `value` at the cursor, advancing it by 1.
        pub fn write_byte(&mut self, value: u8) -> Result<(), IoError> {
            self.pwrite_byte(self.position, value)?;
            self.position += 1;
            Ok(())
        }

        /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, or errors with
        /// [`IoError::UnexpectedEof`].
        pub fn read_i32(&mut self) -> Result<i32, IoError> {
            let value = self.pread_i32(self.position)?;
            self.position += 4;
            Ok(value)
        }

        /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
        pub fn write_i32(&mut self, value: i32) -> Result<(), IoError> {
            self.pwrite_i32(self.position, value)?;
            self.position += 4;
            Ok(())
        }

        /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, or errors with
        /// [`IoError::UnexpectedEof`].
        pub fn read_i64(&mut self) -> Result<i64, IoError> {
            let value = self.pread_i64(self.position)?;
            self.position += 8;
            Ok(value)
        }

        /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
        pub fn write_i64(&mut self, value: i64) -> Result<(), IoError> {
            self.pwrite_i64(self.position, value)?;
            self.position += 8;
            Ok(())
        }

        /// Reads up to `len` **bytes** from the cursor and decodes them as UTF-8 text (clamped
        /// near the end), advancing the cursor by the bytes read, or errors with
        /// [`IoError::InvalidUtf8`] (leaving the cursor put).
        pub fn read_utf8(&mut self, len: usize) -> Result<String, IoError> {
            let text = self.pread_utf8(self.position, len)?;
            self.position += text.len() as u64;
            Ok(text)
        }

        /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
        /// **bytes** written.
        pub fn write_utf8(&mut self, text: &str) -> usize {
            let written = self.pwrite_utf8(self.position, text);
            self.position += written as u64;
            written
        }

        /// Reads up to `len` bytes from the current position into a fresh `Vec` (short near the
        /// end), advancing the cursor by the number read.
        pub fn read_vec(&mut self, len: usize) -> Vec<u8> {
            let out = self.pread_vec(self.position, len);
            self.position += out.len() as u64;
            out
        }

        /// Reads **exactly** `len` bytes into a fresh `Vec`, advancing the cursor, or errors with
        /// [`IoError::UnexpectedEof`]. Caps the working allocation (64 KiB) and grows only as
        /// bytes are actually delivered, so a corrupt/hostile length errors on the (short) source
        /// instead of aborting on a giant up-front allocation.
        pub fn read_exact_vec(&mut self, len: usize) -> Result<Vec<u8>, IoError> {
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

        /// Reads from the current position **to the end** into a fresh `Vec`, advancing the cursor
        /// to the end. One pre-sized allocation.
        pub fn read_to_end(&mut self) -> Vec<u8> {
            let remaining = self.byte_size().saturating_sub(self.position);
            let out = self.pread_vec(self.position, remaining as usize);
            self.position = self.byte_size();
            out
        }
    };
}
pub(crate) use cursor_methods;

/// A **cursor** over any [`IOBase`] source: it owns the source and a moving position that
/// [`read`](IOCursor::read) / [`write`](IOCursor::write) advance, and [`seek`](IOCursor::seek)
/// moves relative to a [`Whence`] anchor. It is the concrete counterpart to a source's positioned
/// primitives — build one from any source with [`IOBase::cursor`](super::IOBase::cursor).
///
/// `IOCursor<T>` is itself an [`IOBase`] (its positioned ops delegate to the wrapped source and
/// its [`uri`](super::IOBase::uri) is the source's), so a cursor can be windowed, re-cursored, or
/// used anywhere a source is. Owning the source keeps the type lifetime-free, so the bindings can
/// hold it; to keep the original, wrap a clone.
///
/// DESIGN: the cursor is **byte-addressed**, so it has no `read_bit` — bit access is positioned
/// only, via [`IOBase::pread_bit`](super::IOBase::pread_bit) with an absolute bit offset.
///
/// ```
/// use yggdryl_core::io::memory::{Heap, IOBase};
///
/// let mut cur = Heap::new().cursor(); // IOCursor<Heap>
/// cur.write_i32(-7).unwrap();
/// cur.write_i64(1 << 40).unwrap();
/// cur.rewind();
/// assert_eq!(cur.read_i32().unwrap(), -7);
/// assert_eq!(cur.read_i64().unwrap(), 1 << 40);
/// assert_eq!(cur.byte_size(), 12); // delegates to the wrapped source
/// ```
#[derive(Clone, Debug, Default)]
pub struct IOCursor<T: IOBase> {
    inner: T,
    /// The cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
}

impl<T: IOBase> IOCursor<T> {
    /// Wraps `inner` in a cursor positioned at the start.
    pub fn new(inner: T) -> Self {
        Self { inner, position: 0 }
    }

    /// Wraps `inner` in a cursor at an explicit `position`.
    pub fn with_position(inner: T, position: u64) -> Self {
        Self { inner, position }
    }

    /// Borrows the wrapped source.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Mutably borrows the wrapped source.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwraps into the source, discarding the cursor position.
    pub fn into_inner(self) -> T {
        self.inner
    }

    cursor_methods!();
}

impl<T: IOBase> IOBase for IOCursor<T> {
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    fn reserve(&mut self, additional: u64) {
        self.inner.reserve(additional);
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        self.inner.pread_byte_array(offset, buf)
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        self.inner.pwrite_byte_array(offset, data)
    }

    fn uri(&self) -> crate::uri::Uri {
        self.inner.uri()
    }

    fn headers(&self) -> &crate::headers::Headers {
        self.inner.headers()
    }

    fn headers_mut(&mut self) -> &mut crate::headers::Headers {
        self.inner.headers_mut()
    }

    fn mode(&self) -> crate::io::IOMode {
        self.inner.mode()
    }

    fn kind(&self) -> crate::io::IOKind {
        self.inner.kind()
    }
}
