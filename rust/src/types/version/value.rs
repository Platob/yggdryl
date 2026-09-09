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
        if bytes.is_empty() {
            return Err(parse_error(0, "expected a decimal major version"));
        }
        let mut parts = [0_u16; Self::MAX_PARTS];
        let mut count = 0;
        let mut position = 0;
        loop {
            if count == Self::MAX_PARTS {
                return Err(parse_error(
                    position,
                    "expected at most three numeric version components",
                ));
            }
            let start = position;
            let maximum = if count == 2 {
                u32::from(u16::MAX)
            } else {
                u32::from(u8::MAX)
            };
            let mut value = 0_u32;
            while let Some(byte @ b'0'..=b'9') = bytes.get(position).copied() {
                value = value * 10 + u32::from(byte - b'0');
                if value > maximum {
                    return Err(parse_error(
                        position,
                        if count == 2 {
                            "expected a patch version in 0..=65535"
                        } else {
                            "expected a major or minor version in 0..=255"
                        },
                    ));
                }
                position += 1;
            }
            if position == start {
                return Err(parse_error(
                    position,
                    "expected a decimal version component",
                ));
            }
            parts[count] = value as u16;
            count += 1;
            match bytes.get(position) {
                None => break,
                Some(b'S' | b's')
                    if count == 2 && matches!(bytes.get(position + 1), Some(b'P' | b'p')) =>
                {
                    position += 2;
                }
                Some(b'.') if position + 1 < bytes.len() => position += 1,
                Some(b'.') => {
                    return Err(parse_error(position, "expected a component after the dot"));
                }
                Some(_) => {
                    return Err(parse_error(
                        position,
                        "expected a dot, SP after the minor version, or the end of a version",
                    ));
                }
            }
        }
        Ok(Self::new(parts[0] as u8, parts[1] as u8, parts[2]))
    }
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
