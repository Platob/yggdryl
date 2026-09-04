//! The content codings Yggdryl understands, and how to apply them.
//!
//! One [`Codec`] vocabulary names every coding, and each format module
//! ([`crate::coding::gzip`], [`crate::coding::zlib`], [`crate::coding::zstd`]) exposes the same four
//! operations: `load`/`dump` for whole buffers and `reader`/`writer` for
//! streams. Nothing buffers a whole object to compress it, so a multi-gigabyte
//! file costs one window.
//!
//! [`Codec::from_media_type`] and [`Codec::from_url`] recover the coding from a
//! filename, which is what lets `trades.json.gz` decode without a caller naming
//! the codec.
//!
//! ```
//! use yggdryl::{Codec, coding::gzip};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let compressed = gzip::dump(b"symbol,price\nAAPL,1\n")?;
//! assert_eq!(gzip::load(&compressed)?, b"symbol,price\nAAPL,1\n");
//!
//! // The coding is recoverable from the filename alone.
//! let url = yggdryl::Url::from_str("file:///trades.csv.gz")?;
//! assert_eq!(Codec::from_url(&url), Codec::Gzip);
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::io::{Read, Write};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::coding::{gzip, zlib, zstd};
use crate::{Error, MediaType, MimeType, Result, Url};

/// How aggressively a codec trades throughput for output size.
///
/// Levels are expressed on one shared 0-to-9 scale and mapped onto each
/// codec's native range, so a caller can raise compression once without
/// learning three numbering schemes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Level(u8);

impl Level {
    /// No compression; the fastest setting a codec offers.
    pub const NONE: Self = Self(0);
    /// Favor throughput over output size.
    pub const FAST: Self = Self(1);
    /// The balanced setting used when a caller expresses no preference.
    pub const DEFAULT: Self = Self(6);
    /// Favor output size over throughput.
    pub const BEST: Self = Self(9);

    /// Clamp an arbitrary level onto the shared 0-to-9 scale.
    pub const fn new(level: u8) -> Self {
        Self(if level > 9 { 9 } else { level })
    }

    /// Return the level on the shared 0-to-9 scale.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Map onto zstd's 1-to-19 range.
    pub(crate) const fn zstd(self) -> i32 {
        match self.0 {
            0 => 1,
            // Round up without `div_ceil`, which is not const on the 1.85 baseline.
            level => (level as i32 * 19 + 8) / 9,
        }
    }
}

impl Default for Level {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for Level {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A content coding applied to a byte payload.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Codec {
    /// Bytes pass through unchanged.
    #[default]
    Identity,
    /// RFC 1952 gzip framing over DEFLATE.
    Gzip,
    /// RFC 1950 zlib framing over DEFLATE.
    Zlib,
    /// Raw RFC 1951 DEFLATE with no framing.
    Deflate,
    /// RFC 8878 Zstandard.
    Zstd,
}

impl Codec {
    /// Every codec in canonical order.
    pub const ALL: [Self; 5] = [
        Self::Identity,
        Self::Gzip,
        Self::Zlib,
        Self::Deflate,
        Self::Zstd,
    ];

    /// Parse a canonical content-coding token.
    ///
    /// Leading/trailing whitespace is accepted so an HTTP `Content-Encoding`
    /// header value parses directly.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the accepted vocabulary and the input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase name without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Zlib => "zlib",
            Self::Deflate => "deflate",
            Self::Zstd => "zstd",
        }
    }

    /// Return the customary filename suffix, when the coding has one.
    pub const fn extension(self) -> Option<&'static str> {
        match self {
            Self::Identity | Self::Deflate => None,
            Self::Gzip => Some("gz"),
            Self::Zlib => Some("zz"),
            Self::Zstd => Some("zst"),
        }
    }

    /// Return whether the coding changes the bytes at all.
    pub const fn is_identity(self) -> bool {
        matches!(self, Self::Identity)
    }

    /// Recover the coding from a MIME type.
    pub fn from_mime_type(value: &MimeType) -> Self {
        if *value == MimeType::GZIP {
            Self::Gzip
        } else if *value == MimeType::ZLIB {
            Self::Zlib
        } else if *value == MimeType::ZSTD {
            Self::Zstd
        } else {
            Self::Identity
        }
    }

    /// Recover the outermost coding from a media type's encoding sequence.
    ///
    /// Encodings are listed in application order, so the last one applied is
    /// the first that must be removed.
    pub fn from_media_type(value: &MediaType) -> Self {
        value
            .encodings()
            .last()
            .map_or(Self::Identity, Self::from_mime_type)
    }

    /// Recover the outermost coding from a location's compound filename.
    pub fn from_url(value: &Url) -> Self {
        Self::from_media_type(&value.media_type())
    }

    /// Decode a complete buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is not valid for this coding.
    pub fn load(self, input: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            Self::Gzip => gzip::load(input),
            Self::Zlib => zlib::load(input),
            Self::Deflate => zlib::load_raw(input),
            Self::Zstd => zstd::load(input),
        }
    }

    /// Encode a complete buffer at the default level.
    ///
    /// # Errors
    ///
    /// Returns the codec's encoding failure.
    pub fn dump(self, input: &[u8]) -> Result<Vec<u8>> {
        self.dump_with_level(input, Level::DEFAULT)
    }

    /// Encode a complete buffer at an explicit level.
    ///
    /// # Errors
    ///
    /// Returns the codec's encoding failure.
    pub fn dump_with_level(self, input: &[u8], level: Level) -> Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            Self::Gzip => gzip::dump_with_level(input, level),
            Self::Zlib => zlib::dump_with_level(input, level),
            Self::Deflate => zlib::dump_raw_with_level(input, level),
            Self::Zstd => zstd::dump_with_level(input, level),
        }
    }

    /// Wrap a reader so it yields decoded bytes.
    ///
    /// Decoding is streaming: neither the encoded nor the decoded payload is
    /// buffered whole.
    pub fn reader<'source, R: Read + 'source>(self, source: R) -> Box<dyn Read + 'source> {
        match self {
            Self::Identity => Box::new(source),
            Self::Gzip => gzip::reader(source),
            Self::Zlib => zlib::reader(source),
            Self::Deflate => zlib::raw_reader(source),
            Self::Zstd => zstd::reader(source),
        }
    }

    /// [`Self::reader`], with the `Send` every decoder already has made
    /// visible in the type, so a decoded stream can cross a thread boundary.
    pub(crate) fn reader_send<'source, R: Read + Send + 'source>(
        self,
        source: R,
    ) -> Box<dyn Read + Send + 'source> {
        match self {
            Self::Identity => Box::new(source),
            Self::Gzip => gzip::reader_send(source),
            Self::Zlib => zlib::reader_send(source),
            Self::Deflate => zlib::raw_reader_send(source),
            Self::Zstd => zstd::reader_send(source),
        }
    }

    /// Wrap a writer so written bytes are encoded at the default level.
    ///
    /// The returned writer must be finished with [`Encoder::finish`]; dropping
    /// it leaves the trailer unwritten.
    pub fn writer<'target, W: Write + 'target>(self, target: W) -> Encoder<'target> {
        self.writer_with_level(target, Level::DEFAULT)
    }

    /// Wrap a writer so written bytes are encoded at an explicit level.
    pub fn writer_with_level<'target, W: Write + 'target>(
        self,
        target: W,
        level: Level,
    ) -> Encoder<'target> {
        match self {
            Self::Identity => Encoder(EncoderKind::Identity(Box::new(target))),
            Self::Gzip => gzip::writer_with_level(target, level),
            Self::Zlib => zlib::writer_with_level(target, level),
            Self::Deflate => zlib::raw_writer_with_level(target, level),
            Self::Zstd => zstd::writer_with_level(target, level),
        }
    }
}

impl FromStr for Codec {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|codec| normalized.eq_ignore_ascii_case(codec.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "content coding",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|codec| codec.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Codec {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Codec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}

/// A streaming encoder that must be explicitly finished.
///
/// Dropping without [`Self::finish`] omits the codec trailer, so the output is
/// not a valid member of its format.
pub struct Encoder<'target>(pub(crate) EncoderKind<'target>);

pub(crate) enum EncoderKind<'target> {
    Identity(Box<dyn Write + 'target>),
    Flate(Box<dyn FlateFinish + 'target>),
    Zstd(Box<dyn FlateFinish + 'target>),
}

/// Erases the concrete encoder so `finish` can be called through the box.
pub(crate) trait FlateFinish: Write {
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()>;
}

impl Encoder<'_> {
    /// Flush the codec trailer and release the underlying writer.
    ///
    /// # Errors
    ///
    /// Returns the codec's flush failure.
    pub fn finish(self) -> Result<()> {
        match self.0 {
            EncoderKind::Identity(mut target) => {
                target.flush()?;
                Ok(())
            }
            EncoderKind::Flate(encoder) | EncoderKind::Zstd(encoder) => {
                encoder.finish_boxed()?;
                Ok(())
            }
        }
    }
}

impl Write for Encoder<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match &mut self.0 {
            EncoderKind::Identity(target) => target.write(buffer),
            EncoderKind::Flate(target) | EncoderKind::Zstd(target) => target.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.0 {
            EncoderKind::Identity(target) => target.flush(),
            EncoderKind::Flate(target) | EncoderKind::Zstd(target) => target.flush(),
        }
    }
}

impl fmt::Debug for Encoder<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.0 {
            EncoderKind::Identity(_) => "identity",
            EncoderKind::Flate(_) => "flate",
            EncoderKind::Zstd(_) => "zstd",
        };
        formatter
            .debug_struct("Encoder")
            .field("kind", &kind)
            .finish()
    }
}

#[cfg(test)]
mod tests;
