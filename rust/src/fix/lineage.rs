//! What one field was called and typed at every FIX version it lived through.
//!
//! A FIX field outlives the version that introduced it and is routinely
//! renamed and retyped on the way: tag 32 is `LastShares` typed `int` in 4.0,
//! `LastShares` typed `Qty` in 4.2, and `LastQty` from 4.3 on. One registry
//! holds every tag ever defined and a version is a filter on the read, so the
//! history has to travel with the field rather than with the dictionary.
//!
//! `fix:lineage` is that history: one [canonical document](super::document),
//! entries ordered oldest first, read back through a borrowed scan that
//! allocates nothing. It is the only version key - `since`, `until` and
//! whether a field is deprecated are derived from it on read, exactly as
//! [`FixId`](crate::FixId) is derived from a branch and a tag rather than
//! stored beside it.

use std::iter::FusedIterator;

use smol_str::format_smolstr;

use super::document::{Cursor, Refusal, Scan, Writer, decode_text};
use crate::{DataType, Error, Result, Version};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix lineage";

/// The one array the document holds.
const ENTRIES: &str = "entries";

/// The version an entry dates itself at; the only required key.
const SINCE: &str = "since";
/// The extension pack that dated the change, beside the version.
const EP: &str = "ep";
/// The spelling the field carries from that version on.
const NAME: &str = "name";
/// The datatype the field carries from that version on, resolved.
const TYPE: &str = "type";
/// Whether the specification deprecated the field at that version.
const DEPRECATED: &str = "deprecated";
/// Whether the specification removed the field at that version.
const REMOVED: &str = "removed";
/// The specification's own wording as of that version.
const DOC: &str = "doc";

/// The keys one entry may state, in the order it states them.
///
/// `since` leads because it is what every read keys on: a version filter
/// compares it and stops, so nothing past the entry asked for is read.
const KEYS: [&str; 7] = [SINCE, EP, NAME, TYPE, DEPRECATED, REMOVED, DOC];

/// One dated point in a field's history.
///
/// The specification dates a change either by version or by extension pack,
/// and routinely by both: `LastQty` is added at `FIX.2.7`, while `BasisPoints`
/// is "Added EP208" against a version that had already shipped. Ordering is
/// therefore on the pair, version first, so `5.0SP2` at EP204 is older than
/// `5.0SP2` at EP309 rather than eleven years in one bucket. An entry stating
/// no extension pack is that version's base statement and orders first.
///
/// ```
/// use yggdryl::{FixPedigree, Version};
///
/// # fn main() -> yggdryl::Result<()> {
/// let base = FixPedigree::new("5.0SP2".parse::<Version>()?, None);
/// let patched = FixPedigree::new("5.0SP2".parse::<Version>()?, Some(309));
/// assert!(base < patched);
/// assert!(patched < FixPedigree::new(Version::MAX, None));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixPedigree {
    // Version first so the derived order is version-major, which is what
    // "5.0SP2 at EP204 is older than 5.0SP2 at EP309" means. `None` before
    // `Some` follows from `Option`'s own order and is the reading wanted: a
    // version's base statement precedes every extension pack against it.
    version: Version,
    ep: Option<u32>,
}

impl FixPedigree {
    /// Builds a pedigree from a version and its optional extension pack.
    #[must_use]
    pub const fn new(version: Version, ep: Option<u32>) -> Self {
        Self { version, ep }
    }

    /// Returns the FIX version this point is dated at.
    #[must_use]
    pub const fn version(self) -> Version {
        self.version
    }

    /// Returns the extension pack that dated the change, when one did.
    #[must_use]
    pub const fn ep(self) -> Option<u32> {
        self.ep
    }
}

/// One entry of a field's lineage: what it was called and typed from a
/// version on.
///
/// Every key beyond [`Self::since`] is optional, because most versions change
/// nothing and an entry stating only a version means "present, unchanged".
/// The borrowed spellings are slices of the field's own stored document, so
/// reading them allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixLineageEntry<'field> {
    pedigree: FixPedigree,
    name: Option<&'field str>,
    dtype: Option<&'field str>,
    doc: Option<&'field str>,
    deprecated: bool,
    removed: bool,
}

impl<'field> FixLineageEntry<'field> {
    /// Builds an entry stating only when it applies.
    #[must_use]
    pub const fn new(pedigree: FixPedigree) -> Self {
        Self {
            pedigree,
            name: None,
            dtype: None,
            doc: None,
            deprecated: false,
            removed: false,
        }
    }

    /// Sets the spelling the field carries from this version on.
    #[must_use]
    pub const fn with_name(mut self, name: &'field str) -> Self {
        self.name = Some(name);
        self
    }

    /// Sets the datatype the field carries from this version on.
    ///
    /// A FIX datatype name and the crate's own spelling are both accepted -
    /// the grammar resolves either - and the resolved type is what the writer
    /// stores.
    #[must_use]
    pub const fn with_dtype(mut self, dtype: &'field str) -> Self {
        self.dtype = Some(dtype);
        self
    }

    /// Sets the specification's own wording as of this version.
    #[must_use]
    pub const fn with_doc(mut self, doc: &'field str) -> Self {
        self.doc = Some(doc);
        self
    }

    /// Marks the version at which the specification deprecated the field.
    #[must_use]
    pub const fn deprecate(mut self) -> Self {
        self.deprecated = true;
        self
    }

    /// Marks the version at which the specification removed the field.
    #[must_use]
    pub const fn remove(mut self) -> Self {
        self.removed = true;
        self
    }

    /// Returns the dated point this entry states.
    #[must_use]
    pub const fn pedigree(self) -> FixPedigree {
        self.pedigree
    }

    /// Returns the version this entry is dated at.
    #[must_use]
    pub const fn since(self) -> Version {
        self.pedigree.version()
    }

    /// Returns the extension pack that dated this entry, when one did.
    #[must_use]
    pub const fn ep(self) -> Option<u32> {
        self.pedigree.ep()
    }

    /// Returns the spelling from this version on, when the entry states one.
    #[must_use]
    pub const fn name(self) -> Option<&'field str> {
        self.name
    }

    /// Returns the datatype from this version on, as text, unparsed.
    ///
    /// A *stored* entry states the crate's serialized datatype - the same
    /// `{"type":"utf8"}` document the field's own datatype is stored as - so
    /// a parameterized type carries its parameters where every other reader
    /// of a datatype expects them. A *built* entry may state a FIX datatype
    /// name instead, because [`Self::with_dtype`] takes whatever spelling the
    /// caller has and the writer resolves it; the two forms never
    /// coexist in one stored document. [`Self::parse_dtype`] answers either.
    ///
    /// Storing the resolved type rather than the FIX name is what makes a
    /// history comparable: `char` and `String` are one type under two
    /// spellings, and a reader asking what changed must not be told a rename
    /// was a retype. The FIX spelling is not recoverable, which is the
    /// accepted cost - it is a spelling, not a type, and every reader wanted
    /// the type.
    #[must_use]
    pub const fn dtype(self) -> Option<&'field str> {
        self.dtype
    }

    /// Resolves the datatype this entry states.
    ///
    /// A stored entry holds a serialized datatype and a built one may hold a
    /// FIX datatype name, so the leading byte says which reader answers: only
    /// a document opens with `{`.
    ///
    /// # Errors
    ///
    /// Returns the reader's own refusal when the text names no datatype.
    pub fn parse_dtype(self) -> Result<Option<DataType>> {
        self.dtype
            .map(|held| {
                if held.starts_with('{') {
                    DataType::from_json(held)
                } else {
                    DataType::from_str(held)
                }
            })
            .transpose()
    }

    /// Returns the specification's wording as of this version, still escaped
    /// exactly as the document holds it.
    ///
    /// The hot path never reads this, so it is handed back as the raw slice
    /// rather than decoded into an allocation nobody asked for.
    /// [`Self::parse_doc`] decodes it.
    #[must_use]
    pub const fn doc(self) -> Option<&'field str> {
        self.doc
    }

    /// Decodes the specification's wording as of this version.
    ///
    /// # Errors
    ///
    /// Returns the JSON codec's refusal when the stored text is not a legal
    /// string body, which the writer never produces.
    pub fn parse_doc(self) -> Result<Option<String>> {
        decode_text(TARGET, DOC, self.doc)
    }

    /// Returns whether the specification deprecated the field at this version.
    #[must_use]
    pub const fn is_deprecated(self) -> bool {
        self.deprecated
    }

    /// Returns whether the specification removed the field at this version.
    ///
    /// A removed entry ends the field's life: the generator writes one when a
    /// version stops naming a field, and a reader never infers it.
    #[must_use]
    pub const fn is_removed(self) -> bool {
        self.removed
    }

    /// Whether this entry states nothing `held` did not already state.
    ///
    /// Every stated fact is compared but the datatype, which the caller has
    /// already resolved and compares itself: two spellings of one type are
    /// equal here, and comparing the raw spellings would call that a change.
    /// The pedigree is deliberately not compared - it dates the statement
    /// rather than being one.
    const fn states_nothing_beyond(self, held: Self) -> bool {
        // `Option<&str>` has no const `PartialEq`, so the two spellings are
        // compared through the same byte equality the rest of this document
        // uses.
        same_text(self.name, held.name)
            && same_text(self.doc, held.doc)
            && self.deprecated == held.deprecated
            && self.removed == held.removed
    }

    /// Renders this entry into the document being written.
    ///
    /// The datatype is written as `dtype` spells it rather than as the entry
    /// carries it, because the resolution the writer already performed is
    /// what gets stored.
    fn write_into(self, writer: &mut Writer, dtype: Option<&DataType>) -> Result<()> {
        writer.open_element();
        writer.text(true, SINCE, &self.since().to_string())?;
        if let Some(ep) = self.ep() {
            writer.number(false, EP, ep);
        }
        if let Some(name) = self.name {
            writer.text(false, NAME, name)?;
        }
        if let Some(dtype) = dtype {
            writer.document(false, TYPE, &dtype.clone().into_json()?);
        }
        if self.deprecated {
            writer.flag(false, DEPRECATED);
        }
        if self.removed {
            writer.flag(false, REMOVED);
        }
        if let Some(doc) = self.doc {
            writer.text(false, DOC, doc)?;
        }
        writer.close_element();
        Ok(())
    }
}

/// Adopts a temporal type backward over the string entries preceding it.
///
/// A field FIX transmitted as text and later declared temporal was always
/// carrying an instant: `20240102-10:15:30` parses the same under FIX.4.2,
/// where the specification called the field a `String`, as under the version
/// that called it a `UTCTimestamp`. Stating the later type at the earlier
/// version is therefore what the crate can consistently parse, and the entry
/// stops claiming a retype that never happened on the wire.
///
/// Only a string yields, and only to a temporal. Every other pair is a real
/// constraint - `String` to `Currency`, `String` to `Boolean`, `int` to
/// `Qty` - where the later type accepts strictly less than the earlier one
/// did, so back-typing it would claim the earlier version refused values it
/// carried. The walk runs newest first so a history with two temporal eras
/// gives each era its own preceding run.
fn back_type(typed: &mut [Option<DataType>]) {
    for at in (1..typed.len()).rev() {
        let Some(later) = typed[at].clone() else {
            continue;
        };
        if !later.id().is_temporal() {
            continue;
        }
        let mut earlier = at;
        while earlier > 0
            && typed[earlier - 1]
                .as_ref()
                .is_some_and(|held| held.id().is_string())
        {
            earlier -= 1;
            typed[earlier] = Some(later.clone());
        }
    }
}

/// Whether two optional borrowed spellings are the same statement.
const fn same_text(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            let (left, right) = (left.as_bytes(), right.as_bytes());
            if left.len() != right.len() {
                return false;
            }
            let mut at = 0;
            while at < left.len() {
                if left[at] != right[at] {
                    return false;
                }
                at += 1;
            }
            true
        }
        _ => false,
    }
}

/// A field's lineage, oldest first.
///
/// Answered by [`FixField::lineage`](crate::FixField). It walks the stored
/// document as it goes and hands back slices of it, so nothing is parsed
/// ahead of the entry being asked for and nothing is allocated. An absent
/// property yields nothing.
#[derive(Clone, Debug)]
pub struct FixLineage<'field> {
    cursor: Cursor<'field>,
    started: bool,
    done: bool,
}

impl<'field> FixLineage<'field> {
    /// Walks one stored `fix:lineage` value, or nothing for an absent one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Renders entries into the one canonical document they have.
    ///
    /// Two derivations are the writer's, so every dialect gets them and no
    /// reader has to repeat them:
    ///
    /// - each entry's datatype is resolved and stored as the crate datatype
    ///   it names, so `char` and `String` become one `utf8`;
    /// - an entry that then states nothing its predecessor did not is
    ///   dropped, because a dated point saying what was already true is not
    ///   a point in a history.
    ///
    /// The oldest entry is never dropped: it is what `since` reads, and a
    /// field with no lineage means "defined at every version", so removing
    /// the last entry would change what the field says rather than shorten
    /// how it says it. `ep` is the entry's date and not one of its
    /// statements, so an extension pack that changed nothing is exactly the
    /// entry worth dropping.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when two entries share a pedigree, because
    /// two statements about one dated point cannot both be the field's, and
    /// the grammar's refusal when an entry names no datatype the crate
    /// resolves.
    pub(super) fn render(entries: &[FixLineageEntry<'_>]) -> Result<String> {
        let mut ordered: Vec<FixLineageEntry<'_>> = entries.to_vec();
        ordered.sort_by_key(|entry| entry.pedigree());
        if let Some(pair) = ordered
            .windows(2)
            .find(|pair| pair[0].pedigree() == pair[1].pedigree())
        {
            let pedigree = pair[0].pedigree();
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: match pedigree.ep() {
                    Some(ep) => format_smolstr!(
                        "expected each pedigree once, got {} at EP{ep} twice",
                        pedigree.version()
                    ),
                    None => format_smolstr!(
                        "expected each pedigree once, got {} twice",
                        pedigree.version()
                    ),
                },
            });
        }
        let mut typed: Vec<Option<DataType>> = Vec::with_capacity(ordered.len());
        for entry in &ordered {
            typed.push(entry.parse_dtype()?);
        }
        back_type(&mut typed);

        let mut writer = Writer::open_array(ENTRIES);
        let mut kept: Option<(FixLineageEntry<'_>, Option<DataType>)> = None;
        for (entry, dtype) in ordered.into_iter().zip(typed) {
            if let Some((held, ref typed)) = kept {
                if entry.states_nothing_beyond(held) && dtype == *typed {
                    continue;
                }
            }
            entry.write_into(&mut writer, dtype.as_ref())?;
            kept = Some((entry, dtype));
        }
        writer.close_array();
        Ok(writer.finish())
    }

    /// Advances one step: the next entry, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixLineageEntry<'field>>> {
        if !self.started {
            self.started = true;
            if !self.cursor.open_array(ENTRIES)? {
                self.cursor.expect(b'}')?;
                return Ok(None);
            }
        } else if !self.cursor.next_element()? {
            self.cursor.expect(b'}')?;
            if !self.cursor.is_done() {
                return Err(Refusal::Trailing);
            }
            return Ok(None);
        }
        self.read_entry().map(Some)
    }

    /// Reads the one entry starting at the cursor.
    fn read_entry(&mut self) -> Scan<FixLineageEntry<'field>> {
        self.cursor.expect(b'{')?;
        let mut since = None;
        let mut ep = None;
        let mut name = None;
        let mut dtype = None;
        let mut doc = None;
        let mut deprecated = false;
        let mut removed = false;
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                SINCE => since = Some(self.cursor.read_version(SINCE)?),
                EP => ep = Some(self.cursor.read_number(EP)?),
                NAME => name = Some(self.cursor.read_word(NAME)?),
                TYPE => dtype = Some(self.cursor.read_document(TYPE)?),
                DEPRECATED => deprecated = self.cursor.read_flag()?,
                REMOVED => removed = self.cursor.read_flag()?,
                _ => doc = Some(self.cursor.read_string()?),
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let Some(version) = since else {
            return Err(Refusal::MissingKey(SINCE));
        };
        Ok(FixLineageEntry {
            pedigree: FixPedigree::new(version, ep),
            name,
            dtype,
            doc,
            deprecated,
            removed,
        })
    }

    /// The next entry, answering nothing where the document does not parse.
    ///
    /// This is what every infallible reader walks, so a malformed document
    /// resolves to no answer rather than to a wrong one - the same
    /// degradation a name-digest collision takes on a registry read - and it
    /// spends no allocation doing it.
    pub(super) fn next_ok(&mut self) -> Option<FixLineageEntry<'field>> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(entry)) => Some(entry),
            Ok(None) | Err(_) => {
                self.done = true;
                None
            }
        }
    }
}

impl<'field> Iterator for FixLineage<'field> {
    type Item = Result<FixLineageEntry<'field>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(refusal) => {
                self.done = true;
                Some(Err(refusal.into_error(
                    TARGET,
                    self.cursor.document(),
                    self.cursor.position(),
                )))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done {
            return (0, Some(0));
        }
        // Every entry costs at least `{"since":"0"}` plus its separator, which
        // bounds how many the remaining bytes can hold.
        (0, Some(self.cursor.document().len() / 14 + 1))
    }
}

impl FusedIterator for FixLineage<'_> {}
