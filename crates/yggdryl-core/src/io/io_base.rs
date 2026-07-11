//! [`IOBase`] — the base cursor-oriented byte-IO contract.

use crate::{IoError, Whence};

/// The chunk size used by the resource-to-resource transfers.
const TRANSFER_CHUNK: usize = 64 * 1024;

/// Generates the little-endian typed read/write accessors for a fixed-width
/// primitive as **default** [`IOBase`] methods, layered on the byte-array surface.
macro_rules! primitive_io {
    ($( ($ty:ty, $read_one:ident, $write_one:ident, $read_arr:ident, $write_arr:ident) ),+ $(,)?) => {
        $(
            #[doc = concat!("Reads a little-endian `", stringify!($ty), "` at `whence`, advancing the cursor past it.")]
            fn $read_one(&mut self, whence: Whence) -> Result<$ty, IoError> {
                const W: usize = core::mem::size_of::<$ty>();
                let bytes = self.pread_byte_array(W, whence)?;
                let array: [u8; W] = bytes.as_slice().try_into().map_err(|_| IoError::UnexpectedEof {
                    needed: W,
                    available: bytes.len(),
                })?;
                Ok(<$ty>::from_le_bytes(array))
            }

            #[doc = concat!("Writes `value` as a little-endian `", stringify!($ty), "` at `whence`, advancing the cursor; returns the number of values written (`1`).")]
            fn $write_one(&mut self, value: $ty, whence: Whence) -> Result<usize, IoError> {
                self.pwrite_byte_array(&value.to_le_bytes(), whence)?;
                Ok(1)
            }

            #[doc = concat!("Reads up to `count` little-endian `", stringify!($ty), "` values at `whence`, advancing the cursor (fewer at EOF).")]
            fn $read_arr(&mut self, count: usize, whence: Whence) -> Result<Vec<$ty>, IoError> {
                const W: usize = core::mem::size_of::<$ty>();
                let bytes = self.pread_byte_array(count.saturating_mul(W), whence)?;
                Ok(bytes
                    .chunks_exact(W)
                    .map(|chunk| <$ty>::from_le_bytes(chunk.try_into().expect("chunks_exact yields W bytes")))
                    .collect())
            }

            #[doc = concat!("Writes the `", stringify!($ty), "` values in `data` (little-endian) at `whence`, advancing the cursor; returns the number written.")]
            fn $write_arr(&mut self, data: &[$ty], whence: Whence) -> Result<usize, IoError> {
                #[cfg(target_endian = "little")]
                {
                    // SAFETY: reinterpreting a slice of a fixed-width numeric primitive as
                    // its bytes is sound (no padding; `u8` alignment is 1), and on a
                    // little-endian target those bytes already equal the wire form.
                    let bytes = unsafe {
                        core::slice::from_raw_parts(data.as_ptr().cast::<u8>(), core::mem::size_of_val(data))
                    };
                    self.pwrite_byte_array(bytes, whence)?;
                }
                #[cfg(not(target_endian = "little"))]
                {
                    let mut buf = Vec::with_capacity(data.len().saturating_mul(core::mem::size_of::<$ty>()));
                    for value in data {
                        buf.extend_from_slice(&value.to_le_bytes());
                    }
                    self.pwrite_byte_array(&buf, whence)?;
                }
                Ok(data.len())
            }
        )+
    };
}

/// The base contract for a **cursor** over a byte resource: a byte position
/// ([`byte_tell`](IOBase::byte_tell) / [`byte_seek`](IOBase::byte_seek), with the
/// bit-unit mirrors [`bit_tell`](IOBase::bit_tell) / [`bit_seek`](IOBase::bit_seek))
/// plus reads and writes that happen at, and **advance**, that position.
///
/// A read/write resolves its start via `whence` (`Start` = 0, `Current` = the
/// cursor, `End` = the size) and then advances the cursor past the bytes moved.
/// Beyond the raw byte-array primitives the trait provides **default** typed
/// accessors for every fixed-width primitive (`pread_i64` / `pwrite_i64_array` / …,
/// little-endian) and resource-to-resource transfers
/// ([`pread_io`](IOBase::pread_io) / [`pwrite_io`](IOBase::pwrite_io)), all layered
/// on `pread_byte_array` / `pwrite_byte_array` + `byte_size` / `byte_capacity` /
/// `with_byte_capacity`. The element-typed cursor
/// [`TypedCursor<T>`](crate::TypedCursor) adds a `T`-unit position on top.
///
/// Implementors are cursors ([`ByteCursor`](crate::ByteCursor)); the storage types
/// ([`ByteBuffer`](crate::ByteBuffer)) hand out cursors and do not implement this.
/// The trait is object-safe (no lifetimes) and can be held behind `dyn IOBase`.
///
/// ```
/// use yggdryl_core::{ByteBuffer, IOBase, Whence};
///
/// let mut cursor = ByteBuffer::new().byte_cursor();
/// cursor.pwrite_i32(-1, Whence::Start).unwrap();
/// cursor.byte_seek(0, Whence::Start).unwrap();
/// assert_eq!(cursor.pread_i32(Whence::Current).unwrap(), -1);
/// ```
///
/// A `&mut dyn IOBase` is also a [`std::io::Read`] + [`Write`](std::io::Write) +
/// [`Seek`](std::io::Seek) — reads/writes happen at [`Current`](Whence::Current) and
/// `Seek` maps to [`byte_seek`](IOBase::byte_seek) — so a cursor plugs straight into the
/// standard streaming ecosystem (`io::copy`, codec backends, `read_to_end`, …) with no
/// wrapper. This interop is Rust-only (`std::io` does not cross the FFI boundary).
///
/// ```
/// use std::io::{Read, Seek, SeekFrom, Write};
/// use yggdryl_core::{ByteBuffer, IOBase};
///
/// let mut cursor = ByteBuffer::new().byte_cursor();
/// let io: &mut dyn IOBase = &mut cursor;
/// io.write_all(b"hi").unwrap();
/// io.seek(SeekFrom::Start(0)).unwrap();
/// let mut buf = Vec::new();
/// io.read_to_end(&mut buf).unwrap();
/// assert_eq!(buf, b"hi");
/// ```
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait IOBase {
    /// Creates a cursor over a fresh resource able to hold `capacity` bytes without
    /// reallocating.
    fn with_byte_capacity(capacity: usize) -> Self
    where
        Self: Sized;

    /// Creates a cursor over a fresh resource able to hold `capacity` bits.
    fn with_bit_capacity(capacity: usize) -> Self
    where
        Self: Sized,
    {
        Self::with_byte_capacity(capacity.div_ceil(8))
    }

    /// Returns the current cursor position, in bytes from the start.
    fn byte_tell(&self) -> Result<u64, IoError>;

    /// Moves the cursor to `offset` bytes relative to `whence`, returning the new
    /// absolute byte position. A negative `offset` seeks backward (from `Current` /
    /// `End`); a resolved position before the start is an
    /// [`IoError::InvalidSeek`].
    fn byte_seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError>;

    /// Returns the current cursor position, in bits from the start (`byte_tell * 8`).
    fn bit_tell(&self) -> Result<u64, IoError> {
        Ok(self.byte_tell()?.saturating_mul(8))
    }

    /// Moves the cursor to `offset` bits relative to `whence`, returning the new
    /// absolute bit position.
    ///
    /// This cursor addresses whole bytes, and every seek origin is byte-aligned, so
    /// `offset` must be a multiple of 8; a negative `offset` seeks backward. The
    /// `End` origin resolves against the resource's **total** extent (like
    /// [`byte_seek`](IOBase::byte_seek)), not the remaining bytes.
    ///
    /// # Errors
    /// [`IoError::UnalignedBitSeek`] if `offset` is not a multiple of 8, or
    /// [`IoError::InvalidSeek`] if it lands before the start or beyond the
    /// addressable range.
    fn bit_seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        if offset % 8 != 0 {
            return Err(IoError::UnalignedBitSeek { offset });
        }
        // Delegate to `byte_seek` so the `End` origin resolves against the total
        // extent (`byte_size` reports the *remaining* bytes, not the total).
        let byte = self.byte_seek(offset / 8, whence)?;
        Ok(byte.saturating_mul(8))
    }

    /// The number of bytes **remaining** from the current position to the end (for a
    /// cursor). Read/write operations consume from this remaining extent.
    fn byte_size(&self) -> Result<usize, IoError>;

    /// The number of bits remaining (`byte_size * 8`).
    fn bit_size(&self) -> Result<usize, IoError> {
        Ok(self.byte_size()?.saturating_mul(8))
    }

    /// The number of bytes remaining as a `u64` (the width-explicit form).
    fn large_byte_size(&self) -> Result<u64, IoError> {
        Ok(self.byte_size()? as u64)
    }

    /// The number of bits remaining as a `u64`.
    fn large_bit_size(&self) -> Result<u64, IoError> {
        Ok(self.large_byte_size()?.saturating_mul(8))
    }

    /// The number of bytes the resource can hold without reallocating.
    fn byte_capacity(&self) -> Result<usize, IoError>;

    /// The number of bits the resource can hold without reallocating.
    fn bit_capacity(&self) -> Result<usize, IoError> {
        Ok(self.byte_capacity()?.saturating_mul(8))
    }

    /// Reads up to `size` bytes starting at `whence`, advancing the cursor past
    /// them. A read at or past the end yields fewer bytes (possibly none).
    fn pread_byte_array(&mut self, size: usize, whence: Whence) -> Result<Vec<u8>, IoError>;

    /// Reads up to `buf.len()` bytes at `whence` **into** `buf`, advancing the
    /// cursor, and returns the number read. Allocation-free counterpart of
    /// [`pread_byte_array`](IOBase::pread_byte_array); contiguous-memory cursors
    /// override it to copy straight into `buf`.
    fn pread_into(&mut self, buf: &mut [u8], whence: Whence) -> Result<usize, IoError> {
        let chunk = self.pread_byte_array(buf.len(), whence)?;
        let n = chunk.len();
        buf[..n].copy_from_slice(&chunk);
        Ok(n)
    }

    /// Writes `data` starting at `whence`, advancing the cursor past it, and
    /// returns the number of bytes written.
    fn pwrite_byte_array(&mut self, data: &[u8], whence: Whence) -> Result<usize, IoError>;

    primitive_io!(
        (i8, pread_i8, pwrite_i8, pread_i8_array, pwrite_i8_array),
        (u8, pread_u8, pwrite_u8, pread_u8_array, pwrite_u8_array),
        (
            i16,
            pread_i16,
            pwrite_i16,
            pread_i16_array,
            pwrite_i16_array
        ),
        (
            u16,
            pread_u16,
            pwrite_u16,
            pread_u16_array,
            pwrite_u16_array
        ),
        (
            i32,
            pread_i32,
            pwrite_i32,
            pread_i32_array,
            pwrite_i32_array
        ),
        (
            u32,
            pread_u32,
            pwrite_u32,
            pread_u32_array,
            pwrite_u32_array
        ),
        (
            i64,
            pread_i64,
            pwrite_i64,
            pread_i64_array,
            pwrite_i64_array
        ),
        (
            u64,
            pread_u64,
            pwrite_u64,
            pread_u64_array,
            pwrite_u64_array
        ),
        (
            f32,
            pread_f32,
            pwrite_f32,
            pread_f32_array,
            pwrite_f32_array
        ),
        (
            f64,
            pread_f64,
            pwrite_f64,
            pread_f64_array,
            pwrite_f64_array
        ),
    );

    /// Copies up to `size` bytes from `self` (starting at `whence`) into `sink` at
    /// its cursor, chunked; advances both cursors and returns the bytes transferred.
    fn pread_io(
        &mut self,
        sink: &mut dyn IOBase,
        size: usize,
        whence: Whence,
    ) -> Result<u64, IoError> {
        let mut scratch = vec![0u8; TRANSFER_CHUNK.min(size).max(1)];
        let mut transferred = 0usize;
        let mut from = whence;
        while transferred < size {
            let want = scratch.len().min(size - transferred);
            let n = self.pread_into(&mut scratch[..want], from)?;
            from = Whence::Current; // continue sequentially after the first read
            if n == 0 {
                break;
            }
            sink.pwrite_byte_array(&scratch[..n], Whence::Current)?;
            transferred += n;
            if n < want {
                break;
            }
        }
        Ok(transferred as u64)
    }

    /// Copies up to `size` bytes from `source` (at its cursor) into `self` starting
    /// at `whence`, chunked; advances both cursors and returns the bytes transferred.
    fn pwrite_io(
        &mut self,
        source: &mut dyn IOBase,
        size: usize,
        whence: Whence,
    ) -> Result<u64, IoError> {
        let mut scratch = vec![0u8; TRANSFER_CHUNK.min(size).max(1)];
        let mut transferred = 0usize;
        let mut into = whence;
        while transferred < size {
            let want = scratch.len().min(size - transferred);
            let n = source.pread_into(&mut scratch[..want], Whence::Current)?;
            if n == 0 {
                break;
            }
            self.pwrite_byte_array(&scratch[..n], into)?;
            into = Whence::Current;
            transferred += n;
        }
        Ok(transferred as u64)
    }
}
