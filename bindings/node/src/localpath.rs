//! The `LocalPath` napi class.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_io::{BytesIO as CoreBytesIO, Io, LocalPath as CoreLocalPath, Mode, Path};

use crate::bytesio::BytesIO;
use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::url::Url;
use crate::whence_from;

/// A local filesystem path opened as a byte-IO handle, memory-mapped lazily.
/// Positional (`pread`) and streamed (`read`) access share one cursor; the
/// `stream` flag toggles whether `read` advances it (as in `BytesIO`). `stats` /
/// `mediaType` expose metadata.
#[napi(js_name = "LocalPath")]
pub struct LocalPath {
    pub(crate) inner: CoreLocalPath,
}

#[napi]
impl LocalPath {
    /// Open a handle for `location`, statting it up front (so `url` / `stats` are
    /// ready). Never throws — a missing path yields a handle whose `stats` report
    /// `kind === "missing"`.
    #[napi(constructor)]
    pub fn new(location: String) -> Self {
        LocalPath {
            inner: CoreLocalPath::open(&location),
        }
    }

    /// Write `data` to this path on disk, auto-creating missing parent
    /// directories (lazily, only on a missing-parent failure).
    #[napi]
    pub fn write(&self, data: Buffer) -> Result<()> {
        self.inner
            .write(data.as_ref())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The resource address as a `Url` (`file://` over the path).
    #[napi(getter)]
    pub fn url(&self) -> Url {
        Url {
            inner: self.inner.url(),
        }
    }

    /// The access mode (always `"r"` — the mapped handle is read-only).
    #[napi(getter)]
    pub fn mode(&self) -> String {
        self.inner.mode().as_str().to_owned()
    }

    /// Whether `read` advances the cursor (the same flag as `BytesIO.stream`).
    #[napi(getter)]
    pub fn stream(&self) -> bool {
        self.inner.stream()
    }

    #[napi(setter)]
    pub fn set_stream(&mut self, value: bool) {
        self.inner.set_stream(value);
    }

    /// Open an in-memory `BytesIO` over this file's bytes, applying `mode`
    /// (default `"r"`) and `stream` (default `true`) — a `LocalPath` and a
    /// `BytesIO` open the same way.
    #[napi]
    pub fn open(&self, mode: Option<String>, stream: Option<bool>) -> Result<BytesIO> {
        let mode = Mode::from_str(mode.as_deref().unwrap_or("r"))
            .map_err(|e| Error::from_reason(e.to_string()))?;
        let parent = CoreBytesIO::from_bytes(self.inner.getvalue().to_vec());
        Ok(BytesIO {
            inner: parent.open(mode, stream.unwrap_or(true)),
        })
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

    /// No-op flush, present for parity with `BytesIO`.
    #[napi]
    pub fn flush(&self) {}

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

    /// Move the cursor to `offset` relative to `whence` (`0` start, `1` current,
    /// `2` end), returning the new position.
    #[napi]
    pub fn seek(&mut self, offset: i64, whence: Option<u8>) -> Result<f64> {
        self.inner
            .seek(offset, whence_from(whence.unwrap_or(0))?)
            .map(|position| position as f64)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The current cursor position.
    #[napi]
    pub fn tell(&self) -> f64 {
        self.inner.tell() as f64
    }

    /// The capacity in bytes (the mapped file size; the handle is read-only, so
    /// resizing is unsupported).
    #[napi(getter)]
    pub fn capacity(&self) -> f64 {
        self.inner.capacity() as f64
    }

    /// Return the entire file contents, ignoring the cursor.
    #[napi(js_name = "getValue")]
    pub fn get_value(&self) -> Buffer {
        Buffer::from(self.inner.as_slice().unwrap_or(&[]).to_vec())
    }

    /// Discover this file's metadata (see `IoStats`).
    #[napi]
    pub fn stats(&self) -> Result<IoStats> {
        self.inner
            .stats()
            .map(|inner| IoStats { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The lazily-inferred `MediaType` of this file, or `null`.
    #[napi(js_name = "mediaType")]
    pub fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// The file location.
    #[napi(getter)]
    pub fn location(&self) -> String {
        self.inner.location().to_owned()
    }

    /// Whether the file currently exists.
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    /// No-op close, present for the `io` API; the mapping is released when the
    /// object is garbage-collected. (Python exposes this as a `with` context
    /// manager.)
    #[napi]
    pub fn close(&self) {}

    /// The total number of bytes.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        self.inner.len() as f64
    }
}
