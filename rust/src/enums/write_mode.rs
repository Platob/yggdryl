//! Explicit intent for a record write.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// The operation a generic record-write entry point performs.
///
/// The mode is required: schema settings and merge keys refine a write but
/// never choose its intent.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WriteMode {
    /// Replace every stored row.
    Overwrite,
    /// Retain stored rows and add incoming rows after them.
    Append,
    /// Update rows matching the declared keys and append misses.
    Merge,
}

impl WriteMode {
    /// Every supported intent in canonical order.
    pub const ALL: [Self; 3] = [Self::Overwrite, Self::Append, Self::Merge];

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
        }
    }
}

impl AsRef<str> for WriteMode {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for WriteMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for WriteMode {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|mode| normalized.eq_ignore_ascii_case(mode.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "write mode",
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

impl Serialize for WriteMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WriteMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}
