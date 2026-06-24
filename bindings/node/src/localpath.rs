//! The `LocalPath` napi class.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_io::{Io, LocalPath as CoreLocalPath, Path, Seek, Whence};

use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::url::Url;
use crate::whence_from;

/// A local filesystem path opened as a byte-IO handle, memory-mapped for
/// zero-copy direct access. Positional (`pread`) and streamed (`read`) access
/// share one cursor; `stats` and `mediaType` expose metadata.
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

    /// Alias for the constructor.
    #[napi(factory)]
    pub fn open(location: String) -> LocalPath {
        LocalPath::new(location)
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

    /// Read up to `size` bytes from the cursor; omit `size` or pass a negative
    /// value to read all remaining bytes. Advances the cursor.
    #[napi]
    pub fn read(&mut self, size: Option<i32>) -> Result<Buffer> {
        let remaining = (self
            .inner
            .stats()
            .map_err(|e| Error::from_reason(e.to_string()))?
            .size()
            - self.inner.stream_position()) as usize;
        let size = match size {
            Some(n) if n >= 0 => (n as usize).min(remaining),
            _ => remaining,
        };
        let mut buf = vec![0u8; size];
        let count = self
            .inner
            .pread(&mut buf, 0, Whence::Current)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        buf.truncate(count);
        Ok(Buffer::from(buf))
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
        self.inner.stream_position() as f64
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

    /// The total number of bytes.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        self.inner.as_slice().map_or(0, <[u8]>::len) as f64
    }
}
