//! [`CompressionEncoder`] — an [`Encoder`] that compresses, with IO streaming.

use std::io::{Read, Write};

use crate::{EncodeError, Encoder, IOBase};

/// An [`Encoder`] whose [`encode_byte_array`](Encoder::encode_byte_array)
/// **compresses** its input: the output is a losslessly compressed form of the
/// bytes, recoverable by the matching
/// [`CompressionDecoder`](crate::CompressionDecoder).
///
/// On top of the one-shot [`Encoder`] surface it adds
/// [`compress_stream`](CompressionEncoder::compress_stream), which compresses from
/// one [`IOBase`] resource into another — the streaming path between, say, two
/// [`ByteBuffer`](crate::ByteBuffer)s. The default reads the source fully then
/// encodes; codecs whose backend streams (e.g. [`Gzip`](crate::Gzip)) override it
/// to run in bounded memory.
///
/// ```
/// use yggdryl_core::{ByteBuffer, CompressionEncoder, Gzip, IOBase};
///
/// let gzip = Gzip::new(6).unwrap();
/// let original = b"stream me ".repeat(64);
/// let mut source = ByteBuffer::from_bytes(&original).byte_cursor();
/// let mut sink = ByteBuffer::new().byte_cursor();
///
/// let written = gzip.compress_stream(&mut source, &mut sink).unwrap();
/// // `byte_size` is the *remaining* bytes (0 at the end); the total is the cursor's bytes.
/// assert_eq!(written, sink.as_bytes().len() as u64);
/// assert!(sink.as_bytes().len() < original.len());
/// ```
pub trait CompressionEncoder: Encoder {
    /// Compresses every byte remaining in `source` (from its cursor) into `sink`,
    /// advancing both cursors, and returns the number of bytes written to `sink`.
    ///
    /// # Errors
    /// Returns [`EncodeError`] on an encoding failure or an underlying IO error.
    fn compress_stream(
        &self,
        source: &mut dyn IOBase,
        sink: &mut dyn IOBase,
    ) -> Result<u64, EncodeError> {
        let mut input = Vec::new();
        source.read_to_end(&mut input)?;
        let output = self.encode_byte_array(&input)?;
        sink.write_all(&output)?;
        Ok(output.len() as u64)
    }
}
