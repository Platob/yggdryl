//! What kind of resource a byte handle addresses.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// The role a resource plays, independent of where it lives.
///
/// Every backend has the same three roles - bytes with no location, a leaf that
/// holds bytes, a container that holds other resources - plus the honest fourth
/// answer for a location that does not exist yet. A generic handle such as
/// [`crate::local::Path`] reads this to decide which specialized implementation
/// to work through, so adding a backend means answering this question rather
/// than inventing new vocabulary.
///
/// ```
/// use yggdryl::io::{Buffer, IOBase};
/// use yggdryl::IOKind;
///
/// assert_eq!(Buffer::new().kind(), IOKind::Memory);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IOKind {
    /// Bytes held in memory, with no location of their own.
    Memory,
    /// A leaf resource holding bytes: a file, an object, a blob.
    #[default]
    File,
    /// A container holding other resources: a directory, a key prefix.
    Directory,
    /// A location that does not exist yet, so its role is not decided.
    ///
    /// A read of an unknown location yields nothing and a write creates it,
    /// exactly as the laziness contract requires.
    Unknown,
}

impl IOKind {
    /// Every kind in canonical order.
    pub const ALL: [Self; 4] = [Self::Memory, Self::File, Self::Directory, Self::Unknown];

    /// Parse a canonical kind name.
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
            Self::Memory => "memory",
            Self::File => "file",
            Self::Directory => "directory",
            Self::Unknown => "unknown",
        }
    }

    /// Return whether this kind holds other resources.
    pub const fn is_container(self) -> bool {
        matches!(self, Self::Directory)
    }

    /// Return whether this kind holds bytes of its own.
    pub const fn is_leaf(self) -> bool {
        matches!(self, Self::Memory | Self::File)
    }

    /// Return whether the resource exists yet.
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

impl FromStr for IOKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|kind| normalized.eq_ignore_ascii_case(kind.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "io kind",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|kind| kind.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl fmt::Display for IOKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for IOKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IOKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}
