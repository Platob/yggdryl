//! Compact identity-preserving values for the core's static enums.

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
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EnumScalar {
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

impl EnumScalar {
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
            "codec" => parse!(Codec, Codec),
            "data_type_id" => parse!(DataTypeId, DataTypeId),
            "data_type_kind" => parse!(DataTypeKind, DataTypeKind),
            "edge_algorithm" => parse!(EdgeAlgorithm, EdgeAlgorithm),
            "io_kind" => parse!(IOKind, IOKind),
            "io_mode" => parse!(IOMode, IOMode),
            "time_unit" => parse!(TimeUnit, TimeUnit),
            "union_mode" => parse!(UnionMode, UnionMode),
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
            Self::Codec(_) => "codec",
            Self::DataTypeId(_) => "data_type_id",
            Self::DataTypeKind(_) => "data_type_kind",
            Self::EdgeAlgorithm(_) => "edge_algorithm",
            Self::IOKind(_) => "io_kind",
            Self::IOMode(_) => "io_mode",
            Self::TimeUnit(_) => "time_unit",
            Self::UnionMode(_) => "union_mode",
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

impl From<EnumScalar> for Scalar {
    fn from(value: EnumScalar) -> Self {
        Self::Enum(value)
    }
}

macro_rules! enum_scalar_from {
    ($($type:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$type> for EnumScalar {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }

        impl From<$type> for Scalar {
            fn from(value: $type) -> Self {
                Self::Enum(EnumScalar::$variant(value))
            }
        }
    )+};
}

enum_scalar_from!(
    Codec => Codec,
    DataTypeId => DataTypeId,
    DataTypeKind => DataTypeKind,
    EdgeAlgorithm => EdgeAlgorithm,
    IOKind => IOKind,
    IOMode => IOMode,
    TimeUnit => TimeUnit,
    UnionMode => UnionMode,
);

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::{
        Codec, DataTypeId, DataTypeKind, EdgeAlgorithm, EnumScalar, IOKind, IOMode, Scalar,
        TimeUnit, UnionMode,
    };

    #[test]
    fn identity_and_compact_ordinal_survive_scalar_conversion() {
        let member = EnumScalar::from_parts("io_mode", "append").unwrap();
        assert_eq!(member, EnumScalar::IOMode(IOMode::Append));
        assert_eq!(member.kind(), "io_mode");
        assert_eq!(member.as_str(), "append");
        assert_eq!(member.ordinal(), 1);
        assert!(size_of::<EnumScalar>() <= 2);
        assert_eq!(Scalar::from(IOMode::Append), Scalar::Enum(member));
    }

    #[test]
    fn every_static_vocabulary_round_trips_and_invalid_parts_fail() {
        let members = [
            EnumScalar::from(Codec::Zstd),
            EnumScalar::from(DataTypeId::Int64),
            EnumScalar::from(DataTypeKind::Integer),
            EnumScalar::from(EdgeAlgorithm::Spherical),
            EnumScalar::from(IOKind::File),
            EnumScalar::from(IOMode::Random),
            EnumScalar::from(TimeUnit::Nanosecond),
            EnumScalar::from(UnionMode::Dense),
        ];

        for member in members {
            assert_eq!(
                EnumScalar::from_parts(member.kind(), member.as_str()).unwrap(),
                member
            );
            assert_ne!(member.ordinal(), u8::MAX);
            assert_eq!(Scalar::from(member).as_enum(), Some(&member));
        }

        assert!(EnumScalar::from_parts("missing", "append").is_err());
        assert!(EnumScalar::from_parts("io_mode", "missing").is_err());
    }
}
