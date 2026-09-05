//! Canonical parsing, rendering, and ordering for [`Version`].

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, ScalarFamily, ScalarValue};

/// One canonical, numerically ordered version in sixteen bytes.
///
/// The required major and optional minor are direct numeric bytes. The
/// optional patch is a fixed fourteen-byte ASCII tail containing any third
/// and fourth numeric components plus an optional qualifier. A non-empty
/// patch makes a zero minor explicit; otherwise a zero minor is absent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Version {
    major: u8,
    minor: u8,
    patch: [u8; Self::PATCH_WIDTH],
}

impl Version {
    /// The maximum number of numeric components.
    pub const MAX_PARTS: usize = 4;

    /// Bytes reserved for the canonical patch and qualifier tail.
    const PATCH_WIDTH: usize = 14;

    /// The lower bound of the version value space.
    pub const MIN: Self = Self {
        major: 0,
        minor: 0,
        patch: [0; Self::PATCH_WIDTH],
    };

    /// The upper bound of the version value space.
    pub const MAX: Self = Self {
        major: u8::MAX,
        minor: u8::MAX,
        patch: [
            b'.', b'6', b'5', b'5', b'3', b'5', b'.', b'6', b'5', b'5', b'3', b'5', 0, 0,
        ],
    };

    fn minor(&self) -> Option<u8> {
        (self.minor != 0 || !self.patch_bytes().is_empty()).then_some(self.minor)
    }

    fn patch(&self) -> Option<&str> {
        let patch = self.patch_bytes();
        if patch.is_empty() {
            None
        } else {
            std::str::from_utf8(patch).ok()
        }
    }

    /// Number of UTF-8 bytes in the canonical rendering.
    pub(crate) fn rendered_len(&self) -> usize {
        decimal_digits_u8(self.major)
            + self.minor().map_or(0, |minor| 1 + decimal_digits_u8(minor))
            + self.patch_len()
    }

    fn patch_len(&self) -> usize {
        self.patch
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(Self::PATCH_WIDTH)
    }

    fn patch_bytes(&self) -> &[u8] {
        &self.patch[..self.patch_len()]
    }

    fn numeric_parts(&self) -> [u16; Self::MAX_PARTS] {
        let mut parts = [u16::from(self.major), u16::from(self.minor), 0, 0];
        let patch = self.patch_bytes();
        let mut position = 0;
        let mut index = 2;
        while index < Self::MAX_PARTS
            && patch.get(position) == Some(&b'.')
            && patch.get(position + 1).is_some_and(u8::is_ascii_digit)
        {
            position += 1;
            let mut value = 0_u16;
            while let Some(byte @ b'0'..=b'9') = patch.get(position).copied() {
                value = value * 10 + u16::from(byte - b'0');
                position += 1;
            }
            parts[index] = value;
            index += 1;
        }
        parts
    }

    fn qualifier(&self) -> Option<(bool, &[u8])> {
        let patch = self.patch_bytes();
        let mut position = 0;
        while patch.get(position) == Some(&b'.')
            && patch.get(position + 1).is_some_and(u8::is_ascii_digit)
        {
            position += 1;
            while patch.get(position).is_some_and(u8::is_ascii_digit) {
                position += 1;
            }
        }
        let qualifier = patch.get(position..)?;
        if qualifier.is_empty() {
            None
        } else if qualifier[0] == b'-' {
            Some((true, &qualifier[1..]))
        } else {
            Some((false, qualifier))
        }
    }
}

const _: () = assert!(std::mem::size_of::<Version>() == 16);

fn decimal_digits_u8(value: u8) -> usize {
    match value {
        0..=9 => 1,
        10..=99 => 2,
        _ => 3,
    }
}

impl Default for Version {
    fn default() -> Self {
        Self::MIN
    }
}

impl FromStr for Version {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return Err(parse_error(0, "expected a decimal major version"));
        }

        let mut parts = [0_u16; Self::MAX_PARTS];
        let mut starts = [0_usize; Self::MAX_PARTS];
        let mut count = 0_usize;
        let mut position = 0_usize;
        let mut qualifier = None;

        loop {
            if count == Self::MAX_PARTS {
                return Err(parse_error(
                    position,
                    "a version has at most four components",
                ));
            }
            let start = position;
            starts[count] = start;
            let maximum = if count < 2 {
                u32::from(u8::MAX)
            } else {
                u32::from(u16::MAX)
            };
            let mut value = 0_u32;
            while let Some(byte @ b'0'..=b'9') = bytes.get(position).copied() {
                value = value
                    .checked_mul(10)
                    .and_then(|held| held.checked_add(u32::from(byte - b'0')))
                    .filter(|held| *held <= maximum)
                    .ok_or_else(|| {
                        parse_error(position, "a version component exceeds its fixed width")
                    })?;
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

            let Some(next) = bytes.get(position).copied() else {
                break;
            };
            match next {
                b'.' => {
                    let introduced = position;
                    position += 1;
                    let Some(after) = bytes.get(position).copied() else {
                        return Err(parse_error(introduced, "an empty version qualifier"));
                    };
                    if after.is_ascii_digit() {
                        continue;
                    }
                    qualifier = Some(parse_qualifier(text, introduced, position, false)?);
                    break;
                }
                b'-' => {
                    let introduced = position;
                    position += 1;
                    qualifier = Some(parse_qualifier(text, introduced, position, true)?);
                    break;
                }
                _ => {
                    qualifier = Some(parse_qualifier(text, position, position, false)?);
                    break;
                }
            }
        }

        while count > 2 && parts[count - 1] == 0 {
            count -= 1;
        }
        let mut patch = [0_u8; Self::PATCH_WIDTH];
        let mut written = 0_usize;
        for index in 2..count {
            push_byte(&mut patch, &mut written, b'.', starts[index])?;
            push_decimal(&mut patch, &mut written, parts[index], starts[index])?;
        }
        if let Some(qualifier) = qualifier {
            if qualifier.pre {
                push_byte(&mut patch, &mut written, b'-', qualifier.introduced)?;
            }
            for (offset, byte) in qualifier.text.bytes().enumerate() {
                push_byte(&mut patch, &mut written, byte, qualifier.position + offset)?;
            }
        }
        Ok(Self {
            major: parts[0] as u8,
            minor: parts[1] as u8,
            patch,
        })
    }
}

struct ParsedQualifier<'a> {
    text: &'a str,
    introduced: usize,
    position: usize,
    pre: bool,
}

fn parse_qualifier(
    text: &str,
    introduced: usize,
    position: usize,
    pre: bool,
) -> Result<ParsedQualifier<'_>> {
    let value = &text[position..];
    if value.is_empty() {
        return Err(parse_error(introduced, "an empty version qualifier"));
    }
    if !value.as_bytes()[0].is_ascii_alphabetic() {
        return Err(parse_error(
            position,
            "a version qualifier must start with an ASCII letter",
        ));
    }
    if let Some(offset) = value
        .bytes()
        .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')))
    {
        return Err(parse_error(
            position + offset,
            "a qualifier contains an unsupported byte",
        ));
    }
    Ok(ParsedQualifier {
        text: value,
        introduced,
        position,
        pre,
    })
}

fn push_byte(
    patch: &mut [u8; Version::PATCH_WIDTH],
    written: &mut usize,
    byte: u8,
    position: usize,
) -> Result<()> {
    let slot = patch
        .get_mut(*written)
        .ok_or_else(|| parse_error(position, "a version patch exceeds fourteen bytes"))?;
    *slot = byte;
    *written += 1;
    Ok(())
}

fn push_decimal(
    patch: &mut [u8; Version::PATCH_WIDTH],
    written: &mut usize,
    value: u16,
    position: usize,
) -> Result<()> {
    let mut digits = [0_u8; 5];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for byte in &digits[start..] {
        push_byte(patch, written, *byte, position)?;
    }
    Ok(())
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
        if let Some(minor) = self.minor() {
            write!(formatter, ".{minor}")?;
        }
        if let Some(patch) = self.patch() {
            formatter.write_str(patch)?;
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
        if self == other {
            return Ordering::Equal;
        }
        if self == &Self::MIN || other == &Self::MAX {
            return Ordering::Less;
        }
        if self == &Self::MAX || other == &Self::MIN {
            return Ordering::Greater;
        }
        self.numeric_parts()
            .cmp(&other.numeric_parts())
            .then_with(|| {
                qualifier_class(self.qualifier()).cmp(&qualifier_class(other.qualifier()))
            })
            .then_with(|| match (self.qualifier(), other.qualifier()) {
                (Some((_, left)), Some((_, right))) => {
                    qualifier_cmp(left, right).then_with(|| left.cmp(right))
                }
                _ => self.minor().cmp(&other.minor()),
            })
    }
}

fn qualifier_class(value: Option<(bool, &[u8])>) -> u8 {
    match value {
        Some((true, _)) => 0,
        None => 1,
        Some((false, _)) => 2,
    }
}

fn qualifier_cmp(left: &[u8], right: &[u8]) -> Ordering {
    let (left_head, left_number) = qualifier_parts(left);
    let (right_head, right_number) = qualifier_parts(right);
    ascii_folded_cmp(left_head, right_head)
        .then_with(|| numeric_text_cmp(left_number, right_number))
}

fn qualifier_parts(value: &[u8]) -> (&[u8], &[u8]) {
    let split = value
        .iter()
        .rposition(|byte| !byte.is_ascii_digit())
        .map_or(0, |position| position + 1);
    value.split_at(split)
}

fn ascii_folded_cmp(left: &[u8], right: &[u8]) -> Ordering {
    left.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(right.iter().map(u8::to_ascii_lowercase))
}

fn numeric_text_cmp(left: &[u8], right: &[u8]) -> Ordering {
    let left = left
        .iter()
        .position(|byte| *byte != b'0')
        .map_or(&[][..], |position| &left[position..]);
    let right = right
        .iter()
        .position(|byte| *byte != b'0')
        .map_or(&[][..], |position| &right[position..]);
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
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
