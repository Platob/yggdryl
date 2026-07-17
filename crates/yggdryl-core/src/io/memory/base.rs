//! [`IOBase`] — positioned (random-access) byte read/write, the base of the I/O trait family.

use crate::io::IoError;

/// Random-access byte storage addressed by absolute offset — no cursor. This is the base
/// every I/O object shares: [`IOCursor`](super::IOCursor) adds a moving position on top, and
/// [`IOSlice`](super::IOSlice) adds bounded sub-views.
///
/// DESIGN: the two **primitives** — [`pread`](IOBase::pread) and [`pwrite`](IOBase::pwrite) —
/// are *infallible* (`-> usize`), because the physical layer is in-memory: a read past the
/// end simply returns fewer bytes (0 at the end) and a write past the end grows the storage,
/// zero-filling any gap. The fallible surface is the **full** helpers built on them
/// ([`pread_exact`](IOBase::pread_exact)), whose contract — *fill exactly this many* — can be
/// broken by the end of the data. Signatures take `&[u8]` / `&mut [u8]`, never an `arrow-rs`
/// type, so the Arrow buffer underneath stays an implementation detail.
pub trait IOBase {
    /// The total length in bytes.
    fn len(&self) -> u64;

    /// Whether the storage is empty (`len() == 0`).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// **Positioned read.** Copies up to `buf.len()` bytes starting at `offset` into `buf`,
    /// returning the number copied — `0` at or past the end, a short count near it. Never
    /// moves a cursor.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOBase};
    ///
    /// let data = Bytes::from_slice(b"hello world");
    /// let mut buf = [0u8; 5];
    /// assert_eq!(data.pread(6, &mut buf), 5);
    /// assert_eq!(&buf, b"world");
    /// assert_eq!(data.pread(11, &mut buf), 0); // at the end -> nothing
    /// ```
    fn pread(&self, offset: u64, buf: &mut [u8]) -> usize;

    /// **Positioned write.** Copies `data` in at `offset`, growing the storage (and
    /// zero-filling any gap between the old end and `offset`) as needed. Returns the number
    /// of bytes written — always `data.len()`.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOBase};
    ///
    /// let mut data = Bytes::from_slice(b"abc");
    /// assert_eq!(data.pwrite(5, b"Z"), 1); // writes past the end, zero-filling the gap
    /// assert_eq!(data.as_slice(), b"abc\0\0Z");
    /// ```
    fn pwrite(&mut self, offset: u64, data: &[u8]) -> usize;

    /// **Full positioned read.** Fills *all* of `buf` starting at `offset`, or errors with
    /// [`IoError::UnexpectedEof`] naming the shortfall if fewer bytes remain.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOBase};
    ///
    /// let data = Bytes::from_slice(b"hello");
    /// let mut buf = [0u8; 3];
    /// data.pread_exact(1, &mut buf).unwrap();
    /// assert_eq!(&buf, b"ell");
    /// assert!(data.pread_exact(3, &mut [0u8; 5]).is_err()); // only 2 remain
    /// ```
    fn pread_exact(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let read = self.pread(offset, buf);
        if read == buf.len() {
            Ok(())
        } else {
            Err(IoError::UnexpectedEof {
                offset: offset + read as u64,
                requested: buf.len(),
                available: read,
            })
        }
    }

    /// **Full positioned write** of *all* of `data` at `offset` — the counterpart of
    /// [`pread_exact`](IOBase::pread_exact). Infallible for in-memory storage (the write
    /// always grows to fit), but returns `Result` so the trait reads uniformly and a
    /// fallible backend can honour it.
    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        self.pwrite(offset, data);
        Ok(())
    }

    /// Reads up to `len` bytes at `offset` into a fresh `Vec` (short near the end) — the
    /// owning read for callers that do not bring their own buffer. One allocation, pre-sized.
    ///
    /// ```
    /// use yggdryl_core::io::{Bytes, IOBase};
    ///
    /// let data = Bytes::from_slice(b"hello world");
    /// assert_eq!(data.pread_vec(6, 100), b"world"); // clamped to what remains
    /// ```
    fn pread_vec(&self, offset: u64, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        let read = self.pread(offset, &mut buf);
        buf.truncate(read);
        buf
    }
}
