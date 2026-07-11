//! [`CompressionDecoder`] — a [`Decoder`] that decompresses, with IO streaming.

use std::io::{Read, Write};

use yggdryl_buffer::IOBase;
use yggdryl_core::{DecodeError, Decoder};

/// A [`Decoder`] whose [`decode_byte_array`](Decoder::decode_byte_array)
/// **decompresses** its input, exactly reversing a matching
/// [`CompressionEncoder`](crate::CompressionEncoder).
///
/// On top of the one-shot [`Decoder`] surface it adds
/// [`decompress_stream`](CompressionDecoder::decompress_stream), which decompresses
/// from one [`IOBase`] resource into another. The default reads the source fully
/// then decodes; codecs whose backend streams (e.g. [`Gzip`](crate::Gzip)) override
/// it to run in bounded memory.
///
/// ```
/// use yggdryl_compression::{ByteBuffer, CompressionDecoder, CompressionEncoder, Gzip, IOBase, Whence};
///
/// let gzip = Gzip::new(6).unwrap();
/// let mut source = ByteBuffer::from_bytes(&b"restore me ".repeat(64)).byte_cursor();
/// let mut compressed = ByteBuffer::new().byte_cursor();
/// gzip.compress_stream(&mut source, &mut compressed).unwrap();
///
/// compressed.byte_seek(0, Whence::Start).unwrap(); // rewind before reading back
/// let mut restored = ByteBuffer::new().byte_cursor();
/// gzip.decompress_stream(&mut compressed, &mut restored).unwrap();
/// assert_eq!(restored.as_bytes(), b"restore me ".repeat(64));
/// ```
pub trait CompressionDecoder: Decoder {
    /// Decompresses every byte remaining in `source` (from its cursor) into `sink`,
    /// advancing both cursors, and returns the number of bytes written to `sink`.
    ///
    /// # Errors
    /// Returns [`DecodeError`] on malformed input or an underlying IO error.
    fn decompress_stream(
        &self,
        source: &mut dyn IOBase,
        sink: &mut dyn IOBase,
    ) -> Result<u64, DecodeError> {
        let mut input = Vec::new();
        source.read_to_end(&mut input)?;
        let output = self.decode_byte_array(&input)?;
        sink.write_all(&output)?;
        Ok(output.len() as u64)
    }
}
