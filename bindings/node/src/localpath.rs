//! The `LocalPath` napi class.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_io::{Io, LocalPath as CoreLocalPath, Path, Seek};

use crate::iostats::IoStats;
use crate::media::MediaType;
use crate::whence_from;

/// A local filesystem path opened as a byte-IO handle, memory-mapped for
/// zero-copy direct access. Random (`readAt`) and streamed (`read`) access share
/// one cursor; `stats` and `mediaType` expose metadata.
#[napi(js_name = "LocalPath")]
pub struct LocalPath {
    pub(crate) inner: CoreLocalPath,
}

#[napi]
impl LocalPath {
    /// Open `location` for reading, throwing if it is missing.
    #[napi(constructor)]
    pub fn new(location: String) -> Result<Self> {
        CoreLocalPath::open(&location)
            .map(|inner| LocalPath { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Alias for the constructor.
    #[napi(factory)]
    pub fn open(location: String) -> Result<LocalPath> {
        LocalPath::new(location)
    }

    /// Write `data` to `location` on disk (creating or truncating it).
    #[napi]
    pub fn write(location: String, data: Buffer) -> Result<()> {
        CoreLocalPath::write(&location, data.as_ref())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Read up to `size` bytes from the cursor; omit `size` or pass a negative
    /// value to read all remaining bytes. Advances the cursor.
    #[napi]
    pub fn read(&mut self, size: Option<i32>) -> Result<Buffer> {
        let size = match size {
            Some(n) if n >= 0 => Some(n as usize),
            _ => None,
        };
        self.inner
            .read_owned(size)
            .map(Buffer::from)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Read up to `size` bytes at absolute `offset` without moving the cursor.
    #[napi(js_name = "readAt")]
    pub fn read_at(&mut self, offset: i64, size: u32) -> Result<Buffer> {
        let mut buf = vec![0u8; size as usize];
        let count = self
            .inner
            .read_at(offset as u64, &mut buf)
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
