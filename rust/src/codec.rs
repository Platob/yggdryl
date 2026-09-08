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

    /// Return whether an encoded stream can be decoded from partway in.
    ///
    /// A coding has *restart points* when its encoder can end one unit and
    /// start the next with no carried state, so a decoder handed the bytes
    /// from that point produces the rest of the payload. Identity has one at
    /// every byte, raw DEFLATE has one after each full flush, and Zstandard
    /// has one at each frame. Gzip and zlib have none: their header and
    /// trailer frame the whole payload, so a decoder cannot begin inside one.
    ///
    /// ```
    /// use yggdryl::Codec;
    ///
    /// assert!(Codec::Deflate.has_restarts());
    /// assert!(!Codec::Gzip.has_restarts());
    /// ```
    pub const fn has_restarts(self) -> bool {
        matches!(self, Self::Identity | Self::Deflate | Self::Zstd)
    }

    /// Scan an encoded stream for the offsets a decoder may begin at.
    ///
    /// The scan is a byte search rather than a decode, so it costs one pass
    /// over the encoded bytes and holds only the few bytes a marker can be
    /// split across. What it answers are *candidates*: the marker a coding
    /// restarts after can also occur inside compressed data, so a caller that
    /// depends on the answer decodes a probe from each offset before trusting
    /// it.
    ///
    /// The start of the stream is never reported: a decoder may always begin
    /// there, so every offset the scan answers is greater than zero. An
    /// identity stream restarts at every offset and reports none, and a
    /// coding with no restart points reports none either - [`has_restarts`]
    /// is what separates the two.
    ///
    /// [`has_restarts`]: Self::has_restarts
    ///
    /// ```
    /// use yggdryl::{Codec, Level};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut encoded = Vec::new();
    /// let mut encoder = Codec::Deflate.writer_with_level(&mut encoded, Level::DEFAULT);
    /// std::io::Write::write_all(&mut encoder, b"symbol,price\n")?;
    /// encoder.restart()?;
    /// std::io::Write::write_all(&mut encoder, b"AAPL,187.23\n")?;
    /// encoder.finish()?;
    ///
    /// let mut offsets = Vec::new();
    /// Codec::Deflate.restart_scan().push(&encoded, &mut offsets);
    /// assert_eq!(offsets.len(), 1);
    ///
    /// // The second half decodes on its own from the offset the scan found.
    /// let start = usize::try_from(offsets[0]).expect("an in-memory offset");
    /// assert_eq!(Codec::Deflate.load(&encoded[start..])?, b"AAPL,187.23\n");
    /// # Ok(())
    /// # }
    /// ```
    pub const fn restart_scan(self) -> RestartScan {
        RestartScan::new(self)
    }

    /// Encode a whole buffer, restarting every `stride` decoded bytes.
    ///
    /// The answer is the encoded payload and the map of where its units
    /// begin, which is what turns a positional read of that payload into a
    /// decode of one stride rather than of everything before the offset. A
    /// stride of zero, an identity coding, and a coding with no restart point
    /// all encode as [`dump_with_level`] does and answer an empty map.
    ///
    /// [`dump_with_level`]: Self::dump_with_level
    ///
    /// # Errors
    ///
    /// Returns the codec's encoding failure.
    ///
    /// ```
    /// use yggdryl::{Codec, Level};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let payload = b"symbol,price\nAAPL,187.23\n".repeat(512);
    /// let (encoded, restarts) =
    ///     Codec::Deflate.dump_with_restarts(&payload, Level::DEFAULT, 4_096)?;
    ///
    /// // The whole payload still decodes as one stream.
    /// assert_eq!(Codec::Deflate.load(&encoded)?, payload);
    ///
    /// // And a position inside it decodes from the point before it.
    /// let (decoded, at) = restarts.before(10_000);
    /// assert_eq!(decoded, 8_192);
    /// let start = usize::try_from(at).expect("an in-memory offset");
    /// assert_eq!(Codec::Deflate.load(&encoded[start..])?, payload[8_192..]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn dump_with_restarts(
        self,
        input: &[u8],
        level: Level,
        stride: u64,
    ) -> Result<(Vec<u8>, Restarts)> {
        let step = usize::try_from(stride).unwrap_or(usize::MAX);
        if step == 0 || self.is_identity() || !self.has_restarts() {
            return Ok((self.dump_with_level(input, level)?, Restarts::default()));
        }
        let mut encoded = Vec::new();
        let mut points = Vec::new();
        {
            let written = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
            let mut encoder = self.writer_with_level(
                Meter {
                    target: &mut encoded,
                    written: std::sync::Arc::clone(&written),
                },
                level,
            );
            for (index, chunk) in input.chunks(step).enumerate() {
                if index > 0 {
                    encoder.restart()?;
                    points.push(written.load(std::sync::atomic::Ordering::Relaxed));
                }
                encoder.write_all(chunk)?;
            }
            encoder.finish()?;
        }
        Ok((encoded, Restarts::new(stride, points)))
    }

    /// The pattern this coding's restart points are found by.
    const fn restart_marker(self) -> Option<Marker> {
        match self {
            Self::Deflate => Some(Marker {
                bytes: zlib::RESTART_MARKER,
                after: true,
            }),
            Self::Zstd => Some(Marker {
                bytes: zstd::FRAME_MAGIC,
                after: false,
            }),
            // Identity restarts everywhere, and a framed coding nowhere.
            Self::Identity | Self::Gzip | Self::Zlib => None,
        }
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
            Self::Identity => Encoder {
                kind: EncoderKind::Identity(Box::new(target)),
                codec: self,
            },
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

/// The byte pattern one coding's restart points are found by.
///
/// The two codings that have restart points mark them differently: a raw
/// DEFLATE stream restarts *after* the empty stored block a full flush emits,
/// and a Zstandard stream restarts *at* the magic each frame begins with.
struct Marker {
    /// The pattern a scan searches for.
    bytes: &'static [u8],
    /// Whether the restart begins after the pattern rather than at it.
    after: bool,
}

/// The longest restart marker, which bounds what a chunk boundary can split.
const MARKER_LEN: usize = 4;

/// A scan for the offsets a decoder may begin an encoded stream at.
///
/// Built by [`Codec::restart_scan`] and fed the encoded bytes in whatever chunks a
/// caller already has them in: the scan holds only the marker-sized tail a
/// chunk boundary can split a pattern across, so scanning a gigabyte costs one
/// pass and three retained bytes.
///
/// ```
/// use yggdryl::Codec;
///
/// let mut scan = Codec::Zstd.restart_scan();
/// let mut offsets = Vec::new();
/// // The frame magic, split across two chunks.
/// scan.push(&[0x00, 0x28, 0xb5], &mut offsets);
/// scan.push(&[0x2f, 0xfd, 0x00], &mut offsets);
/// assert_eq!(offsets, vec![1]);
/// ```
#[derive(Clone, Debug)]
pub struct RestartScan {
    codec: Codec,
    /// Bytes already scanned, which every reported offset is measured from.
    consumed: u64,
    /// The tail of the previous chunk a marker could still be completed from.
    carry: [u8; MARKER_LEN - 1],
    /// How much of `carry` is filled.
    carried: usize,
}

impl RestartScan {
    /// Begin a scan over a stream encoded with `codec`.
    const fn new(codec: Codec) -> Self {
        Self {
            codec,
            consumed: 0,
            carry: [0; MARKER_LEN - 1],
            carried: 0,
        }
    }

    /// How many encoded bytes this scan has seen.
    pub const fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Scan the next chunk, appending the restart offsets it holds.
    ///
    /// Offsets are absolute from the first byte the scan was fed, and are
    /// appended in ascending order, so pushing every chunk of a stream in
    /// order leaves `into` sorted.
    pub fn push(&mut self, chunk: &[u8], into: &mut Vec<u64>) {
        let Some(marker) = self.codec.restart_marker() else {
            self.consumed = self.consumed.saturating_add(chunk.len() as u64);
            return;
        };
        let width = marker.bytes.len();
        // A pattern that began in the previous chunk is completed here, and
        // only here: everything starting inside this chunk is the scan below.
        if self.carried > 0 {
            let mut joined = [0_u8; 2 * (MARKER_LEN - 1)];
            let head = chunk.len().min(width - 1);
            joined[..self.carried].copy_from_slice(&self.carry[..self.carried]);
            joined[self.carried..self.carried + head].copy_from_slice(&chunk[..head]);
            let joined = &joined[..self.carried + head];
            let base = self.consumed - self.carried as u64;
            for start in 0..self.carried {
                if joined.len() >= start + width && &joined[start..start + width] == marker.bytes {
                    marker.push(base + start as u64, width, into);
                }
            }
        }
        for start in memchr::memmem::find_iter(chunk, marker.bytes) {
            marker.push(self.consumed + start as u64, width, into);
        }
        // Whatever a later chunk could still complete a pattern from, which
        // is the last marker-width-minus-one bytes of everything seen so far.
        let keep = MARKER_LEN - 1;
        if chunk.len() >= keep {
            self.carry.copy_from_slice(&chunk[chunk.len() - keep..]);
            self.carried = keep;
        } else {
            let dropped = (self.carried + chunk.len())
                .saturating_sub(keep)
                .min(self.carried);
            self.carry.copy_within(dropped..self.carried, 0);
            self.carried -= dropped;
            self.carry[self.carried..self.carried + chunk.len()].copy_from_slice(chunk);
            self.carried += chunk.len();
        }
        self.consumed = self.consumed.saturating_add(chunk.len() as u64);
    }
}

impl Marker {
    /// Report the restart a pattern found at `position` stands for.
    ///
    /// The stream's own start is left out: it needs no marker to be a restart
    /// point, and a coding whose pattern opens a unit - Zstandard's frame
    /// magic - would otherwise report it as one.
    fn push(&self, position: u64, width: usize, into: &mut Vec<u64>) {
        let restart = if self.after {
            position + width as u64
        } else {
            position
        };
        if restart > 0 {
            into.push(restart);
        }
    }
}

/// Where a decoder may begin inside one encoded payload.
///
/// A map of restart points is what makes a compressed payload addressable:
/// the units are a fixed decoded stride apart, so the point at or before a
/// position is arithmetic, and reading at that position decodes at most one
/// stride rather than everything that precedes it.
///
/// An empty map is the honest answer for a payload written without restart
/// points - every read of one decodes from its first byte.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Restarts {
    /// Decoded bytes between one point and the next; zero when there are none.
    stride: u64,
    /// The encoded offset each unit after the first begins at, ascending.
    points: std::sync::Arc<[u64]>,
}

impl Restarts {
    /// Map the points `stride` decoded bytes apart onto their encoded offsets.
    ///
    /// Point `k` begins the unit at decoded offset `(k + 1) * stride`, so the
    /// map holds no entry for the payload's own start.
    pub fn new(stride: u64, points: impl Into<std::sync::Arc<[u64]>>) -> Self {
        let points = points.into();
        if stride == 0 || points.is_empty() {
            return Self::default();
        }
        Self { stride, points }
    }

    /// Decoded bytes between one point and the next; zero when there are none.
    pub const fn stride(&self) -> u64 {
        self.stride
    }

    /// The encoded offset each unit after the first begins at.
    pub fn points(&self) -> &[u64] {
        &self.points
    }

    /// Whether the payload has no restart point but its own start.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The decoded and encoded offsets of the point at or before `position`.
    ///
    /// The payload's start answers `(0, 0)`, which is what an empty map, a
    /// position inside the first unit, and a coding with no restart points all
    /// resolve to.
    pub fn before(&self, position: u64) -> (u64, u64) {
        if self.stride == 0 {
            return (0, 0);
        }
        let index = usize::try_from(position / self.stride).unwrap_or(usize::MAX);
        let Some(point) = index.checked_sub(1).map(|last| last.min(self.points.len() - 1)) else {
            return (0, 0);
        };
        (
            (point as u64 + 1) * self.stride,
            self.points.get(point).copied().unwrap_or(0),
        )
    }
}

/// A writer that reports how many bytes have reached the target.
///
/// An encoder owns its target, so the only way to learn where a restart point
/// landed is to count the bytes on their way through.
struct Meter<'target> {
    target: &'target mut Vec<u8>,
    written: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Write for Meter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.target.write(buffer)?;
        self.written
            .fetch_add(written as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.target.flush()
    }
}

/// A streaming encoder that must be explicitly finished.
///
/// Dropping without [`Self::finish`] omits the codec trailer, so the output is
/// not a valid member of its format.
pub struct Encoder<'target> {
    pub(crate) kind: EncoderKind<'target>,
    pub(crate) codec: Codec,
}

pub(crate) enum EncoderKind<'target> {
    Identity(Box<dyn Write + 'target>),
    Coded(Box<dyn CodedWrite + 'target>),
}

/// Erases the concrete encoder so the whole contract reaches it through a box.
pub(crate) trait CodedWrite: Write {
    /// Flush the coding's trailer and release the wrapped writer.
    fn finish_boxed(self: Box<Self>) -> std::io::Result<()>;

    /// End the current unit so a decoder can begin afresh at the next byte.
    ///
    /// Answers whether this coding has restart points at all; one that does
    /// not leaves the stream untouched.
    fn restart_unit(&mut self) -> std::io::Result<bool> {
        Ok(false)
    }
}

impl Encoder<'_> {
    /// The coding this encoder writes.
    pub const fn codec(&self) -> Codec {
        self.codec
    }

    /// End the current unit so a decoder can begin afresh at the next byte.
    ///
    /// What follows a restart decodes on its own, which is what makes a
    /// position inside a compressed stream addressable: an index of restart
    /// offsets turns a positional read into a decode of one unit rather than
    /// of everything that precedes it. The cost is compression - each unit
    /// starts with no history of the one before it - so a caller restarts on
    /// a stride it has chosen, never per write.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] naming a coding whose framing has no
    /// restart point - [`Codec::has_restarts`] is the question this answers -
    /// or the encoder's flush failure.
    pub fn restart(&mut self) -> Result<()> {
        match &mut self.kind {
            // Every offset of an identity stream already begins a unit.
            EncoderKind::Identity(target) => Ok(target.flush()?),
            EncoderKind::Coded(target) => {
                if target.restart_unit()? {
                    return Ok(());
                }
                Err(Error::unsupported(
                    "restarting a stream whose framing wraps the whole payload",
                    self.codec.as_str(),
                ))
            }
        }
    }

    /// Flush the codec trailer and release the underlying writer.
    ///
    /// # Errors
    ///
    /// Returns the codec's flush failure.
    pub fn finish(self) -> Result<()> {
        match self.kind {
            EncoderKind::Identity(mut target) => {
                target.flush()?;
                Ok(())
            }
            EncoderKind::Coded(encoder) => {
                encoder.finish_boxed()?;
                Ok(())
            }
        }
    }
}

impl Write for Encoder<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match &mut self.kind {
            EncoderKind::Identity(target) => target.write(buffer),
            EncoderKind::Coded(target) => target.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.kind {
            EncoderKind::Identity(target) => target.flush(),
            EncoderKind::Coded(target) => target.flush(),
        }
    }
}

impl fmt::Debug for Encoder<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Encoder")
            .field("codec", &self.codec)
            .finish()
    }
}

#[cfg(test)]
mod tests;
