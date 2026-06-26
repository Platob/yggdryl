//! The streaming [`Encoder`] / [`Decoder`] [`Io`] adapters and the internal
//! `std::io` shims that bridge to the backend stream codecs.

use std::fmt;

use crate::io::{Io, IoError, IoStats, Whence};
use crate::Url;

/// Bridges an [`Io`] sink to [`std::io::Write`], so the streaming compressors
/// (which speak `std::io`) can push into any handle.
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "snappy",
    feature = "brotli"
))]
pub(crate) struct WriteShim<W: Io>(pub(crate) W);

#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "snappy",
    feature = "brotli"
))]
impl<W: Io> std::io::Write for WriteShim<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf).map_err(into_std)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Io::flush(&mut self.0).map_err(into_std)
    }
}

/// Bridges an [`Io`] source to [`std::io::Read`], so the streaming decompressors
/// can pull from any handle.
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "snappy",
    feature = "brotli"
))]
pub(crate) struct ReadShim<R: Io>(pub(crate) R);

#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "snappy",
    feature = "brotli"
))]
impl<R: Io> std::io::Read for ReadShim<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf).map_err(into_std)
    }
}

/// Maps an [`IoError`] back into a [`std::io::Error`] for the `std::io`-based
/// codecs (the inverse of the byte-IO `From<std::io::Error>`).
#[cfg(any(
    feature = "gzip",
    feature = "zstd",
    feature = "snappy",
    feature = "brotli"
))]
fn into_std(err: IoError) -> std::io::Error {
    std::io::Error::other(err.to_string())
}

/// A streaming compressor returned by [`Compression::encoder`](crate::Compression::encoder).
/// A write-only [`Io`] handle: everything written is compressed into the wrapped
/// sink. Call [`finish`](Encoder::finish) to write the trailer and recover the sink.
pub struct Encoder<W: Io> {
    pub(crate) inner: EncoderInner<W>,
}

// Exactly one variant is ever live and the value is short-lived (built, written,
// finished), so the codec states are kept inline rather than boxed — that keeps
// an extra indirection off the per-write streaming path.
#[allow(clippy::large_enum_variant)]
pub(crate) enum EncoderInner<W: Io> {
    /// Identity: writes pass straight through.
    Store(W),
    #[cfg(feature = "gzip")]
    Gzip(flate2::write::GzEncoder<WriteShim<W>>),
    #[cfg(feature = "gzip")]
    Deflate(flate2::write::ZlibEncoder<WriteShim<W>>),
    #[cfg(feature = "zstd")]
    Zstd(zstd::stream::write::Encoder<'static, WriteShim<W>>),
    #[cfg(feature = "snappy")]
    Snappy(snap::write::FrameEncoder<WriteShim<W>>),
    #[cfg(feature = "brotli")]
    Brotli(brotli::CompressorWriter<WriteShim<W>>),
}

impl<W: Io> Encoder<W> {
    /// Finishes the compressed stream — flushing any buffered bytes and writing
    /// the trailer/checksum — and returns the underlying sink. **Must** be called
    /// to produce a valid stream.
    pub fn finish(self) -> Result<W, IoError> {
        match self.inner {
            EncoderInner::Store(sink) => Ok(sink),
            #[cfg(feature = "gzip")]
            EncoderInner::Gzip(encoder) => Ok(encoder.finish().map_err(IoError::from)?.0),
            #[cfg(feature = "gzip")]
            EncoderInner::Deflate(encoder) => Ok(encoder.finish().map_err(IoError::from)?.0),
            #[cfg(feature = "zstd")]
            EncoderInner::Zstd(encoder) => Ok(encoder.finish().map_err(IoError::from)?.0),
            #[cfg(feature = "snappy")]
            EncoderInner::Snappy(encoder) => Ok(encoder
                .into_inner()
                .map_err(|err| IoError::Io(err.to_string()))?
                .0),
            #[cfg(feature = "brotli")]
            EncoderInner::Brotli(encoder) => Ok(encoder.into_inner().0),
        }
    }
}

impl<W: Io> fmt::Debug for Encoder<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encoder").finish_non_exhaustive()
    }
}

impl<W: Io> Io for Encoder<W> {
    /// A synthetic in-memory address — an encoder is a transient streaming adapter.
    fn url(&self) -> Url {
        Url::new("mem", "encoder")
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(0))
    }

    fn seek(&mut self, _offset: i64, _whence: Whence) -> Result<u64, IoError> {
        Err(IoError::Unsupported(
            "seek on a streaming encoder (it is write-only and forward-only)".to_string(),
        ))
    }

    fn stream_position(&self) -> u64 {
        0
    }

    /// Compresses `bytes` into the wrapped sink, returning the count consumed.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        match &mut self.inner {
            EncoderInner::Store(sink) => sink.write(bytes),
            #[cfg(feature = "gzip")]
            EncoderInner::Gzip(encoder) => {
                std::io::Write::write(encoder, bytes).map_err(IoError::from)
            }
            #[cfg(feature = "gzip")]
            EncoderInner::Deflate(encoder) => {
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
            #[cfg(feature = "brotli")]
            EncoderInner::Brotli(encoder) => {
                std::io::Write::write(encoder, bytes).map_err(IoError::from)
            }
        }
    }

    fn flush(&mut self) -> Result<(), IoError> {
        match &mut self.inner {
            EncoderInner::Store(sink) => Io::flush(sink),
            #[cfg(feature = "gzip")]
            EncoderInner::Gzip(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
            #[cfg(feature = "gzip")]
            EncoderInner::Deflate(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
            #[cfg(feature = "zstd")]
            EncoderInner::Zstd(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
            #[cfg(feature = "snappy")]
            EncoderInner::Snappy(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
            #[cfg(feature = "brotli")]
            EncoderInner::Brotli(encoder) => std::io::Write::flush(encoder).map_err(IoError::from),
        }
    }
}

/// A streaming decompressor returned by [`Compression::decoder`](crate::Compression::decoder).
/// A read-only [`Io`] handle: reads pull compressed bytes from the wrapped source
/// and yield the decompressed stream until it drains.
pub struct Decoder<R: Io> {
    pub(crate) inner: DecoderInner<R>,
}

// As with `EncoderInner`: exactly one variant is ever live and the value is
// short-lived, so the codec states are kept inline rather than boxed.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DecoderInner<R: Io> {
    /// Identity: reads pass straight through.
    Store(R),
    #[cfg(feature = "gzip")]
    Gzip(flate2::read::GzDecoder<ReadShim<R>>),
    #[cfg(feature = "gzip")]
    Deflate(flate2::read::ZlibDecoder<ReadShim<R>>),
    #[cfg(feature = "zstd")]
    Zstd(zstd::stream::read::Decoder<'static, std::io::BufReader<ReadShim<R>>>),
    #[cfg(feature = "snappy")]
    Snappy(snap::read::FrameDecoder<ReadShim<R>>),
    #[cfg(feature = "brotli")]
    Brotli(brotli::Decompressor<ReadShim<R>>),
}

impl<R: Io> fmt::Debug for Decoder<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Decoder").finish_non_exhaustive()
    }
}

impl<R: Io> Io for Decoder<R> {
    /// A synthetic in-memory address — a decoder is a transient streaming adapter.
    fn url(&self) -> Url {
        Url::new("mem", "decoder")
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(0))
    }

    fn seek(&mut self, _offset: i64, _whence: Whence) -> Result<u64, IoError> {
        Err(IoError::Unsupported(
            "seek on a streaming decoder (it is read-only and forward-only)".to_string(),
        ))
    }

    fn stream_position(&self) -> u64 {
        0
    }

    /// Decompresses into `buf` from the wrapped source, returning the count read.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        match &mut self.inner {
            DecoderInner::Store(source) => source.read(buf),
            #[cfg(feature = "gzip")]
            DecoderInner::Gzip(decoder) => std::io::Read::read(decoder, buf).map_err(IoError::from),
            #[cfg(feature = "gzip")]
            DecoderInner::Deflate(decoder) => {
                std::io::Read::read(decoder, buf).map_err(IoError::from)
            }
            #[cfg(feature = "zstd")]
            DecoderInner::Zstd(decoder) => std::io::Read::read(decoder, buf).map_err(IoError::from),
            #[cfg(feature = "snappy")]
            DecoderInner::Snappy(decoder) => {
                std::io::Read::read(decoder, buf).map_err(IoError::from)
            }
            #[cfg(feature = "brotli")]
            DecoderInner::Brotli(decoder) => {
                std::io::Read::read(decoder, buf).map_err(IoError::from)
            }
        }
    }
}
