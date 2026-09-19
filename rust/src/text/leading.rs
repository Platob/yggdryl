//! Treatment of physical lines before the first framed record header.

use std::fmt;
use std::str::FromStr;

use smol_str::format_smolstr;

use crate::{Error, Result};

/// What framed text does with a leading fragment that has no record header.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LeadingFragment {
    /// Emit the complete leading fragment as one record with null captures.
    #[default]
    Keep,
    /// Consume the leading fragment without emitting a record.
    Drop,
    /// Fail when the first physical line does not match `rowheader`.
    Error,
}

impl LeadingFragment {
    /// Every supported treatment in canonical order.
    pub const ALL: [Self; 3] = [Self::Keep, Self::Drop, Self::Error];

    /// Parse one canonical treatment name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for any spelling other than `keep`, `drop`, or
    /// `error`, ignoring surrounding whitespace and ASCII case.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Drop => "drop",
            Self::Error => "error",
        }
    }
}

impl AsRef<str> for LeadingFragment {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for LeadingFragment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LeadingFragment {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|mode| normalized.eq_ignore_ascii_case(mode.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "leading fragment treatment",
                position: 0,
                reason: format_smolstr!("expected one of keep, drop, error, got {value:?}"),
            })
    }
}
