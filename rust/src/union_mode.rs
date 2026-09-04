use std::fmt;

use serde::{Deserialize, Serialize};

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
