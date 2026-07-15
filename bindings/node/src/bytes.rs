//! The `yggdryl.io` namespace — the byte-I/O family: [`Bytes`] (an in-memory buffer with a
//! cursor) and the [`Whence`] seek origin.
//!
//! Mirrors `yggdryl_core::io`'s [`Bytes`](yggdryl_core::io::Bytes), which implements the
//! core's `IOBase` / `IOCursor` / `IOSlice` traits — positioned `pread` / `pwrite`, cursor
//! `read` / `write` with `seek(whence, offset)`, and zero-copy `slice` with copy-on-write
//! writes. Reads return a `Buffer`; writes take a `Buffer` and return the byte count. An
//! end-of-data `readExact`, a seek before the start, or an out-of-bounds `slice` throw a
//! guided `Error`. Offsets and sizes are JS numbers; a negative one throws.

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_core::io::{self, IOBase, IOCursor, IOSlice};

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Rejects a negative offset/size (JS numbers are signed) with a guided `Error`.
fn non_negative(value: i64, name: &str) -> napi::Result<u64> {
    u64::try_from(value)
        .map_err(|_| napi::Error::from_reason(format!("{name} must be non-negative, got {value}")))
}

/// Where a [`Bytes::seek`](Bytes::seek) offset is measured from — POSIX `SEEK_SET` /
/// `SEEK_CUR` / `SEEK_END`.
#[napi(namespace = "io")]
pub enum Whence {
    /// From the start of the data (absolute).
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the data.
    End,
}

impl Whence {
    fn to_core(self) -> io::Whence {
        match self {
            Whence::Start => io::Whence::Start,
            Whence::Current => io::Whence::Current,
            Whence::End => io::Whence::End,
        }
    }
}

/// An in-memory, growable byte buffer with a cursor — Arrow-backed, with zero-copy reads and
/// slices and copy-on-write writes.
#[napi(namespace = "io")]
pub struct Bytes {
    pub(crate) inner: io::Bytes,
}

#[napi(namespace = "io")]
impl Bytes {
    /// Builds a buffer from a `Buffer` (empty by default). The bytes are copied in; the
    /// cursor starts at `0`.
    #[napi(constructor)]
    pub fn new(data: Option<Buffer>) -> Self {
        Self {
            inner: match data {
                Some(bytes) => io::Bytes::from_slice(bytes.as_ref()),
                None => io::Bytes::new(),
            },
        }
    }

    /// An empty buffer that can grow to `capacity` bytes before its first reallocation.
    #[napi(factory)]
    pub fn with_capacity(capacity: i64) -> napi::Result<Self> {
        Ok(Self {
            inner: io::Bytes::with_capacity(non_negative(capacity, "capacity")? as usize),
        })
    }

    /// The total length in bytes.
    #[napi(getter)]
    pub fn length(&self) -> i64 {
        self.inner.len() as i64
    }

    /// The current cursor position (bytes from the start; may sit past the end after a seek).
    #[napi(getter)]
    pub fn position(&self) -> i64 {
        self.inner.position() as i64
    }

    // ---- positioned (random-access) read/write -----------------------------------------

    /// Reads up to `size` bytes starting at `offset` (short near the end), without moving the
    /// cursor.
    #[napi]
    pub fn pread(&self, offset: i64, size: i64) -> napi::Result<Buffer> {
        let offset = non_negative(offset, "offset")?;
        let size = non_negative(size, "size")? as usize;
        Ok(self.inner.pread_vec(offset, size).into())
    }

    /// Reads **exactly** `size` bytes at `offset`, throwing if fewer remain.
    #[napi]
    pub fn pread_exact(&self, offset: i64, size: i64) -> napi::Result<Buffer> {
        let offset = non_negative(offset, "offset")?;
        let mut buf = vec![0u8; non_negative(size, "size")? as usize];
        self.inner.pread_exact(offset, &mut buf).map_err(to_error)?;
        Ok(buf.into())
    }

    /// Writes `data` at `offset`, growing (and zero-filling any gap) as needed; returns the
    /// number of bytes written. Does not move the cursor.
    #[napi]
    pub fn pwrite(&mut self, offset: i64, data: Buffer) -> napi::Result<i64> {
        let offset = non_negative(offset, "offset")?;
        Ok(self.inner.pwrite(offset, data.as_ref()) as i64)
    }

    // ---- cursor read/write -------------------------------------------------------------

    /// Reads up to `size` bytes from the cursor, advancing it (short at the end).
    #[napi]
    pub fn read(&mut self, size: i64) -> napi::Result<Buffer> {
        Ok(self
            .inner
            .read_vec(non_negative(size, "size")? as usize)
            .into())
    }

    /// Reads **exactly** `size` bytes from the cursor, advancing it; throws on end-of-data
    /// (leaving the cursor put).
    #[napi]
    pub fn read_exact(&mut self, size: i64) -> napi::Result<Buffer> {
        let mut buf = vec![0u8; non_negative(size, "size")? as usize];
        self.inner.read_exact(&mut buf).map_err(to_error)?;
        Ok(buf.into())
    }

    /// Writes `data` at the cursor, advancing it; returns the number of bytes written.
    #[napi]
    pub fn write(&mut self, data: Buffer) -> i64 {
        self.inner.write(data.as_ref()) as i64
    }

    /// Reads from the cursor to the end, advancing it to the end.
    #[napi]
    pub fn read_to_end(&mut self) -> Buffer {
        self.inner.read_to_end().into()
    }

    // ---- seek --------------------------------------------------------------------------

    /// Seeks to `whence + offset` (offset defaults to `0`) and returns the new position. A
    /// position past the end is allowed; seeking before the start throws.
    #[napi]
    pub fn seek(&mut self, whence: Whence, offset: Option<i64>) -> napi::Result<i64> {
        self.inner
            .seek(whence.to_core(), offset.unwrap_or(0))
            .map(|position| position as i64)
            .map_err(to_error)
    }

    /// Resets the cursor to the start.
    #[napi]
    pub fn rewind(&mut self) {
        self.inner.rewind();
    }

    // ---- slice + interchange -----------------------------------------------------------

    /// A bounded window `[offset, offset+length)` as a new `Bytes` — zero-copy, sharing the
    /// allocation until either side is written. Throws if it runs past the end.
    #[napi]
    pub fn slice(&self, offset: i64, length: i64) -> napi::Result<Self> {
        let offset = non_negative(offset, "offset")?;
        let length = non_negative(length, "length")?;
        self.inner
            .slice(offset, length)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The whole content as a `Buffer` (one copy).
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_vec().into()
    }

    /// An explicit copy of this buffer (content and cursor).
    #[napi]
    pub fn copy(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// Content equality (the cursor is not part of the value).
    #[napi]
    pub fn equals(&self, other: &Bytes) -> bool {
        self.inner == other.inner
    }

    /// A short debug string, e.g. `"Bytes(len=11, position=6)"`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "Bytes(len={}, position={})",
            self.inner.len(),
            self.inner.position()
        )
    }
}
