//! Generic intent for an IO operation.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// The operation an IO API entry point performs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IOMode {
    /// Replace every stored row.
    Overwrite,
    /// Retain stored rows and add incoming rows after them.
    Append,
    /// Update rows matching the declared keys and append misses.
    Merge,
    /// Read-only access; no write intent and no mutation side effects.
    ReadOnly,
    /// Random-access operation class.
    Random,
}

impl IOMode {
    /// Every supported intent in canonical order.
    pub const ALL: [Self; 5] = [
        Self::Overwrite,
        Self::Append,
        Self::Merge,
        Self::ReadOnly,
        Self::Random,
    ];

    /// Write-class modes only.
    pub const WRITE: [Self; 3] = [Self::Overwrite, Self::Append, Self::Merge];

    /// Parse one canonical mode name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the complete accepted vocabulary.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Overwrite => "overwrite",
            Self::Append => "append",
            Self::Merge => "merge",
            Self::ReadOnly => "readonly",
            Self::Random => "random",
        }
    }
}

impl AsRef<str> for IOMode {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for IOMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for IOMode {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|mode| normalized.eq_ignore_ascii_case(mode.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "mode",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|mode| mode.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl Serialize for IOMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IOMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}
