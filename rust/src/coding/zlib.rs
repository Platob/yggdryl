//! RFC 1950 zlib framing and raw RFC 1951 DEFLATE.
//!
//! The `*_raw` operations omit the two-byte zlib header and Adler-32 trailer,
//! which is what HTTP's `deflate` coding and several container formats
//! actually carry.

use std::io::{Read, Write};

use flate2::read::{DeflateDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, ZlibEncoder};
use flate2::{Compress, Compression, FlushCompress, Status};

use crate::Result;

use crate::IOBase;
use crate::codec::{CodedWrite, Encoder, EncoderKind};
use crate::coding::Coding;
use crate::{Codec, Level};

/// The bytes a full or sync flush ends with, which a restart point follows.
///
/// Both flushes close the current block and emit an empty *stored* block,
/// whose zero length and its complement are these four bytes. A full flush
/// also drops the window, so what follows decodes with no history behind it.
pub(crate) const RESTART_MARKER: &[u8] = &[0x00, 0x00, 0xff, 0xff];

/// The window the DEFLATE encoder drains into, one call at a time.
const ENCODE_CHUNK: usize = 32 * 1024;

/// The largest DEFLATE window, which is what every stream here is written at.
const WINDOW_BITS: u8 = 15;

/// Decode a complete zlib-framed stream.
///
/// # Errors
///
/// Returns an error when `input` is not a valid zlib stream.
pub fn load(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    ZlibDecoder::new(input).read_to_end(&mut decoded)?;
    Ok(decoded)
}

/// Decode a complete raw DEFLATE stream.
///
/// # Errors
///
/// Returns an error when `input` is not a valid DEFLATE stream.
pub fn load_raw(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    DeflateDecoder::new(input).read_to_end(&mut decoded)?;
    Ok(decoded)
}

/// Encode a complete buffer with zlib framing at the default level.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn dump(input: &[u8]) -> Result<Vec<u8>> {
    dump_with_level(input, Level::DEFAULT)
}

/// Encode a complete buffer with zlib framing at an explicit level.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn dump_with_level(input: &[u8], level: Level) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(u32::from(level.get())));
    encoder.write_all(input)?;
    Ok(encoder.finish()?)
}

/// Encode a complete buffer as raw DEFLATE at the default level.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn dump_raw(input: &[u8]) -> Result<Vec<u8>> {
    dump_raw_with_level(input, Level::DEFAULT)
}

/// Encode a complete buffer as raw DEFLATE at an explicit level.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn dump_raw_with_level(input: &[u8], level: Level) -> Result<Vec<u8>> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(u32::from(level.get())));
    encoder.write_all(input)?;
    Ok(encoder.finish()?)
}

/// Wrap a reader so it yields zlib-decoded bytes.
pub fn reader<'source, R: Read + 'source>(source: R) -> Box<dyn Read + 'source> {
    Box::new(ZlibDecoder::new(source))
}

/// [`reader`], with the `Send` the decoder already has made visible.
pub(crate) fn reader_send<'source, R: Read + Send + 'source>(
    source: R,
) -> Box<dyn Read + Send + 'source> {
    Box::new(ZlibDecoder::new(source))
}

/// Wrap a reader so it yields raw-DEFLATE-decoded bytes.
pub fn raw_reader<'source, R: Read + 'source>(source: R) -> Box<dyn Read + 'source> {
    Box::new(DeflateDecoder::new(source))
}

/// [`raw_reader`], with the `Send` the decoder already has made visible.
pub(crate) fn raw_reader_send<'source, R: Read + Send + 'source>(
    source: R,
) -> Box<dyn Read + Send + 'source> {
    Box::new(DeflateDecoder::new(source))
}

/// Wrap a writer so written bytes are zlib-encoded at the default level.
pub fn writer<'target, W: Write + 'target>(target: W) -> Encoder<'target> {
    writer_with_level(target, Level::DEFAULT)
}

/// Wrap a writer so written bytes are zlib-encoded at an explicit level.
pub fn writer_with_level<'target, W: Write + 'target>(target: W, level: Level) -> Encoder<'target> {
    Encoder {
        kind: EncoderKind::Coded(Box::new(ZlibEncoder::new(
            target,
            Compression::new(u32::from(level.get())),
        ))),
        codec: Codec::Zlib,
    }
}

/// Wrap a writer so written bytes are raw-DEFLATE-encoded at the default level.
pub fn raw_writer<'target, W: Write + 'target>(target: W) -> Encoder<'target> {
    raw_writer_with_level(target, Level::DEFAULT)
}

/// Wrap a writer so written bytes are raw-DEFLATE-encoded at an explicit level.
///
/// This is the one coding here whose stream can be restarted partway in, so
/// it is written through the compressor directly rather than through flate2's
/// writer, which only ever performs the sync flush its `Write` impl needs.
pub fn raw_writer_with_level<'target, W: Write + 'target>(
    target: W,
    level: Level,
) -> Encoder<'target> {
    Encoder {
        kind: EncoderKind::Coded(Box::new(RawWriter::new(Box::new(target), level))),
        codec: Codec::Deflate,
    }
}

impl<W: Write> CodedWrite for ZlibEncoder<W> {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()> {
        (*self).finish().map(|_| ())
    }
}

impl<W: Write> CodedWrite for DeflateEncoder<W> {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()> {
        (*self).finish().map(|_| ())
    }
}

/// A raw DEFLATE writer that chooses its own flush modes.
///
/// The compressor is driven directly so a caller can ask for the *full* flush
/// a restart point needs: it closes the block, aligns the stream to a byte,
/// and drops the window, so the bytes after it decode with nothing behind
/// them. Only one encode window is held, whatever the payload's size.
pub(crate) struct RawWriter<'target> {
    compress: Compress,
    target: Box<dyn Write + 'target>,
    /// The window encoded bytes are drained through, reused every call.
    window: Vec<u8>,
}

impl<'target> RawWriter<'target> {
    /// Encode into `target` at `level`, holding one window.
    fn new(target: Box<dyn Write + 'target>, level: Level) -> Self {
        Self {
            compress: Compress::new_with_window_bits(
                Compression::new(u32::from(level.get())),
                false,
                WINDOW_BITS,
            ),
            target,
            window: Vec::with_capacity(ENCODE_CHUNK),
        }
    }

    /// Feed `input` to the compressor under `flush`, draining what it emits.
    fn drive(&mut self, mut input: &[u8], flush: FlushCompress) -> std::io::Result<()> {
        loop {
            let consumed_before = self.compress.total_in();
            let produced_before = self.compress.total_out();
            self.window.clear();
            let status = self
                .compress
                .compress_vec(input, &mut self.window, flush)
                .map_err(std::io::Error::other)?;
            let consumed = usize::try_from(self.compress.total_in() - consumed_before)
                .unwrap_or(input.len());
            let produced = self.compress.total_out() - produced_before;
            self.target.write_all(&self.window)?;
            input = input.get(consumed..).unwrap_or_default();

            if matches!(status, Status::StreamEnd) {
                return Ok(());
            }
            // A filled window says the compressor had more to give; unread
            // input says the same. Either way, call it again.
            if !input.is_empty() || produced as usize == ENCODE_CHUNK {
                continue;
            }
            if !matches!(flush, FlushCompress::Finish) {
                return Ok(());
            }
            if consumed == 0 && produced == 0 {
                return Err(std::io::Error::other(
                    "the DEFLATE encoder stopped before the stream ended",
                ));
            }
        }
    }
}

impl Write for RawWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.drive(buffer, FlushCompress::None)?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.drive(&[], FlushCompress::Sync)?;
        self.target.flush()
    }
}

impl CodedWrite for RawWriter<'_> {
    fn finish_boxed(mut self: Box<Self>) -> std::io::Result<()> {
        self.drive(&[], FlushCompress::Finish)?;
        self.target.flush()
    }

    /// End the unit with a full flush, which also drops the window.
    fn restart_unit(&mut self) -> std::io::Result<bool> {
        self.drive(&[], FlushCompress::Full)?;
        Ok(true)
    }
}

/// A transparent zlib buffer over one byte handle.
///
/// Reads decompress and writes compress, so anything that takes an [`IOBase`] -
/// a media reader, a codec, another handle - sees the decoded bytes while the
/// wrapped handle holds the zlib form. Sequential and closed positional reads
/// decode through bounded windows; positional mutation and explicit
/// [`IOBase::open`] materialize the decoded value until close.
///
/// ```
/// use yggdryl::{IOBase, holder::Buffer};
/// use yggdryl::coding::zlib::Zlib;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut handle = Zlib::new(Buffer::new());
/// handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
/// handle.flush()?;
///
/// // What the wrapper reads is the plain text.
/// assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
/// // What the wrapped handle holds is the compressed form.
/// assert!(handle.handle().size() > 0);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Zlib<H: IOBase> {
    coding: Coding<H>,
}

impl<H: IOBase> Zlib<H> {
    /// Wrap a handle in a zlib coding without touching it.
    pub fn new(handle: H) -> Self {
        Self {
            coding: Coding::new(handle, Codec::Zlib),
        }
    }

    /// Return this handle with a different compression level.
    #[must_use]
    pub fn with_level(mut self, level: Level) -> Self {
        self.coding = self.coding.with_level(level);
        self
    }

    /// Return the compression level writes use.
    pub const fn level(&self) -> Level {
        self.coding.level()
    }

    /// Borrow the wrapped handle, which holds the compressed bytes.
    pub const fn handle(&self) -> &H {
        self.coding.handle()
    }

    /// Consume this handle, publishing any pending write first.
    ///
    /// # Errors
    ///
    /// Returns the encode or write failure.
    pub fn into_handle(self) -> crate::Result<H> {
        self.coding.into_handle()
    }
}

impl<H: IOBase> crate::IOMedia for Zlib<H> {
    crate::delegate_iomedia!(coding);
}

impl<H: IOBase> IOBase for Zlib<H> {
    crate::delegate_iobase!(coding);
}

#[cfg(test)]
mod tests;
