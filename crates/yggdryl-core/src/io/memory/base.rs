//! [`IOBase`] — positioned (random-access) byte read/write, the base of the I/O trait family.

use super::{IOCursor, IOSlice, IoError};
use crate::io::uri::Uri;

/// Random-access byte storage addressed by absolute offset — no cursor. This is the base
/// every I/O **source** shares: [`IOCursor`](super::IOCursor) adds a moving position on top, and
/// [`IOSlice`](super::IOSlice) adds bounded sub-views.
///
/// # Shape
///
/// - **Size** — [`byte_size`](IOBase::byte_size) / [`bit_size`](IOBase::bit_size).
/// - **Capacity** — [`capacity`](IOBase::capacity) / [`reserve`](IOBase::reserve), the `Vec`-like
///   amortized-growth hooks (a source with no spare capacity, e.g. a memory-map, reports its size
///   and ignores `reserve`).
/// - **Byte-array primitives** — [`pread_byte_array`](IOBase::pread_byte_array) /
///   [`pwrite_byte_array`](IOBase::pwrite_byte_array): the two methods a source must implement.
/// - **Typed accessors** — `byte` / `bit` / `i32` / `i64`, positioned little-endian
///   read/write built on the primitives (`pread_byte`, `pread_bit`, `pread_i32`, `pread_i64`
///   and their `pwrite_*` twins).
///
/// DESIGN: the two **primitives** are *infallible* (`-> usize`), because the physical layer is
/// in-memory: a read past the end simply returns fewer bytes (0 at the end) and a write past the
/// end grows the storage, zero-filling any gap. The fallible surface is the **full** and **typed**
/// helpers built on them, whose contract — *fill exactly this many* — can be broken by the end of
/// the data. Signatures take `&[u8]` / `&mut [u8]` and native integers, never a foreign buffer
/// type, so the storage underneath stays an implementation detail. Bit addressing is **LSB-first**
/// (bit `i` is bit `i % 8` of byte `i / 8`, least-significant first), matching Arrow validity
/// bitmaps.
pub trait IOBase {
    /// The total length in bytes.
    fn byte_size(&self) -> u64;

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.byte_size() * 8
    }

    /// Whether the storage is empty (`byte_size() == 0`).
    fn is_empty(&self) -> bool {
        self.byte_size() == 0
    }

    /// The number of bytes the storage can hold before it must reallocate — like
    /// [`Vec::capacity`]. A source with no distinct spare capacity (a fixed map, a view) returns
    /// [`byte_size`](IOBase::byte_size); a growable one (e.g. [`Heap`](super::Heap)) overrides it.
    fn capacity(&self) -> u64 {
        self.byte_size()
    }

    /// Reserves capacity for at least `additional` more bytes past the current
    /// [`byte_size`](IOBase::byte_size), amortizing later writes — like [`Vec::reserve`]. The
    /// default is a **no-op** (a fixed source cannot grow); a growable source overrides it.
    fn reserve(&mut self, additional: u64) {
        let _ = additional;
    }

    /// The [`Uri`] that **addresses** this source — every source is locatable. The default is an
    /// empty (opaque) URI for a source with no meaningful address (a scratch buffer); a source
    /// that has one (an in-heap [`Heap`](super::Heap) with a set address, a future file/network
    /// source) overrides it to return its own.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    /// use yggdryl_core::io::uri::Uri;
    ///
    /// // An unaddressed source reports the empty URI…
    /// assert_eq!(Heap::new().uri(), Uri::default());
    /// // …and one can be attached.
    /// let named = Heap::new().with_uri(Uri::parse_str("mem://scratch/a").unwrap());
    /// assert_eq!(named.uri().host(), Some("scratch"));
    /// ```
    fn uri(&self) -> Uri {
        Uri::default()
    }

    /// **Positioned read** (primitive). Copies up to `buf.len()` bytes starting at `offset` into
    /// `buf`, returning the number copied — `0` at or past the end, a short count near it. Never
    /// moves a cursor.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// let mut buf = [0u8; 5];
    /// assert_eq!(data.pread_byte_array(6, &mut buf), 5);
    /// assert_eq!(&buf, b"world");
    /// assert_eq!(data.pread_byte_array(11, &mut buf), 0); // at the end -> nothing
    /// ```
    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize;

    /// **Positioned write** (primitive). Copies `data` in at `offset`, growing the storage (and
    /// zero-filling any gap between the old end and `offset`) as needed. Returns the number of
    /// bytes written — always `data.len()`.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::from_slice(b"abc");
    /// assert_eq!(data.pwrite_byte_array(5, b"Z"), 1); // writes past the end, zero-filling the gap
    /// assert_eq!(data.as_slice(), b"abc\0\0Z");
    /// ```
    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize;

    /// **Full positioned read.** Fills *all* of `buf` starting at `offset`, or errors with
    /// [`IoError::UnexpectedEof`] naming the shortfall if fewer bytes remain.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello");
    /// let mut buf = [0u8; 3];
    /// data.pread_exact(1, &mut buf).unwrap();
    /// assert_eq!(&buf, b"ell");
    /// assert!(data.pread_exact(3, &mut [0u8; 5]).is_err()); // only 2 remain
    /// ```
    fn pread_exact(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let read = self.pread_byte_array(offset, buf);
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
    /// [`pread_exact`](IOBase::pread_exact). Infallible for in-memory storage (the write always
    /// grows to fit), but returns `Result` so the trait reads uniformly and a fallible backend
    /// can honour it.
    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        self.pwrite_byte_array(offset, data);
        Ok(())
    }

    /// Reads up to `len` bytes at `offset` into a fresh `Vec` (short near the end) — the owning
    /// read for callers that do not bring their own buffer. One allocation, pre-sized.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// assert_eq!(data.pread_vec(6, 100), b"world"); // clamped to what remains
    /// ```
    fn pread_vec(&self, offset: u64, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        let read = self.pread_byte_array(offset, &mut buf);
        buf.truncate(read);
        buf
    }

    /// Reads up to `len` bytes at `offset` into `dst`, **reusing `dst`'s existing allocation** —
    /// the allocation-free bulk read for a transfer loop that reads chunk after chunk into one
    /// scratch buffer. `dst` is cleared, grown once to fit only if its capacity is too small
    /// (its spare capacity is reused otherwise), and filled; returns the number of bytes read
    /// (short near the end). Unlike [`pread_vec`](IOBase::pread_vec) — a fresh `Vec` every call —
    /// this keeps a caller's buffer hot across a whole transfer.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let src = Heap::from_slice(b"hello world");
    /// let mut scratch = Vec::new();
    /// assert_eq!(src.pread_into(0, 5, &mut scratch), 5);
    /// assert_eq!(&scratch, b"hello");
    /// let cap = scratch.capacity();
    /// assert_eq!(src.pread_into(6, 5, &mut scratch), 5); // reuses the allocation
    /// assert_eq!(&scratch, b"world");
    /// assert_eq!(scratch.capacity(), cap);
    /// ```
    fn pread_into(&self, offset: u64, len: usize, dst: &mut Vec<u8>) -> usize {
        dst.clear();
        dst.resize(len, 0); // reuses `dst`'s capacity when it already fits; one grow otherwise
        let read = self.pread_byte_array(offset, dst);
        dst.truncate(read);
        read
    }

    /// Reads the single byte at `offset`, or errors with [`IoError::UnexpectedEof`] if it is past
    /// the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"abc");
    /// assert_eq!(data.pread_byte(1).unwrap(), b'b');
    /// assert!(data.pread_byte(3).is_err());
    /// ```
    fn pread_byte(&self, offset: u64) -> Result<u8, IoError> {
        let mut buf = [0u8; 1];
        self.pread_exact(offset, &mut buf)?;
        Ok(buf[0])
    }

    /// Writes the single byte `value` at `offset`, growing the storage as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> Result<(), IoError> {
        self.pwrite_all(offset, &[value])
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), or errors with [`IoError::UnexpectedEof`] if its byte is past the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&[0b0000_0101]);
    /// assert!(data.pread_bit(0).unwrap());  // bit 0 set
    /// assert!(!data.pread_bit(1).unwrap()); // bit 1 clear
    /// assert!(data.pread_bit(2).unwrap());  // bit 2 set
    /// ```
    fn pread_bit(&self, offset: u64) -> Result<bool, IoError> {
        let byte = self.pread_byte(offset / 8)?;
        Ok((byte >> (offset % 8)) & 1 != 0)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing its
    /// byte and growing the storage (zero-filled) if the bit is past the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_bit(10, true).unwrap();      // grows to 2 bytes, sets bit 2 of byte 1
    /// assert_eq!(data.as_slice(), &[0b0000_0000, 0b0000_0100]);
    /// assert!(data.pread_bit(10).unwrap());
    /// ```
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> Result<(), IoError> {
        let byte_offset = offset / 8;
        let mask = 1u8 << (offset % 8);
        let mut buf = [0u8; 1];
        self.pread_byte_array(byte_offset, &mut buf); // 0 if past the end (about to grow)
        if value {
            buf[0] |= mask;
        } else {
            buf[0] &= !mask;
        }
        self.pwrite_all(byte_offset, &buf)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&(-42i32).to_le_bytes());
    /// assert_eq!(data.pread_i32(0).unwrap(), -42);
    /// ```
    fn pread_i32(&self, offset: u64) -> Result<i32, IoError> {
        let mut buf = [0u8; 4];
        self.pread_exact(offset, &mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> Result<(), IoError> {
        self.pwrite_all(offset, &value.to_le_bytes())
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&(1234567890123i64).to_le_bytes());
    /// assert_eq!(data.pread_i64(0).unwrap(), 1234567890123);
    /// ```
    fn pread_i64(&self, offset: u64) -> Result<i64, IoError> {
        let mut buf = [0u8; 8];
        self.pread_exact(offset, &mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> Result<(), IoError> {
        self.pwrite_all(offset, &value.to_le_bytes())
    }

    /// Wraps this source in an [`IOCursor`] positioned at the start — the standard way to add a
    /// moving read/write position to any source. Consumes the source (zero-copy); wrap a clone to
    /// keep the original.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut cur = Heap::from_slice(b"hi").cursor();
    /// assert_eq!(cur.read_byte().unwrap(), b'h');
    /// ```
    fn cursor(self) -> IOCursor<Self>
    where
        Self: Sized,
    {
        IOCursor::new(self)
    }

    /// Wraps this source in an [`IOSlice`] — the bounded window `[offset, offset + len)` addressed
    /// from its own `0`. Errors with [`IoError::SliceOutOfBounds`] if it runs past the end.
    /// Consumes the source (zero-copy); wrap a clone to keep the original.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let win = Heap::from_slice(b"hello world").window(6, 5).unwrap();
    /// assert_eq!(win.pread_vec(0, 5), b"world");
    /// ```
    fn window(self, offset: u64, len: u64) -> Result<IOSlice<Self>, IoError>
    where
        Self: Sized,
    {
        IOSlice::new(self, offset, len)
    }
}
