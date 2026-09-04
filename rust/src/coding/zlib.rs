//! RFC 1950 zlib framing and raw RFC 1951 DEFLATE.
//!
//! The `*_raw` operations omit the two-byte zlib header and Adler-32 trailer,
//! which is what HTTP's `deflate` coding and several container formats
//! actually carry.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::{DeflateDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, ZlibEncoder};

use crate::Result;

use crate::IOBase;
use crate::codec::{Encoder, EncoderKind, FlateFinish};
use crate::coding::Coding;
use crate::{Codec, Level};

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
    Encoder(EncoderKind::Flate(Box::new(ZlibEncoder::new(
        target,
        Compression::new(u32::from(level.get())),
    ))))
}

/// Wrap a writer so written bytes are raw-DEFLATE-encoded at the default level.
pub fn raw_writer<'target, W: Write + 'target>(target: W) -> Encoder<'target> {
    raw_writer_with_level(target, Level::DEFAULT)
}

/// Wrap a writer so written bytes are raw-DEFLATE-encoded at an explicit level.
pub fn raw_writer_with_level<'target, W: Write + 'target>(
    target: W,
    level: Level,
) -> Encoder<'target> {
    Encoder(EncoderKind::Flate(Box::new(DeflateEncoder::new(
        target,
        Compression::new(u32::from(level.get())),
    ))))
}

impl<W: Write> FlateFinish for ZlibEncoder<W> {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()> {
        (*self).finish().map(|_| ())
    }
}

impl<W: Write> FlateFinish for DeflateEncoder<W> {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()> {
        (*self).finish().map(|_| ())
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
/// use yggdryl::{IOBase, io::Buffer};
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

    fn read_all_bytes(&self) -> crate::Result<Vec<u8>> {
        self.coding.read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> crate::Result<Vec<u8>> {
        self.coding.read_range_bytes(offset, length)
    }
}

#[cfg(test)]
mod tests;
