//! Compact identity-preserving values for the core's static enums.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::format_smolstr;

use super::scalar::Scalar;
use crate::{
    Codec, DataTypeId, DataTypeKind, EdgeAlgorithm, Error, IOKind, IOMode, Result, TimeUnit,
    UnionMode,
};

/// One member of a shared static enum.
///
/// Every payload fits in one byte. The outer discriminant preserves which
/// vocabulary the member belongs to even when two enums share a spelling.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "value")]
pub enum Enum {
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

impl Enum {
    /// Parse a member while retaining its enum identity.
    pub fn from_parts(kind: &str, value: &str) -> Result<Self> {
        macro_rules! parse {
            ($type:ty, $variant:ident) => {
                <$type>::ALL
                    .into_iter()
                    .find(|member| value.eq_ignore_ascii_case(member.as_str()))
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

impl fmt::Display for Enum {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<Enum> for Scalar {
    fn from(value: Enum) -> Self {
        Self::Enum(value)
    }
}

macro_rules! enumeration_from {
    ($($type:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$type> for Enum {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }

        impl From<$type> for Scalar {
            fn from(value: $type) -> Self {
                Self::Enum(Enum::$variant(value))
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
