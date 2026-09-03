//! RFC 1952 gzip framing over DEFLATE.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::Result;

use crate::generic::codec::{Encoder, EncoderKind, FlateFinish};
use crate::io::{Coded, IOBase};
use crate::{Codec, Level};

/// Decode a complete gzip member.
///
/// # Errors
///
/// Returns an error when `input` is not a valid gzip stream.
pub fn load(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    GzDecoder::new(input).read_to_end(&mut decoded)?;
    Ok(decoded)
}

/// Encode a complete buffer at the default level.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn dump(input: &[u8]) -> Result<Vec<u8>> {
    dump_with_level(input, Level::DEFAULT)
}

/// Encode a complete buffer at an explicit level.
///
/// # Errors
///
/// Returns the encoder's failure.
pub fn dump_with_level(input: &[u8], level: Level) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(u32::from(level.get())));
    encoder.write_all(input)?;
    Ok(encoder.finish()?)
}

/// Wrap a reader so it yields decoded bytes without buffering the payload.
pub fn reader<'source, R: Read + 'source>(source: R) -> Box<dyn Read + 'source> {
    Box::new(GzDecoder::new(source))
}

/// [`reader`], with the `Send` the decoder already has made visible.
pub(crate) fn reader_send<'source, R: Read + Send + 'source>(
    source: R,
) -> Box<dyn Read + Send + 'source> {
    Box::new(GzDecoder::new(source))
}

/// Wrap a writer so written bytes are gzip-encoded at the default level.
pub fn writer<'target, W: Write + 'target>(target: W) -> Encoder<'target> {
    writer_with_level(target, Level::DEFAULT)
}

/// Wrap a writer so written bytes are gzip-encoded at an explicit level.
pub fn writer_with_level<'target, W: Write + 'target>(target: W, level: Level) -> Encoder<'target> {
    Encoder(EncoderKind::Flate(Box::new(GzEncoder::new(
        target,
        Compression::new(u32::from(level.get())),
    ))))
}

impl<W: Write> FlateFinish for GzEncoder<W> {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()> {
        (*self).finish().map(|_| ())
    }
}

/// A transparent gzip buffer over one byte handle.
///
/// Reads decompress and writes compress, so anything that takes an [`IOBase`] -
/// a media reader, a codec, another handle - sees the decoded bytes while the
/// wrapped handle holds the gzip form. Sequential and closed positional reads
/// decode through bounded windows; positional mutation and explicit
/// [`IOBase::open`] materialize the decoded value until close.
///
/// ```
/// use yggdryl::io::{Buffer, IOBase};
/// use yggdryl::gzip::Gzip;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut handle = Gzip::new(Buffer::new());
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
pub struct Gzip<H: IOBase> {
    coded: Coded<H>,
}

impl<H: IOBase> Gzip<H> {
    /// Wrap a handle in a gzip coding without touching it.
    pub fn new(handle: H) -> Self {
        Self {
            coded: Coded::new(handle, Codec::Gzip),
        }
    }

    /// Return this handle with a different compression level.
    #[must_use]
    pub fn with_level(mut self, level: Level) -> Self {
        self.coded = self.coded.with_level(level);
        self
    }

    /// Return the compression level writes use.
    pub const fn level(&self) -> Level {
        self.coded.level()
    }

    /// Borrow the wrapped handle, which holds the compressed bytes.
    pub const fn handle(&self) -> &H {
        self.coded.handle()
    }

    /// Consume this handle, publishing any pending write first.
    ///
    /// # Errors
    ///
    /// Returns the encode or write failure.
    pub fn into_handle(self) -> crate::Result<H> {
        self.coded.into_handle()
    }
}

impl<H: IOBase> crate::io::IOMedia for Gzip<H> {
    crate::delegate_iomedia!(coded);
}

impl<H: IOBase> IOBase for Gzip<H> {
    crate::delegate_iobase!(coded);

    fn read_all_bytes(&self) -> crate::Result<Vec<u8>> {
        self.coded.read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> crate::Result<Vec<u8>> {
        self.coded.read_range_bytes(offset, length)
    }
}

#[cfg(test)]
mod tests;
