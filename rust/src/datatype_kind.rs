use std::fmt;
use std::ops::RangeInclusive;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{DataTypeId, Error, Result};

/// A coarse datatype category shared by every variant of one family.
///
/// [`DataTypeKind`] mirrors the responsibility split of the `datatype` module,
/// so behavior that is uniform across a family dispatches on one value instead
/// of re-listing variants. A family is a range of [`DataTypeId`] bytes -
/// [`Self::range`] - so "is this an integer" is [`Self::contains`], one
/// comparison of two bounds, and no family has a value type of its own: a
/// value is its leaf. Use [`DataTypeId`] when a specific variant matters and
/// this value when only the family does.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DataTypeKind {
    /// The unit type carrying only nulls.
    Null,
    /// Boolean values.
    Boolean,
    /// Signed and unsigned fixed-width integers.
    Integer,
    /// IEEE binary floating point.
    Floating,
    /// Exact base-10 decimals: four widths with a precision and scale, and
    /// the two fixed leaves `decimal` and `bigdecimal`.
    Decimal,
    /// Dates, times, timestamps, durations, and calendar intervals.
    Temporal,
    /// Strings in every layout and charset, and the version and URL values.
    Text,
    /// The registered fixed-width codes: identities with a US-ASCII storage.
    Code,
    /// Byte strings in variable, fixed, large, and view layouts.
    Bytes,
    /// Series, structs, unions, maps, wrappers, and self-describing values.
    Nested,
    /// Geometries and geographies carried as Well-Known Binary.
    ///
    /// One family for both: a geometry lives on a planar coordinate system
    /// and a geography on a sphere or spheroid, and everything family-uniform
    /// (WKB payloads, bounding-box statistics, the refusal of min/max) is the
    /// same for the pair.
    Geospatial,
    /// One 128-bit universally unique identifier.
    ///
    /// It is not `Binary` wearing an extension name: no binary-family
    /// behavior - concatenation, prefix matching, a variable width - is
    /// correct for an identifier, and it is not `String` either, because the
    /// hyphenated spelling is a rendering of sixteen bytes rather than the
    /// value itself.
    Uuid,
}

impl DataTypeKind {
    /// Every category in canonical order.
    pub const ALL: [Self; 12] = [
        Self::Null,
        Self::Boolean,
        Self::Integer,
        Self::Floating,
        Self::Decimal,
        Self::Temporal,
        Self::Text,
        Self::Code,
        Self::Bytes,
        Self::Nested,
        Self::Geospatial,
        Self::Uuid,
    ];

    /// Parse a canonical lowercase category name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the unrecognized input and the accepted
    /// vocabulary.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase name without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Floating => "floating",
            Self::Decimal => "decimal",
            Self::Temporal => "temporal",
            Self::Text => "text",
            Self::Code => "code",
            Self::Bytes => "bytes",
            Self::Nested => "nested",
            Self::Geospatial => "geospatial",
            Self::Uuid => "uuid",
        }
    }

    /// The family's own number: the first byte of the range its leaves
    /// take, and - for every family but [`Self::Null`], whose one leaf
    /// states it - a byte no leaf takes, so a value tagged with it is a
    /// value of the family and of no leaf: the placeholder the encodings
    /// keep.
    ///
    /// ```
    /// use yggdryl::{DataTypeId, DataTypeKind};
    ///
    /// assert_eq!(DataTypeKind::Text.id(), 0x50);
    /// assert_eq!(DataTypeKind::of_u8(DataTypeId::Utf8String.as_u8()), Some(DataTypeKind::Text));
    /// assert_eq!(DataTypeKind::of_u8(0xf0), None);
    /// ```
    pub const fn id(self) -> u8 {
        match self {
            Self::Null => 0x00,
            Self::Boolean => 0x08,
            Self::Integer => 0x10,
            Self::Floating => 0x20,
            Self::Decimal => 0x28,
            Self::Temporal => 0x30,
            Self::Bytes => 0x40,
            Self::Text => 0x50,
            Self::Code => 0x70,
            Self::Uuid => 0x80,
            Self::Nested => 0x90,
            Self::Geospatial => 0xb0,
        }
    }

    /// The last byte of the range the family owns: its leaves, stated or
    /// still placeholders, sit between [`Self::id`] and this.
    pub const fn last(self) -> u8 {
        match self {
            Self::Null => 0x07,
            Self::Boolean => 0x0f,
            Self::Integer => 0x1f,
            Self::Floating => 0x27,
            Self::Decimal => 0x2f,
            Self::Temporal => 0x3f,
            Self::Bytes => 0x4f,
            Self::Text => 0x6f,
            Self::Code => 0x7f,
            Self::Uuid => 0x8f,
            Self::Nested => 0xaf,
            Self::Geospatial => 0xbf,
        }
    }

    /// The range of bytes the family owns, [`Self::id`] to [`Self::last`]:
    /// a family *is* this range, so which family an identifier belongs to is
    /// which range its byte is in.
    ///
    /// ```
    /// use yggdryl::{DataTypeId, DataTypeKind};
    ///
    /// assert_eq!(DataTypeKind::Temporal.range(), 0x30..=0x3f);
    /// assert!(DataTypeKind::Temporal.range().contains(&DataTypeId::Date32.as_u8()));
    /// ```
    pub const fn range(self) -> RangeInclusive<u8> {
        RangeInclusive::new(self.id(), self.last())
    }

    /// Whether `id` is one of this family's leaves: its byte is in
    /// [`Self::range`].
    ///
    /// ```
    /// use yggdryl::{DataTypeId, DataTypeKind};
    ///
    /// assert!(DataTypeKind::Integer.contains(DataTypeId::UInt128));
    /// assert!(!DataTypeKind::Integer.contains(DataTypeId::Float32));
    /// assert!(DataTypeKind::Text.contains(DataTypeId::Url));
    /// assert!(!DataTypeKind::Text.contains(DataTypeId::Country));
    /// ```
    pub const fn contains(self, id: DataTypeId) -> bool {
        let byte = id.as_u8();
        byte >= self.id() && byte <= self.last()
    }

    /// The family whose range one byte is in - the family's own number or
    /// one of its leaves, stated or still a placeholder - or nothing for a
    /// byte past every family.
    pub const fn of_u8(byte: u8) -> Option<Self> {
        OF_U8[byte as usize]
    }

    /// Return whether the category is a fixed-width or exact number.
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Floating | Self::Decimal)
    }

    /// Return whether the category stores an opaque or textual byte payload.
    pub const fn is_bytes(self) -> bool {
        matches!(self, Self::Bytes | Self::Text | Self::Code)
    }

    /// Return whether values of the category have a total order.
    pub const fn is_ordered(self) -> bool {
        !matches!(self, Self::Nested | Self::Geospatial)
    }
}

/// Every byte's family, built once from each family's [`DataTypeKind::range`]:
/// the ranges are laid out by hand, so two that overlap fail to compile.
const OF_U8: [Option<DataTypeKind>; 256] = {
    let mut table = [None; 256];
    let mut index = 0;
    while index < DataTypeKind::ALL.len() {
        let kind = DataTypeKind::ALL[index];
        let mut byte = kind.id() as usize;
        while byte <= kind.last() as usize {
            assert!(table[byte].is_none(), "two families claim one byte");
            table[byte] = Some(kind);
            byte += 1;
        }
        index += 1;
    }
    table
};

impl FromStr for DataTypeKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| value.eq_ignore_ascii_case(kind.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "datatype kind",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    canonical_vocabulary()
                ),
            })
    }
}

fn canonical_vocabulary() -> String {
    DataTypeKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for DataTypeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DataTypeKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DataTypeKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(D::Error::custom)
    }
}
