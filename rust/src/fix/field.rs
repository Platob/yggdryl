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

use super::FixId;
use super::codes::{FixCode, FixCodeValue, FixCodes};
use super::lineage::{FixLineage, FixLineageEntry};
use super::replacements::{FixReplacement, FixReplacements};
use crate::types::folds_equal;
use crate::{DataType, Error, FixField, FixFieldMut, Result, Version};

/// The dictionaries that contributed this field, folded and sorted; absent
/// for a field the specification alone defines.
const BRANCHES: &str = "branches";
/// The canonical tag.
const TAG: &str = "tag";
/// The full key the canonical tag is stored under.
pub(super) const TAG_KEY: &str = "fix:tag";
/// The alternate tags, comma-separated, highest priority first.
const TAGS: &str = "tags";
/// The alternate names, comma-separated, highest priority first.
const ALIASES: &str = "aliases";
/// The spellings that mean "nothing was sent" for this field.
const NULLS: &str = "nulls";
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
/// How a value of this field is restated at a later version.
const REPLACEMENTS: &str = "replacements";
const COUNTER: &str = "counter";
const COMPONENT: &str = "component";
const FIELD_REF: &str = "field";
const GROUP: &str = "group";
const MSGTYPE: &str = "msgtype";
/// What separates the elements of a list-valued property.
const SEPARATOR: char = ',';

/// What a tag is, spelled once for every refusal.
const TAG_SHAPE: &str = "a FIX tag, a decimal integer from 0 to 2147483647";

/// Parse one tag strictly: decimal digits only, never negative, never signed.
///
/// `i32::from_str` would also accept `+35`, which the writer never emits, so
/// the digits are checked first and the width second.
pub(super) fn parse_tag(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl<'field> FixField<'field> {
    /// The component referenced by a catalog occurrence.
    pub fn component(&self) -> Option<&'field str> {
        self.get(COMPONENT)
    }

    /// The scalar field referenced by a catalog occurrence.
    pub fn field_ref(&self) -> Option<&'field str> {
        self.get(FIELD_REF)
    }

    /// The repeating group referenced by a catalog occurrence.
    pub fn group(&self) -> Option<&'field str> {
        self.get(GROUP)
    }

    /// The wire message type declared by a message definition.
    pub fn msgtype(&self) -> Option<&'field str> {
        self.get(MSGTYPE)
    }

    /// The tag of a group's separate integer count field.
    pub fn counter(&self) -> Result<Option<i32>> {
        self.get(COUNTER)
            .map(|stored| parse_tag(stored).ok_or_else(|| self.invalid(COUNTER, TAG_SHAPE, stored)))
            .transpose()
    }

    /// Iterates the dictionaries that contributed this field, folded and
    /// sorted.
    ///
    /// Membership is provenance: a dialect merged into a registry names
    /// itself on every field it touched, and a caller filters on it. It is
    /// never consulted to resolve a tag or a name. The iterator is lazy and
    /// allocates nothing, and an absent property - every field the
    /// specification alone defines - yields nothing.
    pub fn branches(&self) -> FixSpellings<'field> {
        FixSpellings::over(self.get(BRANCHES))
    }

    /// Whether `dialect` is one of the dictionaries that contributed this
    /// field, under the crate's one fold - the fold the list is deduplicated
    /// by, so a spelling that would have folded into a listed name is a
    /// member.
    pub fn has_branch(&self, dialect: &str) -> bool {
        self.branches().any(|held| folds_equal(held, dialect))
    }

    /// Builds this field's identity, absent exactly when `fix:tag` is.
    ///
    /// Derived from the canonical tag and the field's own name on every ask;
    /// nothing stores it, so a rename is never stale.
    ///
    /// # Errors
    ///
    /// Returns the failure [`Self::tag`] raises, or [`FixId::of`]'s refusal.
    pub fn id(&self) -> Result<Option<FixId>> {
        let Some(tag) = self.tag()? else {
            return Ok(None);
        };
        FixId::of(tag, self.as_field().name()).map(Some)
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
    pub fn aliases(&self) -> FixSpellings<'field> {
        FixSpellings::over(self.get(ALIASES))
    }

    /// Iterates the spellings that mean "nothing was sent" for this field.
    ///
    /// A venue writes an absence in its own vocabulary - `N/A` on a price,
    /// `0` on an identifier, `NONE` on a party - and which spelling means it
    /// is a fact about the field rather than about the capture. A value the
    /// list names types as null in the row while the entry keeps it exactly
    /// as it arrived, because the row is the interpretation and the entries
    /// are what the wire carried.
    ///
    /// This is the narrow half of the pair.
    /// [`FixCodec::with_null_values`](crate::FixCodec::with_null_values) is
    /// the capture's own convention and is applied to every key before one is
    /// resolved at all; this is applied once the field is known. The iterator
    /// is lazy and allocates nothing, and an absent property yields nothing.
    pub fn nulls(&self) -> FixSpellings<'field> {
        FixSpellings::over(self.get(NULLS))
    }

    /// Whether `value` is a spelling this field states as an absence.
    ///
    /// Compared ASCII case-insensitively against the trimmed text, exactly as
    /// the capture-wide list compares: a venue writing `n/a` and `N/A` in one
    /// file means the same absence twice.
    pub fn is_null_value(&self, value: &str) -> bool {
        spells_absence(self.nulls(), value)
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
    /// [`FixId`] is derived from a tag and a name. A field with no lineage,
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
            if entry.dtype().is_some() {
                newest = Some(entry);
            }
        }
        newest
            .map(FixLineageEntry::parse_dtype)
            .transpose()
            .map(Option::flatten)
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

    /// Walks how a value of this field is restated at a later version, in
    /// document order.
    ///
    /// The first entry whose conditions a held value meets is the one that
    /// answers, which is why the order is the document's and not sorted. The
    /// iterator is lazy and allocates nothing: every spelling is a slice of
    /// the stored document, which the field already owns. An absent property
    /// yields nothing, which is what a field the specification never
    /// replaced answers.
    ///
    /// ```
    /// use yggdryl::fix::{FixFill, FixFillSource, FixReplacement};
    /// use yggdryl::{DataType, Version};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut max_floor = DataType::Float64.nullable_field("maxfloor");
    /// max_floor.as_fix_mut().set_tag(111)?;
    /// // MaxFloor(111) was replaced by DisplayQty(1138), which takes its value.
    /// max_floor.as_fix_mut().set_replacements(&[
    ///     FixReplacement::new("5.0".parse::<Version>()?)
    ///         .with_fills([FixFill::Field { tag: 1138, value: FixFillSource::Source }]),
    /// ])?;
    ///
    /// let entry = max_floor.as_fix().replacements().next().expect("one rule")?;
    /// assert_eq!(entry.since(), "5.0".parse::<Version>()?);
    /// assert_eq!(entry.when(), None, "any stated value");
    /// let fill = entry.fills().next().expect("one fill")?;
    /// assert!(matches!(fill, yggdryl::fix::FixFillEntry::Field { tag: 1138, .. }));
    /// # Ok(())
    /// # }
    /// ```
    pub fn replacements(&self) -> FixReplacements<'field> {
        FixReplacements::over(self.get(REPLACEMENTS))
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
        translate(self.codes_document()?, text, at)
    }

    /// The stored code-set document, when this field carries one.
    ///
    /// What every code read scans; a reader remembering translations across
    /// a run keys them by this document, because the answer is a fact of the
    /// document, the version and the text alone.
    pub(super) fn codes_document(&self) -> Option<&'field str> {
        self.get(CODES)
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
    /// References one component by its catalog name.
    pub fn set_component(&mut self, name: &str) -> Result<()> {
        self.set_reference(COMPONENT, name)
    }

    /// References one scalar field by its catalog name.
    pub fn set_field_ref(&mut self, name: &str) -> Result<()> {
        self.set_reference(FIELD_REF, name)
    }

    /// References one repeating group by its catalog name.
    pub fn set_group(&mut self, name: &str) -> Result<()> {
        self.set_reference(GROUP, name)
    }

    /// Declares the exact wire value of a message type.
    pub fn set_msgtype(&mut self, value: &str) -> Result<()> {
        super::msgtype::validate_code(value)?;
        self.store(MSGTYPE, value.to_owned())
    }

    /// Declares the tag of the group's separate integer count field.
    pub fn set_counter(&mut self, tag: i32) -> Result<()> {
        if tag < 0 {
            return Err(self.rejected(COUNTER, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
        }
        self.store(COUNTER, tag.to_string())
    }

    /// Removes the component reference.
    pub fn remove_component(&mut self) -> Option<String> {
        self.remove(COMPONENT)
    }

    /// Removes the scalar field reference.
    pub fn remove_field_ref(&mut self) -> Option<String> {
        self.remove(FIELD_REF)
    }

    /// Removes the repeating group reference.
    pub fn remove_group(&mut self) -> Option<String> {
        self.remove(GROUP)
    }

    /// Removes the declared message type.
    pub fn remove_msgtype(&mut self) -> Option<String> {
        self.remove(MSGTYPE)
    }

    /// Removes the counter reference after validating it.
    pub fn remove_counter(&mut self) -> Result<Option<i32>> {
        let tag = self.as_protocol().counter()?;
        self.remove(COUNTER);
        Ok(tag)
    }

    fn set_reference(&mut self, key: &str, name: &str) -> Result<()> {
        if name.is_empty()
            || matches!(name, "." | "..")
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(self.rejected(key, format_smolstr!("expected a nonempty catalog name of ASCII letters, digits, underscore, hyphen or dot, got {name:?}")));
        }
        self.store(key, name.to_ascii_lowercase())
    }

    /// Records the dictionaries that contributed this field.
    ///
    /// Each name is held to the alias grammar - non-empty, no separator -
    /// folded by ASCII case once, deduplicated under the crate fold, and the
    /// list is stored sorted, so two registries built from the same
    /// dictionaries in any order hash alike. Empty input removes the
    /// property, so a field the specification alone defines states nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when a name is empty or contains the separator,
    /// leaving the field unchanged.
    pub fn set_branches<I, S>(&mut self, dialects: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut held: Vec<String> = Vec::new();
        for dialect in dialects {
            let dialect = dialect.as_ref();
            if dialect.is_empty() {
                return Err(
                    self.rejected(BRANCHES, "expected a non-empty dialect, got \"\"".into())
                );
            }
            if dialect.contains(SEPARATOR) {
                return Err(self.rejected(
                    BRANCHES,
                    format_smolstr!("expected a dialect without {SEPARATOR:?}, got {dialect:?}"),
                ));
            }
            let folded = dialect.to_ascii_lowercase();
            if !held.iter().any(|known| folds_equal(known, &folded)) {
                held.push(folded);
            }
        }
        if held.is_empty() {
            self.remove(BRANCHES);
            return Ok(());
        }
        held.sort();
        self.store(BRANCHES, held.join(","))
    }

    /// Adds one dictionary to those that contributed this field.
    ///
    /// Idempotent under the fold: a dialect already listed is listed once.
    ///
    /// # Errors
    ///
    /// Returns [`set_branches`](Self::set_branches)'s refusal.
    pub fn add_branch(&mut self, dialect: &str) -> Result<()> {
        if self.as_protocol().has_branch(dialect) {
            return Ok(());
        }
        let mut held: Vec<String> = self.as_protocol().branches().map(str::to_owned).collect();
        held.push(dialect.to_owned());
        self.set_branches(held)
    }

    /// Records the canonical FIX tag.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag is negative, or when the property write
    /// fails the validation every metadata write goes through. Either leaves
    /// the field unchanged.
    pub fn set_tag(&mut self, tag: i32) -> Result<()> {
        if tag < 0 {
            return Err(self.rejected(TAG, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
        }
        self.store(TAG, tag.to_string())
    }

    /// Records the alternate tags in the given order, highest priority first.
    ///
    /// An empty slice removes the property.
    ///
    /// # Errors
    ///
    /// Returns an error when a tag is negative or repeated, leaving the field
    /// unchanged.
    pub fn set_tags(&mut self, tags: &[i32]) -> Result<()> {
        if tags.is_empty() {
            self.remove(TAGS);
            return Ok(());
        }
        let mut rendered = String::new();
        for (index, tag) in tags.iter().enumerate() {
            if *tag < 0 {
                return Err(self.rejected(TAGS, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
            }
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

    /// Records the spellings that mean "nothing was sent" for this field.
    ///
    /// Empty input removes the property. A spelling is stored exactly as
    /// given, because a venue's own casing is what a reader recognizes it by,
    /// and matched case-insensitively on the way back.
    ///
    /// # Errors
    ///
    /// Returns an error when a spelling contains the separator or repeats an
    /// earlier one with ASCII case folded, leaving the field unchanged. An
    /// empty spelling is admitted, and is how a field states that the empty
    /// value is its absence.
    pub fn set_nulls<I, S>(&mut self, spellings: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut rendered = String::new();
        let mut count = 0;
        for spelling in spellings {
            let spelling = spelling.as_ref();
            if spelling.contains(SEPARATOR) {
                return Err(self.rejected(
                    NULLS,
                    format_smolstr!("expected a spelling without {SEPARATOR:?}, got {spelling:?}"),
                ));
            }
            if count > 0
                && rendered
                    .split(SEPARATOR)
                    .any(|held| held.eq_ignore_ascii_case(spelling))
            {
                return Err(self.rejected(
                    NULLS,
                    format_smolstr!("expected each spelling once, got {spelling:?} twice"),
                ));
            }
            if count > 0 {
                rendered.push(SEPARATOR);
            }
            rendered.push_str(spelling);
            count += 1;
        }
        if count == 0 {
            self.remove(NULLS);
            return Ok(());
        }
        self.store(NULLS, rendered)
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

    /// Records how a value of this field is restated at a later version.
    ///
    /// Entries are rendered canonically in the order given, because the
    /// order is what the document says: the first entry whose conditions a
    /// held value meets answers, so a catch-all entry comes last.
    ///
    /// An empty slice removes the property, exactly as an empty tag or alias
    /// list removes its own.
    ///
    /// ```
    /// use yggdryl::fix::{FixFill, FixFillSource, FixReplacement};
    /// use yggdryl::{DataType, Version};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut broker = DataType::utf8().nullable_field("execbroker");
    /// broker.as_fix_mut().set_tag(76)?;
    /// // ExecBroker(76) became one Parties occurrence: PartyID(448) takes the
    /// // broker, PartyRole(452) says it is an executing firm.
    /// broker.as_fix_mut().set_replacements(&[
    ///     FixReplacement::new("4.3".parse::<Version>()?).with_fills([FixFill::Group {
    ///         name: "parties".into(),
    ///         members: vec![
    ///             FixFill::Field { tag: 448, value: FixFillSource::Source },
    ///             FixFill::Field { tag: 452, value: FixFillSource::Constant("1".into()) },
    ///         ],
    ///     }]),
    /// ])?;
    /// assert_eq!(
    ///     broker.get_metadata("fix:replacements"),
    ///     Some(concat!(
    ///         r#"{"replacements":[{"since":"4.3","fills":[{"group":"parties","members":"#,
    ///         r#"[{"tag":448},{"tag":452,"value":"1"}]}]}]}"#,
    ///     ))
    /// );
    ///
    /// broker.as_fix_mut().set_replacements(&[])?;
    /// assert_eq!(broker.get_metadata("fix:replacements"), None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when an entry states no fill, a group fill
    /// states no member, a join names fewer than two tags, a tag is negative,
    /// or a message type, group name, held value or constant is not a word
    /// the reader reads back; and the property write's refusal otherwise.
    /// Either leaves the field unchanged.
    pub fn set_replacements(&mut self, entries: &[FixReplacement]) -> Result<()> {
        if entries.is_empty() {
            self.remove(REPLACEMENTS);
            return Ok(());
        }
        let rendered = FixReplacements::render(entries)?;
        self.store(REPLACEMENTS, rendered)
    }

    /// Removes the replacement rules, answering what they held.
    ///
    /// ```
    /// use yggdryl::fix::{FixFill, FixFillSource, FixReplacement};
    /// use yggdryl::{DataType, Version};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut odd_lot = DataType::Boolean.nullable_field("oddlot");
    /// odd_lot.as_fix_mut().set_tag(575)?;
    /// let rules = [FixReplacement::new("5.0".parse::<Version>()?)
    ///     .with_when("Y")
    ///     .with_fills([FixFill::Field { tag: 1093, value: FixFillSource::Constant("1".into()) }])];
    /// odd_lot.as_fix_mut().set_replacements(&rules)?;
    ///
    /// assert_eq!(odd_lot.as_fix_mut().remove_replacements()?, Some(rules.to_vec()));
    /// assert_eq!(odd_lot.as_fix_mut().remove_replacements()?, None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the stored
    /// document does not parse, having already removed it: a document a
    /// reader refuses is one a caller asked to take away.
    pub fn remove_replacements(&mut self) -> Result<Option<Vec<FixReplacement>>> {
        let Some(stored) = self.remove(REPLACEMENTS) else {
            return Ok(None);
        };
        FixReplacements::over(Some(stored.as_str()))
            .map(|entry| entry.map(FixReplacement::from))
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
    /// | `fix:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
    /// | `fix:branches` | union, folded, sorted: every dictionary that contributed either side |
    /// | `fix:tags` | union, incoming first, order kept, deduplicated |
    /// | `fix:aliases` | union, folded, incoming first, then rewritten from the merged lineage |
    /// | `description` | not folded here at all: it is a generic key, so the metadata merge every protocol shares carries it |
    /// | `fix:lineage` | merged by pedigree, incoming winning an equal pair, re-sorted oldest first |
    /// | `fix:codes` | merged by wire value, incoming winning a shared value |
    /// | `fix:replacements` | incoming wins whole: the order of its entries is the rule, and two documents have no order between them |
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
    /// Returns a typed conflict naming both sides when the tag disagrees,
    /// and the write's refusal otherwise. Either leaves the field exactly as
    /// it was.
    pub fn merge_with(&mut self, other: &FixField<'_>) -> Result<()> {
        let held = self.as_protocol();
        // Identity is checked before anything is built, so a refusal costs
        // neither a render nor a write.
        if held.tag()? != other.tag()? {
            return Err(Error::conflict(
                "fix field",
                "fix field",
                format_smolstr!("tag {:?} merged with {:?}", held.tag()?, other.tag()?),
            ));
        }
        for key in [COUNTER, COMPONENT, FIELD_REF, GROUP, MSGTYPE] {
            if let (Some(left), Some(right)) = (held.get(key), other.get(key)) {
                let equal = if key == MSGTYPE {
                    left == right
                } else {
                    left.eq_ignore_ascii_case(right)
                };
                if !equal {
                    return Err(Error::conflict(
                        "one FIX reference",
                        "conflicting references",
                        format_smolstr!("{key}: expected {left:?}, got {right:?}"),
                    ));
                }
            }
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
        let branches = render_branches(held.branches().chain(other.branches()));
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
                BRANCHES => branches.clone(),
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

    /// Name the full key a value was refused under.
    fn rejected(&self, name: &str, reason: SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason,
        }
    }
}

/// Whether `text` is one of `nulls`, the spellings a field states as an
/// absence: ASCII case-insensitively, against the trimmed text.
///
/// The one predicate behind [`FixField::is_null_value`] and the codec's
/// memo of a field's facts, so a spelling reads as an absence the same way
/// whether the field was looked up or remembered.
pub(super) fn spells_absence<'a>(nulls: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    let trimmed = text.trim_ascii();
    nulls
        .into_iter()
        .any(|spelling| spelling.eq_ignore_ascii_case(trimmed))
}

/// The aliases a field declares, in stored priority order.
///
/// Answered by [`FixField::aliases`]. It walks the stored comma-separated
/// text as it goes and hands back slices of it, so nothing is parsed ahead
/// of the alias being asked for and nothing is allocated. An empty element,
/// which the writer never produces, is skipped rather than reported: the
/// typed rejection belongs to the write, and a read stays cheap.
#[derive(Clone, Debug)]
pub struct FixSpellings<'field> {
    parts: Option<Split<'field, char>>,
}

impl<'field> FixSpellings<'field> {
    /// Walk one stored comma-separated value, or nothing for an absent one.
    fn over(stored: Option<&'field str>) -> Self {
        Self {
            parts: stored.map(|stored| stored.split(SEPARATOR)),
        }
    }
}

impl<'field> Iterator for FixSpellings<'field> {
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

impl DoubleEndedIterator for FixSpellings<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.parts.as_mut()?.rfind(|alias| !alias.is_empty())
    }
}

impl FusedIterator for FixSpellings<'_> {}

/// The `fix:` keys a merge folds, as a `const` listing.
///
/// A merge walks this rather than collecting the keys a field holds, because
/// the held names are owned `String`s behind a generic snapshot and building
/// a vector of them to scan `O(n*m)` is what this replaced.
const MERGED_KEYS: [&str; 8] = [
    TAG,
    BRANCHES,
    TAGS,
    ALIASES,
    NULLS,
    LINEAGE,
    CODES,
    REPLACEMENTS,
];

/// Render aliases the way the setter renders them.
fn render_aliases(aliases: &[&str]) -> Option<String> {
    if aliases.is_empty() {
        return None;
    }
    Some(aliases.join(","))
}

/// Render the dictionaries that contributed a field the way the setter
/// renders them: folded, deduplicated under the crate fold, sorted.
fn render_branches<'a>(dialects: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut held: Vec<String> = Vec::new();
    for dialect in dialects {
        let folded = dialect.to_ascii_lowercase();
        if !held.iter().any(|known| folds_equal(known, &folded)) {
            held.push(folded);
        }
    }
    if held.is_empty() {
        return None;
    }
    held.sort();
    Some(held.join(","))
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
///
/// A code stated under a value the winner already holds is not dropped whole:
/// its name and its own aliases become spellings on the code that stays,
/// because a name one side declared is one the merged set has to answer to.
///
/// What cannot be kept is a spelling another code already answers to, folded:
/// two codes one spelling reaches resolve to nothing rather than to either,
/// and two sharing a name are refused outright. So that spelling is dropped,
/// and a code whose own *name* is taken is dropped with it, having no other
/// name to arrive under.
fn merge_codes(winner: &FixField<'_>, other: &FixField<'_>) -> Result<Option<String>> {
    let mut codes: Vec<FixCode> = Vec::new();
    for code in winner.codes() {
        codes.push(FixCode::from(code?));
    }
    for code in other.codes() {
        let incoming = FixCode::from(code?);
        // Every spelling this code arrives with, its name first, held apart
        // from the code so the code itself can move into the set.
        let mut spellings: Vec<SmolStr> = vec![SmolStr::new(incoming.name())];
        spellings.extend(incoming.aliases().iter().cloned());
        let at = match codes
            .iter()
            .position(|held| held.value() == incoming.value())
        {
            Some(at) => at,
            None if codes.iter().any(|held| held.is_spelled(&spellings[0])) => continue,
            None => {
                codes.push(incoming.with_aliases(std::iter::empty::<SmolStr>()));
                codes.len() - 1
            }
        };
        // A code answers its own name, so this adds it where the value was
        // already held and skips it where the code was just pushed.
        for spelling in &spellings {
            if !codes.iter().any(|held| held.is_spelled(spelling)) {
                codes[at].push_alias(spelling.clone());
            }
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
/// [`FixField::code_value_at`] over a stored document: the three tiers as the
/// version knows them, then as every version does.
pub(super) fn translate<'field>(
    stored: &'field str,
    text: &str,
    at: Option<Version>,
) -> Option<&'field str> {
    if at.is_some() {
        if let Some(held) = resolve_in(stored, text, at) {
            return Some(held);
        }
    }
    resolve_in(stored, text, None)
}

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
