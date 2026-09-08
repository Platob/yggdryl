//! RFC 8878 Zstandard framing.

use std::io::{Read, Write};

use crate::Result;

use crate::IOBase;
use crate::codec::{CodedWrite, Encoder, EncoderKind};
use crate::coding::Coding;
use crate::{Codec, Level};

/// The four bytes every Zstandard frame begins with.
///
/// Frames are self-contained and concatenate, so each one begins a restart
/// point: a decoder handed the bytes from a frame's magic reads that frame and
/// every frame behind it.
pub(crate) const FRAME_MAGIC: &[u8] = &[0x28, 0xb5, 0x2f, 0xfd];

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
    let kind = match FrameWriter::new(Box::new(target), level) {
        Ok(writer) => EncoderKind::Coded(Box::new(writer)),
        Err(error) => EncoderKind::Coded(Box::new(FailingWrite(Some(error)))),
    };
    Encoder {
        kind,
        codec: Codec::Zstd,
    }
}

/// A Zstandard writer that can close one frame and open the next.
///
/// A restart point is a frame boundary, and a frame is only closed by
/// finishing the encoder, so the encoder is held in an option: restarting
/// takes it, finishes it, and builds the next one over the writer it gives
/// back. Only the encoder's own window is held.
struct FrameWriter<'target> {
    encoder: Option<zstd::stream::write::Encoder<'static, Box<dyn Write + 'target>>>,
    level: Level,
}

impl<'target> FrameWriter<'target> {
    /// Open the first frame over `target`.
    fn new(target: Box<dyn Write + 'target>, level: Level) -> std::io::Result<Self> {
        Ok(Self {
            encoder: Some(zstd::stream::write::Encoder::new(target, level.zstd())?),
            level,
        })
    }

    /// Borrow the open frame, naming a writer whose frame was already closed.
    fn open(
        &mut self,
    ) -> std::io::Result<&mut zstd::stream::write::Encoder<'static, Box<dyn Write + 'target>>> {
        self.encoder
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the Zstandard frame was already closed"))
    }
}

impl Write for FrameWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.open()?.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.open()?.flush()
    }
}

impl CodedWrite for FrameWriter<'_> {
    fn finish_boxed(mut self: Box<Self>) -> std::io::Result<()> {
        let encoder = self
            .encoder
            .take()
            .ok_or_else(|| std::io::Error::other("the Zstandard frame was already closed"))?;
        encoder.finish()?.flush()
    }

    /// Close this frame and open the next over the same writer.
    fn restart_unit(&mut self) -> std::io::Result<bool> {
        let encoder = self
            .encoder
            .take()
            .ok_or_else(|| std::io::Error::other("the Zstandard frame was already closed"))?;
        let target = encoder.finish()?;
        self.encoder = Some(zstd::stream::write::Encoder::new(
            target,
            self.level.zstd(),
        )?);
        Ok(true)
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

impl CodedWrite for FailingWrite {
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
/// wrapped handle holds the Zstandard form. Sequential and closed positional
/// reads decode through bounded windows; positional mutation and explicit
/// [`IOBase::open`] materialize the decoded value until close.
///
/// ```
/// use yggdryl::{IOBase, holder::Buffer};
/// use yggdryl::coding::zstd::Zstd;
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
    coding: Coding<H>,
}

impl<H: IOBase> Zstd<H> {
    /// Wrap a handle in a Zstandard coding without touching it.
    pub fn new(handle: H) -> Self {
        Self {
            coding: Coding::new(handle, Codec::Zstd),
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

impl<H: IOBase> crate::IOMedia for Zstd<H> {
    crate::delegate_iomedia!(coding);
}

impl<H: IOBase> IOBase for Zstd<H> {
    crate::delegate_iobase!(coding);
}

#[cfg(test)]
mod tests;
