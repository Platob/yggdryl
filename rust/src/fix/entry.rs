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
    // Last, so the four scalar members keep their positions in every
    // positional read. `Vec` rather than a boxed slice: it already provides
    // the sizing indirection, and an empty one allocates nothing - which is
    // every entry on a wire that never nests.
    children: Vec<FixEntry>,
}

impl FixEntry {
    /// Records one arriving pair, in the standard branch until told otherwise.
    pub fn new(tag: i32, key: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        Self {
            tag,
            bid: i64::from(FixBranch::STANDARD.digest()),
            key: key.into(),
            value: value.into(),
            children: Vec::new(),
        }
    }

    /// Records the dialect this pair resolved in, as that dialect's digest.
    ///
    /// Set for every entry, the standard branch included: the value is fixed
    /// width, so omitting it saves nothing and only forces every reader to
    /// branch on an absence.
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
    /// The same value [`FixId`] packs into its low 32 bits, so an entry and a
    /// field identity say branch identity the same way. `0` is the standard
    /// branch and is never absent: the column carries no validity bitmap and
    /// a reader never branches on a null.
    ///
    /// Held as the digest rather than the name because an entry is a *row*.
    /// A name is readable in a debug line and nothing else: it is variable
    /// width in a fixed-width column, and joining a capture to a dialect
    /// manifest by it means string comparison. The digest is eight bytes, and
    /// [`FixRegistry::branch_by_bid`](crate::FixRegistry::branch_by_bid)
    /// resolves it back to the whole declaration - which is what makes the
    /// capture self-describing rather than merely legible.
    ///
    /// Signed 64-bit rather than the digest's own `u32`: values above
    /// `i32::MAX` must not read as negative, and Avro has no unsigned integer,
    /// so this is the narrowest type that round-trips exactly through Arrow
    /// IPC, Parquet and Avro alike.
    #[must_use]
    pub const fn bid(&self) -> i64 {
        self.bid
    }

    /// Returns the entries that arrived under this one, in arrival order.
    ///
    /// A repeating group's members ride under the counter pair that heads
    /// them - when that pair actually arrived. The tree is never truncated
    /// here: depth is an Arrow materialization concern, and the Rust record
    /// holds whatever the wire nested.
    #[must_use]
    pub fn children(&self) -> &[FixEntry] {
        &self.children
    }

    /// Attaches one entry under this one.
    ///
    /// Only a fixture builds a tree by hand; the builder nests through
    /// [`Self::adopt`], which is what keeps a parent from being invented.
    #[cfg(test)]
    pub(crate) fn push(&mut self, child: FixEntry) {
        self.children.push(child);
    }

    /// Attaches `child` under the latest arrival in this subtree carrying
    /// `tag`, or answers it back when none does.
    ///
    /// Children are tried before their parent, latest first, so a repeated
    /// group attaches each member to its own occurrence and a nested counter
    /// takes its members before the counter above it is considered. A parent
    /// that never arrived is never invented: the caller keeps the child flat.
    pub(crate) fn adopt(&mut self, tag: i32, mut child: FixEntry) -> Option<FixEntry> {
        for held in self.children.iter_mut().rev() {
            match held.adopt(tag, child) {
                None => return None,
                Some(back) => child = back,
            }
        }
        if self.tag == tag {
            self.children.push(child);
            return None;
        }
        Some(child)
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
    /// Packed directly from the stored digest, which is the same half of the
    /// identifier `FixId` holds. Admissibility was decided when the pair
    /// resolved - only an admitted branch ever reaches an entry - so there is
    /// nothing left to check and nothing to re-derive.
    #[must_use]
    pub fn id(&self) -> Option<FixId> {
        if self.tag == 0 {
            return None;
        }
        u32::try_from(self.bid)
            .ok()
            .map(|digest| FixId::pack(digest, self.tag))
    }
}
