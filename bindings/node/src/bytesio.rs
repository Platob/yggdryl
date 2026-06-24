//! The `BytesIO` napi class.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_io::{BytesIO as CoreBytesIO, Io};

use crate::url::Url;
use crate::whence_from;

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// `io.BytesIO`: `read` / `write` / `seek` / `tell` / `getValue` / `truncate`,
/// plus a `stream` flag that toggles whether the cursor advances on reads and
/// writes.
#[napi(js_name = "BytesIO")]
pub struct BytesIO {
    pub(crate) inner: CoreBytesIO,
}

#[napi]
impl BytesIO {
    /// Construct from optional `initial` bytes, with the cursor at the start.
    /// `stream` (default `true`) toggles cursor advancement.
    #[napi(constructor)]
    pub fn new(initial: Option<Buffer>, stream: Option<bool>) -> Self {
        let mut inner = CoreBytesIO::from_bytes(initial.map(|b| b.to_vec()).unwrap_or_default());
        inner.set_stream(stream.unwrap_or(true));
        BytesIO { inner }
    }

    /// Read up to `size` bytes from the cursor; omit `size` or pass a negative
    /// value to read all remaining bytes. Advances the cursor when `stream`.
    #[napi]
    pub fn read(&mut self, size: Option<i32>) -> Buffer {
        let size = match size {
            Some(n) if n >= 0 => Some(n as usize),
            _ => None,
        };
        Buffer::from(self.inner.read(size))
    }

    /// Read from the cursor through the next newline (inclusive), or to the end.
    /// Advances the cursor when `stream`.
    #[napi(js_name = "readLine")]
    pub fn read_line(&mut self) -> Buffer {
        Buffer::from(self.inner.read_line())
    }

    /// Write `data` at the cursor (overwriting and zero-filling as needed) and
    /// return the count written. Advances the cursor when `stream`.
    #[napi]
    pub fn write(&mut self, data: Buffer) -> u32 {
        self.inner.write(data.as_ref()) as u32
    }

    /// The resource address as a `Url` (`mem://<address>`).
    #[napi(getter)]
    pub fn url(&self) -> Url {
        Url {
            inner: self.inner.url(),
        }
    }

    /// Positional read of up to `size` bytes at `offset` relative to `whence`
    /// (`0` start, `1` current, `2` end). With `0`/`2` the cursor is untouched;
    /// with `1` it is used and advanced.
    #[napi]
    pub fn pread(&mut self, size: u32, offset: i64, whence: Option<u8>) -> Result<Buffer> {
        let mut buf = vec![0u8; size as usize];
        let count = self
            .inner
            .pread(&mut buf, offset, whence_from(whence.unwrap_or(0))?)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        buf.truncate(count);
        Ok(Buffer::from(buf))
    }

    /// Positional write of `data` at `offset` relative to `whence`, returning the
    /// count written. With `0`/`2` the cursor is untouched; with `1` it advances.
    #[napi]
    pub fn pwrite(&mut self, data: Buffer, offset: i64, whence: Option<u8>) -> Result<u32> {
        self.inner
            .pwrite(data.as_ref(), offset, whence_from(whence.unwrap_or(0))?)
            .map(|count| count as u32)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Move the cursor to `offset` relative to `whence` (`0` start, `1` current,
    /// `2` end), returning the new position. Throws if it would land before the
    /// start.
    #[napi]
    pub fn seek(&mut self, offset: i64, whence: Option<u8>) -> Result<u32> {
        self.inner
            .seek(offset, whence_from(whence.unwrap_or(0))?)
            .map(|pos| pos as u32)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The current cursor position.
    #[napi]
    pub fn tell(&self) -> u32 {
        self.inner.tell() as u32
    }

    /// Return the entire buffer, ignoring the cursor.
    #[napi(js_name = "getValue")]
    pub fn get_value(&self) -> Buffer {
        Buffer::from(self.inner.getvalue().to_vec())
    }

    /// Resize the buffer to `size` bytes (the current cursor when omitted),
    /// returning the new length. The cursor is left where it is.
    #[napi]
    pub fn truncate(&mut self, size: Option<u32>) -> u32 {
        self.inner.truncate(size.map(|s| s as usize)) as u32
    }

    /// No-op flush, present for parity with Python's `io.BytesIO`.
    #[napi]
    pub fn flush(&self) {}

    /// The total number of bytes held, regardless of the cursor.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        self.inner.len() as u32
    }

    /// Whether reads and writes advance the cursor (Python-stream semantics).
    #[napi(getter)]
    pub fn stream(&self) -> bool {
        self.inner.stream()
    }

    #[napi(setter)]
    pub fn set_stream(&mut self, value: bool) {
        self.inner.set_stream(value);
    }
}
