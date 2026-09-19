//! Compact identity-preserving values for the core's static enums.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::format_smolstr;

use crate::scalar::Scalar;
use crate::string::Str;
use crate::{
    Codec, DataTypeId, DataTypeKind, EdgeAlgorithm, Error, IOKind, IOMode, Result, TimeUnit,
    UnionMode,
};

/// One member of a shared static vocabulary.
///
/// Every payload fits in one byte. The outer discriminant preserves which
/// vocabulary the member belongs to even when two vocabularies share a
/// spelling. This is the closed set of names a core enum draws from, not the
/// dictionary encoding [`DataType::Enum`] describes.
///
/// [`DataType::Enum`]: crate::DataType::Enum
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum Vocabulary {
    /// A content codec.
    Codec(Codec),
    /// An exact datatype identifier.
    DataTypeId(DataTypeId),
    /// A datatype family.
    DataTypeKind(DataTypeKind),
    /// A geospatial edge algorithm.
    EdgeAlgorithm(EdgeAlgorithm),
    /// An I/O resource kind.
    IOKind(IOKind),
    /// An I/O operation mode.
    IOMode(IOMode),
    /// A temporal or interval unit.
    TimeUnit(TimeUnit),
    /// An Arrow union layout.
    UnionMode(UnionMode),
}

impl Vocabulary {
    /// Parse a member while retaining its enum identity.
    ///
    /// Each vocabulary reads through its own `FromStr`, not through a second
    /// scan of `as_str`. That matters where the two differ: `TimeUnit::as_str`
    /// answers the short `ns`, while the serialized vocabulary is the full
    /// `nanosecond` - the only unit spelling written on disk, 1,017 times -
    /// and only `TimeUnit::from_str` accepts both. Scanning `as_str` refused
    /// a spelling this crate itself writes.
    pub fn from_parts(kind: &str, value: &str) -> Result<Self> {
        macro_rules! parse {
            ($type:ty, $variant:ident) => {
                <$type as std::str::FromStr>::from_str(value)
                    .ok()
                    .map(Self::$variant)
            };
        }
        let parsed = match kind.trim() {
            "Codec" => parse!(Codec, Codec),
            "DataTypeId" => parse!(DataTypeId, DataTypeId),
            "DataTypeKind" => parse!(DataTypeKind, DataTypeKind),
            "EdgeAlgorithm" => parse!(EdgeAlgorithm, EdgeAlgorithm),
            "IOKind" => parse!(IOKind, IOKind),
            "IOMode" => parse!(IOMode, IOMode),
            "TimeUnit" => parse!(TimeUnit, TimeUnit),
            "UnionMode" => parse!(UnionMode, UnionMode),
            _ => None,
        };
        parsed.ok_or_else(|| Error::Parse {
            target: "enum scalar",
            position: 0,
            reason: format_smolstr!("unknown {kind:?} member {value:?}"),
        })
    }

    /// Return the enum vocabulary name.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Codec(_) => "Codec",
            Self::DataTypeId(_) => "DataTypeId",
            Self::DataTypeKind(_) => "DataTypeKind",
            Self::EdgeAlgorithm(_) => "EdgeAlgorithm",
            Self::IOKind(_) => "IOKind",
            Self::IOMode(_) => "IOMode",
            Self::TimeUnit(_) => "TimeUnit",
            Self::UnionMode(_) => "UnionMode",
        }
    }

    /// Return the canonical member spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codec(value) => value.as_str(),
            Self::DataTypeId(value) => value.as_str(),
            Self::DataTypeKind(value) => value.as_str(),
            Self::EdgeAlgorithm(value) => value.as_str(),
            Self::IOKind(value) => value.as_str(),
            Self::IOMode(value) => value.as_str(),
            Self::TimeUnit(value) => value.as_str(),
            Self::UnionMode(value) => value.as_str(),
        }
    }

    /// Return the zero-based member index using the smallest public integer.
    pub fn ordinal(self) -> u8 {
        macro_rules! ordinal {
            ($type:ty, $value:expr) => {
                <$type>::ALL
                    .iter()
                    .position(|candidate| candidate == &$value)
                    .and_then(|index| u8::try_from(index).ok())
                    .unwrap_or(u8::MAX)
            };
        }
        match self {
            Self::Codec(value) => ordinal!(Codec, value),
            Self::DataTypeId(value) => ordinal!(DataTypeId, value),
            Self::DataTypeKind(value) => ordinal!(DataTypeKind, value),
            Self::EdgeAlgorithm(value) => ordinal!(EdgeAlgorithm, value),
            Self::IOKind(value) => ordinal!(IOKind, value),
            Self::IOMode(value) => ordinal!(IOMode, value),
            Self::TimeUnit(value) => ordinal!(TimeUnit, value),
            Self::UnionMode(value) => ordinal!(UnionMode, value),
        }
    }
}

impl fmt::Display for Vocabulary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<Vocabulary> for Scalar {
    /// A member is its canonical name, which is what a column holds.
    ///
    /// A member's datatype is `string`, so the value is the text and the
    /// vocabulary is the column's business. [`DataType::Enum`] is a
    /// different fact: it is the dictionary encoding a column is stored
    /// under, not the closed set a name is drawn from.
    ///
    /// [`DataType::Enum`]: crate::DataType::Enum
    fn from(value: Vocabulary) -> Self {
        Self::String(Str::new_static(value.as_str()))
    }
}

macro_rules! enumeration_from {
    ($($type:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$type> for Vocabulary {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }

        impl From<$type> for Scalar {
            fn from(value: $type) -> Self {
                Self::from(Vocabulary::$variant(value))
            }
        }
    )+};
}

enumeration_from!(
    Codec => Codec,
    DataTypeId => DataTypeId,
    DataTypeKind => DataTypeKind,
    EdgeAlgorithm => EdgeAlgorithm,
    IOKind => IOKind,
    IOMode => IOMode,
    TimeUnit => TimeUnit,
    UnionMode => UnionMode,
);
