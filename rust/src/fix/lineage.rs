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
/// The FIX datatype name the field carries from that version on.
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

    /// Sets the FIX datatype name the field carries from this version on.
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

    /// Returns the FIX datatype name from this version on, unparsed.
    ///
    /// The spelling is the one the schema grammar already resolves - `Qty`,
    /// `int`, `UTCTimestamp` - so a reader needs no second table and the
    /// stored document stays readable. [`Self::parse_dtype`] resolves it.
    #[must_use]
    pub const fn dtype(self) -> Option<&'field str> {
        self.dtype
    }

    /// Resolves the FIX datatype name this entry states.
    ///
    /// # Errors
    ///
    /// Returns the grammar's own refusal when the stored spelling names no
    /// datatype.
    pub fn parse_dtype(self) -> Result<Option<DataType>> {
        self.dtype.map(DataType::from_str).transpose()
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

    /// Renders this entry into the document being written.
    fn write_into(self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, SINCE, &self.since().to_string())?;
        if let Some(ep) = self.ep() {
            writer.number(false, EP, ep);
        }
        if let Some(name) = self.name {
            writer.text(false, NAME, name)?;
        }
        if let Some(dtype) = self.dtype {
            writer.text(false, TYPE, dtype)?;
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
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when two entries share a pedigree, because
    /// two statements about one dated point cannot both be the field's.
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
        let mut writer = Writer::open_array(ENTRIES);
        for entry in ordered {
            entry.write_into(&mut writer)?;
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
                TYPE => dtype = Some(self.cursor.read_word(TYPE)?),
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
