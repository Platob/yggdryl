//! The `yggdryl.compression` namespace — thin wrappers over the compression
//! codecs in `yggdryl-core`.
//!
//! Only the concrete codecs cross the FFI boundary. The core's `Encoder` /
//! `Decoder` / `Compression` traits (and their generic `Typed*` variants) are
//! Rust-only contracts — generics and marker traits cannot be expressed across the
//! binding — so they are **not** replicated here; this is a deliberate, documented
//! omission per `CLAUDE.md`. Exposes [`Gzip`] and [`Zstd`]. The one-shot
//! `CompressIO` (compress/decompress an IO with any codec) is Rust-only — it takes a
//! generic `dyn` codec that does not cross the FFI boundary; use `compressStream` or
//! `encodeByteArray` instead.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_compression::{Compression, CompressionDecoder, CompressionEncoder};
use yggdryl_core::{Decoder, Encoder};

use crate::io::ByteCursor;

/// Maps any core codec error to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// The gzip (RFC 1952) compression codec.
///
/// Mirrors `yggdryl_compression::Gzip`: construct with a level in `0..=9` (default `6`),
/// then `encodeByteArray` / `decodeByteArray` to compress / decompress.
#[napi(namespace = "compression")]
pub struct Gzip {
    inner: yggdryl_compression::Gzip,
}

#[napi(namespace = "compression")]
impl Gzip {
    /// Creates a gzip codec at `level` (`0..=9`, default `6`).
    #[napi(constructor)]
    pub fn new(level: Option<u32>) -> napi::Result<Self> {
        let level = level.unwrap_or(yggdryl_compression::Gzip::DEFAULT_LEVEL);
        Ok(Self {
            inner: yggdryl_compression::Gzip::new(level).map_err(to_error)?,
        })
    }

    /// The lowercase codec name (`"gzip"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The configured compression level.
    #[napi(getter)]
    pub fn level(&self) -> u32 {
        self.inner.level()
    }

    /// Compresses `data`, returning the gzip stream.
    #[napi]
    pub fn encode_byte_array(&self, data: Buffer) -> napi::Result<Buffer> {
        let out = self
            .inner
            .encode_byte_array(data.as_ref())
            .map_err(to_error)?;
        Ok(out.into())
    }

    /// Decompresses the gzip `data` stream.
    #[napi]
    pub fn decode_byte_array(&self, data: Buffer) -> napi::Result<Buffer> {
        let out = self
            .inner
            .decode_byte_array(data.as_ref())
            .map_err(to_error)?;
        Ok(out.into())
    }

    /// Stream-compresses every byte remaining under `source` into `sink`, returning
    /// the number of bytes written. Both cursors advance.
    #[napi]
    pub fn compress_stream(
        &self,
        source: &mut ByteCursor,
        sink: &mut ByteCursor,
    ) -> napi::Result<i64> {
        let n = self
            .inner
            .compress_stream(&mut source.inner, &mut sink.inner)
            .map_err(to_error)?;
        Ok(n as i64)
    }

    /// Stream-decompresses every byte remaining under `source` into `sink`,
    /// returning the number of bytes written. Both cursors advance.
    #[napi]
    pub fn decompress_stream(
        &self,
        source: &mut ByteCursor,
        sink: &mut ByteCursor,
    ) -> napi::Result<i64> {
        let n = self
            .inner
            .decompress_stream(&mut source.inner, &mut sink.inner)
            .map_err(to_error)?;
        Ok(n as i64)
    }

    /// Serialises the codec to bytes (the single level byte).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a codec from `serializeBytes`.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        Ok(Self {
            inner: yggdryl_compression::Gzip::deserialize_bytes(bytes.as_ref())
                .map_err(to_error)?,
        })
    }

    /// Content equality — two codecs are equal iff their `serializeBytes` match.
    #[napi]
    pub fn equals(&self, other: &Gzip) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash, consistent with [`equals`](Gzip::equals).
    #[napi]
    pub fn hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        let hash = hasher.finish();
        // Fold the 64-bit hash into an i32 so it maps to a plain JS number.
        (hash as u32 ^ (hash >> 32) as u32) as i32
    }
}

/// The Zstandard (RFC 8878) compression codec.
#[napi(namespace = "compression")]
pub struct Zstd {
    inner: yggdryl_compression::Zstd,
}

#[napi(namespace = "compression")]
impl Zstd {
    /// Creates a zstd codec at `level` (default `3`).
    #[napi(constructor)]
    pub fn new(level: Option<i32>) -> napi::Result<Self> {
        let level = level.unwrap_or(yggdryl_compression::Zstd::DEFAULT_LEVEL);
        Ok(Self {
            inner: yggdryl_compression::Zstd::new(level).map_err(to_error)?,
        })
    }

    /// The inclusive `[min, max]` levels this build of zstd accepts.
    #[napi]
    pub fn level_range() -> Vec<i32> {
        let (min, max) = yggdryl_compression::Zstd::level_range();
        vec![min, max]
    }

    /// The lowercase codec name (`"zstd"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The configured compression level.
    #[napi(getter)]
    pub fn level(&self) -> i32 {
        self.inner.level()
    }

    /// Compresses `data`, returning the zstd frame.
    #[napi]
    pub fn encode_byte_array(&self, data: Buffer) -> napi::Result<Buffer> {
        let out = self
            .inner
            .encode_byte_array(data.as_ref())
            .map_err(to_error)?;
        Ok(out.into())
    }

    /// Decompresses the zstd `data` frame.
    #[napi]
    pub fn decode_byte_array(&self, data: Buffer) -> napi::Result<Buffer> {
        let out = self
            .inner
            .decode_byte_array(data.as_ref())
            .map_err(to_error)?;
        Ok(out.into())
    }

    /// Stream-compresses everything under `source` into `sink`.
    #[napi]
    pub fn compress_stream(
        &self,
        source: &mut ByteCursor,
        sink: &mut ByteCursor,
    ) -> napi::Result<i64> {
        let n = self
            .inner
            .compress_stream(&mut source.inner, &mut sink.inner)
            .map_err(to_error)?;
        Ok(n as i64)
    }

    /// Stream-decompresses everything under `source` into `sink`.
    #[napi]
    pub fn decompress_stream(
        &self,
        source: &mut ByteCursor,
        sink: &mut ByteCursor,
    ) -> napi::Result<i64> {
        let n = self
            .inner
            .decompress_stream(&mut source.inner, &mut sink.inner)
            .map_err(to_error)?;
        Ok(n as i64)
    }

    /// Serialises the codec to bytes (the 4-byte level).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a codec from `serializeBytes`.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        Ok(Self {
            inner: yggdryl_compression::Zstd::deserialize_bytes(bytes.as_ref())
                .map_err(to_error)?,
        })
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &Zstd) -> bool {
        self.inner == other.inner
    }

    /// Java-style `i32` content hash.
    #[napi]
    pub fn hash_code(&self) -> i32 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        let hash = hasher.finish();
        (hash as u32 ^ (hash >> 32) as u32) as i32
    }
}
