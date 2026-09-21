//! The `FIX:` vocabulary, on the field views that carry it.
//!
//! One type reads each property and one type writes it, and both reach the
//! metadata only through the view's own `get`, `insert` and `remove`, so
//! [`Field`](crate::Field)'s cache-aware mutation and metadata validation
//! apply to every write. The property names are private to this module: a
//! caller writes `set_tag(35)`, never `"FIX:tag"`.

use std::iter::FusedIterator;
use std::str::Split;

use smol_str::{SmolStr, format_smolstr};

use super::FixId;
use super::constants::MSGCATEGORIES;
use super::directions::{FixDirection, FixDirections};
use super::document::{Cursor, Numbers, Words, Writer, is_word, repeated_number, repeated_word};
use super::replacements::{FixReplacement, FixReplacements};
use crate::expression::Term;
use crate::folds_equal;
use crate::{DataType, Error, FixField, FixFieldMut, Result};

/// The dictionaries that contributed this field, folded and sorted; absent
/// for a field the specification alone defines.
const BRANCHES: &str = "branches";
/// The canonical tag.
const TAG: &str = "tag";
/// The full key the canonical tag is stored under.
pub(super) const TAG_KEY: &str = "FIX:tag";
/// The alternate tags, a JSON array of tags, highest priority first.
const TAGS: &str = "tags";
/// The alternate names, a JSON array of words, highest priority first.
const NAMES: &str = "names";
/// The direct scalar members identifying one component, in member order.
const IDENTIFIERS: &str = "identifiers";
/// The spellings that mean "nothing was sent" for this field.
const NULLS: &str = "nulls";
/// The specification's own wording.
/// What a field is for is not FIX's to own.
///
/// A description is a property of the *field*, not of the protocol quoting
/// it: the same sentence is what an Iceberg doc, a SQL column comment and a
/// FIX definition each publish. It is therefore read and written on the
/// generic key every catalog already reads, rather than under `FIX:` where
/// only a FIX reader would find it.
/// The name of the FIX code set this field's values are drawn from; the
/// dictionary holds its members.
const CODESET: &str = "codeset";
/// How a value of this field is restated at a later version.
const REPLACEMENTS: &str = "replacements";
/// The rules naming a code of this field's set from the prose in front of a
/// payload; tag 385's.
const DIRECTIONS: &str = "directions";
/// How this field's value is derived from the message where the message
/// states none: the canonical text of one term over the message's fields.
const DERIVATION: &str = "derivation";
const COUNTER: &str = "counter";
/// Whether this field travels from one message of a chain to the next.
const TRANSIENT: &str = "transient";
const DEPRECATED: &str = "deprecated";
const COMPONENT: &str = "component";
const FIELD_REF: &str = "field";
const GROUP: &str = "group";
const MSGTYPE: &str = "msgtype";
const MSGCAT: &str = "msgcat";
pub(super) fn is_msgcat(value: &str) -> bool {
    MSGCATEGORIES.contains(&value)
}
/// What separates the elements of a comma-separated property: the
/// memberships, the identifiers and the null spellings, whose elements can
/// hold no comma. The names and the tags are JSON arrays instead.
const SEPARATOR: char = ',';

/// What a tag is, spelled once for every refusal.
const TAG_SHAPE: &str = "a FIX tag, a decimal integer from 1 to 2147483647";

/// Parse one positive tag strictly: decimal digits only, never signed.
///
/// `i32::from_str` would also accept `+35`, which the writer never emits, so
/// the digits are checked first and the width second.
pub(super) fn parse_tag(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|tag| *tag > 0)
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

    /// The version at which the specification deprecated this field, where
    /// it did: the dictionary keeps the field so an old message still
    /// resolves, and a reader restates it under what replaced it - see
    /// [`replacements`](Self::replacements) - and keeps no value of its own
    /// for it.
    pub fn deprecated(&self) -> Option<&'field str> {
        self.get(DEPRECATED)
    }

    /// The repeating group referenced by a catalog occurrence.
    pub fn group(&self) -> Option<&'field str> {
        self.get(GROUP)
    }

    /// The wire message type declared by a message definition.
    pub fn msgtype(&self) -> Option<&'field str> {
        self.get(MSGTYPE)
    }

    /// The four-byte business category declared by a message definition.
    pub fn msgcat(&self) -> Option<&'field str> {
        self.get(MSGCAT)
    }

    /// The positive tag of a group's count field or intrinsic Map counter.
    pub fn counter(&self) -> Result<Option<i32>> {
        self.get(COUNTER)
            .map(|stored| parse_tag(stored).ok_or_else(|| self.invalid(COUNTER, TAG_SHAPE, stored)))
            .transpose()
    }

    /// Whether this field's value carries from one message of a chain to the
    /// next, `true` where the field says nothing.
    ///
    /// A chain is one instrument's run of messages, and most of what a
    /// message says about the instrument is still true of the next one: the
    /// classification, the ISIN, the market, the currency. A field that is
    /// *transient* in this sense is carried forward, so a venue that states
    /// `CFICode` once and then sends twenty updates that do not repeat it
    /// still has twenty rows that know what the instrument is.
    ///
    /// The default is `true`, and it is the safe one: carrying a fact that
    /// is still true costs a column fill, while failing to carry one loses
    /// what the capture knew. A field that is genuinely about the single
    /// message rather than the instrument - a sequence number, a clock, an
    /// identity - says `false` and is left where it was stated.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `FIX:transient` key when the stored
    /// text is not `true` or `false`.
    pub fn is_transient(&self) -> Result<bool> {
        match self.get(TRANSIENT) {
            None => Ok(true),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(stored) => Err(self.invalid(TRANSIENT, "true or false", stored)),
        }
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

    /// Builds this field's identity, absent exactly when `FIX:tag` is.
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
    /// Returns an error naming the full `FIX:tag` key when the stored text is
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
    /// Returns an error naming the full `FIX:tags` key when the stored text
    /// is not the compact JSON array of tags [`FixFieldMut::set_tags`]
    /// writes, holds a tag that is not positive, or names one twice.
    pub fn tags(&self) -> Result<Vec<i32>> {
        let Some(stored) = self.get(TAGS) else {
            return Ok(Vec::new());
        };
        let mut cursor = Cursor::new(stored);
        let body = cursor
            .read_numbers(TAGS)
            .ok()
            .filter(|_| cursor.is_done())
            .ok_or_else(|| self.invalid(TAGS, "a JSON array of FIX tags", stored))?;
        if Numbers::over(body).any(|tag| tag <= 0) {
            return Err(self.invalid(TAGS, TAG_SHAPE, stored));
        }
        if repeated_number(Numbers::over(body)).is_some() {
            return Err(self.invalid(TAGS, "each tag once", stored));
        }
        Ok(Numbers::over(body).collect())
    }

    /// Iterates the alternate names, highest priority first.
    ///
    /// The iterator is lazy and allocates nothing: every name is a slice of
    /// the stored array, which the field already owns, so reading them costs
    /// the same whether one is taken or all are. An absent property yields
    /// nothing, and so does a stored text that is not the JSON array of
    /// words [`FixFieldMut::set_names`] writes: the typed refusal belongs to
    /// the write, and a read stays cheap.
    pub fn names(&self) -> Words<'field> {
        Words::over(self.get(NAMES).and_then(word_list).unwrap_or_default())
    }

    /// Holds the stored alternate names to what [`FixFieldMut::set_names`]
    /// writes, which is what a registry asks before it takes a field: the
    /// infallible read above answers nothing for a text it cannot walk, and
    /// a dictionary must not hold one.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `FIX:names` key when the stored text
    /// is not the compact JSON array of words the setter writes, or names one
    /// twice with ASCII case folded.
    pub(super) fn validate_names(&self) -> Result<()> {
        let Some(stored) = self.get(NAMES) else {
            return Ok(());
        };
        let body = word_list(stored)
            .ok_or_else(|| self.invalid(NAMES, "a JSON array of names", stored))?;
        if repeated_word(Words::over(body)).is_some() {
            return Err(self.invalid(NAMES, "each name once", stored));
        }
        Ok(())
    }

    /// Borrows the canonical identifier member names, in component order.
    /// An absent declaration yields nothing; the iterator allocates nothing.
    pub fn identifiers(&self) -> FixSpellings<'field> {
        FixSpellings::over(self.get(IDENTIFIERS))
    }

    /// Resolves intake spellings once against this component's own children.
    pub(super) fn identifier_positions<I, S>(&self, spellings: I) -> Result<Vec<usize>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut positions = Vec::new();
        for (offset, spelling) in spellings.into_iter().enumerate() {
            let spelling = spelling.as_ref();
            let refused = |expected: &str| {
                self.invalid(
                    IDENTIFIERS,
                    &format!(
                        "{expected} at {}.fix:identifiers[{offset}]",
                        self.as_field().name()
                    ),
                    spelling,
                )
            };
            if spelling.is_empty() || spelling.contains(SEPARATOR) {
                return Err(refused("a nonempty member spelling without a comma"));
            }
            if !matches!(self.as_field().dtype(), DataType::Struct(_)) {
                return Err(refused(
                    "a Struct component declaring its own scalar members",
                ));
            }
            let tag = parse_tag(spelling);
            let mut reached = None;
            for (position, child) in self.as_field().fields().iter().enumerate() {
                let view = child.as_fix();
                let named = folds_equal(child.name(), spelling)
                    || view.names().any(|name| folds_equal(name, spelling));
                let tagged = match tag {
                    Some(tag) => view.tag()? == Some(tag) || view.tags()?.contains(&tag),
                    None => false,
                };
                if !named && !tagged {
                    continue;
                }
                if reached.replace(position).is_some() {
                    return Err(refused("one unambiguous direct scalar member"));
                }
                if child.dtype().is_nested() {
                    return Err(refused("a direct scalar member, not a nested member"));
                }
                if child.name().is_empty() || child.name().contains(SEPARATOR) {
                    return Err(refused(
                        "a canonical member name without an empty element or comma",
                    ));
                }
            }
            let position = reached.ok_or_else(|| refused("an existing direct scalar member"))?;
            if positions.contains(&position) {
                return Err(refused("each identifier member exactly once"));
            }
            positions.push(position);
        }
        positions.sort_unstable();
        Ok(positions)
    }

    pub(super) fn compiled_identifier_positions(&self) -> Result<Vec<usize>> {
        self.identifier_positions(
            self.get(IDENTIFIERS)
                .into_iter()
                .flat_map(|text| text.split(SEPARATOR)),
        )
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
    /// Read from the generic `description` key rather than from `FIX:`,
    /// because what a field is for belongs to the field. See
    /// [`Field::description`](crate::Field::description).
    pub fn description(&self) -> Option<&'field str> {
        self.as_field().description()
    }

    /// Returns the name of the FIX code set this field's values are drawn
    /// from.
    ///
    /// A field states which vocabulary it reads by, never a copy of its
    /// members: the [dictionary](super::FixRegistry) holds each set once
    /// under this name, and
    /// [`FixRegistry::codeset_of`](super::FixRegistry::codeset_of) is what
    /// answers the members. A field drawing on no set answers nothing.
    pub fn codeset(&self) -> Option<&'field str> {
        self.get(CODESET)
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
    /// use yggdryl::fix::FixReplacement;
    /// use yggdryl::{DataType, Plan};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut max_floor = DataType::Float64.nullable_field("maxfloor");
    /// max_floor.as_fix_mut().set_tag(111)?;
    /// // MaxFloor(111) was replaced by DisplayQty(1138), which takes its value.
    /// let plan: Plan = "select maxfloor as displayqty".parse()?;
    /// max_floor.as_fix_mut().set_replacements(&[FixReplacement::new(plan.clone())])?;
    ///
    /// let entry = max_floor.as_fix().replacements().next().expect("one rule")?;
    /// assert_eq!(entry.parse_plan()?, plan);
    /// assert_eq!(entry.doc(), None, "no wording stated");
    /// # Ok(())
    /// # }
    /// ```
    pub fn replacements(&self) -> FixReplacements<'field> {
        FixReplacements::over(self.get(REPLACEMENTS))
    }

    /// Walks the rules naming a code of this field's set from the prose in
    /// front of a payload, in document order.
    ///
    /// Tag 385's field carries them; the reading
    /// [`FixRegistry::msgdirection`](crate::FixRegistry::msgdirection)
    /// compiles them once and answers the defaults where the property is
    /// absent. The iterator is lazy and allocates nothing: every spelling is
    /// a slice of the stored document, which the field already owns. An
    /// absent property yields nothing.
    ///
    /// ```
    /// use yggdryl::fix::FixDirection;
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut direction = DataType::utf8().nullable_field("msgdirection");
    /// direction.as_fix_mut().set_tag(385)?;
    /// direction.as_fix_mut().set_directions(&[
    ///     FixDirection::new("S", ["^TX "]),
    ///     FixDirection::new("R", ["^RX "]),
    /// ])?;
    ///
    /// let entry = direction.as_fix().directions().next().expect("one rule")?;
    /// assert_eq!(entry.code(), "S");
    /// assert_eq!(entry.parse_patterns()?, ["^TX "]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn directions(&self) -> FixDirections<'field> {
        FixDirections::over(self.get(DIRECTIONS))
    }

    /// The term this field's value is derived from the message with, where
    /// the message states none.
    ///
    /// One term in the crate's expression grammar over the message's fields,
    /// spelled by their canonical folded names - `orderqty`, `cumqty`,
    /// `secaltids` - and evaluated by every
    /// [parse](crate::FixCodec::parse_line): a field carrying one is a
    /// column the parse fills where the message left it unsaid. `None` is a
    /// field nothing derives.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::expression::Term;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut leaves = DataType::Float64.nullable_field("leavesqty");
    /// leaves.as_fix_mut().set_tag(151)?;
    /// leaves.as_fix_mut().set_derivation(&"orderqty - cumqty".parse::<Term>()?)?;
    ///
    /// let term = leaves.as_fix().derivation()?.expect("a derivation");
    /// assert_eq!(term.to_string(), "orderqty - cumqty");
    /// assert_eq!(term.columns(), ["orderqty", "cumqty"]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidMetadataValue`] naming `FIX:derivation` when
    /// the stored text is not a term, or one past the depth or node budget.
    pub fn derivation(&self) -> Result<Option<Term>> {
        let Some(stored) = self.get(DERIVATION) else {
            return Ok(None);
        };
        let term: Term = stored
            .parse()
            .map_err(|error: Error| self.rejected(DERIVATION, format_smolstr!("{error}")))?;
        term.check_budget()
            .map_err(|error| self.rejected(DERIVATION, format_smolstr!("{error}")))?;
        Ok(Some(term))
    }

    /// Name the full key a stored value failed under, and what it should be.
    fn invalid(&self, name: &str, expected: &str, actual: &str) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason: format_smolstr!("expected {expected}, got {actual:?}"),
        }
    }

    /// Name the full key a stored value was refused under, with the reader's
    /// own reason.
    fn rejected(&self, name: &str, reason: SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason,
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

    /// Records the version at which the specification deprecated this
    /// field; `None` states it is current.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidMetadataValue`] for an empty version.
    pub fn set_deprecated(&mut self, version: Option<&str>) -> Result<()> {
        match version {
            None => {
                self.remove(DEPRECATED);
                Ok(())
            }
            Some(version) if version.trim().is_empty() => Err(Error::InvalidMetadataValue {
                key: DEPRECATED.into(),
                reason: "expected a version, got an empty text".into(),
            }),
            Some(version) => self.store(DEPRECATED, version.trim().to_owned()),
        }
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

    /// Declares one fixed FIX message category.
    pub fn set_msgcat(&mut self, value: &str) -> Result<()> {
        if !is_msgcat(value) {
            return Err(self.rejected(
                MSGCAT,
                format_smolstr!("expected one fixed FIX message category, got {value:?}"),
            ));
        }
        self.store(MSGCAT, value.to_owned())
    }

    /// Declares the positive tag of the group's count field or Map counter.
    pub fn set_counter(&mut self, tag: i32) -> Result<()> {
        if tag <= 0 {
            return Err(self.rejected(COUNTER, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
        }
        self.store(COUNTER, tag.to_string())
    }

    /// Says whether this field carries from one message of a chain to the
    /// next.
    ///
    /// `true` is the default, so setting it stores nothing and clears any
    /// stored `false`: a property every field would carry identically is not
    /// a property worth writing on every field.
    ///
    /// # Errors
    ///
    /// Returns the metadata layer's refusal when the write does not land.
    pub fn set_transient(&mut self, transient: bool) -> Result<()> {
        if transient {
            self.remove(TRANSIENT);
            return Ok(());
        }
        self.store(TRANSIENT, "false".to_owned())
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

    /// Removes the declared message category.
    pub fn remove_msgcat(&mut self) -> Option<String> {
        self.remove(MSGCAT)
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
    /// Each name is held to the membership grammar - non-empty, no separator -
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
    /// Returns an error when the tag is not positive, or when the property write
    /// fails the validation every metadata write goes through. Either leaves
    /// the field unchanged.
    pub fn set_tag(&mut self, tag: i32) -> Result<()> {
        if tag <= 0 {
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
    /// Returns an error when a tag is not positive or repeated, leaving the field
    /// unchanged.
    pub fn set_tags(&mut self, tags: &[i32]) -> Result<()> {
        if tags.is_empty() {
            self.remove(TAGS);
            return Ok(());
        }
        if let Some(tag) = tags.iter().find(|tag| **tag <= 0) {
            return Err(self.rejected(TAGS, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
        }
        if let Some(tag) = repeated_number(tags.iter().copied()) {
            return Err(self.rejected(
                TAGS,
                format_smolstr!("expected each tag once, got {tag} twice"),
            ));
        }
        self.store(TAGS, Writer::list_of_numbers(tags.iter().copied()))
    }

    /// Records the alternate names in the given order, highest priority
    /// first.
    ///
    /// Empty input removes the property.
    ///
    /// # Errors
    ///
    /// Returns an error when a name is empty, holds a quote, a backslash or
    /// a control character, or repeats an earlier one with ASCII case folded,
    /// leaving the field unchanged.
    pub fn set_names<I, S>(&mut self, names: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let held: Vec<S> = names.into_iter().collect();
        if let Some(text) = held.iter().map(AsRef::as_ref).find(|text| !is_word(text)) {
            return Err(self.rejected(
                NAMES,
                format_smolstr!(
                    "expected a non-empty name without a quote, a backslash or a control character, got {text:?}"
                ),
            ));
        }
        if let Some(text) = repeated_word(held.iter().map(AsRef::as_ref)) {
            return Err(self.rejected(
                NAMES,
                format_smolstr!("expected each name once, got {text:?} twice"),
            ));
        }
        if held.is_empty() {
            self.remove(NAMES);
            return Ok(());
        }
        self.store(
            NAMES,
            Writer::list_of_words(held.iter().map(AsRef::as_ref))?,
        )
    }

    /// Declares direct scalar identifiers by member name, alias or decimal tag.
    ///
    /// Names are stored canonically in component order. Empty input removes
    /// the declaration. Refused, ambiguous, repeated or nested members leave
    /// the field unchanged.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut order = DataType::utf8().nullable_field("clordid");
    /// order.as_fix_mut().set_tag(11)?;
    /// let mut component = DataType::from(StructType::from_fields([order])?).required_field("order");
    /// component.as_fix_mut().set_identifiers(["11"])?;
    /// assert_eq!(component.as_fix().identifiers().collect::<Vec<_>>(), ["clordid"]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_identifiers<I, S>(&mut self, identifiers: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let positions = self.as_protocol().identifier_positions(identifiers)?;
        self.store_identifier_positions(positions)
    }

    /// Reorders a stored declaration at a schema mutation's intake, keeping
    /// malformed empty elements visible to the same resolver as the setter.
    pub(super) fn normalize_identifiers(&mut self) -> Result<()> {
        let positions = self.as_protocol().compiled_identifier_positions()?;
        self.store_identifier_positions(positions)
    }

    fn store_identifier_positions(&mut self, positions: Vec<usize>) -> Result<()> {
        if positions.is_empty() {
            self.remove(IDENTIFIERS);
            return Ok(());
        }
        let mut rendered = String::new();
        for position in positions {
            if !rendered.is_empty() {
                rendered.push(SEPARATOR);
            }
            rendered.push_str(self.as_field().fields()[position].name());
        }
        self.store(IDENTIFIERS, rendered)
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

    /// Names the FIX code set this field's values are drawn from.
    ///
    /// A field states which vocabulary it reads by; the
    /// [dictionary](super::FixRegistry) holds the members, once, under this
    /// name. So a set is named, documented and aliased in one place however
    /// many fields draw on it, and
    /// [`FixRegistry::set_codeset`](super::FixRegistry::set_codeset) is where
    /// its members are stated.
    ///
    /// An empty name removes the property, exactly as an empty tag or alias
    /// list removes its own.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the name is not one a store can
    /// file - the rule a named definition's is held to - and the property
    /// write's refusal otherwise. Either leaves the field unchanged.
    pub fn set_codeset(&mut self, name: &str) -> Result<()> {
        if name.is_empty() {
            self.remove(CODESET);
            return Ok(());
        }
        super::catalog::validate_definition_name(name)?;
        self.store(CODESET, name.to_owned())
    }

    /// Removes the code set reference, answering the name it held.
    pub fn remove_codeset(&mut self) -> Option<String> {
        self.remove(CODESET)
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
    /// use yggdryl::fix::FixReplacement;
    /// use yggdryl::{DataType, Plan};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut rule80a = DataType::utf8().nullable_field("rule80a");
    /// rule80a.as_fix_mut().set_tag(47)?;
    /// // Rule80A(47) `A` is an agency order: OrderCapacity(528) takes `A`.
    /// let plan: Plan = "select 'A' as ordercapacity where rule80a = 'A'".parse()?;
    /// rule80a.as_fix_mut().set_replacements(&[
    ///     FixReplacement::new(plan).with_doc("Agency single order"),
    /// ])?;
    /// assert_eq!(
    ///     rule80a.get_metadata("FIX:replacements"),
    ///     Some(concat!(
    ///         r#"[{"plan":"select 'A' as ordercapacity where rule80a = 'A'","#,
    ///         r#""doc":"Agency single order"}]"#,
    ///     ))
    /// );
    ///
    /// rule80a.as_fix_mut().set_replacements(&[])?;
    /// assert_eq!(rule80a.get_metadata("FIX:replacements"), None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when an entry's plan is past the expression
    /// budget or fills no named column; and the property write's refusal
    /// otherwise. Either leaves the field unchanged.
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
    /// use yggdryl::fix::FixReplacement;
    /// use yggdryl::{DataType, Plan};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut odd_lot = DataType::Boolean.nullable_field("oddlot");
    /// odd_lot.as_fix_mut().set_tag(575)?;
    /// // OddLot(575) `Y` became LotType(1093) `1`, an odd lot.
    /// let plan: Plan = "select '1' as lottype where oddlot = true".parse()?;
    /// let rules = [FixReplacement::new(plan)];
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

    /// Records the rules naming a code of this field's set from the prose in
    /// front of a payload.
    ///
    /// Entries are rendered canonically in the order given. A rule's code is
    /// any spelling of a code of the set this field declares - the value or
    /// the name, else the specification's `S` and `R` where it declares
    /// none - resolved here exactly as the reading resolves it, so the
    /// door admits what the reading answers and each code is named once
    /// under any spelling. Every pattern is compiled here as the reading
    /// compiles it, so what is stored is what a codec can use.
    ///
    /// An empty slice removes the property, exactly as an empty tag or alias
    /// list removes its own, and the reading answers its defaults again.
    ///
    /// ```
    /// use yggdryl::fix::FixDirection;
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut direction = DataType::utf8().nullable_field("msgdirection");
    /// direction.as_fix_mut().set_tag(385)?;
    /// direction.as_fix_mut().set_directions(&[
    ///     FixDirection::new("S", [r"^TX\b"]),
    ///     FixDirection::new("R", [r"^RX\b"]),
    /// ])?;
    /// assert_eq!(
    ///     direction.get_metadata("FIX:directions"),
    ///     Some(concat!(
    ///         r#"[{"code":"S","patterns":["^TX\\b"]},"#,
    ///         r#"{"code":"R","patterns":["^RX\\b"]}]"#,
    ///     ))
    /// );
    ///
    /// direction.as_fix_mut().set_directions(&[])?;
    /// assert_eq!(direction.get_metadata("FIX:directions"), None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when an entry names no code of the set, two
    /// entries name one code under any spelling, an entry states an empty
    /// code or one the reader would not read back as a word, an entry
    /// states no pattern, or a pattern is empty or one the regex crate
    /// refuses; and the property write's refusal otherwise. Either leaves
    /// the field unchanged.
    pub fn set_directions(&mut self, directions: &[FixDirection]) -> Result<()> {
        if directions.is_empty() {
            self.remove(DIRECTIONS);
            return Ok(());
        }
        // Whether a code is one of the set is the dictionary's question, not
        // the field's: a field names its set and the registry holds it, so
        // `MsgDirection::from_registry` resolves every spelling against the
        // set in force and drops - with this module's own refusal as the
        // warning - a rule naming a code outside it. What the field can still
        // answer alone is whether one spelling was stated twice.
        let mut named: Vec<&str> = Vec::with_capacity(directions.len());
        for direction in directions {
            let code = direction.code().trim();
            if let Some(held) = named.iter().find(|held| folds_equal(held, code)) {
                return Err(super::directions::repeated(direction.code(), held));
            }
            named.push(code);
        }
        let rendered = FixDirections::render(directions)?;
        self.store(DIRECTIONS, rendered)
    }

    /// Removes the direction rules, answering what they held.
    ///
    /// ```
    /// use yggdryl::fix::FixDirection;
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut direction = DataType::utf8().nullable_field("msgdirection");
    /// direction.as_fix_mut().set_tag(385)?;
    /// let rules = [FixDirection::new("S", [">>>"]), FixDirection::new("R", ["<<<"])];
    /// direction.as_fix_mut().set_directions(&rules)?;
    ///
    /// assert_eq!(direction.as_fix_mut().remove_directions()?, Some(rules.to_vec()));
    /// assert_eq!(direction.as_fix_mut().remove_directions()?, None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the stored
    /// document does not parse, having already removed it: a document a
    /// reader refuses is one a caller asked to take away.
    pub fn remove_directions(&mut self) -> Result<Option<Vec<FixDirection>>> {
        let Some(stored) = self.remove(DIRECTIONS) else {
            return Ok(None);
        };
        FixDirections::over(Some(stored.as_str()))
            .map(|entry| entry.map(FixDirection::from))
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// Records the term this field's value is derived from the message with.
    ///
    /// The term is stored as its canonical text, so what is read back is
    /// what was written whatever spelling the caller parsed it from, and a
    /// registry validates it exactly as this does when a field carrying one
    /// is inserted, updated or loaded.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::expression::Term;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut gross = DataType::Float64.nullable_field("grosstradeamt");
    /// gross.as_fix_mut().set_tag(381)?;
    /// gross.as_fix_mut().set_derivation(&"lastqty*lastpx".parse::<Term>()?)?;
    /// assert_eq!(gross.get_metadata("FIX:derivation"), Some("lastqty * lastpx"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the term is past the depth or node
    /// budget, and the property write's refusal otherwise; either leaves the
    /// field unchanged.
    pub fn set_derivation(&mut self, term: &Term) -> Result<()> {
        term.check_budget()?;
        self.store(DERIVATION, term.to_string())
    }

    /// Removes the derivation, answering the term it spelled.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::expression::Term;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut gross = DataType::Float64.nullable_field("grosstradeamt");
    /// gross.as_fix_mut().set_tag(381)?;
    /// let term: Term = "lastqty * lastpx".parse()?;
    /// gross.as_fix_mut().set_derivation(&term)?;
    ///
    /// assert_eq!(gross.as_fix_mut().remove_derivation()?, Some(term));
    /// assert_eq!(gross.as_fix_mut().remove_derivation()?, None);
    /// assert_eq!(gross.get_metadata("FIX:derivation"), None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidMetadataValue`] when the stored text does not
    /// parse, having already removed it: a text a reader refuses is one a
    /// caller asked to take away.
    pub fn remove_derivation(&mut self) -> Result<Option<Term>> {
        let Some(stored) = self.remove(DERIVATION) else {
            return Ok(None);
        };
        stored
            .parse()
            .map(Some)
            .map_err(|error: Error| self.rejected(DERIVATION, format_smolstr!("{error}")))
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
    /// | `FIX:tag` | MUST agree; a disagreement is a typed refusal naming both. Identity is not merged. |
    /// | `FIX:branches` | union, folded, sorted: every dictionary that contributed either side |
    /// | `FIX:tags` | union, incoming first, order kept, deduplicated |
    /// | `FIX:names` | union, folded, incoming first |
    /// | `description` | not folded here at all: it is a generic key, so the metadata merge every protocol shares carries it |
    /// | `FIX:codeset` | the stored set's name is kept; a stored field naming none takes the incoming name |
    /// | `FIX:replacements` | incoming wins whole: the order of its entries is the rule, and two documents have no order between them |
    /// | `FIX:directions` | incoming wins whole: a rule table is one statement, and two tables have no order between them |
    /// | `FIX:derivation` | incoming wins whole: a derivation is one term, and a field derives one way |
    /// | `FIX:identifiers` | incoming wins whole: identifiers are one ordered component declaration |
    /// | any other `FIX:` key | incoming wins; stored keeps what only it has |
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

        // One pass over the `FIX:` key set, which is a const listing beside
        // these accessors, so no held key name is ever collected into a
        // `String` to be walked.
        let mut tags = held.tags()?;
        for tag in other.tags()? {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        // A names text the read walks as nothing would merge as nothing and
        // be dropped; it is refused instead, as a tags text is.
        held.validate_names()?;
        other.validate_names()?;
        let mut names: Vec<&str> = held.names().collect();
        for name in other.names() {
            if !names.iter().any(|kept| kept.eq_ignore_ascii_case(name)) {
                names.push(name);
            }
        }
        let branches = render_branches(held.branches().chain(other.branches()));

        let mut merged: Vec<(&'static str, String)> = Vec::with_capacity(MERGED_KEYS.len());
        for key in MERGED_KEYS {
            let value = match key {
                TAGS => render_tags(&tags),
                NAMES => render_names(&names)?,
                BRANCHES => branches.clone(),
                // The one key where the *stored* side wins, and `other` is
                // the stored one: a registry fold hands the incoming field in
                // as `self`. A field keeps the vocabulary it already reads by
                // because the members are the dictionary's to fold -
                // `unify_codeset` has already folded the incoming set into
                // the held one under the held name - so taking the incoming
                // name here would move the field to a set holding strictly
                // less than the one it already reads by.
                CODESET => other
                    .get(CODESET)
                    .or_else(|| held.get(CODESET))
                    .map(str::to_owned),
                // Every other key is "incoming wins, stored keeps what only
                // it has".
                _ => held.get(key).or_else(|| other.get(key)).map(str::to_owned),
            };
            if let Some(value) = value {
                merged.push((key, value));
            }
        }
        // A `FIX:` key this vocabulary does not name is still one side's
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

/// The spellings one comma-separated `FIX:` property holds, in stored order.
///
/// Answered by [`FixField::branches`], [`FixField::identifiers`] and
/// [`FixField::nulls`]. It walks the stored text as it goes and hands back
/// slices of it, so nothing is parsed ahead of the spelling being asked for
/// and nothing is allocated. An empty element, which the writer never
/// produces, is skipped rather than reported: the typed rejection belongs to
/// the write, and a read stays cheap.
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
        self.parts.as_mut()?.find(|spelling| !spelling.is_empty())
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
        self.parts.as_mut()?.rfind(|spelling| !spelling.is_empty())
    }
}

impl FusedIterator for FixSpellings<'_> {}

/// The `FIX:` keys a merge folds, as a `const` listing.
///
/// A merge walks this rather than collecting the keys a field holds, because
/// the held names are owned `String`s behind a generic snapshot and building
/// a vector of them to scan `O(n*m)` is what this replaced.
const MERGED_KEYS: [&str; 12] = [
    TAG,
    BRANCHES,
    TAGS,
    NAMES,
    NULLS,
    CODESET,
    REPLACEMENTS,
    DIRECTIONS,
    DERIVATION,
    IDENTIFIERS,
    MSGCAT,
    DEPRECATED,
];

/// The body of one stored array of words, or nothing for a text that is not
/// one: the setter never writes such a text, so a read walks nothing rather
/// than mis-reading a hand edit.
fn word_list(stored: &str) -> Option<&str> {
    let mut cursor = Cursor::new(stored);
    let body = cursor.read_words(NAMES).ok()?;
    cursor.is_done().then_some(body)
}

/// Render alternate names the way the setter renders them.
fn render_names(names: &[&str]) -> Result<Option<String>> {
    if names.is_empty() {
        return Ok(None);
    }
    Writer::list_of_words(names.iter().copied()).map(Some)
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
    Some(Writer::list_of_numbers(tags.iter().copied()))
}
