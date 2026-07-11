//! [`Gzip`] — the gzip (RFC 1952) compression codec.

use std::io::{self, Read, Seek, Write};

// `flate2` is the de-facto gzip/deflate backend; pulled in only under the
// off-by-default `gzip` feature so a build that does not need gzip stays lean.
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression as Flate2Compression;

use yggdryl_buffer::IOBase;
use yggdryl_core::{DecodeError, Decoder, EncodeError, Encoder, TypedDecoder, TypedEncoder};

use crate::{Compression, CompressionDecoder, CompressionEncoder};

/// The gzip (RFC 1952) compression codec, backed by `flate2`.
///
/// Construct with a compression level in `0..=9` (`0` = store, `9` = best), then
/// [`encode_byte_array`](Encoder::encode_byte_array) to compress and
/// [`decode_byte_array`](Decoder::decode_byte_array) to decompress. The codec
/// round-trips through bytes via [`serialize_bytes`](Gzip::serialize_bytes) /
/// [`deserialize_bytes`](Gzip::deserialize_bytes).
///
/// ```
/// use yggdryl_compression::{Decoder, Encoder, Gzip};
///
/// let gzip = Gzip::new(6).unwrap();
/// let original = b"the quick brown fox".repeat(8);
/// let compressed = gzip.encode_byte_array(&original).unwrap();
/// let restored = gzip.decode_byte_array(&compressed).unwrap();
/// assert_eq!(restored, original);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Gzip {
    level: u32,
}

impl Gzip {
    /// The default compression level (`6`), balancing speed against ratio.
    pub const DEFAULT_LEVEL: u32 = 6;

    /// Creates a gzip codec at `level` (`0..=9`).
    ///
    /// # Errors
    /// Returns [`EncodeError::InvalidLevel`] if `level` exceeds `9`.
    pub fn new(level: u32) -> Result<Self, EncodeError> {
        if level > 9 {
            return Err(EncodeError::InvalidLevel {
                level: level.into(),
                min: 0,
                max: 9,
            });
        }
        Ok(Self { level })
    }

    /// The configured compression level.
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Serialises this codec to its single-byte form (the level).
    ///
    /// ```
    /// use yggdryl_compression::Gzip;
    ///
    /// let gzip = Gzip::new(9).unwrap();
    /// assert_eq!(Gzip::deserialize_bytes(&gzip.serialize_bytes()).unwrap(), gzip);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        vec![self.level as u8]
    }

    /// Reconstructs a codec from [`serialize_bytes`](Gzip::serialize_bytes).
    ///
    /// # Errors
    /// Returns [`DecodeError::InvalidData`] if `bytes` is not exactly one byte,
    /// or encodes a level outside `0..=9`.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        match bytes {
            [level] if u32::from(*level) <= 9 => Ok(Self {
                level: u32::from(*level),
            }),
            [level] => Err(DecodeError::InvalidData(format!(
                "gzip level {level} out of range; expected 0..=9"
            ))),
            _ => Err(DecodeError::InvalidData(format!(
                "expected 1 byte for a gzip codec, got {}",
                bytes.len()
            ))),
        }
    }
}

impl Default for Gzip {
    fn default() -> Self {
        Self {
            level: Self::DEFAULT_LEVEL,
        }
    }
}

impl Encoder for Gzip {
    fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError> {
        let mut encoder = GzEncoder::new(Vec::new(), Flate2Compression::new(self.level));
        encoder.write_all(bytes)?;
        Ok(encoder.finish()?)
    }
}

impl Decoder for Gzip {
    fn decode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
        let mut decoder = GzDecoder::new(bytes);
        let mut out = Vec::with_capacity(decompressed_size_hint(bytes));
        decoder.read_to_end(&mut out)?;
        Ok(out)
    }
}

/// Preallocation hint from the gzip trailer's ISIZE (last 4 bytes, little-endian =
/// the uncompressed size mod 2^32), capped at a generous multiple of the compressed
/// length so a corrupt/hostile ISIZE cannot trigger a huge allocation.
fn decompressed_size_hint(bytes: &[u8]) -> usize {
    let Some(start) = bytes.len().checked_sub(4) else {
        return 0;
    };
    let isize_field = u32::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
    ]) as usize;
    // DEFLATE's maximum ratio is ~1032×; 4096× bounds the trust in ISIZE.
    isize_field.min(bytes.len().saturating_mul(4096))
}

impl TypedEncoder<u8> for Gzip {
    fn encode(&self, items: &[u8]) -> Result<Vec<u8>, EncodeError> {
        self.encode_byte_array(items)
    }
}

impl TypedDecoder<u8> for Gzip {
    fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
        self.decode_byte_array(bytes)
    }
}

impl CompressionEncoder for Gzip {
    /// True streaming compression: `flate2` deflates `source` into `sink` through a
    /// fixed-size copy buffer, so memory stays bounded regardless of input size.
    fn compress_stream(
        &self,
        source: &mut dyn IOBase,
        sink: &mut dyn IOBase,
    ) -> Result<u64, EncodeError> {
        let start = sink.stream_position()?;
        let mut encoder = GzEncoder::new(&mut *sink, Flate2Compression::new(self.level));
        io::copy(&mut *source, &mut encoder)?;
        encoder.finish()?;
        // Bytes written = how far the sink cursor advanced (writes only move forward).
        Ok(sink.stream_position()?.saturating_sub(start))
    }
}

impl CompressionDecoder for Gzip {
    /// True streaming decompression: `flate2` inflates `source` into `sink` through
    /// a fixed-size copy buffer, so memory stays bounded regardless of input size.
    fn decompress_stream(
        &self,
        source: &mut dyn IOBase,
        sink: &mut dyn IOBase,
    ) -> Result<u64, DecodeError> {
        let start = sink.stream_position()?;
        io::copy(&mut GzDecoder::new(&mut *source), &mut *sink)?;
        Ok(sink.stream_position()?.saturating_sub(start))
    }
}

impl Compression for Gzip {
    fn name(&self) -> &'static str {
        "gzip"
    }
}
