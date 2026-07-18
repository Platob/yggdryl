//! [`IOCursor`] — a concrete moving read/write position over any [`IOBase`].

use super::{IOBase, IoError, Whence};

/// Emits a batch of **cursor scalar** read/write pairs — one `read_<t>` / `write_<t>` per
/// `(type, width, read, write, pread, pwrite)` tuple — each reading the positioned typed value at
/// the cursor and advancing it by the type's byte width. Invoked from inside
/// [`cursor_methods!`](cursor_methods) by absolute path (it re-exports at the `memory` module
/// level), so it resolves at every expansion site regardless of the module doing the wrapping.
macro_rules! cursor_scalar_pairs {
    ( $( ( $t:ty, $width:literal, $read:ident, $write:ident, $pread:ident, $pwrite:ident ) ),+ $(,)? ) => {
        $(
            #[doc = concat!("Reads a little-endian `", stringify!($t), "` at the cursor, advancing \
                it by ", stringify!($width), " bytes; errors with [`IoError::UnexpectedEof`] on EOF.")]
            pub fn $read(&mut self) -> Result<$t, IoError> {
                let value = self.$pread(self.position)?;
                self.position += $width;
                Ok(value)
            }
            #[doc = concat!("Writes `value` as a little-endian `", stringify!($t), "` at the \
                cursor, advancing it by ", stringify!($width), " bytes.")]
            pub fn $write(&mut self, value: $t) -> Result<(), IoError> {
                self.$pwrite(self.position, value)?;
                self.position += $width;
                Ok(())
            }
        )+
    };
}
pub(crate) use cursor_scalar_pairs;

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

        // The remaining native widths as cursor read/write — each reads the positioned typed
        // value at the cursor and advances by the type's byte width (the `i32`/`i64`/byte forms
        // above are the reference). Every scalar native type therefore streams like a file.
        $crate::io::memory::cursor_scalar_pairs! {
            (i8, 1, read_i8, write_i8, pread_i8, pwrite_i8),
            (u8, 1, read_u8, write_u8, pread_u8, pwrite_u8),
            (i16, 2, read_i16, write_i16, pread_i16, pwrite_i16),
            (u16, 2, read_u16, write_u16, pread_u16, pwrite_u16),
            (u32, 4, read_u32, write_u32, pread_u32, pwrite_u32),
            (u64, 8, read_u64, write_u64, pread_u64, pwrite_u64),
            (i128, 16, read_i128, write_i128, pread_i128, pwrite_i128),
            (u128, 16, read_u128, write_u128, pread_u128, pwrite_u128),
            (f32, 4, read_f32, write_f32, pread_f32, pwrite_f32),
            (f64, 8, read_f64, write_f64, pread_f64, pwrite_f64),
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

        /// **Reads one line** from the cursor — the bytes through the next `\n` **inclusive** (or to
        /// the end if none), decoded as UTF-8 — and advances the cursor past it (leaving it put on a
        /// UTF-8 error). Returns `""` **only** at the true end, so a blank line (which still carries
        /// its `\n`) is distinct from EOF; this is the [`readlines`](Self::readlines) /
        /// Python-`readline` semantics. The scan stages through a fixed stack buffer — no per-line
        /// heap churn beyond the returned string.
        ///
        /// ```
        /// use yggdryl_core::io::memory::{Heap, IOBase};
        ///
        /// let mut cur = Heap::from_slice(b"first\nsecond").cursor();
        /// assert_eq!(cur.readline().unwrap(), "first\n");
        /// assert_eq!(cur.readline().unwrap(), "second"); // no trailing newline at the end
        /// assert_eq!(cur.readline().unwrap(), "");        // now at EOF
        /// ```
        pub fn readline(&mut self) -> Result<String, IoError> {
            let start = self.position;
            let mut scan = [0u8; 256];
            let mut end = start;
            loop {
                let read = self.pread_byte_array(end, &mut scan);
                if read == 0 {
                    break; // end of source — the line is everything scanned so far
                }
                if let Some(at) = scan[..read].iter().position(|&byte| byte == b'\n') {
                    end += (at + 1) as u64; // include the newline, like Python's readline
                    break;
                }
                end += read as u64;
            }
            let text = self.pread_utf8(start, (end - start) as usize)?;
            self.position = end;
            Ok(text)
        }

        /// **Reads every remaining line** from the cursor into a `Vec`, advancing it to the end —
        /// each element keeps its trailing `\n` except possibly the last (see
        /// [`readline`](Self::readline)). The eager counterpart of looping `readline` line by line.
        ///
        /// ```
        /// use yggdryl_core::io::memory::{Heap, IOBase};
        ///
        /// let mut cur = Heap::from_slice(b"a\n\nb\n").cursor();
        /// assert_eq!(cur.readlines().unwrap(), vec!["a\n", "\n", "b\n"]); // blank line kept
        /// ```
        pub fn readlines(&mut self) -> Result<Vec<String>, IoError> {
            let mut lines = Vec::new();
            loop {
                let line = self.readline()?;
                if line.is_empty() {
                    break;
                }
                lines.push(line);
            }
            Ok(lines)
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

    fn reserve_exact(&mut self, additional: u64) {
        self.inner.reserve_exact(additional);
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        self.inner.try_reserve(additional)
    }

    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        self.inner.try_reserve_exact(additional)
    }

    fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    fn shrink_to(&mut self, min_capacity: u64) {
        self.inner.shrink_to(min_capacity);
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        self.inner.pread_byte_array(offset, buf)
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        self.inner.pwrite_byte_array(offset, data)
    }

    // A cursor addresses its source 1:1 (only the *position* differs), so every positioned op
    // forwards straight to the wrapped source — inheriting its **fast contiguous overrides**
    // (a cursor over a `Heap`/`Mmap` reads/writes through the same direct-slice path, not the
    // staged fallback) and its zero-copy `as_bytes`.
    #[inline]
    fn as_bytes(&self) -> Option<&[u8]> {
        self.inner.as_bytes()
    }

    fn pread_exact(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoError> {
        self.inner.pread_exact(offset, buf)
    }

    // Forward EVERY typed bulk array + repeat (all native widths) to the wrapped source, so a
    // cursor over a `Heap`/`Mmap` reaches the backing's fast contiguous overrides for all of them,
    // not just `i32`/`i64` — one delegating line each, generated by the shared macro.
    crate::io::memory::forward_bulk_ops!(inner);

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

    fn exists(&self) -> bool {
        // Forward the source's own notion (e.g. a live `Heap` exists although its kind is
        // neither file nor directory) instead of re-deriving from `kind` alone.
        self.inner.exists()
    }

    // A wrapper is a **leaf byte view**: the graph surface lives on the wrapped source.
    type Children = super::NoChildren<Self>;
    type Walk = super::NoChildren<Self>;

    fn ls(&self) -> Result<Self::Children, crate::io::IoError> {
        Ok(std::iter::empty())
    }

    fn ls_recursive(&self) -> Result<Self::Walk, crate::io::IoError> {
        Ok(std::iter::empty())
    }
}
