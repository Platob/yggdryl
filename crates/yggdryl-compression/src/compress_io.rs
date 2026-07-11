//! [`CompressIO`] — compress/decompress an IO's bytes with a codec.

use yggdryl_buffer::{ByteBuffer, ByteCursor, IOBase, IoError, Whence};
use yggdryl_core::{DecodeError, EncodeError};

use crate::{CompressionDecoder, CompressionEncoder};

/// Reads every byte from the current position to the end of `io`.
fn read_remaining<T: IOBase + ?Sized>(io: &mut T) -> Result<Vec<u8>, IoError> {
    // `byte_size` already reports the bytes remaining from the current position.
    let remaining = io.byte_size()?;
    io.pread_byte_array(remaining, Whence::Current)
}

/// Codec integration for any [`IOBase`]: compress or decompress the resource's
/// remaining bytes with a [`Compression`](crate::Compression) codec, returning a new
/// IO ([`ByteCursor`]) over the result.
///
/// Blanket-implemented for every `IOBase`, so any cursor gains
/// [`compress`](CompressIO::compress) / [`decompress`](CompressIO::decompress). It
/// is the one-shot counterpart of the streaming
/// [`compress_stream`](CompressionEncoder::compress_stream).
///
/// ```
/// use yggdryl_compression::{ByteBuffer, CompressIO, Gzip, IOBase, Whence};
///
/// let gzip = Gzip::new(6).unwrap();
/// let mut data = ByteBuffer::from_bytes(&b"compress me ".repeat(50)).byte_cursor();
///
/// let mut compressed = data.compress(&gzip).unwrap();
/// compressed.byte_seek(0, Whence::Start).unwrap();
/// let restored = compressed.decompress(&gzip).unwrap();
/// assert_eq!(restored.as_bytes(), b"compress me ".repeat(50));
/// ```
#[allow(clippy::upper_case_acronyms)] // `IO` matches the project's IO-trait naming.
pub trait CompressIO: IOBase {
    /// Compresses this resource's remaining bytes with `codec`, returning a cursor
    /// over the compressed bytes. Advances this cursor to the end.
    fn compress(&mut self, codec: &dyn CompressionEncoder) -> Result<ByteCursor, EncodeError> {
        let bytes = read_remaining(self).map_err(|error| EncodeError::Io(error.to_string()))?;
        let compressed = codec.encode_byte_array(&bytes)?;
        Ok(ByteBuffer::from_vec(compressed).byte_cursor())
    }

    /// Decompresses this resource's remaining bytes with `codec`, returning a cursor
    /// over the decompressed bytes. Advances this cursor to the end.
    fn decompress(&mut self, codec: &dyn CompressionDecoder) -> Result<ByteCursor, DecodeError> {
        let bytes = read_remaining(self).map_err(|error| DecodeError::Io(error.to_string()))?;
        let decompressed = codec.decode_byte_array(&bytes)?;
        Ok(ByteBuffer::from_vec(decompressed).byte_cursor())
    }
}

impl<T: IOBase + ?Sized> CompressIO for T {}
