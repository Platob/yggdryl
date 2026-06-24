//! # yggdryl-compression
//!
//! Streamed byte **compression** — gzip, Zstandard or Snappy — layered on top of
//! the [`yggdryl-io`](yggdryl_io) handle abstraction. A [`Compression`] codec
//! wraps any [`Io`](yggdryl_io::Io) handle (or raw [`ReadBytes`](yggdryl_io::ReadBytes)
//! / [`WriteBytes`](yggdryl_io::WriteBytes)) to compress and decompress **a chunk
//! at a time**, never buffering the whole payload:
//!
//! - [`encoder`](Compression::encoder) wraps a sink so everything written is
//!   compressed on the way out ([`finish`](Encoder::finish) flushes the trailer);
//! - [`decoder`](Compression::decoder) wraps a source so reads decompress on the
//!   way in;
//! - [`compress`](Compression::compress) / [`decompress`](Compression::decompress)
//!   are the one-shot `&[u8] -> Vec<u8>` conveniences.
//!
//! The [`CompressIo`] extension trait adds [`compress`](CompressIo::compress) /
//! [`decompress`](CompressIo::decompress) straight onto every `Io` handle,
//! returning a fresh in-memory [`BytesIO`](yggdryl_io::BytesIO). On decompress the
//! codec may be left to inference — from the handle's URL extension, then (under
//! the `media` feature) its discovered media type (magic bytes for a buffer, the
//! file name for a path) and finally its content type.
//!
//! ## Optional features (off by default)
//!
//! - `gzip` / `zstd` / `snappy` — the matching backend (`flate2` / `zstd` /
//!   `snap`). A codec whose feature is off still parses and names itself but
//!   reports [`IoError::Unsupported`] on encode/decode; check
//!   [`is_available`](Compression::is_available) ahead of time.
//! - `media` — infer a codec from an `Io`'s MIME / media type via `yggdryl-media`.
//! - `log` — structured `log` events on the hot paths.
//!
//! ```
//! use yggdryl_compression::Compression;
//!
//! let codec = Compression::from_str("gzip").unwrap();
//! assert_eq!(codec.as_str(), "gzip");
//! assert_eq!(codec.extension(), Some("gz"));
//! # #[cfg(feature = "gzip")]
//! # {
//! let packed = codec.compress(b"hello hello hello").unwrap();
//! assert_eq!(codec.decompress(&packed).unwrap(), b"hello hello hello");
//! # }
//! ```

use std::fmt;

use yggdryl_io::{copy, BytesIO, Io, IoError, ReadBytes, Whence, WriteBytes};

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate pulls no `log` dependency by default and pays no
/// runtime cost).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

/// A byte-stream **compression codec** — gzip, Zstandard or Snappy — that wraps
/// any [`Io`] handle (or raw [`ReadBytes`] / [`WriteBytes`]) to compress and
/// decompress **in a streamed way**, a chunk at a time, never buffering the whole
/// payload.
///
/// [`encoder`](Compression::encoder) wraps a sink so everything written to it is
/// compressed on the way out (call [`finish`](Encoder::finish) to flush the
/// trailer); [`decoder`](Compression::decoder) wraps a source so reads
/// decompress on the way in. [`compress`](Compression::compress) /
/// [`decompress`](Compression::decompress) are the one-shot `&[u8] -> Vec<u8>`
/// conveniences built on top.
///
/// Each backend is an **optional, off-by-default feature** (`gzip` → `flate2`,
/// `zstd` → `zstd`, `snappy` → `snap`); a variant whose feature is not compiled
/// in still parses and names itself, but [`encoder`](Compression::encoder) /
/// [`decoder`](Compression::decoder) report [`IoError::Unsupported`]. Check
/// [`is_available`](Compression::is_available) to tell ahead of time.
/// [`None`](Compression::None) is always available — the `store` identity codec
/// that passes bytes through unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Compression {
    /// No compression: bytes pass through unchanged (the `store` identity codec).
    /// Always available.
    #[default]
    None,
    /// gzip (RFC 1952), via `flate2` — the `gzip` feature.
    Gzip,
    /// Zstandard, via `zstd` — the `zstd` feature.
    Zstd,
    /// Snappy frame format, via `snap` — the `snappy` feature.
    Snappy,
}

impl Compression {
    /// Parses a codec name — `none` / `identity` / `store`, `gzip` / `gz`,
    /// `zstd` / `zst`, `snappy` / `snap` / `sz` (case-insensitive) — returning
    /// [`IoError::Invalid`] on an unknown one.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Compression, IoError> {
        log_event!(trace, "Compression::from_str {value:?}");
        let codec = match value.trim().to_ascii_lowercase().as_str() {
            "none" | "identity" | "store" => Compression::None,
            "gzip" | "gz" => Compression::Gzip,
            "zstd" | "zst" => Compression::Zstd,
            "snappy" | "snap" | "sz" => Compression::Snappy,
            _ => return Err(IoError::Invalid(format!("unknown compression {value:?}"))),
        };
        Ok(codec)
    }

    /// Infers the codec from a file extension (`gz`, `zst`, `sz`, with or without
    /// a leading dot), or `None` if the extension names no known codec.
    pub fn from_extension(extension: &str) -> Option<Compression> {
        let codec = match extension
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "gz" | "gzip" => Compression::Gzip,
            "zst" | "zstd" => Compression::Zstd,
            "sz" | "snappy" | "snap" => Compression::Snappy,
            _ => return None,
        };
        Some(codec)
    }

    /// Maps a single [`MimeType`](yggdryl_media::MimeType) to its codec — the
    /// "optional MIME type to get compression" entry point — or `None` if the MIME
    /// names no supported codec. Only present under the `media` feature.
    #[cfg(feature = "media")]
    pub fn from_mime(mime: &yggdryl_media::MimeType) -> Option<Compression> {
        use yggdryl_media::MimeType;
        match mime {
            MimeType::Gzip => Some(Compression::Gzip),
            MimeType::Zstd => Some(Compression::Zstd),
            _ => None,
        }
    }

    /// Infers the codec from a layered [`MediaType`](yggdryl_media::MediaType)
    /// stack — its outermost (container) MIME, e.g. `Gzip` for `data.csv.gz`. Only
    /// present under the `media` feature.
    #[cfg(feature = "media")]
    pub fn from_media(media: &yggdryl_media::MediaType) -> Option<Compression> {
        media.last().and_then(Compression::from_mime)
    }

    /// Infers the codec from an [`IoStats`](yggdryl_io::IoStats): its discovered
    /// media type first, then its transport content type. Returns `None` when
    /// neither names a codec. Only present under the `media` feature — this is the
    /// signal [`CompressIo::decompress`] uses when no codec is given.
    #[cfg(feature = "media")]
    pub fn from_stats(stats: &yggdryl_io::IoStats) -> Option<Compression> {
        use yggdryl_media::FromInput;
        if let Some(media) = stats.media_type() {
            if let Some(codec) = Compression::from_media(media) {
                return Some(codec);
            }
        }
        stats
            .content_type()
            .and_then(|content_type| yggdryl_media::MimeType::from_str(content_type).ok())
            .and_then(|mime| Compression::from_mime(&mime))
    }

    /// The canonical codec name (`"none"` / `"gzip"` / `"zstd"` / `"snappy"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Compression::None => "none",
            Compression::Gzip => "gzip",
            Compression::Zstd => "zstd",
            Compression::Snappy => "snappy",
        }
    }

    /// The conventional file extension for this codec (`"gz"` / `"zst"` / `"sz"`),
    /// or `None` for [`None`](Compression::None).
    pub fn extension(&self) -> Option<&'static str> {
        match self {
            Compression::None => None,
            Compression::Gzip => Some("gz"),
            Compression::Zstd => Some("zst"),
            Compression::Snappy => Some("sz"),
        }
    }

    /// Whether this codec's backend is compiled in, so
    /// [`encoder`](Compression::encoder) / [`decoder`](Compression::decoder) will
    /// work. [`None`](Compression::None) is always available.
    pub fn is_available(&self) -> bool {
        match self {
            Compression::None => true,
            #[cfg(feature = "gzip")]
            Compression::Gzip => true,
            #[cfg(feature = "zstd")]
            Compression::Zstd => true,
            #[cfg(feature = "snappy")]
            Compression::Snappy => true,
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    /// Wraps `sink` in a streaming [`Encoder`]: bytes written to the encoder are
    /// compressed and forwarded to `sink`. Call [`finish`](Encoder::finish) to
    /// write the trailer and recover the sink. Returns [`IoError::Unsupported`]
    /// if the codec's feature is not compiled in.
    pub fn encoder<W: WriteBytes>(self, sink: W) -> Result<Encoder<W>, IoError> {
        log_event!(debug, "Compression::{} encoder", self.as_str());
        let inner = match self {
            Compression::None => EncoderInner::Store(sink),
            #[cfg(feature = "gzip")]
            Compression::Gzip => EncoderInner::Gzip(flate2::write::GzEncoder::new(
                WriteShim(sink),
                flate2::Compression::default(),
            )),
            #[cfg(feature = "zstd")]
            Compression::Zstd => EncoderInner::Zstd(
                zstd::stream::write::Encoder::new(WriteShim(sink), 0).map_err(IoError::from)?,
            ),
            #[cfg(feature = "snappy")]
            Compression::Snappy => {
                EncoderInner::Snappy(snap::write::FrameEncoder::new(WriteShim(sink)))
            }
            #[allow(unreachable_patterns)]
            other => return Err(other.unavailable()),
        };
        Ok(Encoder { inner })
    }

    /// Wraps `source` in a streaming [`Decoder`]: reads from the decoder pull
    /// compressed bytes from `source` and yield the decompressed stream. Returns
    /// [`IoError::Unsupported`] if the codec's feature is not compiled in.
    pub fn decoder<R: ReadBytes>(self, source: R) -> Result<Decoder<R>, IoError> {
        log_event!(debug, "Compression::{} decoder", self.as_str());
        let inner = match self {
            Compression::None => DecoderInner::Store(source),
            #[cfg(feature = "gzip")]
            Compression::Gzip => DecoderInner::Gzip(flate2::read::GzDecoder::new(ReadShim(source))),
            #[cfg(feature = "zstd")]
            Compression::Zstd => DecoderInner::Zstd(
                zstd::stream::read::Decoder::new(ReadShim(source)).map_err(IoError::from)?,
            ),
            #[cfg(feature = "snappy")]
            Compression::Snappy => {
                DecoderInner::Snappy(snap::read::FrameDecoder::new(ReadShim(source)))
            }
            #[allow(unreachable_patterns)]
            other => return Err(other.unavailable()),
        };
        Ok(Decoder { inner })
    }

    /// Compresses `data` in full, returning the encoded bytes — the one-shot form
    /// of [`encoder`](Compression::encoder) over an in-memory `Vec<u8>`.
    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>, IoError> {
        let mut encoder = self.encoder(Vec::new())?;
        encoder.write_all(data)?;
        encoder.finish()
    }

    /// Decompresses `data` in full, returning the decoded bytes — the one-shot
    /// form of [`decoder`](Compression::decoder) over an in-memory slice.
    pub fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, IoError> {
        let mut decoder = self.decoder(data)?;
        let mut out = Vec::new();
        decoder.read_to_end(&mut out)?;
        Ok(out)
    }

    /// The [`IoError::Unsupported`] raised when this codec's feature is off.
    fn unavailable(&self) -> IoError {
        log_event!(
            warn,
            "Compression::{} unavailable: feature not enabled",
            self.as_str()
        );
        IoError::Unsupported(format!(
            "{} compression: enable the `{}` cargo feature",
            self.as_str(),
            self.as_str()
        ))
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Compression-aware extension methods for every [`Io`] handle: compress or
/// decompress a handle's bytes (from its cursor) into a fresh in-memory
/// [`BytesIO`], using [`Compression`]'s streaming codecs internally. Blanket-
/// implemented for all `Io`, so it is in scope wherever the trait is imported.
pub trait CompressIo: Io {
    /// Streams this handle (from its cursor to the end) through `codec` and
    /// returns the compressed bytes in a fresh [`BytesIO`] positioned at the
    /// start. Errors if the codec's backend is not available.
    fn compress(&mut self, codec: Compression) -> Result<BytesIO, IoError> {
        let mut encoder = codec.encoder(BytesIO::new())?;
        copy(self, &mut encoder)?;
        let mut out = encoder.finish()?;
        out.seek(0, Whence::Start)?;
        Ok(out)
    }

    /// Streams this handle (from its cursor to the end) through a decompressor and
    /// returns the decoded bytes in a fresh [`BytesIO`]. When `codec` is `None` it
    /// is inferred from this handle (see [`compression`](CompressIo::compression));
    /// if nothing indicates one the bytes pass through unchanged.
    fn decompress(&mut self, codec: Option<Compression>) -> Result<BytesIO, IoError> {
        let codec = match codec {
            Some(codec) => codec,
            None => self.compression(),
        };
        let mut decoder = codec.decoder(&mut *self)?;
        let mut out = Vec::new();
        decoder.read_to_end(&mut out)?;
        Ok(BytesIO::from_bytes(out))
    }

    /// The [`Compression`] inferred for this handle: its URL extension first
    /// (always available), then its discovered media type — magic bytes for an
    /// in-memory buffer, the file name for a path — and finally its
    /// [`stats`](Io::stats) content type (all under the `media` feature).
    /// [`Compression::None`] when nothing indicates a codec.
    fn compression(&mut self) -> Compression {
        // The URL extension is always available (e.g. `…/data.csv.gz` → gzip).
        let url = self.url();
        if let Some(extension) = url.path().rsplit('.').next() {
            if let Some(codec) = Compression::from_extension(extension) {
                return codec;
            }
        }
        // The handle's media type, discovered lazily: this is the magic-byte sniff
        // for a `BytesIO` (whose `stats()` carries no media type) and the file-name
        // lookup for a `LocalPath`.
        #[cfg(feature = "media")]
        {
            if let Some(media) = self.media_type() {
                if let Some(codec) = Compression::from_media(&media) {
                    return codec;
                }
            }
            // Finally a stats-borne content type (e.g. a cloud `Content-Type`).
            if let Ok(stats) = self.stats() {
                if let Some(codec) = Compression::from_stats(&stats) {
                    return codec;
                }
            }
        }
        Compression::None
    }
}

impl<T: Io + ?Sized> CompressIo for T {}

/// Bridges a [`WriteBytes`] sink to [`std::io::Write`], so the streaming
/// compressors (which speak `std::io`) can push into any [`Io`] handle.
#[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
struct WriteShim<W: WriteBytes>(W);

#[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
impl<W: WriteBytes> std::io::Write for WriteShim<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write_bytes(buf).map_err(into_std)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush().map_err(into_std)
    }
}

/// Bridges a [`ReadBytes`] source to [`std::io::Read`], so the streaming
/// decompressors can pull from any [`Io`] handle.
#[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
struct ReadShim<R: ReadBytes>(R);

#[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
impl<R: ReadBytes> std::io::Read for ReadShim<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read_bytes(buf).map_err(into_std)
    }
}

/// Maps an [`IoError`] back into a [`std::io::Error`] for the `std::io`-based
/// codecs (the inverse of `yggdryl-io`'s `From<std::io::Error>`).
#[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
fn into_std(err: IoError) -> std::io::Error {
    std::io::Error::other(err.to_string())
}

/// A streaming compressor returned by [`Compression::encoder`]. Implements
/// [`WriteBytes`]; everything written is compressed into the wrapped sink. Call
/// [`finish`](Encoder::finish) to write the trailer and recover the sink.
pub struct Encoder<W: WriteBytes> {
    inner: EncoderInner<W>,
}

// Exactly one variant is ever live and the value is short-lived (built, written,
// finished), so the codec states are kept inline rather than boxed — that keeps
// an extra indirection off the per-write streaming path.
#[allow(clippy::large_enum_variant)]
enum EncoderInner<W: WriteBytes> {
    /// Identity: writes pass straight through.
    Store(W),
    #[cfg(feature = "gzip")]
    Gzip(flate2::write::GzEncoder<WriteShim<W>>),
    #[cfg(feature = "zstd")]
    Zstd(zstd::stream::write::Encoder<'static, WriteShim<W>>),
    #[cfg(feature = "snappy")]
    Snappy(snap::write::FrameEncoder<WriteShim<W>>),
}

impl<W: WriteBytes> Encoder<W> {
    /// Finishes the compressed stream — flushing any buffered bytes and writing
    /// the trailer/checksum — and returns the underlying sink. **Must** be called
    /// to produce a valid stream.
    pub fn finish(self) -> Result<W, IoError> {
        match self.inner {
            EncoderInner::Store(sink) => Ok(sink),
            #[cfg(feature = "gzip")]
            EncoderInner::Gzip(encoder) => Ok(encoder.finish().map_err(IoError::from)?.0),
            #[cfg(feature = "zstd")]
            EncoderInner::Zstd(encoder) => Ok(encoder.finish().map_err(IoError::from)?.0),
            #[cfg(feature = "snappy")]
            EncoderInner::Snappy(encoder) => Ok(encoder
                .into_inner()
                .map_err(|err| IoError::Io(err.to_string()))?
                .0),
        }
    }
}

impl<W: WriteBytes> WriteBytes for Encoder<W> {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        match &mut self.inner {
            EncoderInner::Store(sink) => sink.write_bytes(bytes),
            #[cfg(feature = "gzip")]
            EncoderInner::Gzip(encoder) => {
                std::io::Write::write(encoder, bytes).map_err(IoError::from)
            }
            #[cfg(feature = "zstd")]
            EncoderInner::Zstd(encoder) => {
                std::io::Write::write(encoder, bytes).map_err(IoError::from)
            }
            #[cfg(feature = "snappy")]
            EncoderInner::Snappy(encoder) => {
                std::io::Write::write(encoder, bytes).map_err(IoError::from)
            }
        }
    }

    fn flush(&mut self) -> Result<(), IoError> {
        match &mut self.inner {
            EncoderInner::Store(sink) => sink.flush(),
            #[cfg(feature = "gzip")]
            EncoderInner::Gzip(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
            #[cfg(feature = "zstd")]
            EncoderInner::Zstd(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
            #[cfg(feature = "snappy")]
            EncoderInner::Snappy(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
        }
    }
}

/// A streaming decompressor returned by [`Compression::decoder`]. Implements
/// [`ReadBytes`]; reads pull compressed bytes from the wrapped source and yield
/// the decompressed stream until it drains.
pub struct Decoder<R: ReadBytes> {
    inner: DecoderInner<R>,
}

enum DecoderInner<R: ReadBytes> {
    /// Identity: reads pass straight through.
    Store(R),
    #[cfg(feature = "gzip")]
    Gzip(flate2::read::GzDecoder<ReadShim<R>>),
    #[cfg(feature = "zstd")]
    Zstd(zstd::stream::read::Decoder<'static, std::io::BufReader<ReadShim<R>>>),
    #[cfg(feature = "snappy")]
    Snappy(snap::read::FrameDecoder<ReadShim<R>>),
}

impl<R: ReadBytes> ReadBytes for Decoder<R> {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        match &mut self.inner {
            DecoderInner::Store(source) => source.read_bytes(buf),
            #[cfg(feature = "gzip")]
            DecoderInner::Gzip(decoder) => std::io::Read::read(decoder, buf).map_err(IoError::from),
            #[cfg(feature = "zstd")]
            DecoderInner::Zstd(decoder) => std::io::Read::read(decoder, buf).map_err(IoError::from),
            #[cfg(feature = "snappy")]
            DecoderInner::Snappy(decoder) => {
                std::io::Read::read(decoder, buf).map_err(IoError::from)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_extensions() {
        assert_eq!(Compression::from_str("gzip").unwrap(), Compression::Gzip);
        assert_eq!(Compression::from_str("GZ").unwrap(), Compression::Gzip);
        assert_eq!(Compression::from_str("zst").unwrap(), Compression::Zstd);
        assert_eq!(
            Compression::from_str(" snappy ").unwrap(),
            Compression::Snappy
        );
        assert_eq!(Compression::from_str("store").unwrap(), Compression::None);
        assert!(matches!(
            Compression::from_str("lzo"),
            Err(IoError::Invalid(_))
        ));

        assert_eq!(Compression::from_extension(".gz"), Some(Compression::Gzip));
        assert_eq!(Compression::from_extension("zst"), Some(Compression::Zstd));
        assert_eq!(Compression::from_extension("txt"), None);

        assert_eq!(Compression::Gzip.as_str(), "gzip");
        assert_eq!(Compression::Zstd.extension(), Some("zst"));
        assert_eq!(Compression::None.extension(), None);
        assert!(Compression::None.is_available());
    }

    #[test]
    fn none_is_an_identity_passthrough() {
        let codec = Compression::None;
        assert!(codec.is_available());
        let payload = b"the quick brown fox";
        let packed = codec.compress(payload).unwrap();
        assert_eq!(packed, payload); // store: bytes unchanged
        assert_eq!(codec.decompress(&packed).unwrap(), payload);
    }

    #[test]
    fn unavailable_codec_reports_unsupported() {
        // A codec whose feature is off cannot build an encoder/decoder, but it
        // still parses and names itself.
        for codec in [Compression::Gzip, Compression::Zstd, Compression::Snappy] {
            if !codec.is_available() {
                assert!(matches!(codec.compress(b"x"), Err(IoError::Unsupported(_))));
                assert!(matches!(
                    codec.decompress(b"x"),
                    Err(IoError::Unsupported(_))
                ));
            }
        }
    }

    /// Round-trips each compiled-in codec both one-shot and **streamed** over a
    /// [`BytesIO`] handle, proving `Compression` composes with `Io`.
    #[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
    #[test]
    fn round_trips_each_available_codec() {
        let payload: Vec<u8> = (0..4096u32).map(|n| (n % 251) as u8).collect();
        for codec in [Compression::Gzip, Compression::Zstd, Compression::Snappy] {
            if !codec.is_available() {
                continue;
            }
            // One-shot.
            let packed = codec.compress(&payload).unwrap();
            assert_eq!(codec.decompress(&packed).unwrap(), payload, "{codec}");

            // Streamed compress into a BytesIO sink…
            let mut encoder = codec.encoder(BytesIO::new()).unwrap();
            encoder.write_all(&payload).unwrap();
            let mut sink = encoder.finish().unwrap();
            sink.seek(0, Whence::Start).unwrap();

            // …then streamed decompress straight out of that handle.
            let mut decoder = codec.decoder(sink).unwrap();
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).unwrap();
            assert_eq!(out, payload, "{codec} streamed");
        }
    }

    /// The `CompressIo` extension trait round-trips an `Io` handle into a
    /// compressed `BytesIO` and back.
    #[cfg(any(feature = "gzip", feature = "zstd", feature = "snappy"))]
    #[test]
    fn io_compress_then_decompress_round_trips() {
        let payload: Vec<u8> = (0..2048u32).map(|n| (n % 251) as u8).collect();
        for codec in [Compression::Gzip, Compression::Zstd, Compression::Snappy] {
            if !codec.is_available() {
                continue;
            }
            let mut src = BytesIO::from_bytes(payload.clone());
            let mut packed = src.compress(codec).unwrap();
            assert!(!packed.is_empty());
            // Round-trip back, passing the codec explicitly.
            let out = packed.decompress(Some(codec)).unwrap();
            assert_eq!(out.getvalue(), &payload[..], "{codec}");
        }
    }

    /// Decompress with no codec given infers gzip from the **magic bytes** of an
    /// in-memory `BytesIO` (whose `mem://` URL has no extension and whose `stats()`
    /// carries no media type), exercising the lazy `media_type()` sniff.
    #[cfg(all(feature = "gzip", feature = "media"))]
    #[test]
    fn io_decompress_infers_codec_from_magic_bytes() {
        let packed = Compression::Gzip.compress(b"sniffed from magic").unwrap();
        let mut handle = BytesIO::from_bytes(packed);
        assert_eq!(handle.compression(), Compression::Gzip);
        let out = handle.decompress(None).unwrap();
        assert_eq!(out.getvalue(), b"sniffed from magic");
    }

    /// Decompress with no codec given infers gzip from a `.gz` URL extension.
    #[cfg(feature = "gzip")]
    #[test]
    fn io_decompress_infers_codec_from_url_extension() {
        use std::io::Write;
        let path = std::env::temp_dir().join("yggdryl_compression_infer.gz");
        let mut raw = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        raw.write_all(b"inferred payload").unwrap();
        std::fs::write(&path, raw.finish().unwrap()).unwrap();

        let mut handle = yggdryl_io::LocalPath::open(path.to_str().unwrap());
        assert_eq!(handle.compression(), Compression::Gzip);
        let out = handle.decompress(None).unwrap();
        assert_eq!(out.getvalue(), b"inferred payload");
        std::fs::remove_file(&path).ok();
    }

    #[cfg(feature = "media")]
    #[test]
    fn infers_codec_from_mime_and_stats() {
        use yggdryl_io::IoStats;
        use yggdryl_media::{FromInput, MediaType, MimeType};

        assert_eq!(
            Compression::from_mime(&MimeType::Gzip),
            Some(Compression::Gzip)
        );
        assert_eq!(Compression::from_mime(&MimeType::Json), None);

        let media = MediaType::from_str("data.csv.gz").unwrap();
        assert_eq!(Compression::from_media(&media), Some(Compression::Gzip));

        let stats = IoStats::new(0).with_media_type(media);
        assert_eq!(Compression::from_stats(&stats), Some(Compression::Gzip));

        let by_content = IoStats::new(0).with_content_type("application/zstd");
        assert_eq!(
            Compression::from_stats(&by_content),
            Some(Compression::Zstd)
        );
    }
}
