//! What arrived on the wire, held beside what it was interpreted as.
//!
//! A [`FixMsg`](crate::FixMsg) carries two facts about one message and this
//! is the second of them. The row is the *interpretation*: values typed,
//! codes translated, names canonical, groups nested, header ordered. The
//! entries are what *arrived*: raw text, arrival order, untranslated,
//! including pairs no dictionary explained.
//!
//! Neither derives from the other. A translated `4` cannot say whether the
//! wire carried `4` or `PercentageWaivedCashDiscount`, so lossless
//! re-emission is impossible from the row alone - which is exactly what makes
//! the round trip work. This is the one place the FIX briefs admit two facts
//! about one thing, and it is deliberate.

use smol_str::SmolStr;

use super::{FixBranch, FixId};

/// One key/value pair as it arrived, beside the field it named.
///
/// Every part is present. `tag` is `0` when the key named no field, which is
/// safe rather than a hack: the specification numbers tags from `1`, so no
/// field can carry it, and a key that literally parses to `0` names no field
/// either way, so the sentinel and the parse agree.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FixEntry {
    tag: i32,
    branch: Option<SmolStr>,
    key: SmolStr,
    value: SmolStr,
}

impl FixEntry {
    /// Records one arriving pair.
    pub fn new(tag: i32, key: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        Self {
            tag,
            branch: None,
            key: key.into(),
            value: value.into(),
        }
    }

    /// Records the dialect this pair resolved in.
    ///
    /// The branch is its *name*, not its digest, because an entry is the
    /// arrival record: a branch held as an integer would be the one part a
    /// reader cannot read - unprintable in a debug line, unjoinable in a
    /// column, and resolvable only by someone holding the registry that
    /// produced it. `SmolStr` inlines a name of 23 bytes, which every dialect
    /// name is, so the common entry still allocates nothing.
    #[must_use]
    pub fn with_branch(mut self, branch: &FixBranch) -> Self {
        if !branch.is_standard() {
            self.branch = Some(SmolStr::new(branch.name()));
        }
        self
    }

    /// Returns the tag this entry's key named, or `0` when it named none.
    #[must_use]
    pub const fn tag(&self) -> i32 {
        self.tag
    }

    /// Returns the dialect this pair resolved in.
    ///
    /// `None` means the standard branch *and* "not resolved yet". The two are
    /// one state on purpose: both say no dialect claimed this pair, and
    /// separating them would put a resolution state into a record of what
    /// arrived. A standard tag therefore never spells its branch, which is
    /// also what keeps the overwhelming majority of entries free of a string.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    /// Returns the key exactly as it arrived.
    ///
    /// `"54"` for a tag-keyed pair, `Side` for a named one, `VenueOwnThing`
    /// for one no dictionary explains. This is where a venue's own casing
    /// survives, because the row holds the canonical spelling instead.
    #[must_use]
    pub fn key(&self) -> &str {
        self.key.as_str()
    }

    /// Returns the value exactly as it arrived, untranslated.
    #[must_use]
    pub fn value(&self) -> &str {
        self.value.as_str()
    }

    /// Builds the identity this entry names, absent when its key named none.
    ///
    /// The branch digest is computed here rather than stored, because it is
    /// `FixId`'s packing detail and an entry carries the name. Hashing a
    /// short name is a few nanoseconds and this is not on the parse path, so
    /// nothing is paid for the readability.
    #[must_use]
    pub fn id(&self) -> Option<FixId> {
        if self.tag == 0 {
            return None;
        }
        match &self.branch {
            None => Some(FixId::standard(self.tag)),
            Some(name) => FixBranch::from_str(name)
                .ok()
                .and_then(|branch| FixId::from_parts(&branch, self.tag).ok()),
        }
    }
}
