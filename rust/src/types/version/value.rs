//! Canonical parsing, rendering, and ordering for [`Version`].

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, ScalarFamily, ScalarValue};

/// Three numeric version components in four bytes.
///
/// Major and minor are unsigned bytes; patch is an unsigned 16-bit number.
/// The numeric tuple owns equality and ordering. Text omits trailing zero
/// components, so `5`, `5.0`, and `5.0.0` are the same value.
/// A case-insensitive FIX `SP` suffix supplies the patch: `5.0sp250` is
/// `5.0.250`, with no separately stored qualifier.
///
/// ```
/// use yggdryl::Version;
/// let version = "5.0sp250".parse::<Version>()?;
/// assert_eq!(version, Version::new(5, 0, 250));
/// assert_eq!(version.to_string(), "5.0.250");
/// # Ok::<(), yggdryl::Error>(())
/// ```
///
/// # The patch is best effort
///
/// Major and minor are strict: a version whose first two components are not
/// decimal numbers under 256 is refused, and so is empty text. The patch is
/// not. A tail stating no number - a qualifier, a fourth component, an
/// extension pack - folds into the patch's sixteen bits through the crate's
/// stable XXH3 rather than refusing the version, so anything that names a
/// major parses.
///
/// A folded patch is an identity, not a quantity: it orders arbitrarily
/// against a stated one, two unlike tails can fold together, and the
/// canonical text states the fold rather than the tail it came from. What it
/// buys is that the same tail always reads as the same version.
///
/// ```
/// use yggdryl::Version;
/// let qualified = "1.0-rc1".parse::<Version>()?;
/// assert_eq!((qualified.major(), qualified.minor()), (1, 0));
/// assert_eq!(qualified, "1.0-rc1".parse::<Version>()?);
/// assert_ne!(qualified, "1.0-rc2".parse::<Version>()?);
/// assert!("".parse::<Version>().is_err());
/// assert!("256.0".parse::<Version>().is_err());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(C)]
pub struct Version {
    major: u8,
    minor: u8,
    patch: u16,
}

impl Version {
    /// The maximum number of numeric components.
    pub const MAX_PARTS: usize = 3;

    /// The lower bound of the numeric version value space.
    pub const MIN: Self = Self::new(0, 0, 0);

    /// The upper bound of the numeric version value space.
    pub const MAX: Self = Self::new(u8::MAX, u8::MAX, u16::MAX);

    /// Constructs a version from its exact-width numeric components.
    ///
    /// ```
    /// use yggdryl::Version;
    /// let version = Version::new(5, 2, 300);
    /// assert_eq!(version.to_string(), "5.2.300");
    /// assert_eq!((version.major(), version.minor(), version.patch()), (5, 2, 300));
    /// ```
    pub const fn new(major: u8, minor: u8, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// The major component.
    pub const fn major(self) -> u8 {
        self.major
    }

    /// The minor component, zero when omitted in text.
    pub const fn minor(self) -> u8 {
        self.minor
    }

    /// The patch component, zero when omitted in text.
    pub const fn patch(self) -> u16 {
        self.patch
    }

    /// Number of UTF-8 bytes in the canonical rendering.
    pub(crate) fn rendered_len(&self) -> usize {
        decimal_digits(u16::from(self.major))
            + if self.minor != 0 || self.patch != 0 {
                1 + decimal_digits(u16::from(self.minor))
            } else {
                0
            }
            + if self.patch != 0 {
                1 + decimal_digits(self.patch)
            } else {
                0
            }
    }
}

const _: () = assert!(std::mem::size_of::<Version>() == 4);

const fn decimal_digits(value: u16) -> usize {
    match value {
        0..=9 => 1,
        10..=99 => 2,
        100..=999 => 3,
        1_000..=9_999 => 4,
        _ => 5,
    }
}

impl FromStr for Version {
    type Err = Error;

    #[allow(clippy::cast_possible_truncation)] // Each component is bounded before its narrowing cast.
    fn from_str(text: &str) -> Result<Self> {
        let bytes = text.as_bytes();
        let (major, mut position) = component(bytes, 0, u32::from(u8::MAX), "major")?;
        let mut minor = 0_u32;
        if bytes.get(position) == Some(&b'.') && digit_at(bytes, position + 1) {
            let (value, after) = component(bytes, position + 1, u32::from(u8::MAX), "minor")?;
            minor = value;
            position = after;
        }
        Ok(Self::new(
            major as u8,
            minor as u8,
            patch_of(&text[position..]),
        ))
    }
}

/// Whether a decimal digit stands at `position`.
fn digit_at(bytes: &[u8], position: usize) -> bool {
    matches!(bytes.get(position), Some(b'0'..=b'9'))
}

/// One strict decimal component, bounded before it narrows.
///
/// Major and minor stay strict because they are what a version is ordered by
/// first: a byte that is not a digit there is a refusal, not a fallback.
fn component(bytes: &[u8], start: usize, maximum: u32, what: &'static str) -> Result<(u32, usize)> {
    let mut position = start;
    let mut value = 0_u32;
    while let Some(byte @ b'0'..=b'9') = bytes.get(position).copied() {
        value = value * 10 + u32::from(byte - b'0');
        if value > maximum {
            return Err(parse_error(
                position,
                if what == "major" {
                    "expected a major version in 0..=255"
                } else {
                    "expected a minor version in 0..=255"
                },
            ));
        }
        position += 1;
    }
    if position == start {
        return Err(parse_error(
            start,
            if what == "major" {
                "expected a decimal major version"
            } else {
                "expected a decimal minor version"
            },
        ));
    }
    Ok((value, position))
}

/// The patch a tail states, or the stable hash of a tail that states no number.
///
/// The tail is whatever follows the major and minor. `.250` and a
/// case-insensitive FIX `sp250` both state the number 250. Anything else -
/// a qualifier, a fourth component, an extension pack, trailing bytes - is
/// hashed into the patch rather than refused, so a version always parses.
fn patch_of(tail: &str) -> u16 {
    if tail.is_empty() {
        return 0;
    }
    let digits = match tail.as_bytes() {
        [b'.', rest @ ..] => Some(rest),
        [b'S' | b's', b'P' | b'p', rest @ ..] => Some(rest),
        _ => None,
    };
    if let Some(digits) = digits
        && !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
    {
        let mut value = 0_u32;
        for byte in digits {
            value = value * 10 + u32::from(byte - b'0');
            if value > u32::from(u16::MAX) {
                return hashed_patch(tail);
            }
        }
        return value as u16;
    }
    hashed_patch(tail)
}

/// A tail no number can be read from, folded into the patch's sixteen bits.
///
/// The fold lands in `1..=65535` so a tail that says something never renders
/// as a version that says nothing. Sixteen bits cannot separate every tail
/// there is: two unlike tails can fold together, and a fold can equal a patch
/// some other version states as a number.
fn hashed_patch(tail: &str) -> u16 {
    let mut hasher = crate::xxhash::Xxh3::new();
    hasher.write_bytes(tail.as_bytes());
    let hash = hasher.as_u64();
    let folded = (hash ^ (hash >> 16) ^ (hash >> 32) ^ (hash >> 48)) as u16;
    1 + (folded % u16::MAX)
}

fn parse_error(position: usize, reason: &'static str) -> Error {
    Error::Parse {
        target: "version",
        position,
        reason: SmolStr::new_static(reason),
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.major)?;
        if self.minor != 0 || self.patch != 0 {
            write!(formatter, ".{}", self.minor)?;
        }
        if self.patch != 0 {
            write!(formatter, ".{}", self.patch)?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = <&str>::deserialize(deserializer)?;
        text.parse().map_err(D::Error::custom)
    }
}

impl ScalarFamily for Version {
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn id(&self) -> DataTypeId {
        DataTypeId::Version
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Version)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Version(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Version(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarValue for Version {
    type Family = Self;
    type Type = super::VersionType;

    const ID: DataTypeId = DataTypeId::Version;
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Version)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Version(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        <Self as ScalarFamily>::from_scalar(value)
    }
}

impl From<Version> for Scalar {
    fn from(value: Version) -> Self {
        Self::Version(value)
    }
}

impl TryFrom<&str> for Version {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        value.parse()
    }
}

impl TryFrom<String> for Version {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        value.parse()
    }
}
