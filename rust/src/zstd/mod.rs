//! RFC 8878 Zstandard framing.

use std::io::{Read, Write};

use crate::Result;

use crate::enums::codec::{Encoder, EncoderKind, FlateFinish};
use crate::io::{Coded, IOBase};
use crate::{Codec, Level};

/// Decode a complete Zstandard frame.
///
/// # Errors
///
/// Returns an error when `input` is not a valid Zstandard stream.
pub fn load(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoded = Vec::new();
    zstd::stream::read::Decoder::new(input)?.read_to_end(&mut decoded)?;
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
    Ok(zstd::stream::encode_all(input, level.zstd())?)
}

/// Wrap a reader so it yields decoded bytes without buffering the payload.
pub fn reader<'source, R: Read + 'source>(source: R) -> Box<dyn Read + 'source> {
    match zstd::stream::read::Decoder::new(source) {
        Ok(decoder) => Box::new(decoder),
        // Construction only fails when the decoder cannot allocate its window.
        // Surface that on the first read rather than panicking here.
        Err(error) => Box::new(FailingRead(Some(error))),
    }
}

/// [`reader`], with the `Send` the decoder already has made visible.
pub(crate) fn reader_send<'source, R: Read + Send + 'source>(
    source: R,
) -> Box<dyn Read + Send + 'source> {
    match zstd::stream::read::Decoder::new(source) {
        Ok(decoder) => Box::new(decoder),
        Err(error) => Box::new(FailingRead(Some(error))),
    }
}

/// Wrap a writer so written bytes are Zstandard-encoded at the default level.
pub fn writer<'target, W: Write + 'target>(target: W) -> Encoder<'target> {
    writer_with_level(target, Level::DEFAULT)
}

/// Wrap a writer so written bytes are Zstandard-encoded at an explicit level.
pub fn writer_with_level<'target, W: Write + 'target>(target: W, level: Level) -> Encoder<'target> {
    match zstd::stream::write::Encoder::new(target, level.zstd()) {
        Ok(encoder) => Encoder(EncoderKind::Zstd(Box::new(encoder))),
        Err(error) => Encoder(EncoderKind::Zstd(Box::new(FailingWrite(Some(error))))),
    }
}

impl<W: Write> FlateFinish for zstd::stream::write::Encoder<'_, W> {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()> {
        (*self).finish().map(|_| ())
    }
}

/// A reader that reports a deferred construction failure.
struct FailingRead(Option<std::io::Error>);

impl Read for FailingRead {
    fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
        Err(self.0.take().unwrap_or_else(|| {
            std::io::Error::other("the Zstandard decoder could not be constructed")
        }))
    }
}

/// A writer that reports a deferred construction failure.
struct FailingWrite(Option<std::io::Error>);

impl Write for FailingWrite {
    fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
        Err(self.0.take().unwrap_or_else(|| {
            std::io::Error::other("the Zstandard encoder could not be constructed")
        }))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl FlateFinish for FailingWrite {
    fn finish_boxed(mut self: Box<Self>) -> std::io::Result<()> {
        Err(self.0.take().unwrap_or_else(|| {
            std::io::Error::other("the Zstandard encoder could not be constructed")
        }))
    }
}

/// A transparent Zstandard buffer over one byte handle.
///
/// Reads decompress and writes compress, so anything that takes an [`IOBase`] -
/// a media reader, a codec, another handle - sees the decoded bytes while the
/// wrapped handle holds the Zstandard form. The decoded value is materialized on
/// first use and published on [`IOBase::flush`] or [`IOBase::close`], because a
/// coding cannot be written positionally.
///
/// ```
/// use yggdryl::io::{Buffer, IOBase};
/// use yggdryl::zstd::Zstd;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut handle = Zstd::new(Buffer::new());
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
pub struct Zstd<H: IOBase> {
    coded: Coded<H>,
}

impl<H: IOBase> Zstd<H> {
    /// Wrap a handle in a Zstandard coding without touching it.
    pub fn new(handle: H) -> Self {
        Self {
            coded: Coded::new(handle, Codec::Zstd),
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

impl<H: IOBase> IOBase for Zstd<H> {
    crate::delegate_iobase!(coded);

    /// The borrowed projection reads a snapshot of the *decoded* value: this
    /// view presents bytes its location does not hold, so the default's
    /// reopen-the-location route would hand back the encoded form. See
    /// [`crate::io::Coded`]'s override.
    #[cfg(feature = "arrow")]
    fn read_arrow_lines(
        &self,
        options: &crate::io::LineRecordOptions,
    ) -> crate::Result<crate::arrow::BatchReader>
    where
        Self: Sized,
    {
        crate::io::lines::snapshot_arrow_lines(self, options)
    }

    fn open(&mut self) -> crate::Result<()> {
        self.coded.open()
    }

    fn is_open(&self) -> bool {
        self.coded.is_open()
    }

    fn close(&mut self) -> crate::Result<()> {
        self.coded.close()
    }
}

#[cfg(test)]
mod tests;
