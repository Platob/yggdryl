//! What arrived on the wire, held beside what it was interpreted as.
//!
//! A [`FixMsg`](crate::FixMsg) carries two facts about one message and this
//! is the second of them. The row is the *interpretation*: values typed,
//! codes translated, names canonical, groups nested, header ordered. The
//! entries are what *arrived*, as the reader read it: raw bytes, arrival
//! order, untranslated, including pairs no dictionary explained - and
//! without the pairs the reader reads as never sent, a stated absence and a
//! bridge's marked restatement of a bare pair, which the capture edges list.
//!
//! Neither derives from the other. A translated `4` cannot say whether the
//! wire carried `4` or `PercentageWaivedCashDiscount`, so lossless
//! re-emission is impossible from the row alone - which is exactly what makes
//! the round trip work. This is the one place the FIX briefs admit two facts
//! about one thing, and it is deliberate.
//!
//! # An arrival is a range of the line
//!
//! A key and a value are [`TextBytes`] - counted ranges of the one page the
//! line was read into - so a message read from a line copies none of the bytes
//! its entries name, however wide the line or however long a data field's
//! value. The consequence is a rule about what an entry may say: a key that
//! appears nowhere in the line is not an arrival. A packed occurrence is
//! recorded as the pair the bridge wrote, and unpacking it into
//! `NOPARTYIDS[0].PARTYID` and its siblings builds fields, which is a reading
//! of that arrival and not a second one.

use crate::media::text::TextBytes;

/// One key/value pair as it arrived, beside the field it named.
///
/// Every part is present. `tag` is `0` when the key named no field, which is
/// safe rather than a hack: the specification numbers tags from `1`, so no
/// field can carry it, and a key that literally parses to `0` names no field
/// either way, so the sentinel and the parse agree.
///
/// The tag is the whole of what FIX adds here. A key, a value and what nested
/// under them are what the line said, and the text reader already says them;
/// which field the tag names is what a *dictionary* decided, and the
/// message's [registry](super::FixMsg::registry) is where it is asked for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FixEntry {
    tag: i32,
    key: TextBytes,
    value: TextBytes,
    // Last, so the three members a row reads positionally keep their places.
    // `Vec` rather than a boxed slice: it already provides the sizing
    // indirection, and an empty one allocates nothing - which is every entry
    // on a wire that never nests.
    children: Vec<FixEntry>,
}

impl FixEntry {
    /// Records one arriving pair, as the ranges of the line that carried it.
    #[must_use]
    pub const fn new(tag: i32, key: TextBytes, value: TextBytes) -> Self {
        Self {
            tag,
            key,
            value,
            children: Vec::new(),
        }
    }

    /// Records what arrived under this entry, in the order it arrived.
    ///
    /// The other way a tree is built beside [`Self::adopt`]: a row holds each
    /// entry's children already grouped under it, so reading the row back
    /// attaches them whole rather than re-deciding which counter each
    /// member belongs to.
    #[must_use]
    pub(super) fn with_children(mut self, children: Vec<FixEntry>) -> Self {
        self.children = children;
        self
    }

    /// Returns the tag this entry's key named, or `0` when it named none.
    #[must_use]
    pub const fn tag(&self) -> i32 {
        self.tag
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
            // A child that was adopted answers `None`, which is this
            // function's own answer for the same thing.
            child = held.adopt(tag, child)?;
        }
        if self.tag == tag {
            self.children.push(child);
            return None;
        }
        Some(child)
    }

    /// Returns the key exactly as it arrived, as a range of the line.
    ///
    /// `54` for a tag-keyed pair, `Side` for a named one, `VenueOwnThing` for
    /// one no dictionary explains, `#ORDERID` where a bridge marked a
    /// restatement the row kept whole. This is where a venue's own casing
    /// survives, because the row holds the canonical spelling instead.
    #[must_use]
    pub const fn key(&self) -> &TextBytes {
        &self.key
    }

    /// Returns the value exactly as it arrived, untranslated.
    #[must_use]
    pub const fn value(&self) -> &TextBytes {
        &self.value
    }

    /// The key as a row column holds it: text, and lossily where the wire was
    /// not UTF-8.
    ///
    /// A row materializes the arrival record into a `Utf8` column, which is
    /// the one place the bytes are read as text at all. The entry itself keeps
    /// the bytes, so re-emission stays exact whatever the column had to spell.
    pub(super) fn key_text(&self) -> smol_str::SmolStr {
        text_of(&self.key)
    }

    /// The value as a row column holds it.
    pub(super) fn value_text(&self) -> smol_str::SmolStr {
        text_of(&self.value)
    }
}

/// One range of a line as the text a column holds.
fn text_of(bytes: &TextBytes) -> smol_str::SmolStr {
    match bytes.as_str() {
        Some(text) => smol_str::SmolStr::new(text),
        None => smol_str::SmolStr::new(String::from_utf8_lossy(bytes.as_bytes())),
    }
}
