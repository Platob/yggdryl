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

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::{
        Codec, DataTypeId, DataTypeKind, EdgeAlgorithm, Enum, IOKind, IOMode, Scalar, TimeUnit,
        UnionMode,
    };

    #[test]
    fn the_serde_tag_is_the_name_kind_answers() {
        // The vocabulary name is the Rust type name, so there is one
        // spelling and no casing rule to disagree with. Every vocabulary is
        // checked, not just the two an acronym once broke.
        for (kind, value) in [
            ("Codec", "identity"),
            ("DataTypeId", "null"),
            ("DataTypeKind", "null"),
            ("EdgeAlgorithm", "spherical"),
            ("IOKind", "file"),
            ("IOMode", "append"),
            ("TimeUnit", "s"),
            ("UnionMode", "dense"),
        ] {
            let member = Enum::from_parts(kind, value).expect("a known member");
            assert_eq!(member.kind(), kind);

            let json = serde_json::to_string(&member).expect("Enum serializes");
            assert!(
                json.contains(&format!("\"kind\":\"{kind}\"")),
                "serde wrote {json}, but kind() answers {kind}"
            );
            assert_eq!(
                Enum::from_parts(kind, value).expect("a known member"),
                serde_json::from_str::<Enum>(&json).expect("Enum round-trips")
            );
        }
    }

    #[test]
    fn identity_and_compact_ordinal_survive_scalar_conversion() {
        let member = Enum::from_parts("IOMode", "append").unwrap();
        assert_eq!(member, Enum::IOMode(IOMode::Append));
        assert_eq!(member.kind(), "IOMode");
        assert_eq!(member.as_str(), "append");
        assert_eq!(member.ordinal(), 1);
        assert!(size_of::<Enum>() <= 2);
        assert_eq!(Scalar::from(IOMode::Append), Scalar::Enum(member));
    }

    #[test]
    fn every_static_vocabulary_round_trips_and_invalid_parts_fail() {
        let members = [
            Enum::from(Codec::Zstd),
            Enum::from(DataTypeId::Int64),
            Enum::from(DataTypeKind::Integer),
            Enum::from(EdgeAlgorithm::Spherical),
            Enum::from(IOKind::File),
            Enum::from(IOMode::Random),
            Enum::from(TimeUnit::Nanosecond),
            Enum::from(UnionMode::Dense),
        ];

        for member in members {
            assert_eq!(
                Enum::from_parts(member.kind(), member.as_str()).unwrap(),
                member
            );
            assert_ne!(member.ordinal(), u8::MAX);
            assert_eq!(Scalar::from(member).as_enum(), Some(&member));
        }

        assert!(Enum::from_parts("missing", "append").is_err());
        assert!(Enum::from_parts("IOMode", "missing").is_err());
    }
}
