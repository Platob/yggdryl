use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// The physical layout of a tagged union.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnionMode {
    /// Every child has the union's full logical length.
    Sparse,
    /// Values are packed in each child and addressed by an offset buffer.
    Dense,
}

impl UnionMode {
    /// Both modes, sparse first - the order Arrow declares them in.
    pub const ALL: [Self; 2] = [Self::Sparse, Self::Dense];

    /// Return the canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sparse => "sparse",
            Self::Dense => "dense",
        }
    }
}

impl fmt::Display for UnionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for UnionMode {
    type Err = Error;

    /// Read a mode by its canonical name, ASCII case-insensitively.
    ///
    /// The seven other vocabularies `Enum::from_parts` dispatches over all
    /// have one of these; this one did not, which is why that function could
    /// not simply route through the parsers.
    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| value.trim().eq_ignore_ascii_case(mode.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "union mode",
                position: 0,
                reason: format_smolstr!("expected one of sparse, dense, got {value:?}"),
            })
    }
}
