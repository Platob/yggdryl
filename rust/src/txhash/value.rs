//! One time-coupled hash: the instant first, the digest after it.

use std::fmt;
use std::num::NonZeroU32;
use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::types::{Bytes, BytesLayout, BytesParameters};
use crate::{DataType, Digest, DigestAlgorithm, Error, Result, Scalar, TimeUnit, Timezone};

use super::time::{DEFAULT_UNIT, restate_unix, validate_unit};

/// The bytes the instant takes at the front of every value: a big-endian
/// `i64`, whatever the unit.
pub const UNIX_WIDTH: usize = 8;

/// The widest value, in bytes: the instant beside an XXH3-128 digest.
const MAX_WIDTH: usize = UNIX_WIDTH + 16;

/// Return the width of a value coupling an instant with this algorithm.
///
/// ```
/// use yggdryl::{DigestAlgorithm, txhash};
///
/// assert_eq!(txhash::width(DigestAlgorithm::Xxh32), 12);
/// assert_eq!(txhash::width(DigestAlgorithm::Xxh3), 16);
/// assert_eq!(txhash::width(DigestAlgorithm::Xxh128), 24);
/// ```
pub const fn width(algorithm: DigestAlgorithm) -> usize {
    UNIX_WIDTH + algorithm.width()
}

/// Return the datatype a column of these values is stored under.
///
/// A `fixed_size_binary` of the exact width, because no Arrow integer is 96
/// or 192 bits wide and the sixteen-byte value is a whole rather than two
/// numbers: sorting the bytes sorts the instants, then the digests.
pub const fn dtype(algorithm: DigestAlgorithm) -> DataType {
    match NonZeroU32::new(fixed_width(algorithm)) {
        Some(width) => {
            DataType::Bytes(BytesParameters::new(BytesLayout::FixedSizeBinary).with_bound(width))
        }
        // Every width below is a literal above zero.
        None => DataType::binary(),
    }
}

/// The width as the datatype spells it.
pub(crate) const fn fixed_width(algorithm: DigestAlgorithm) -> u32 {
    match algorithm {
        DigestAlgorithm::Xxh32 => 12,
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => 16,
        DigestAlgorithm::Xxh128 => 24,
    }
}

/// Return the algorithm a stored width implies, when exactly one does.
///
/// Sixteen bytes answer XXH3-64 rather than XXH64 for the reason the digest
/// vocabulary defaults the same way: it is the algorithm every `stable_hash`
/// in the project answers.
pub(crate) const fn algorithm_of_width(width: u32) -> Option<DigestAlgorithm> {
    // The three defaults in width order; the width each takes is read off
    // the one layout rule rather than restated here.
    const DEFAULTS: [DigestAlgorithm; 3] = [
        DigestAlgorithm::Xxh32,
        DigestAlgorithm::Xxh3,
        DigestAlgorithm::Xxh128,
    ];
    let mut index = 0;
    while index < DEFAULTS.len() {
        if fixed_width(DEFAULTS[index]) == width {
            return Some(DEFAULTS[index]);
        }
        index += 1;
    }
    None
}

/// One instant coupled with one digest.
///
/// The instant is a unix count, UTC, at a clock resolution the value carries;
/// the digest is any [`Digest`]. Two values compare by unit, then instant,
/// then digest, which is the order their canonical bytes sort in for every
/// instant from the epoch on - the reason the instant comes first.
///
/// ```
/// use yggdryl::{DigestAlgorithm, TimeUnit, txhash};
///
/// # fn main() -> yggdryl::Result<()> {
/// let earlier = txhash::txh3(b"AAPL", 1_700_000_000_000_000);
/// let later = txhash::txh3(b"AAPL", 1_700_000_000_000_001);
/// assert!(earlier < later);
/// assert!(earlier.into_bytes() < later.into_bytes());
/// assert_eq!(earlier.digest(), later.digest());
///
/// assert_eq!(earlier.unit(), TimeUnit::Microsecond);
/// assert_eq!(earlier.width(), 16);
/// assert_eq!(earlier.algorithm(), DigestAlgorithm::Xxh3);
/// assert!(earlier.to_string().starts_with("1700000000000000@us:xxh3-64:"));
/// assert_eq!(txhash::TxHash::from_str(&earlier.to_string())?, earlier);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TxHash {
    unit: TimeUnit,
    unix: i64,
    digest: Digest,
}

impl TxHash {
    /// Couple a microsecond instant with a digest.
    pub const fn new(unix: i64, digest: Digest) -> Self {
        Self {
            unit: DEFAULT_UNIT,
            unix,
            digest,
        }
    }

    /// Couple an instant at an explicit clock resolution with a digest.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when `unit` is not a clock
    /// resolution.
    pub fn new_in(unix: i64, unit: TimeUnit, digest: Digest) -> Result<Self> {
        validate_unit(unit)?;
        Ok(Self::couple(unit, unix, digest))
    }

    /// Couple under a unit a caller has already validated once for a whole
    /// column, so the per-row path carries no check.
    pub(crate) const fn couple(unit: TimeUnit, unix: i64, digest: Digest) -> Self {
        Self { unit, unix, digest }
    }

    /// Return the instant as a unix count of [`Self::unit`].
    pub const fn unix(&self) -> i64 {
        self.unix
    }

    /// Return the clock resolution the instant is counted in.
    pub const fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// Return the digest half.
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    /// Return the digest's algorithm.
    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.digest.algorithm()
    }

    /// Return the width of the canonical bytes.
    pub const fn width(&self) -> usize {
        width(self.digest.algorithm())
    }

    /// Return the datatype a column of values like this one is stored under.
    pub const fn dtype(&self) -> DataType {
        dtype(self.digest.algorithm())
    }

    /// Restate the instant at another clock resolution, keeping the digest.
    ///
    /// [`restate_unix`] carries the rule: a coarser target floors, a finer one
    /// scales exactly.
    ///
    /// # Errors
    ///
    /// Returns an error for a unit that is not a clock resolution or a count
    /// that does not fit it.
    pub fn with_unit(self, unit: TimeUnit) -> Result<Self> {
        Ok(Self {
            unit,
            unix: restate_unix(self.unix, self.unit, unit)?,
            digest: self.digest,
        })
    }

    /// Return the canonical bytes: the instant big-endian, then the digest's
    /// canonical bytes.
    ///
    /// The instant leads so that the bytes sort as the instants do, most
    /// significant byte first, exactly as [`Digest::into_bytes`] lays out its
    /// half. A big-endian two's complement count sorts every instant from the
    /// epoch on in order; one before the epoch sorts after them all.
    ///
    /// ```
    /// use yggdryl::{DigestAlgorithm, txhash};
    ///
    /// let value = txhash::txh32(b"", 1);
    /// let bytes = value.into_bytes();
    /// assert_eq!(bytes.len(), 12);
    /// assert_eq!(&bytes[..8], &[0, 0, 0, 0, 0, 0, 0, 1]);
    /// assert_eq!(&bytes[8..], &*DigestAlgorithm::Xxh32.digest(b"").into_bytes());
    /// ```
    pub fn into_bytes(self) -> TxHashBytes {
        let mut bytes = [0_u8; MAX_WIDTH];
        bytes[..UNIX_WIDTH].copy_from_slice(&self.unix.to_be_bytes());
        let digest = self.digest.into_bytes();
        bytes[UNIX_WIDTH..UNIX_WIDTH + digest.len()].copy_from_slice(&digest);
        TxHashBytes {
            bytes,
            length: self.width() as u8,
        }
    }

    /// Rebuild a value from its canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when `bytes` is not exactly the width the
    /// algorithm calls for, and [`Error::InvalidDataType`] for a unit that
    /// is not a clock resolution.
    pub fn from_bytes(unit: TimeUnit, algorithm: DigestAlgorithm, bytes: &[u8]) -> Result<Self> {
        validate_unit(unit)?;
        let expected = width(algorithm);
        if bytes.len() != expected {
            return Err(Error::Parse {
                target: "txhash",
                position: 0,
                reason: format_smolstr!(
                    "expected {expected} bytes for {algorithm} after an instant, got {}",
                    bytes.len()
                ),
            });
        }
        let mut instant = [0_u8; UNIX_WIDTH];
        instant.copy_from_slice(&bytes[..UNIX_WIDTH]);
        Ok(Self {
            unit,
            unix: i64::from_be_bytes(instant),
            digest: Digest::from_bytes(algorithm, &bytes[UNIX_WIDTH..])?,
        })
    }

    /// Parse the canonical `<unix>@<unit>:<algorithm>:<hex>` spelling.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the failure.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the instant as a UTC datetime at this value's resolution.
    ///
    /// ```
    /// use yggdryl::{Scalar, TimeUnit, Timezone, txhash};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let value = txhash::txh3(b"AAPL", 1_700_000_000_000_000);
    /// assert_eq!(
    ///     value.into_datetime(),
    ///     Scalar::from_datetime(1_700_000_000_000_000, TimeUnit::Microsecond, Timezone::UTC)?,
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn into_datetime(self) -> Scalar {
        Scalar::datetime64(self.unix, self.unit, Timezone::UTC)
            .unwrap_or_else(|_| unreachable!("the unit was validated at construction"))
    }

    /// Return the canonical bytes as a fixed-width byte value.
    ///
    /// This is the cell a column of these values holds, so a value and the
    /// array it came from spell the same bytes: the canonical layout at the
    /// algorithm's exact width, which every width here fits inline.
    pub fn into_scalar(self) -> Scalar {
        let DataType::Bytes(parameters) = self.dtype() else {
            unreachable!("a coupled hash is stored as fixed-width bytes")
        };
        Scalar::Bytes(Bytes::from_storage(&self.into_bytes(), parameters))
    }

    /// Read a value back out of the two representations a value has.
    ///
    /// A byte value is the canonical layout at the algorithm's exact width,
    /// and text is the canonical spelling, which must name this unit and
    /// algorithm. The arguments sit in the order [`Self::from_bytes`] takes
    /// them.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is neither, when bytes have the wrong
    /// width, or when a spelling names another unit or algorithm.
    pub fn from_scalar(unit: TimeUnit, algorithm: DigestAlgorithm, value: &Scalar) -> Result<Self> {
        if let Scalar::Bytes(bytes) = value {
            return Self::from_bytes(unit, algorithm, bytes.as_bytes());
        }
        if let Some(text) = value.as_str() {
            let parsed = Self::from_str(text)?;
            if parsed.unit != unit || parsed.algorithm() != algorithm {
                return Err(Error::Parse {
                    target: "txhash",
                    position: 0,
                    reason: format_smolstr!(
                        "expected a {unit} instant with a {algorithm} digest, got {} with {}",
                        parsed.unit,
                        parsed.algorithm()
                    ),
                });
            }
            return Ok(parsed);
        }
        Err(Error::Parse {
            target: "txhash",
            position: 0,
            reason: format_smolstr!(
                "expected canonical bytes or the canonical spelling, got {}",
                value.kind()
            ),
        })
    }

    /// Return the deterministic 64-bit hash used by every binding.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
}

/// The canonical bytes of one [`TxHash`].
///
/// Inline like [`crate::DigestBytes`], so reading a value's bytes never
/// allocates; it dereferences to the exact-width slice and compares as it.
#[derive(Clone, Copy)]
pub struct TxHashBytes {
    bytes: [u8; MAX_WIDTH],
    length: u8,
}

impl Deref for TxHashBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

impl AsRef<[u8]> for TxHashBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl fmt::Debug for TxHashBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl PartialEq for TxHashBytes {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for TxHashBytes {}

impl PartialOrd for TxHashBytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TxHashBytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

impl std::hash::Hash for TxHashBytes {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl fmt::Display for TxHash {
    /// Render `<unix>@<unit>:<algorithm>:<hex>`.
    ///
    /// The unit and the algorithm are part of the value, so they are part of
    /// the spelling: without them a microsecond and a nanosecond count of the
    /// same digits would share one rendering and [`FromStr`] could not be the
    /// exact inverse this contract requires.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}@{}:{}", self.unix, self.unit, self.digest)
    }
}

impl FromStr for TxHash {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        let parse_error = |position: usize, reason: smol_str::SmolStr| Error::Parse {
            target: "txhash",
            position,
            reason,
        };
        // A failure inside one part is reported at that part's offset in the
        // whole spelling and under this value's own target, so a caller reads
        // one position against the text it handed over.
        let nested = |offset: usize, error: Error| match error {
            Error::Parse {
                position, reason, ..
            } => parse_error(offset + position, reason),
            other => parse_error(offset, smol_str::SmolStr::new(other.to_string())),
        };
        let (unix, rest) = normalized.split_once('@').ok_or_else(|| {
            parse_error(
                0,
                format_smolstr!(
                    "expected <unix>@<unit>:<algorithm>:<hex>, got {}",
                    crate::text::elide_to(normalized, crate::text::ERROR_TEXT_LIMIT)
                ),
            )
        })?;
        let unix = unix.parse::<i64>().map_err(|error| {
            parse_error(
                0,
                format_smolstr!("expected a signed 64-bit unix count, got {error}"),
            )
        })?;
        let rest_offset = normalized.len() - rest.len();
        let (unit, digest) = rest.split_once(':').ok_or_else(|| {
            parse_error(
                rest_offset,
                "expected <unit>:<algorithm>:<hex> after the instant".into(),
            )
        })?;
        let unit = TimeUnit::from_str(unit)
            .and_then(|unit| validate_unit(unit).map(|()| unit))
            .map_err(|error| nested(rest_offset, error))?;
        let digest_offset = rest_offset + rest.len() - digest.len();
        let digest = Digest::from_str(digest).map_err(|error| nested(digest_offset, error))?;
        Ok(Self { unit, unix, digest })
    }
}

impl Serialize for TxHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TxHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}
