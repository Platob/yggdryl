//! What a message states that its reading could not take as it stands.
//!
//! A value that will not type is null in the row rather than a failure, a
//! counter that disagrees with the group it counts is kept as it arrived,
//! and a settle drops a stated identifier that conflicts with a stated one:
//! each is a fact about the message worth more than a null nobody can
//! explain. An anomaly is that fact, read off the message beside the row -
//! never a column, never a digest input.

use std::fmt;

use smol_str::SmolStr;

/// One thing a message states that its reading could not take as it stands:
/// the field it was stated under, and why.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FixAnomaly {
    field: SmolStr,
    reason: SmolStr,
}

impl FixAnomaly {
    /// An anomaly about `field`, for `reason`.
    #[must_use]
    pub fn new(field: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// The field the message stated the value under: the dictionary's name,
    /// else the key as it arrived.
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Why the reading could not take the value as it stands.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for FixAnomaly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.field, self.reason)
    }
}
