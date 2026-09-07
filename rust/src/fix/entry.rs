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
    bid: i64,
    key: SmolStr,
    value: SmolStr,
}

impl FixEntry {
    /// Records one arriving pair.
    pub fn new(tag: i32, key: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        Self {
            tag,
            bid: i64::from(FixBranch::STANDARD.digest()),
            key: key.into(),
            value: value.into(),
        }
    }

    /// Records the dialect this pair resolved in.
    ///
    /// The digest, not the name. It is the same value
    /// [`FixId`] packs into its low 32 bits, so an entry and a field identity
    /// say "which dictionary" the same way, and a capture's `bid` column
    /// joins the dialect manifest that publishes it beside each branch name.
    /// The objection this replaces - that an integer is the one part a reader
    /// cannot read - is answered by that manifest and by
    /// [`FixRegistry::branch_by_bid`](crate::FixRegistry::branch_by_bid):
    /// the capture is self-describing without carrying a string on every row
    /// that resolved in a dialect.
    #[must_use]
    pub fn with_branch(mut self, branch: &FixBranch) -> Self {
        self.bid = i64::from(branch.digest());
        self
    }

    /// Returns the tag this entry's key named, or `0` when it named none.
    #[must_use]
    pub const fn tag(&self) -> i32 {
        self.tag
    }

    /// Returns the digest of the dialect this pair resolved in.
    ///
    /// `0` is the standard branch and is written on every standard entry
    /// rather than left absent: the column is fixed width either way, so
    /// omitting it would buy nothing and make every reader branch.
    ///
    /// `i64` rather than the `u32` the digest is. The digest uses its whole
    /// range, so half of it does not fit `int32` without going negative, and
    /// Avro has no unsigned integer at all - signed 64-bit is the narrowest
    /// type that round-trips the value exactly through Arrow IPC, Parquet and
    /// Avro alike.
    #[must_use]
    pub const fn bid(&self) -> i64 {
        self.bid
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
    /// The branch digest is what the entry stores, so this is a pack rather
    /// than a hash: the admissibility rule that decides whether a dictionary
    /// may claim a tag is still the identifier's own.
    #[must_use]
    pub fn id(&self) -> Option<FixId> {
        if self.tag == 0 {
            return None;
        }
        FixId::from_digest(self.bid, self.tag)
    }
}
