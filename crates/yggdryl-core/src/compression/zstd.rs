//! [`Zstd`] — the Zstandard (RFC 8878) compression codec.

use std::io;

// `zstd` bundles libzstd (built via `cc`); pulled in only under the `zstd` feature.
use zstd::stream::read::Decoder as ZstdDecoder;
use zstd::stream::write::Encoder as ZstdEncoder;

use super::stream::{IoReader, IoWriter};
use crate::{
    Compression, CompressionDecoder, CompressionEncoder, DecodeError, Decoder, EncodeError,
    Encoder, IOBase, TypedDecoder, TypedEncoder,
};

/// The Zstandard (RFC 8878) compression codec, backed by `zstd`.
///
/// Construct with a compression level in [`level_range`](Zstd::level_range) (the
/// default is `3`, matching upstream zstd), then
/// [`encode_byte_array`](Encoder::encode_byte_array) to compress and
/// [`decode_byte_array`](Decoder::decode_byte_array) to decompress. The codec
/// round-trips through bytes via [`serialize_bytes`](Zstd::serialize_bytes) /
/// [`deserialize_bytes`](Zstd::deserialize_bytes).
///
/// ```
/// use yggdryl_core::{Decoder, Encoder, Zstd};
///
/// let zstd = Zstd::default();
/// let original = b"the quick brown fox".repeat(8);
/// let compressed = zstd.encode_byte_array(&original).unwrap();
/// assert_eq!(zstd.decode_byte_array(&compressed).unwrap(), original);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Zstd {
    level: i32,
}

impl Zstd {
    /// The default compression level (`3`), matching upstream zstd.
    pub const DEFAULT_LEVEL: i32 = 3;

    /// The inclusive `(min, max)` compression levels this build of zstd accepts.
    pub fn level_range() -> (i32, i32) {
        let range = zstd::compression_level_range();
        (*range.start(), *range.end())
    }

    /// Creates a zstd codec at `level`.
    ///
    /// # Errors
    /// Returns [`EncodeError::InvalidLevel`] if `level` is outside
    /// [`level_range`](Zstd::level_range).
    pub fn new(level: i32) -> Result<Self, EncodeError> {
        let (min, max) = Self::level_range();
        if level < min || level > max {
            return Err(EncodeError::InvalidLevel {
                level: level.into(),
                min: min.into(),
                max: max.into(),
            });
        }
        Ok(Self { level })
    }

    /// The configured compression level.
    pub fn level(&self) -> i32 {
        self.level
    }

    /// Serialises this codec to its 4-byte form (the level, little-endian).
    ///
    /// ```
    /// use yggdryl_core::Zstd;
    ///
    /// let zstd = Zstd::new(9).unwrap();
    /// assert_eq!(Zstd::deserialize_bytes(&zstd.serialize_bytes()).unwrap(), zstd);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        self.level.to_le_bytes().to_vec()
    }

    /// Reconstructs a codec from [`serialize_bytes`](Zstd::serialize_bytes).
    ///
    /// # Errors
    /// Returns [`DecodeError::InvalidData`] if `bytes` is not exactly 4 bytes or
    /// encodes an out-of-range level.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let array: [u8; 4] = bytes.try_into().map_err(|_| {
            DecodeError::InvalidData(format!(
                "expected 4 bytes for a zstd codec, got {}",
                bytes.len()
            ))
        })?;
        Self::new(i32::from_le_bytes(array))
            .map_err(|error| DecodeError::InvalidData(error.to_string()))
    }
}

impl Default for Zstd {
    fn default() -> Self {
        Self {
            level: Self::DEFAULT_LEVEL,
        }
    }
}

impl Encoder for Zstd {
    fn encode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, EncodeError> {
        Ok(zstd::encode_all(bytes, self.level)?)
    }
}

impl Decoder for Zstd {
    fn decode_byte_array(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
        Ok(zstd::decode_all(bytes)?)
    }
}

impl TypedEncoder<u8> for Zstd {
    fn encode(&self, items: &[u8]) -> Result<Vec<u8>, EncodeError> {
        self.encode_byte_array(items)
    }
}

impl TypedDecoder<u8> for Zstd {
    fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>, DecodeError> {
        self.decode_byte_array(bytes)
    }
}

impl CompressionEncoder for Zstd {
    /// True streaming compression: `zstd` frames `source` into `sink` through a
    /// fixed-size copy buffer, so memory stays bounded regardless of input size.
    fn compress_stream(
        &self,
        source: &mut dyn IOBase,
        sink: &mut dyn IOBase,
    ) -> Result<u64, EncodeError> {
        let mut encoder = ZstdEncoder::new(IoWriter::new(sink), self.level)?;
        io::copy(&mut IoReader::new(source), &mut encoder)?;
        Ok(encoder.finish()?.written())
    }
}

impl CompressionDecoder for Zstd {
    /// True streaming decompression: `zstd` deframes `source` into `sink` through a
    /// fixed-size copy buffer, so memory stays bounded regardless of input size.
    fn decompress_stream(
        &self,
        source: &mut dyn IOBase,
        sink: &mut dyn IOBase,
    ) -> Result<u64, DecodeError> {
        let mut writer = IoWriter::new(sink);
        io::copy(&mut ZstdDecoder::new(IoReader::new(source))?, &mut writer)?;
        Ok(writer.written())
    }
}

impl Compression for Zstd {
    fn name(&self) -> &'static str {
        "zstd"
    }
}
