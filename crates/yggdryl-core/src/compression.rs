//! [`Compression`] — the codec contract: compress / decompress byte arrays, and stream through
//! the [`IOBase`](crate::io::memory::IOBase) abstraction. The trait and its resolution live in
//! the dependency-free core; the concrete [`Gzip`] / [`Zlib`] / [`Zstd`] / [`Lzma`] codecs are
//! behind the **`compression`** cargo feature, which pulls the native codec cores (flate2 /
//! zstd / xz2). Without the feature the trait is still there — [`codec_for`] returns `None` and
//! the io helpers raise a guided [`IoError::Compression`].
//!
//! The io layer wires these in: [`IOBase::compress_into`](crate::io::memory::IOBase::compress_into)
//! / [`decompress_into`](crate::io::memory::IOBase::decompress_into) run a codec over a source's
//! bytes into a destination source with the fewest copies (a mapped source hands its bytes to the
//! codec directly; the output buffer is pre-sized), so the end-to-end path beats a naive
//! read-to-`Vec` → decode-to-`Vec`.

use crate::io::IoError;
use crate::mimetype::MimeType;

/// A compression codec over byte arrays. **Object-safe** (`dyn Compression`), so a codec is
/// resolved at runtime from a media type. The default level is the codec's own balanced
/// default; use [`with_level`](Compression) via the concrete constructors for a specific level.
///
/// The two byte-array methods are the contract; the io layer builds the streaming
/// `compress_into` / `decompress_into` on top of them and the [`IOBase`] byte access.
pub trait Compression {
    /// The codec's mime **essence** (`"application/gzip"`).
    fn essence(&self) -> &'static str;

    /// The codec's short **name** (`"gzip"`, `"zstd"`, `"xz"`, `"zlib"`).
    fn name(&self) -> &'static str;

    /// Compresses `input` into a new buffer.
    fn compress(&self, input: &[u8]) -> Result<Vec<u8>, IoError>;

    /// Decompresses `input` (produced by [`compress`](Compression::compress) or a compatible
    /// encoder) into a new buffer.
    fn decompress(&self, input: &[u8]) -> Result<Vec<u8>, IoError>;

    /// Decompresses **at most `max_out` bytes** from `input`, tolerating a **truncated** input
    /// (best-effort, no error) — the peek used by recursive magic inference to read the inner
    /// stream's head without decoding the whole thing. The default decodes fully then truncates
    /// (returning empty on a hard error); the native codecs override it with a bounded streaming
    /// read that stops early and ignores the trailing truncation.
    fn decompress_prefix(&self, input: &[u8], max_out: usize) -> Vec<u8> {
        match self.decompress(input) {
            Ok(mut out) => {
                out.truncate(max_out);
                out
            }
            Err(_) => Vec::new(),
        }
    }
}

/// The guided error for a compression op that could not run.
pub(crate) fn compression_err(codec: &str, op: &'static str, detail: impl Into<String>) -> IoError {
    IoError::Compression {
        codec: codec.to_string(),
        op,
        detail: detail.into(),
    }
}

/// Resolves a boxed [`Compression`] codec for a mime **essence** (`"application/gzip"`), or
/// `None` when the essence is not a supported compression format (or the `compression` feature
/// is off). See [`codec_for_mime`].
pub fn codec_for(essence: &str) -> Option<Box<dyn Compression>> {
    #[cfg(feature = "compression")]
    {
        codecs::codec_for(essence)
    }
    #[cfg(not(feature = "compression"))]
    {
        let _ = essence;
        None
    }
}

/// Resolves a codec for a [`MimeType`] — its essence via [`codec_for`]. `None` unless the type
/// [`is_compression`](MimeType::is_compression) *and* the feature is enabled.
pub fn codec_for_mime(mime: &MimeType) -> Option<Box<dyn Compression>> {
    codec_for(mime.essence())
}

// -------------------------------------------------------------------------------------
// Concrete codecs — behind the `compression` feature (the native codec cores).
// -------------------------------------------------------------------------------------

#[cfg(feature = "compression")]
mod codecs {
    use super::{compression_err, Compression};
    use crate::io::IoError;
    use std::io::{Read, Write};

    /// Resolves a codec for a mime essence.
    pub(super) fn codec_for(essence: &str) -> Option<Box<dyn Compression>> {
        match essence {
            "application/gzip" => Some(Box::new(Gzip::new())),
            "application/zlib" => Some(Box::new(Zlib::new())),
            "application/zstd" => Some(Box::new(Zstd::new())),
            "application/x-xz" | "application/x-lzma" => Some(Box::new(Lzma::new())),
            _ => None,
        }
    }

    /// Reads up to `max_out` decompressed bytes from `reader`, stopping at the limit or the end
    /// and **ignoring** a truncation/corruption error in the tail — the bounded, tolerant peek
    /// behind [`Compression::decompress_prefix`].
    fn read_prefix<R: Read>(mut reader: R, max_out: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(max_out.min(4096));
        let mut buf = [0u8; 512];
        while out.len() < max_out {
            let want = (max_out - out.len()).min(buf.len());
            match reader.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => break, // a truncated/short input head is expected here
            }
        }
        out
    }

    /// **Gzip** (RFC 1952) over the native DEFLATE core (`flate2`).
    #[derive(Clone, Copy, Debug)]
    pub struct Gzip {
        level: u32,
    }

    impl Gzip {
        /// A gzip codec at the balanced default level (6).
        pub fn new() -> Gzip {
            Gzip { level: 6 }
        }

        /// A gzip codec at compression `level` (`0` fastest/none … `9` smallest).
        pub fn with_level(level: u32) -> Gzip {
            Gzip {
                level: level.min(9),
            }
        }
    }

    impl Default for Gzip {
        fn default() -> Self {
            Gzip::new()
        }
    }

    impl Compression for Gzip {
        fn essence(&self) -> &'static str {
            "application/gzip"
        }
        fn name(&self) -> &'static str {
            "gzip"
        }
        fn compress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            let mut enc = flate2::write::GzEncoder::new(
                Vec::with_capacity(input.len() / 2 + 64),
                flate2::Compression::new(self.level),
            );
            enc.write_all(input)
                .and_then(|()| enc.finish())
                .map_err(|e| compression_err(self.essence(), "compress", e.to_string()))
        }
        fn decompress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            let mut out = Vec::with_capacity(input.len().saturating_mul(3));
            flate2::read::GzDecoder::new(input)
                .read_to_end(&mut out)
                .map(|_| out)
                .map_err(|e| compression_err(self.essence(), "decompress", e.to_string()))
        }
        fn decompress_prefix(&self, input: &[u8], max_out: usize) -> Vec<u8> {
            read_prefix(flate2::read::GzDecoder::new(input), max_out)
        }
    }

    /// **Zlib** (RFC 1950) over the native DEFLATE core (`flate2`).
    #[derive(Clone, Copy, Debug)]
    pub struct Zlib {
        level: u32,
    }

    impl Zlib {
        /// A zlib codec at the balanced default level (6).
        pub fn new() -> Zlib {
            Zlib { level: 6 }
        }
        /// A zlib codec at compression `level` (`0` … `9`).
        pub fn with_level(level: u32) -> Zlib {
            Zlib {
                level: level.min(9),
            }
        }
    }

    impl Default for Zlib {
        fn default() -> Self {
            Zlib::new()
        }
    }

    impl Compression for Zlib {
        fn essence(&self) -> &'static str {
            "application/zlib"
        }
        fn name(&self) -> &'static str {
            "zlib"
        }
        fn compress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            let mut enc = flate2::write::ZlibEncoder::new(
                Vec::with_capacity(input.len() / 2 + 64),
                flate2::Compression::new(self.level),
            );
            enc.write_all(input)
                .and_then(|()| enc.finish())
                .map_err(|e| compression_err(self.essence(), "compress", e.to_string()))
        }
        fn decompress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            let mut out = Vec::with_capacity(input.len().saturating_mul(3));
            flate2::read::ZlibDecoder::new(input)
                .read_to_end(&mut out)
                .map(|_| out)
                .map_err(|e| compression_err(self.essence(), "decompress", e.to_string()))
        }
        fn decompress_prefix(&self, input: &[u8], max_out: usize) -> Vec<u8> {
            read_prefix(flate2::read::ZlibDecoder::new(input), max_out)
        }
    }

    /// **Zstandard** over the native `libzstd` core (`zstd`).
    #[derive(Clone, Copy, Debug)]
    pub struct Zstd {
        level: i32,
    }

    impl Zstd {
        /// A zstd codec at the balanced default level (3).
        pub fn new() -> Zstd {
            Zstd { level: 3 }
        }
        /// A zstd codec at compression `level` (`1` fastest … `22` smallest).
        pub fn with_level(level: i32) -> Zstd {
            Zstd {
                level: level.clamp(1, 22),
            }
        }
    }

    impl Default for Zstd {
        fn default() -> Self {
            Zstd::new()
        }
    }

    impl Compression for Zstd {
        fn essence(&self) -> &'static str {
            "application/zstd"
        }
        fn name(&self) -> &'static str {
            "zstd"
        }
        fn compress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            zstd::stream::encode_all(input, self.level)
                .map_err(|e| compression_err(self.essence(), "compress", e.to_string()))
        }
        fn decompress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            zstd::stream::decode_all(input)
                .map_err(|e| compression_err(self.essence(), "decompress", e.to_string()))
        }
        fn decompress_prefix(&self, input: &[u8], max_out: usize) -> Vec<u8> {
            match zstd::stream::read::Decoder::new(input) {
                Ok(dec) => read_prefix(dec, max_out),
                Err(_) => Vec::new(),
            }
        }
    }

    /// **LZMA / XZ** over the native `liblzma` core (`xz2`).
    #[derive(Clone, Copy, Debug)]
    pub struct Lzma {
        preset: u32,
    }

    impl Lzma {
        /// An xz codec at the balanced default preset (6).
        pub fn new() -> Lzma {
            Lzma { preset: 6 }
        }
        /// An xz codec at compression `preset` (`0` fastest … `9` smallest).
        pub fn with_level(preset: u32) -> Lzma {
            Lzma {
                preset: preset.min(9),
            }
        }
    }

    impl Default for Lzma {
        fn default() -> Self {
            Lzma::new()
        }
    }

    impl Compression for Lzma {
        fn essence(&self) -> &'static str {
            "application/x-xz"
        }
        fn name(&self) -> &'static str {
            "xz"
        }
        fn compress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            let mut enc =
                xz2::write::XzEncoder::new(Vec::with_capacity(input.len() / 2 + 128), self.preset);
            enc.write_all(input)
                .and_then(|()| enc.finish())
                .map_err(|e| compression_err(self.essence(), "compress", e.to_string()))
        }
        fn decompress(&self, input: &[u8]) -> Result<Vec<u8>, IoError> {
            let mut out = Vec::with_capacity(input.len().saturating_mul(4));
            xz2::read::XzDecoder::new(input)
                .read_to_end(&mut out)
                .map(|_| out)
                .map_err(|e| compression_err(self.essence(), "decompress", e.to_string()))
        }
        fn decompress_prefix(&self, input: &[u8], max_out: usize) -> Vec<u8> {
            read_prefix(xz2::read::XzDecoder::new(input), max_out)
        }
    }
}

#[cfg(feature = "compression")]
pub use codecs::{Gzip, Lzma, Zlib, Zstd};
