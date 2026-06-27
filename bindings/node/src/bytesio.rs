//! The `BytesIO` napi class.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{BytesIO as CoreBytesIO, Io, Mode};
use yggdryl_core::{CompressIo, Compression as CoreCompression};

use crate::iostats::IoStats;
use crate::media::MediaType;
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
    /// Construct from optional `initial` contents — a `Buffer` taken verbatim, or a
    /// `string` resolved through `fromStr` (an existing file is read in, else the
    /// text is UTF-8 encoded). `stream` (default `true`) toggles cursor advancement.
    /// `mediaType` seeds the cached `mediaType` so it is not inferred from the magic
    /// bytes.
    #[napi(constructor)]
    pub fn new(
        initial: Option<Either<String, Buffer>>,
        stream: Option<bool>,
        media_type: Option<&MediaType>,
    ) -> Self {
        let mut inner = match initial {
            Some(Either::A(value)) => CoreBytesIO::from_str(&value),
            Some(Either::B(buffer)) => CoreBytesIO::from_bytes(buffer.to_vec()),
            None => CoreBytesIO::new(),
        };
        inner.set_stream(stream.unwrap_or(true));
        if let Some(media_type) = media_type {
            inner = inner.with_media_type(media_type.inner.clone());
        }
        BytesIO { inner }
    }

    /// Build from a string: if `value` names an existing file, read its bytes;
    /// otherwise UTF-8-encode the string as the contents. `stream` (default `true`)
    /// toggles cursor advancement.
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String, stream: Option<bool>) -> Self {
        let mut inner = CoreBytesIO::from_str(&value);
        inner.set_stream(stream.unwrap_or(true));
        BytesIO { inner }
    }

    /// Create an empty buffer preallocated to hold `capacity` bytes.
    #[napi(factory, js_name = "withCapacity")]
    pub fn with_capacity(capacity: u32) -> Self {
        BytesIO {
            inner: CoreBytesIO::with_capacity(capacity as usize),
        }
    }

    /// The reserved capacity (bytes the buffer can hold before reallocating).
    #[napi(getter)]
    pub fn capacity(&self) -> f64 {
        self.inner.capacity() as f64
    }

    /// Reserve room for `additional` more bytes beyond the current length.
    #[napi(js_name = "reserveCapacity")]
    pub fn reserve_capacity(&mut self, additional: u32) -> Result<()> {
        self.inner
            .reserve_capacity(additional as usize)
            .map_err(|e| Error::from_reason(e.to_string()))
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
    /// return the count written. Advances the cursor when `stream`. Throws if the
    /// write would extend the buffer past the addressable range.
    #[napi]
    pub fn write(&mut self, data: Buffer) -> Result<u32> {
        self.inner
            .write(data.as_ref())
            .map(|count| count as u32)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The resource address as a `Url` (`mem://<address>`).
    #[napi(getter)]
    pub fn url(&self) -> Url {
        Url {
            inner: self.inner.url(),
        }
    }

    /// Discover this handle's metadata (see `IoStats`): `kind === "file"` and the
    /// buffer `size`. The live byte count always wins; any `setStats` override
    /// supplies the rest and the cached `mediaType` is folded in.
    #[napi]
    pub fn stats(&self) -> Result<IoStats> {
        self.inner
            .stats()
            .map(|inner| IoStats { inner })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// The `MediaType` of this buffer — inferred from the magic bytes once and
    /// **cached**, or the one seeded via the `mediaType` constructor argument.
    /// `null` when no type can be inferred.
    #[napi(getter, js_name = "mediaType")]
    pub fn media_type(&self) -> Option<MediaType> {
        self.inner.media_type().map(|inner| MediaType { inner })
    }

    /// The cached `IoStats` if one has been installed with `setStats`, else `null`
    /// — the *get* side of the stats cache.
    #[napi(js_name = "cachedStats")]
    pub fn cached_stats(&self) -> Option<IoStats> {
        self.inner.cached_stats().map(|inner| IoStats { inner })
    }

    /// Install `stats` as this handle's cached metadata — the *set* side. The live
    /// byte count still wins in `stats`; the slot supplies the rest.
    #[napi(js_name = "setStats")]
    pub fn set_stats(&mut self, stats: &IoStats) {
        self.inner.set_stats(stats.inner.clone());
    }

    /// The access mode: `"r"`, `"w"`, `"a"` or `"r+"`.
    #[napi(getter)]
    pub fn mode(&self) -> String {
        self.inner.mode().as_str().to_owned()
    }

    /// Open a new `BytesIO` derived from this one (a snapshot of the current
    /// bytes), applying `mode` (default `"r"`) and `stream` (default `true`).
    /// `mode` accepts the Python forms (`r` / `w` / `a` / `r+` / `rb` / `a+` / …):
    /// `w` truncates, `a` appends.
    #[napi]
    pub fn open(&self, mode: Option<String>, stream: Option<bool>) -> Result<BytesIO> {
        let mode = Mode::from_str(mode.as_deref().unwrap_or("r"))
            .map_err(|e| Error::from_reason(e.to_string()))?;
        let parent = CoreBytesIO::from_bytes(self.inner.getvalue().to_vec());
        Ok(BytesIO {
            inner: parent.open(mode, stream.unwrap_or(true)),
        })
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

    /// The current cursor position — the cross-language `Io` cursor accessor
    /// (same value as `tell`).
    #[napi(js_name = "streamPosition")]
    pub fn stream_position(&self) -> f64 {
        self.inner.stream_position() as f64
    }

    /// The total length in bytes when known without I/O, else `null`.
    #[napi(js_name = "streamLen")]
    pub fn stream_len(&self) -> Option<f64> {
        self.inner.stream_len().map(|n| n as f64)
    }

    /// Return the entire buffer, ignoring the cursor.
    #[napi(js_name = "getValue")]
    pub fn get_value(&self) -> Buffer {
        Buffer::from(self.inner.getvalue().to_vec())
    }

    /// Parse the buffer's bytes as JSON (in Rust), returning the JS value.
    #[napi]
    pub fn json(&mut self) -> Result<serde_json::Value> {
        self.inner
            .json()
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Compress this buffer's bytes (from the cursor) with `codec` — a name like
    /// `"gzip"` / `"zstd"` / `"snappy"` — into a new `BytesIO`.
    #[napi]
    pub fn compress(&mut self, codec: String) -> Result<BytesIO> {
        let codec =
            CoreCompression::from_str(&codec).map_err(|e| Error::from_reason(e.to_string()))?;
        let inner = self
            .inner
            .compress(codec)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(BytesIO { inner })
    }

    /// Decompress this buffer's bytes (from the cursor) into a new `BytesIO`.
    /// `codec` names the codec; when omitted it is inferred from this buffer's
    /// magic bytes (e.g. a gzip header → `gzip`).
    #[napi]
    pub fn decompress(&mut self, codec: Option<String>) -> Result<BytesIO> {
        let codec = match codec {
            Some(name) => Some(
                CoreCompression::from_str(&name).map_err(|e| Error::from_reason(e.to_string()))?,
            ),
            None => None,
        };
        let inner = self
            .inner
            .decompress(codec)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(BytesIO { inner })
    }

    /// Resize the buffer to `size` bytes (the current cursor when omitted),
    /// returning the new length. The cursor is left where it is. Throws when
    /// growing past the addressable range.
    #[napi]
    pub fn truncate(&mut self, size: Option<u32>) -> Result<u32> {
        self.inner
            .truncate(size.map(|s| s as usize))
            .map(|len| len as u32)
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// No-op flush, present for parity with Python's `io.BytesIO`.
    #[napi]
    pub fn flush(&self) {}

    /// Release the handle (a no-op for an in-memory buffer; the bytes are freed
    /// when the object is garbage-collected). Idempotent. (Python exposes this as
    /// a `with` context manager.)
    #[napi]
    pub fn close(&mut self) -> Result<()> {
        self.inner
            .close()
            .map_err(|e| Error::from_reason(e.to_string()))
    }

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
