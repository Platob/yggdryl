//! The `fix:` vocabulary, on the field views that carry it.
//!
//! One type reads each property and one type writes it, and both reach the
//! metadata only through the view's own `get`, `insert` and `remove`, so
//! [`Field`](crate::Field)'s cache-aware mutation and metadata validation
//! apply to every write. The property names are private to this module: a
//! caller writes `set_tag(35)`, never `"fix:tag"`.

use std::fmt::Write as _;
use std::iter::FusedIterator;
use std::str::Split;

use smol_str::{SmolStr, format_smolstr};

use super::codes::{FixCode, FixCodeValue, FixCodes};
use super::lineage::{FixLineage, FixLineageEntry};
use super::{FixBranch, FixId};
use crate::types::folds_equal;
use crate::{DataType, Error, FixField, FixFieldMut, Result, Version};

/// The dictionary a field belongs to; absent means the standard one.
const BRANCH: &str = "branch";
/// The full key the branch is stored under, spelled once.
pub(super) const BRANCH_KEY: &str = "fix:branch";
/// The canonical tag.
const TAG: &str = "tag";
/// The full key the canonical tag is stored under.
pub(super) const TAG_KEY: &str = "fix:tag";
/// The alternate tags, comma-separated, highest priority first.
const TAGS: &str = "tags";
/// The alternate names, comma-separated, highest priority first.
const ALIASES: &str = "aliases";
/// The specification's own wording.
/// What a field is for is not FIX's to own.
///
/// A description is a property of the *field*, not of the protocol quoting
/// it: the same sentence is what an Iceberg doc, a SQL column comment and a
/// FIX definition each publish. It is therefore read and written on the
/// generic key every catalog already reads, rather than under `fix:` where
/// only a FIX reader would find it.
/// What this field was called and typed at each version it lived through.
const LINEAGE: &str = "lineage";
/// The FIX code set this field's values are drawn from.
const CODES: &str = "codes";
/// What separates the elements of a list-valued property.
const SEPARATOR: char = ',';

/// What a tag is, spelled once for every refusal.
const TAG_SHAPE: &str = "a FIX tag, a decimal integer from 0 to 2147483647";

/// What a branch is, spelled once for every refusal.
const BRANCH_SHAPE: &str = "a FIX branch, at most 23 ASCII letters, digits, hyphen, dot or underscore, starting with a letter";

/// Parse one tag strictly: decimal digits only, never negative, never signed.
///
/// `i32::from_str` would also accept `+35`, which the writer never emits, so
/// the digits are checked first and the width second. [`FixId::from_str`]
/// parses the tail of an identifier through this same one strict parse.
pub(super) fn parse_tag(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl<'field> FixField<'field> {
    /// Parses the dictionary this field belongs to.
    ///
    /// An absent property is [`FixBranch::STANDARD`]: the FIX
    /// specification's own fields are the common case and state nothing.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `fix:branch` key when the stored
    /// text is not a branch: [`FixFieldMut::set_branch`] never writes
    /// one, so this can only come from externally edited state.
    pub fn branch(&self) -> Result<FixBranch> {
        match self.get(BRANCH) {
            Some(stored) => {
                FixBranch::from_str(stored).map_err(|_| self.invalid(BRANCH, BRANCH_SHAPE, stored))
            }
            None => Ok(FixBranch::STANDARD),
        }
    }

    /// Builds this field's identity, absent exactly when `fix:tag` is.
    ///
    /// Derived from the branch and the canonical tag on every ask; nothing
    /// stores it. Building it through [`FixId::from_parts`] is what refuses a
    /// hand-edited record that claims a specification tag for another
    /// dictionary at the door rather than after it is indexed.
    ///
    /// # Errors
    ///
    /// Returns the failure [`Self::tag`] or [`Self::branch`] raises, or
    /// [`FixId::from_parts`]'s refusal.
    pub fn id(&self) -> Result<Option<FixId>> {
        let Some(tag) = self.tag()? else {
            return Ok(None);
        };
        let branch = self.branch()?;
        FixId::from_parts(&branch, tag).map(Some)
    }

    /// Parses the canonical FIX tag.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `fix:tag` key when the stored text is
    /// not a tag: [`FixFieldMut::set_tag`] never writes one, so this can only
    /// come from externally edited state.
    pub fn tag(&self) -> Result<Option<i32>> {
        self.get(TAG)
            .map(|stored| parse_tag(stored).ok_or_else(|| self.invalid(TAG, TAG_SHAPE, stored)))
            .transpose()
    }

    /// Parses the alternate tags, highest priority first.
    ///
    /// An absent property is an empty list: a field states alternate tags
    /// only when it has them.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `fix:tags` key when the stored text
    /// holds an empty element, a duplicate, or anything that is not a tag.
    pub fn tags(&self) -> Result<Vec<i32>> {
        let Some(stored) = self.get(TAGS) else {
            return Ok(Vec::new());
        };
        let mut tags = Vec::new();
        for element in stored.split(SEPARATOR) {
            if element.is_empty() {
                return Err(self.invalid(TAGS, "no empty element among the tags", stored));
            }
            let tag = parse_tag(element)
                .ok_or_else(|| self.invalid(TAGS, "a comma-separated list of FIX tags", stored))?;
            if tags.contains(&tag) {
                return Err(self.invalid(TAGS, "each tag once", stored));
            }
            tags.push(tag);
        }
        Ok(tags)
    }

    /// Iterates the aliases, highest priority first.
    ///
    /// The iterator is lazy and allocates nothing: every alias is a slice of
    /// the stored text, which the field already owns, so reading them costs
    /// the same whether one is taken or all are. An absent property yields
    /// nothing.
    pub fn aliases(&self) -> FixAliases<'field> {
        FixAliases::over(self.get(ALIASES))
    }

    /// Returns the specification's own wording for this field.
    ///
    /// Read from the generic `description` key rather than from `fix:`,
    /// because what a field is for belongs to the field. See
    /// [`Field::description`](crate::Field::description).
    pub fn description(&self) -> Option<&'field str> {
        self.as_field().description()
    }

    /// Walks what this field was called and typed at each version, oldest
    /// first.
    ///
    /// The iterator is lazy and allocates nothing: every spelling is a slice
    /// of the stored document, which the field already owns. An absent
    /// property yields nothing, which is what a field the dictionary has
    /// never dated answers.
    pub fn lineage(&self) -> FixLineage<'field> {
        FixLineage::over(self.get(LINEAGE))
    }

    /// Returns the version this field was first defined at.
    ///
    /// Derived from the first entry rather than stored beside it, the way
    /// [`FixId`] is derived from a branch and a tag. A field with no lineage,
    /// and a malformed document, both answer `None`: a version filter that
    /// cannot read a history must not act as though the field had none it
    /// disagreed with.
    pub fn since(&self) -> Option<Version> {
        self.lineage().next_ok().map(FixLineageEntry::since)
    }

    /// Returns the version this field was removed at, when one removed it.
    ///
    /// A version that stops naming a field has removed it, and the generator
    /// writes that entry; a reader never infers one.
    pub fn until(&self) -> Option<Version> {
        let mut walk = self.lineage();
        while let Some(entry) = walk.next_ok() {
            if entry.is_removed() {
                return Some(entry.since());
            }
        }
        None
    }

    /// Returns whether this field exists at `at`.
    ///
    /// A field with no lineage is defined at every version: the dictionary
    /// states no history to filter it by, which is how a registry that has
    /// never been dated behaves today.
    pub fn defined_at(&self, at: Version) -> bool {
        let mut dated = false;
        let mut defined = false;
        let mut walk = self.lineage();
        while let Some(entry) = walk.next_ok() {
            // Seeing any entry is what says the field has a history to be
            // filtered by, including one whose every entry postdates `at`:
            // a field introduced in 2.7 did not exist in 2.6.
            dated = true;
            if entry.since() > at {
                break;
            }
            defined = !entry.is_removed();
        }
        !dated || defined
    }

    /// Returns whether the specification had deprecated this field by `at`.
    ///
    /// Deprecation is a state a field enters and does not leave, so the newest
    /// entry at or before `at` is the one that answers - the same walk
    /// [`Self::defined_at`] makes, asked a different question. A field with no
    /// lineage is deprecated at no version, because the dictionary states no
    /// history to say it was.
    pub fn deprecated_at(&self, at: Version) -> bool {
        let mut deprecated = false;
        let mut walk = self.lineage();
        while let Some(entry) = walk.next_ok() {
            if entry.since() > at {
                break;
            }
            deprecated = entry.is_deprecated();
        }
        deprecated
    }

    /// Returns the spelling this field carries at `at`.
    ///
    /// The newest entry at or before `at` that states a name wins, because an
    /// entry stating only a version means "present, unchanged". A field with
    /// no lineage answers `None`, and the caller reads the field's own name.
    pub fn name_at(&self, at: Version) -> Option<&'field str> {
        self.newest_at(at, FixLineageEntry::name)
    }

    /// Returns the datatype this field carries at `at`.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when a stored FIX datatype name
    /// resolves to nothing, and [`Error::Parse`] naming the byte position
    /// when the document is malformed.
    pub fn dtype_at(&self, at: Version) -> Result<Option<DataType>> {
        let mut newest = None;
        for entry in self.lineage() {
            let entry = entry?;
            if entry.since() > at {
                break;
            }
            if let Some(dtype) = entry.dtype() {
                newest = Some(dtype);
            }
        }
        newest.map(DataType::from_str).transpose()
    }

    /// The newest value at or before `at` that an entry states.
    fn newest_at<T>(
        &self,
        at: Version,
        read: impl Fn(FixLineageEntry<'field>) -> Option<T>,
    ) -> Option<T> {
        let mut newest = None;
        let mut walk = self.lineage();
        while let Some(entry) = walk.next_ok() {
            if entry.since() > at {
                break;
            }
            if let Some(value) = read(entry) {
                newest = Some(value);
            }
        }
        newest
    }

    /// Walks this field's FIX code set, ordered by wire value.
    ///
    /// The iterator is lazy and allocates nothing: every spelling is a slice
    /// of the stored document, which the field already owns. An absent
    /// property yields nothing.
    pub fn codes(&self) -> FixCodes<'field> {
        FixCodes::over(self.get(CODES))
    }

    /// Returns the code one wire value stands for.
    ///
    /// The scan stops at the match: `value` leads each record, so this reads
    /// one key per code passed and no more.
    pub fn code(&self, value: &str) -> Option<FixCodeValue<'field>> {
        FixCodes::seek_value(self.get(CODES)?, value)
    }

    /// Returns the code one symbolic name or alias stands for, folded.
    ///
    /// This does **not** stop at the first match. Two codes folding to one
    /// spelling answer nothing rather than whichever the scan met first, so
    /// the whole set runs and exactly one match answers. It is affordable
    /// because [`Self::code`] is the hot path and a spelling lookup comes
    /// from human or JSON input.
    pub fn code_by_name(&self, name: &str) -> Option<FixCodeValue<'field>> {
        self.one_matching(|code| code.is_spelled(name))
    }

    /// Returns the code one wire value stands for at `at`.
    ///
    /// The version is a preference here too: a value the message actually
    /// carries is named whether or not the version it claims had heard of it,
    /// because a value in the data is a fact and a version in the frame is an
    /// assertion.
    pub fn code_at(&self, at: Version, value: &str) -> Option<FixCodeValue<'field>> {
        let _ = at;
        self.code(value)
    }

    /// Resolves any spelling of a code to its wire value.
    ///
    /// Composes the three tiers this module documents. An unresolved spelling
    /// answers `None` and the caller keeps its own text: a venue sends codes
    /// no dictionary lists, and refusing one would drop data.
    pub fn code_value(&self, text: &str) -> Option<&'field str> {
        self.resolve_value(text, None)
    }

    /// Returns the symbolic name one wire value stands for.
    pub fn code_name(&self, value: &str) -> Option<&'field str> {
        self.code(value).map(FixCodeValue::name)
    }

    /// Resolves any spelling of a code to its wire value, at one version.
    ///
    /// A code added after `at`, and one deprecated at or before it, are both
    /// invisible: a 4.2 message cannot resolve a name added in 4.4.
    pub fn code_value_at(&self, at: Version, text: &str) -> Option<&'field str> {
        self.resolve_value(text, Some(at))
    }

    /// Returns the symbolic name one wire value stands for, at one version.
    pub fn code_name_at(&self, at: Version, value: &str) -> Option<&'field str> {
        self.code_at(at, value).map(FixCodeValue::name)
    }

    /// The three tiers, preferring what the version knows.
    ///
    /// The version is a *preference*, not a gate. A capture whose frame says
    /// 4.2 routinely carries values the specification added in 4.4 - a venue
    /// upgrades one side, a bridge relabels a session, a configuration is
    /// copied from another desk - and a reader that refused them would drop
    /// exactly the traffic someone is trying to explain. So a code the
    /// version knows wins, and a code it does not is still read rather than
    /// discarded.
    ///
    /// The preference is what keeps it honest: where two spellings differ
    /// only by version, the one the message's own version declares answers,
    /// so a dated read is still a dated read.
    fn resolve_value(&self, text: &str, at: Option<Version>) -> Option<&'field str> {
        let stored = self.get(CODES)?;
        if at.is_some() {
            if let Some(held) = resolve_in(stored, text, at) {
                return Some(held);
            }
        }
        resolve_in(stored, text, None)
    }

    /// The one code a predicate matches, or nothing when several do.
    fn one_matching(
        &self,
        matches: impl Fn(&FixCodeValue<'field>) -> bool,
    ) -> Option<FixCodeValue<'field>> {
        one_matching(self.get(CODES)?, matches)
    }

    /// Name the full key a stored value failed under, and what it should be.
    fn invalid(&self, name: &str, expected: &str, actual: &str) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason: format_smolstr!("expected {expected}, got {actual:?}"),
        }
    }
}

impl FixFieldMut<'_> {
    /// Records the dictionary this field belongs to.
    ///
    /// [`FixBranch::STANDARD`] removes the property rather than writing an
    /// empty name, exactly as an empty tag or alias list removes its own, so
    /// one declaration has one stored form. The canonical tag and every
    /// alternate tag are held to the standard-tag rule against the new
    /// branch before anything is written.
    ///
    /// # Errors
    ///
    /// Returns [`FixId::from_parts`]'s refusal when a tag this field holds is
    /// one the FIX specification assigns, the parse failure when a stored
    /// `fix:` property is malformed, or the property write's refusal. Any of
    /// them leaves the field unchanged.
    pub fn set_branch(&mut self, branch: &FixBranch) -> Result<()> {
        let view = self.as_protocol();
        if let Some(tag) = view.tag()? {
            FixId::from_parts(branch, tag)?;
        }
        for tag in view.tags()? {
            FixId::from_parts(branch, tag)?;
        }
        self.put_branch(branch)?;
        Ok(())
    }

    /// Records both halves of an identity at once.
    ///
    /// Moving a field between branches one property at a time works in only
    /// one order and refuses the other, because each setter holds the field to
    /// the standard-tag rule as it stands. This writes the branch, then the
    /// tag, and restores the prior branch entry if the tag write fails, so
    /// either move succeeds and a failure leaves the field unchanged. A
    /// The two native parts pass through [`FixId::from_parts`] before either
    /// property changes.
    ///
    /// # Errors
    ///
    /// Returns the tag write's refusal, having restored the branch.
    pub fn set_id(&mut self, branch: &FixBranch, tag: i32) -> Result<()> {
        FixId::from_parts(branch, tag)?;
        let prior = self.put_branch(branch)?;
        match self.set_tag(tag) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.restore(BRANCH, prior);
                Err(error)
            }
        }
    }

    /// Records the canonical FIX tag.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag is negative, when this field's branch
    /// may not claim it - a non-standard branch is limited to
    /// [`FixId::USER_TAG_MIN`] through [`FixId::USER_TAG_MAX`] (exclusive) -
    /// or when the property write fails the validation every metadata write
    /// goes through. Any of them leaves the field unchanged.
    pub fn set_tag(&mut self, tag: i32) -> Result<()> {
        if tag < 0 {
            return Err(self.rejected(TAG, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
        }
        let branch = self.as_protocol().branch()?;
        FixId::from_parts(&branch, tag)?;
        self.store(TAG, tag.to_string())
    }

    /// Records the alternate tags in the given order, highest priority first.
    ///
    /// An empty slice removes the property.
    ///
    /// # Errors
    ///
    /// Returns an error when a tag is negative, repeated, or one this field's
    /// branch may not claim, leaving the field unchanged. An alternate tag
    /// resolves exactly as a canonical one does, so it is held to the same
    /// standard-tag rule.
    pub fn set_tags(&mut self, tags: &[i32]) -> Result<()> {
        if tags.is_empty() {
            self.remove(TAGS);
            return Ok(());
        }
        let branch = self.as_protocol().branch()?;
        let mut rendered = String::new();
        for (index, tag) in tags.iter().enumerate() {
            if *tag < 0 {
                return Err(self.rejected(TAGS, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
            }
            FixId::from_parts(&branch, *tag)?;
            if tags[..index].contains(tag) {
                return Err(self.rejected(
                    TAGS,
                    format_smolstr!("expected each tag once, got {tag} twice"),
                ));
            }
            if index > 0 {
                rendered.push(SEPARATOR);
            }
            // Writing into a `String` cannot fail.
            let _ = write!(rendered, "{tag}");
        }
        self.store(TAGS, rendered)
    }

    /// Records the aliases in the given order, highest priority first.
    ///
    /// Empty input removes the property.
    ///
    /// # Errors
    ///
    /// Returns an error when an alias is empty, contains the separator, or
    /// repeats an earlier one with ASCII case folded, leaving the field
    /// unchanged.
    pub fn set_aliases<I, S>(&mut self, aliases: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut rendered = String::new();
        let mut count = 0;
        for alias in aliases {
            let alias = alias.as_ref();
            if alias.is_empty() {
                return Err(self.rejected(ALIASES, "expected a non-empty alias, got \"\"".into()));
            }
            if alias.contains(SEPARATOR) {
                return Err(self.rejected(
                    ALIASES,
                    format_smolstr!("expected an alias without {SEPARATOR:?}, got {alias:?}"),
                ));
            }
            if rendered
                .split(SEPARATOR)
                .any(|held| held.eq_ignore_ascii_case(alias))
            {
                return Err(self.rejected(
                    ALIASES,
                    format_smolstr!("expected each alias once, got {alias:?} twice"),
                ));
            }
            if count > 0 {
                rendered.push(SEPARATOR);
            }
            rendered.push_str(alias);
            count += 1;
        }
        if count == 0 {
            self.remove(ALIASES);
            return Ok(());
        }
        self.store(ALIASES, rendered)
    }

    /// Records the specification's own wording for this field.
    ///
    /// # Errors
    ///
    /// Returns an error when the property write fails the validation every
    /// metadata write goes through, leaving the field unchanged.
    pub fn set_description(&mut self, value: impl Into<String>) -> Result<()> {
        self.as_field_mut().set_description(value)
    }

    /// Records what this field was called and typed at each version.
    ///
    /// Entries are sorted oldest first and rendered canonically, so one
    /// history has one stored text however it was built. Two derivations are
    /// the writer's rather than a caller's, which is what keeps them from
    /// drifting:
    ///
    /// - the newest entry must agree with the field's own name and datatype,
    ///   so the lineage is the authority and the field cannot contradict it;
    /// - `fix:aliases` is rewritten from the historical spellings, so a query
    ///   by an old name resolves through the index that already exists.
    ///
    /// An empty slice removes both the lineage and the aliases it derived.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when two entries share a pedigree, and a
    /// typed conflict naming both sides when the newest entry disagrees with
    /// the field's own name or datatype. Either leaves the field unchanged.
    pub fn set_lineage(&mut self, entries: &[FixLineageEntry<'_>]) -> Result<()> {
        if entries.is_empty() {
            let prior = self.remove(LINEAGE);
            if let Err(error) = self.set_aliases::<[&str; 0], &str>([]) {
                self.restore(LINEAGE, prior);
                return Err(error);
            }
            return Ok(());
        }
        let rendered = FixLineage::render(entries)?;
        let newest = entries
            .iter()
            .max_by_key(|entry| entry.pedigree())
            .copied()
            .ok_or_else(|| self.rejected(LINEAGE, "expected at least one entry".into()))?;
        let field = self.as_field();
        if let Some(name) = newest.name() {
            if name != field.name() {
                return Err(self.disagreement("name", name, field.name()));
            }
        }
        if let Some(dtype) = newest.parse_dtype()? {
            if dtype != *field.dtype() {
                return Err(self.disagreement(
                    "datatype",
                    &dtype.to_string(),
                    &field.dtype().to_string(),
                ));
            }
        }
        // The aliases the lineage implies are derived before either is
        // written, so one refusal cannot leave half a declaration behind.
        let aliases = derived_aliases(field.name(), &rendered);
        let prior = self.insert(LINEAGE, rendered)?;
        match self.set_aliases(&aliases) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.restore(LINEAGE, prior);
                Err(error)
            }
        }
    }

    /// Records the FIX code set this field's values are drawn from.
    ///
    /// Codes are ordered by wire value and rendered canonically, so one code
    /// set is one text however it was built. Two names may share a value -
    /// that is an alias - but two codes may not share a name.
    ///
    /// An empty slice removes the property, exactly as an empty tag or alias
    /// list removes its own.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when two codes share a name or one states an
    /// empty value or name, and the property write's refusal otherwise.
    /// Either leaves the field unchanged.
    pub fn set_codes(&mut self, codes: &[FixCode]) -> Result<()> {
        if codes.is_empty() {
            self.remove(CODES);
            return Ok(());
        }
        let rendered = FixCodes::render(codes)?;
        self.store(CODES, rendered)
    }

    /// Removes the FIX code set, answering what it held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the stored
    /// document does not parse, having already removed it: a document a
    /// reader refuses is one a caller asked to take away.
    pub fn remove_codes(&mut self) -> Result<Option<Vec<FixCode>>> {
        let Some(stored) = self.remove(CODES) else {
            return Ok(None);
        };
        FixCodes::over(Some(stored.as_str()))
            .map(|code| code.map(FixCode::from))
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// Folds another definition of the same field into this one.
    ///
    /// This field is the incoming definition and wins every shared key; the
    /// other keeps only what it alone declares. Several sources describe one
    /// tag - FIX Latest, a QuickFIX dictionary, a vendor orchestration, a
    /// `.cfb` - and folding them is one pass with a rule per key, because
    /// "merge" alone decides nothing:
    ///
    /// | key | rule |
    /// | --- | --- |
    /// | `fix:branch`, `fix:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
    /// | `fix:tags` | union, incoming first, order kept, deduplicated |
    /// | `fix:aliases` | union, folded, incoming first, then rewritten from the merged lineage |
    /// | `description` | not folded here at all: it is a generic key, so the metadata merge every protocol shares carries it |
    /// | `fix:lineage` | merged by pedigree, incoming winning an equal pair, re-sorted oldest first |
    /// | `fix:codes` | merged by wire value, incoming winning a shared value |
    /// | any other `fix:` key | incoming wins; stored keeps what only it has |
    ///
    /// Precedence is the caller's ordering rather than a field on the merge:
    /// a generator merges its lowest-priority source first, so the highest
    /// wins by being the last one folded in. One concept, in the one place
    /// that knows about sources.
    ///
    /// The description is deliberately absent from that table. It is a
    /// property of the field rather than of FIX, so it folds through the
    /// generic metadata merge with every other field-owned key - which is
    /// also why a dictionary and an Iceberg catalog now disagree about a
    /// field's meaning in exactly zero places.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict naming both sides when the branch or the tag
    /// disagrees, and the write's refusal otherwise. Either leaves the field
    /// exactly as it was.
    pub fn merge_with(&mut self, other: &FixField<'_>) -> Result<()> {
        let held = self.as_protocol();
        // Identity is checked before anything is built, so a refusal costs
        // neither a render nor a write.
        let (branch, incoming_branch) = (held.branch()?, other.branch()?);
        if !branch.has_identity(&incoming_branch) {
            return Err(Error::conflict(
                "fix field",
                "fix field",
                format_smolstr!(
                    "branch {:?} merged with {:?}",
                    branch.name(),
                    incoming_branch.name()
                ),
            ));
        }
        if held.tag()? != other.tag()? {
            return Err(Error::conflict(
                "fix field",
                "fix field",
                format_smolstr!("tag {:?} merged with {:?}", held.tag()?, other.tag()?),
            ));
        }

        // One pass over the `fix:` key set, which is a const listing beside
        // these accessors, so no held key name is ever collected into a
        // `String` to be walked.
        let mut tags = held.tags()?;
        for tag in other.tags()? {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        let mut aliases: Vec<&str> = held.aliases().collect();
        for alias in other.aliases() {
            if !aliases.iter().any(|kept| kept.eq_ignore_ascii_case(alias)) {
                aliases.push(alias);
            }
        }
        let lineage = merge_lineage(&held, other)?;
        let codes = merge_codes(&held, other)?;
        // The aliases the merged lineage implies replace the union, because
        // that derivation is the writer's and must not drift.
        let derived = lineage
            .as_deref()
            .map(|lineage| derived_aliases(held.as_field().name(), lineage));
        if let Some(derived) = &derived {
            aliases = derived.iter().map(SmolStr::as_str).collect();
        }

        let mut merged: Vec<(&'static str, String)> = Vec::with_capacity(MERGED_KEYS.len());
        for key in MERGED_KEYS {
            let value = match key {
                TAGS => render_tags(&tags),
                ALIASES => render_aliases(&aliases),
                LINEAGE => lineage.clone(),
                CODES => codes.clone(),
                // Every other key is "incoming wins, stored keeps what only
                // it has".
                _ => held.get(key).or_else(|| other.get(key)).map(str::to_owned),
            };
            if let Some(value) = value {
                merged.push((key, value));
            }
        }
        // A `fix:` key this vocabulary does not name is still one side's
        // statement, so it travels rather than being dropped by the replace.
        let mut extra: Vec<(String, String)> = Vec::new();
        for (name, value) in held.iter().chain(other.iter()) {
            if MERGED_KEYS.contains(&name) || extra.iter().any(|(kept, _)| kept == name) {
                continue;
            }
            extra.push((name.to_owned(), value.to_owned()));
        }
        // Every borrow of this field ends here, so the one write below is the
        // only thing holding it.
        drop(held);

        // One write. `set` replaces this protocol's properties and validates
        // the whole replacement first, so three rewrites and their Arrow
        // invalidations collapse into one and a refusal changes nothing.
        self.set(
            merged
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .chain(
                    extra
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str())),
                ),
        )
    }

    /// Name both sides of a lineage that contradicts the field carrying it.
    fn disagreement(&self, what: &str, stated: &str, held: &str) -> Error {
        Error::conflict(
            "fix lineage",
            "fix field",
            format_smolstr!(
                "the newest lineage entry states {what} {stated:?}, the field holds {held:?}"
            ),
        )
    }

    /// Put back what an insert or a remove answered.
    ///
    /// The value was read out of this very field, so re-inserting it cannot
    /// fail validation; a failure here would be reported instead of the one
    /// being unwound, which is why the result is dropped.
    fn restore(&mut self, name: &str, prior: Option<String>) {
        match prior {
            Some(value) => {
                let _ = self.insert(name, value);
            }
            None => {
                self.remove(name);
            }
        }
    }

    /// Write one property, dropping the prior value a generic insert answers.
    fn store(&mut self, name: &str, value: impl Into<String>) -> Result<()> {
        self.insert(name, value)?;
        Ok(())
    }

    /// Put the branch entry in the one form that declaration has, and
    /// answer what stood there before.
    fn put_branch(&mut self, branch: &FixBranch) -> Result<Option<String>> {
        if branch.is_standard() {
            Ok(self.remove(BRANCH))
        } else {
            self.insert(BRANCH, branch.name())
        }
    }

    /// Name the full key a value was refused under.
    fn rejected(&self, name: &str, reason: SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason,
        }
    }
}

/// The aliases a field declares, in stored priority order.
///
/// Answered by [`FixField::aliases`]. It walks the stored comma-separated
/// text as it goes and hands back slices of it, so nothing is parsed ahead
/// of the alias being asked for and nothing is allocated. An empty element,
/// which the writer never produces, is skipped rather than reported: the
/// typed rejection belongs to the write, and a read stays cheap.
#[derive(Clone, Debug)]
pub struct FixAliases<'field> {
    parts: Option<Split<'field, char>>,
}

impl<'field> FixAliases<'field> {
    /// Walk one stored `fix:aliases` value, or nothing for an absent one.
    fn over(stored: Option<&'field str>) -> Self {
        Self {
            parts: stored.map(|stored| stored.split(SEPARATOR)),
        }
    }
}

impl<'field> Iterator for FixAliases<'field> {
    type Item = &'field str;

    fn next(&mut self) -> Option<Self::Item> {
        self.parts.as_mut()?.find(|alias| !alias.is_empty())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.parts {
            Some(parts) => (0, parts.size_hint().1),
            None => (0, Some(0)),
        }
    }
}

impl DoubleEndedIterator for FixAliases<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.parts.as_mut()?.rfind(|alias| !alias.is_empty())
    }
}

impl FusedIterator for FixAliases<'_> {}

/// The `fix:` keys a merge folds, as a `const` listing.
///
/// A merge walks this rather than collecting the keys a field holds, because
/// the held names are owned `String`s behind a generic snapshot and building
/// a vector of them to scan `O(n*m)` is what this replaced.
const MERGED_KEYS: [&str; 6] = [BRANCH, TAG, TAGS, ALIASES, LINEAGE, CODES];

/// Render aliases the way the setter renders them.
fn render_aliases(aliases: &[&str]) -> Option<String> {
    if aliases.is_empty() {
        return None;
    }
    Some(aliases.join(","))
}

/// Render alternate tags the way the setter renders them.
fn render_tags(tags: &[i32]) -> Option<String> {
    if tags.is_empty() {
        return None;
    }
    let mut rendered = String::new();
    for (index, tag) in tags.iter().enumerate() {
        if index > 0 {
            rendered.push(SEPARATOR);
        }
        // Writing into a `String` cannot fail.
        let _ = write!(rendered, "{tag}");
    }
    Some(rendered)
}

/// Fold two lineages by pedigree, the incoming winning an equal pair.
///
/// The merged document is re-rendered once, oldest first, so the result is
/// the same text whichever order the two arrived in.
fn merge_lineage(winner: &FixField<'_>, other: &FixField<'_>) -> Result<Option<String>> {
    let mut entries: Vec<FixLineageEntry<'_>> = Vec::new();
    for entry in winner.lineage() {
        entries.push(entry?);
    }
    for entry in other.lineage() {
        let entry = entry?;
        if !entries
            .iter()
            .any(|held| held.pedigree() == entry.pedigree())
        {
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    FixLineage::render(&entries).map(Some)
}

/// Fold two code sets by wire value, the incoming winning a shared value.
fn merge_codes(winner: &FixField<'_>, other: &FixField<'_>) -> Result<Option<String>> {
    let mut codes: Vec<FixCode> = Vec::new();
    for code in winner.codes() {
        codes.push(FixCode::from(code?));
    }
    for code in other.codes() {
        let code = code?;
        if !codes.iter().any(|held| held.value() == code.value()) {
            codes.push(FixCode::from(code));
        }
    }
    if codes.is_empty() {
        return Ok(None);
    }
    FixCodes::render(&codes).map(Some)
}

/// The aliases one lineage implies: its historical spellings, in order.
///
/// The field's own name is not an alias of itself, and a spelling already
/// held is written once, so the derivation is stable under repetition. A
/// document that does not parse implies nothing, which is the same answer
/// every other lineage read gives it.
fn derived_aliases(canonical: &str, lineage: &str) -> Vec<SmolStr> {
    let mut aliases: Vec<SmolStr> = Vec::new();
    let mut walk = FixLineage::over(Some(lineage));
    while let Some(entry) = walk.next_ok() {
        let Some(name) = entry.name() else {
            continue;
        };
        if name.eq_ignore_ascii_case(canonical)
            || aliases.iter().any(|held| held.eq_ignore_ascii_case(name))
        {
            continue;
        }
        aliases.push(SmolStr::new(name));
    }
    aliases
}

/// The one code in `stored` a predicate matches, or nothing when several do.
///
/// Ambiguity answers nothing: two codes a caller's spelling reaches are two
/// answers, and picking one is a guess. Free rather than a method so the
/// tiers can share one already-read document.
fn one_matching<'field>(
    stored: &'field str,
    matches: impl Fn(&FixCodeValue<'field>) -> bool,
) -> Option<FixCodeValue<'field>> {
    let mut found = None;
    let mut walk = FixCodes::over(Some(stored));
    while let Some(code) = walk.next_ok() {
        if !matches(&code) {
            continue;
        }
        if found.is_some_and(|held: FixCodeValue<'field>| held.value() != code.value()) {
            return None;
        }
        found = Some(code);
    }
    found
}

/// The three tiers over one already-read document, at one visibility.
fn resolve_in<'field>(stored: &'field str, text: &str, at: Option<Version>) -> Option<&'field str> {
    let visible = |code: &FixCodeValue<'field>| at.is_none_or(|at| code.defined_at(at));
    // Tier 1: the text as a wire value, exactly. A spelling that is already a
    // legal code is never reinterpreted as somebody's name, and the record a
    // value opens is addressed rather than searched for.
    if let Some(code) = FixCodes::seek_value(stored, text) {
        if visible(&code) {
            return Some(code.value());
        }
    }
    // Tier 2: the folded symbolic name, then any alias.
    if let Some(code) = one_matching(stored, |code| visible(code) && code.is_spelled(text)) {
        return Some(code.value());
    }
    // Tier 3: the leading parenthesized abbreviation of the description.
    one_matching(stored, |code| {
        visible(code)
            && code
                .abbreviation()
                .is_some_and(|short| folds_equal(short, text))
    })
    .map(FixCodeValue::value)
}
