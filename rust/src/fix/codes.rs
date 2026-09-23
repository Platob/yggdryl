//! The named FIX code sets a dictionary holds, and every spelling that
//! reaches a value.
//!
//! Most FIX fields with a vocabulary are `int`, `Boolean` or `String` and
//! draw their values from a *code set*: a wire value, a symbolic name, and
//! usually a sentence of documentation. `Side(54)` is `1` for `Buy`; a
//! message may spell that `1`, `Buy`, `buy` or `BUY`, and a JSON or human
//! caller routinely spells it the long way. One field answers all of them.
//!
//! # The set is named, and the dictionary owns it
//!
//! A code set is a vocabulary rather than a property of one field: the
//! specification names it - `SideCodeSet`, `UnitOfMeasureCodeSet` - and
//! names it from as many fields as draw on it, 103 of them in the shipped
//! dictionary for one offset-unit set alone. So the [registry](super::FixRegistry)
//! holds each set once under its name, stored beside the fields in
//! [`codesets/`](super::FixRegistry::commit), and a field's `FIX:codeset`
//! states which set it draws from rather than a copy of its members. One
//! owner per vocabulary: a code named, aliased or documented once is named
//! for every field that reads it, and two fields cannot drift apart while
//! claiming one set.
//!
//! `msgcatcodeset` is the deliberate exception: it renders the crate-owned
//! integer identifiers that cross into generic graph operations. A registry
//! may assign one of its symbolic categories to a custom message type, but it
//! may not replace, widen or remove that intrinsic category-to-ID mapping.
//!
//! [`FixCodeSet`] is that set, borrowed from the dictionary: one [canonical
//! document](super::document) ordered by wire value, read without parsing
//! ahead or allocating. It is a second key beside
//! [`StringEnum`](crate::StringEnum) rather than a second copy of it - that
//! type is name to ASCII value packed through the field's own width, so it
//! accepts only fixed US-ASCII strings of at most sixteen bytes and coded
//! datatypes and carries no description or pedigree. A field may carry both
//! and neither derives from the other.
//!
//! # Resolving a spelling
//!
//! [`FixCodeSet::code_value`] composes three tiers, and a spelling that
//! reaches none falls through unchanged rather than failing: a venue sends
//! codes no dictionary lists exactly as it sends fields no dictionary names.
//!
//! 1. **The text as a wire value, exactly.** `4` is `4`. A spelling that is
//!    already a legal code is never reinterpreted as somebody's name, and
//!    this is the early-exit fast path.
//! 2. **The folded symbolic name, then any alias.** The fold is the crate's
//!    one fold, so `PercentageWaivedCashDiscount`,
//!    `percentage_waived_cash_discount` and `PERCENTAGE WAIVED CASH DISCOUNT`
//!    are one spelling.
//! 3. **The leading parenthesized abbreviation of the description.** `"Good
//!    Till Date (GTD)"` answers `gtd`.
//!
//! # Merging two sets
//!
//! Two dictionaries stating one set state it from different sources: a
//! venue's `CBlock` knows a value exists and calls it after itself, the
//! specification knows what it is called, and an older version knows a
//! spelling the newest dropped. [`FixCodes::merge`] folds them into the one
//! set that answers every spelling either declared - keyed by wire value,
//! a placeholder name yielding to a real one, every surviving spelling kept
//! as an alias - which is why a merge adds to a dictionary's vocabulary and
//! never narrows it.

use std::collections::btree_map::Entry;
use std::iter::FusedIterator;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::FixRegistry;
use super::document::{Cursor, Refusal, Scan, Words, Writer, decode_text};
use crate::folds_equal;
use crate::{Error, Field, Result, Scalar};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix codes";

/// Leaves ordinary dictionary vocabularies mutable while pinning the one
/// code set whose values are generic graph identifiers.
fn validate_intrinsic_codeset(key: &str, document: Option<&str>) -> Result<()> {
    if !folds_equal(key, super::crated::MSGCAT_CODESET_NAME) {
        return Ok(());
    }
    let canonical = super::crated::msgcat_codeset()
        .ok_or_else(|| Error::absent("the intrinsic MsgCat code set", key))?;
    if document == Some(canonical.as_ref()) {
        return Ok(());
    }
    Err(Error::conflict(
        "the fixed MsgCat operation identifiers",
        "a changed or removed MsgCat code set",
        key,
    ))
}

/// Refuses an intrinsic merge whose input attempts to remap or widen one
/// category even when the ordinary vocabulary merge would discard that
/// conflicting spelling and leave the stored document unchanged.
fn validate_intrinsic_merge(key: &str, codes: &[FixCode]) -> Result<()> {
    if !folds_equal(key, super::crated::MSGCAT_CODESET_NAME)
        || codes.iter().all(|code| {
            super::constants::MSGCATEGORY_CODES
                .iter()
                .any(|(name, _, canonical)| {
                    let is_same_spelling = |spelling: &str| folds_equal(name, spelling);
                    code.value() == *canonical
                        && is_same_spelling(code.name())
                        && code.aliases().iter().all(|alias| is_same_spelling(alias))
                        && code.description().is_none()
                        && code.group().is_none()
                })
        })
    {
        return Ok(());
    }
    Err(Error::conflict(
        "the fixed MsgCat operation identifiers",
        "a changed or removed MsgCat code set",
        key,
    ))
}

/// The wire value, and the key every lookup keys on.
const VALUE: &str = "value";
/// The symbolic name.
const NAME: &str = "name";
/// The group the specification files this code under.
const GROUP: &str = "group";
/// Venue and per-version spellings of the same code.
const ALIASES: &str = "aliases";
/// The specification's own wording.
const DOC: &str = "doc";

/// How long a needle addressing one record may be.
///
/// `{"value":"` plus the value plus its closing quote. Sixty-four bytes holds
/// every FIX code value and every instrument identifier a venue codes with;
/// a longer one falls back to the ordinary walk rather than allocating.
const NEEDLE_CAPACITY: usize = 64;

/// The keys one code may state, in the order it states them.
///
/// `value` leads because it is the key every lookup keys on, so tier 1 reads
/// one key per record and stops.
pub(super) const KEYS: [&str; 5] = [VALUE, NAME, GROUP, ALIASES, DOC];

/// Whether the spelling `held` - one a code carries, as its name or as an
/// alias - is the spelling `text`, under the one rule every code set answers
/// spellings by.
///
/// The fold reaches every spelling but one: a spelling that *is* the code's
/// own wire value is matched exactly. A wire value is the code's identity
/// rather than a name a person chose for it, and the values a FIX dialect
/// states are case-bearing - `b` and `B` are two messages, `c` and `C` are
/// two more - so folding them together would answer one code for another.
/// Every other spelling folds, because that is what a name is for.
///
/// Free rather than a method so the owned [`FixCode`] and the borrowed
/// [`FixCodeValue`] answer one rule rather than two that drift.
fn spelled_as(held: &str, value: &str, text: &str) -> bool {
    if held == value {
        return held == text;
    }
    folds_equal(held, text)
}

/// One member of a FIX code set, as a caller states it.
///
/// The borrowed [`FixCodeValue`] is what a read answers; this is what a writer
/// hands [`FixRegistry::set_codeset`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixCode {
    name: SmolStr,
    value: SmolStr,
    description: Option<SmolStr>,
    aliases: Vec<SmolStr>,
    group: Option<SmolStr>,
}

impl FixCode {
    /// Builds one code from the two facts every member has.
    pub fn new(name: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            description: None,
            aliases: Vec::new(),
            group: None,
        }
    }

    /// Sets the specification's own wording for this code.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<SmolStr>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Whether this code carries no name beyond the wire value it is.
    ///
    /// A source that knows a value exists but not what anyone calls it names
    /// it after itself - an Ullink `CBlock` declaring `6 Inbound` states tag
    /// 35 `6` and a qualifier, never a name for the type - and that is a
    /// placeholder rather than a name. A merge is where a real one takes its
    /// place, which is what [`FixCodes`] states under merging.
    pub(super) fn is_unnamed(&self) -> bool {
        self.name == self.value
    }

    /// This code under `name`, everything else untouched.
    ///
    /// The value already answers to itself, so a placeholder name it replaces
    /// leaves no spelling behind to keep.
    pub(super) fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.name = name.into();
        self
    }

    /// Sets the venue and per-version spellings that also reach this code.
    #[must_use]
    pub fn with_aliases<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        self.aliases = aliases.into_iter().map(Into::into).collect();
        self
    }

    /// Adds one more spelling that reaches this code.
    ///
    /// [`Self::with_aliases`] states the whole list at once, which is what a
    /// generator holding a specification has. A reader meeting one spelling
    /// at a time has this instead: a source that names one wire value twice
    /// is stating an alias, and a set that drops the second name answers to
    /// one spelling fewer than the source declared.
    ///
    /// Nothing is checked here. [`Self::is_spelled`] is what a caller asks
    /// before adding, because whether a spelling is free is a question about
    /// the whole set and not about one code.
    pub fn push_alias(&mut self, alias: impl Into<SmolStr>) {
        self.aliases.push(alias.into());
    }

    /// Sets the group the specification files this code under.
    #[must_use]
    pub fn with_group(mut self, group: impl Into<SmolStr>) -> Self {
        self.group = Some(group.into());
        self
    }

    /// Returns the symbolic name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns the wire value.
    #[must_use]
    pub fn value(&self) -> &str {
        self.value.as_str()
    }

    /// Returns the specification's own wording.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the venue and per-version spellings that also reach this code.
    #[must_use]
    pub fn aliases(&self) -> &[SmolStr] {
        &self.aliases
    }

    /// Whether `text` is this code's name or one of its aliases, folded -
    /// except for the spelling that *is* this code's wire value, which is
    /// matched exactly.
    ///
    /// The same fold a stored set answers spellings by, so a set answers the
    /// same ones before it is rendered as after. A
    /// caller assembling one asks this to keep a spelling from reaching two
    /// codes. It is deliberately stricter than rendering refuses - the whole
    /// fold rather than ASCII case - because rendering only has to keep a
    /// document readable, while two codes one spelling reaches resolve to
    /// nothing rather than to whichever was met first.
    ///
    /// **The one spelling the fold does not reach is the code's own wire
    /// value.** A wire value is the code's identity rather than a name for
    /// it, and tag 35 is case-bearing: `b` is MassQuoteAcknowledgement and
    /// `B` is News, `c` is SecurityDefinitionRequest and `C` is Email. A
    /// source stating both states two messages, so folding the two values
    /// together would answer one message for the other - which is the fold
    /// deciding what a counterparty meant by a byte it was explicit about.
    /// A code carrying no name of its own is named after its value, so
    /// without this a dialect's whole lower-case half is unreachable.
    ///
    /// Every other spelling still folds, which is what a name is for:
    /// `NewOrderSingle`, `new_order_single` and `NEW ORDER SINGLE` are one
    /// spelling, and `b Inbound` reaches `b` however it is punctuated.
    #[must_use]
    pub fn is_spelled(&self, text: &str) -> bool {
        let spelled = |held: &str| spelled_as(held, self.value.as_str(), text);
        spelled(&self.name) || self.aliases.iter().any(|alias| spelled(alias))
    }

    /// Returns the group the specification files this code under.
    #[must_use]
    pub fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }

    /// Renders this code into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, VALUE, self.value())?;
        writer.text(false, NAME, self.name())?;
        if let Some(group) = self.group() {
            writer.text(false, GROUP, group)?;
        }
        if !self.aliases.is_empty() {
            writer.words(false, ALIASES, self.aliases.iter().map(SmolStr::as_str))?;
        }
        if let Some(description) = self.description() {
            writer.text(false, DOC, description)?;
        }
        writer.close_element();
        Ok(())
    }
}

/// One member of a field's code set, borrowed from the stored document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixCodeValue<'field> {
    value: &'field str,
    name: &'field str,
    aliases: &'field str,
    doc: &'field str,
    group: Option<&'field str>,
}

impl<'field> FixCodeValue<'field> {
    /// Returns the wire value this code stands for.
    #[must_use]
    pub const fn value(self) -> &'field str {
        self.value
    }

    /// Returns the symbolic name.
    #[must_use]
    pub const fn name(self) -> &'field str {
        self.name
    }

    /// Walks the venue and per-version spellings that also reach this code.
    #[must_use]
    pub const fn aliases(self) -> Words<'field> {
        Words::over(self.aliases)
    }

    /// Returns the group the specification files this code under.
    #[must_use]
    pub const fn group(self) -> Option<&'field str> {
        self.group
    }

    /// Returns the specification's wording, still escaped as stored.
    #[must_use]
    pub const fn doc(self) -> Option<&'field str> {
        if self.doc.is_empty() {
            None
        } else {
            Some(self.doc)
        }
    }

    /// Decodes the specification's wording.
    ///
    /// # Errors
    ///
    /// Returns the JSON codec's refusal when the stored text is not a legal
    /// string body, which the writer never produces.
    pub fn parse_doc(self) -> Result<Option<String>> {
        decode_text(TARGET, DOC, self.doc())
    }

    /// Decodes the group the specification files this code under.
    ///
    /// [`Self::group`] answers the stored slice, which is borrowed and may
    /// still carry escapes; this is the text it stands for.
    ///
    /// # Errors
    ///
    /// Returns the JSON codec's refusal when the stored text is not a legal
    /// string body, which the writer never produces.
    pub fn parse_group(self) -> Result<Option<String>> {
        decode_text(TARGET, GROUP, self.group())
    }

    /// Whether `text` is this code's name or one of its aliases, under the
    /// rule [`FixCode::is_spelled`] states: folded, except for the spelling
    /// that is this code's own wire value, which is matched exactly.
    pub(super) fn is_spelled(self, text: &str) -> bool {
        let spelled = |held: &str| spelled_as(held, self.value, text);
        spelled(self.name) || self.aliases().any(spelled)
    }

    /// The leading parenthesized abbreviation of the description, when it has
    /// one that is a spelling rather than a cross-reference.
    ///
    /// Two traps live here, both real. A *numeric* parenthesization is a tag
    /// cross-reference and never a spelling, so `"Broken date; SettlDate (64)
    /// is required"` leaves `64` alone. And only the abbreviation on the
    /// leading phrase counts, so `"Swap Value Factor (SVP) through a central
    /// counterparty (CCP)"` answers `svp` and not `ccp`.
    pub(super) fn abbreviation(self) -> Option<&'field str> {
        let doc = self.doc()?;
        let (_, tail) = doc.split_once('(')?;
        let (inside, _) = tail.split_once(')')?;
        let inside = inside.trim();
        if inside.is_empty() || inside.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(inside)
    }
}

/// A field's code set, ordered by wire value.
///
/// Answered by [`FixCodeSet::codes`]. It walks the stored
/// document as it goes and hands back slices of it, so nothing is parsed
/// ahead of the code being asked for and nothing is allocated. An absent
/// property yields nothing.
#[derive(Clone, Debug)]
pub struct FixCodes<'field> {
    cursor: Cursor<'field>,
    started: bool,
    done: bool,
}

impl<'field> FixCodes<'field> {
    /// Walks one registry-owned code-set document, or nothing for an absent one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Renders codes into the one canonical document they have.
    ///
    /// The order is the caller's, kept: a code set is a list, and where one
    /// sits in it is the presentation rank the specification gives it. There
    /// is no rank key beside the order, because a list already has one.
    /// Two names may share a value - that is an alias, the rule `StringEnum`
    /// already states - but two codes may not share a name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when two codes share a name, or when one
    /// states an empty value or an empty name.
    pub(super) fn render(codes: &[FixCode]) -> Result<String> {
        let ordered: Vec<&FixCode> = codes.iter().collect();
        for (index, code) in ordered.iter().enumerate() {
            if code.value.is_empty() || code.name.is_empty() {
                return Err(Error::Parse {
                    target: TARGET,
                    position: 0,
                    reason: format_smolstr!(
                        "expected every code to state a value and a name, got {:?} and {:?}",
                        code.value(),
                        code.name()
                    ),
                });
            }
            // Names against names, as before, and under the rule
            // [`spelled_as`] states: two whose names are their own wire
            // values - `b` and `B` - are two codes rather than one name
            // twice, because a wire value is matched exactly. Aliases stay
            // out of it. A set is refused here only for what rendering
            // itself cannot represent, and widening this to the spellings a
            // code also answers to would refuse documents that have always
            // been legal; an alias two codes share still names neither, which
            // is [`FixCodes::merge`]'s rule and a lookup's, not a write's.
            if ordered[..index].iter().any(|held| {
                spelled_as(&held.name, &held.value, &code.name)
                    || spelled_as(&code.name, &code.value, &held.name)
            }) {
                return Err(Error::Parse {
                    target: TARGET,
                    position: 0,
                    reason: format_smolstr!("expected each name once, got {:?} twice", code.name()),
                });
            }
        }
        let mut writer = Writer::open_array();
        for code in ordered {
            code.write_into(&mut writer)?;
        }
        Ok(writer.finish())
    }

    /// The codes one canonical document states, owned.
    ///
    /// The one boundary between a code set as text - what a CLI flag, a
    /// binding argument or a hand-edited file carries - and the typed
    /// [`FixCode`] the dictionary's doors take. A caller resolves once here
    /// and hands [`FixRegistry::set_codeset`] a value whose shape is already
    /// proven.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the text is not
    /// an array of code objects, or one states a key a code does not declare.
    pub fn parse(document: &'field str) -> Result<Vec<FixCode>> {
        Self::over(Some(document))
            .map(|code| code.map(FixCode::from))
            .collect()
    }

    /// Advances one step: the next code, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixCodeValue<'field>>> {
        if !self.cursor.next_entry(&mut self.started)? {
            return Ok(None);
        }
        self.read_code().map(Some)
    }

    /// Reads the one code starting at the cursor.
    fn read_code(&mut self) -> Scan<FixCodeValue<'field>> {
        self.cursor.expect(b'{')?;
        let mut value = None;
        let mut name = None;
        let mut aliases = "";
        let mut doc = "";
        let mut group = None;
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                VALUE => value = Some(self.cursor.read_word(VALUE)?),
                NAME => name = Some(self.cursor.read_word(NAME)?),
                // Read as prose, not as a word: the specification files
                // codes under labels like `For PartyRole = "InvestorID"`, so
                // a group carries escapes exactly as a description does.
                GROUP => group = Some(self.cursor.read_string()?),
                ALIASES => aliases = self.cursor.read_words(ALIASES)?,
                _ => doc = self.cursor.read_string()?,
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let (Some(value), Some(name)) = (value, name) else {
            return Err(Refusal::MissingKey(if value.is_none() {
                VALUE
            } else {
                NAME
            }));
        };
        Ok(FixCodeValue {
            value,
            name,
            aliases,
            doc,
            group,
        })
    }

    /// Finds the one code a wire value stands for, without reading the rest.
    ///
    /// `value` leads every record, so the record a value opens is a literal
    /// byte sequence - and one that cannot occur inside a value either, since
    /// a quote inside a stored string is escaped. So the search is one pass
    /// over the bytes at `memmem` speed rather than a parse per code, and
    /// only the record it lands on is read.
    ///
    /// A value too long for the stack needle, and a hand-edited document that
    /// does not lead with `value`, both fall back to the ordinary walk, which
    /// answers the same thing more slowly.
    pub(super) fn seek_value(stored: &'field str, value: &str) -> Option<FixCodeValue<'field>> {
        let mut needle = [0_u8; NEEDLE_CAPACITY];
        let opening = br#"{"value":""#;
        let length = opening.len() + value.len() + 1;
        if length > needle.len() {
            let mut walk = Self::over(Some(stored));
            while let Some(code) = walk.next_ok() {
                if code.value() == value {
                    return Some(code);
                }
            }
            return None;
        }
        needle[..opening.len()].copy_from_slice(opening);
        needle[opening.len()..length - 1].copy_from_slice(value.as_bytes());
        needle[length - 1] = b'"';
        let at = memchr::memmem::find(stored.as_bytes(), &needle[..length])?;
        let mut cursor = Cursor::new(stored);
        cursor.seek(at);
        let mut walk = Self {
            cursor,
            started: true,
            done: false,
        };
        walk.read_code().ok()
    }

    /// The next code, answering nothing where the document does not parse.
    pub(super) fn next_ok(&mut self) -> Option<FixCodeValue<'field>> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(code)) => Some(code),
            Ok(None) | Err(_) => {
                self.done = true;
                None
            }
        }
    }
}

impl<'field> Iterator for FixCodes<'field> {
    type Item = Result<FixCodeValue<'field>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(code)) => Some(Ok(code)),
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
        // Every code costs at least `{"value":"1","name":"B"}` plus its
        // separator, which bounds how many the remaining bytes can hold.
        (0, Some(self.cursor.document().len() / 24 + 1))
    }
}

impl FusedIterator for FixCodes<'_> {}

impl From<FixCodeValue<'_>> for FixCode {
    /// Owns what a borrowed code holds, for a caller taking the set away.
    ///
    /// The description is decoded here rather than kept escaped, because an
    /// owned value has no document behind it to decode against later; a body
    /// the codec refuses keeps its escaped text, which is what arrived.
    fn from(code: FixCodeValue<'_>) -> Self {
        let mut owned = Self::new(code.name(), code.value());
        owned.aliases = code.aliases().map(SmolStr::new).collect();
        owned.description = code.doc().map(|doc| {
            code.parse_doc()
                .ok()
                .flatten()
                .map_or_else(|| SmolStr::new(doc), SmolStr::new)
        });
        owned.group = code.group().map(|group| {
            code.parse_group()
                .ok()
                .flatten()
                .map_or_else(|| SmolStr::new(group), SmolStr::new)
        });
        owned
    }
}

impl FixCodes<'_> {
    /// Folds two stored code sets by wire value, `other` winning a shared
    /// value.
    ///
    /// A code stated under a value the winner already holds is not dropped
    /// whole: its name and its own aliases become spellings on the code that
    /// stays, because a name one side declared is one the merged set has to
    /// answer to.
    ///
    /// What cannot be kept is a spelling another code already answers to,
    /// folded: two codes one spelling reaches resolve to nothing rather than
    /// to either, and two sharing a name are refused outright. So that
    /// spelling is dropped, and a code whose own *name* is taken is dropped
    /// with it, having no other name to arrive under.
    ///
    /// One exception to the winner keeping its name: a code named after its
    /// own wire value carries no name at all - it is what a source that knows
    /// the value exists but not what anyone calls it writes - so a real name
    /// from either side takes its place. That is what folds a dialect's
    /// `6 Inbound` into whatever the dictionary already calls tag 35 `6`,
    /// rather than renaming the type after the dialect's qualifier.
    ///
    /// Either side may be absent, which is what a dictionary meeting a set it
    /// does not hold has; two absences answer nothing, and a fold that keeps
    /// no member writes no document.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when either document does not parse, and what
    /// [`Self::render`] refuses the fold on.
    pub(super) fn merge(winner: Option<&str>, other: Option<&str>) -> Result<Option<String>> {
        let mut codes: Vec<FixCode> = Vec::new();
        for code in FixCodes::over(winner) {
            codes.push(FixCode::from(code?));
        }
        for code in FixCodes::over(other) {
            let incoming = FixCode::from(code?);
            // Every spelling this code arrives with, its name first, held
            // apart from the code so the code itself can move into the set.
            let mut spellings: Vec<SmolStr> = vec![SmolStr::new(incoming.name())];
            spellings.extend(incoming.aliases().iter().cloned());
            let named = !incoming.is_unnamed();
            let at = match codes
                .iter()
                .position(|held| held.value() == incoming.value())
            {
                Some(at) => {
                    // A placeholder name yields to a real one, whichever side
                    // carries it. The incoming name is a spelling either way,
                    // so it is added below like any other spelling; taking it
                    // here is only a question of which one leads.
                    if named
                        && codes[at].is_unnamed()
                        && !codes
                            .iter()
                            .enumerate()
                            .any(|(index, held)| index != at && held.is_spelled(&spellings[0]))
                    {
                        codes[at] = codes[at].clone().with_name(spellings[0].clone());
                    }
                    at
                }
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
}

/// The canonical text one stored array of codes restates.
///
/// The members keep the order the file gave them - a set is ranked by
/// position, which is the specification's business rather than this one's -
/// while each code's own keys are put back into [the order](KEYS) this
/// module's reader walks them, whatever order the file spelled them in.
///
/// # Errors
///
/// Returns [`Error::Parse`] when the value is not an array of code objects,
/// or one states a key a code does not declare.
pub(super) fn codes_text(codes: &Scalar) -> Result<String> {
    super::document::ordered_entries(TARGET, &KEYS, codes)
        .and_then(|ordered| crate::into_json_scalar(&ordered))
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

/// [`FixCodeSet::code_value`] over a stored document: the three tiers, in order.
///
/// A set states one reading of every code it declares and dates none of them,
/// so every code it holds is a candidate and there is no version to prefer by.
pub(super) fn translate<'field>(stored: &'field str, text: &str) -> Option<&'field str> {
    // Tier 1: the text as a wire value, exactly. A spelling that is already a
    // legal code is never reinterpreted as somebody's name, and the record a
    // value opens is addressed rather than searched for.
    if let Some(code) = FixCodes::seek_value(stored, text) {
        return Some(code.value());
    }
    // Tier 2: the folded symbolic name, then any alias.
    if let Some(code) = one_matching(stored, |code| code.is_spelled(text)) {
        return Some(code.value());
    }
    // Tier 3: the leading parenthesized abbreviation of the description.
    one_matching(stored, |code| {
        code.abbreviation()
            .is_some_and(|short| folds_equal(short, text))
    })
    .map(FixCodeValue::value)
}

/// One named code set a dictionary holds, borrowed from its document.
///
/// Answered by [`FixRegistry::codeset`](super::FixRegistry::codeset) and by
/// [`FixRegistry::codeset_of`](super::FixRegistry::codeset_of), which reads
/// the name a field's `FIX:codeset` states. Every read walks the stored
/// document as it goes and hands back slices of it, so nothing is parsed
/// ahead of the code being asked for and nothing is allocated.
///
/// ```
/// use yggdryl::{DataType, FixCode, FixRegistry};
/// # fn main() -> yggdryl::Result<()> {
/// let mut registry = FixRegistry::new();
/// registry.set_codeset(
///     "sidecodeset",
///     &[FixCode::new("Buy", "1"), FixCode::new("Sell", "2")],
/// )?;
/// let mut side = DataType::utf8().nullable_field("side");
/// side.as_fix_mut().set_tag(54)?;
/// side.as_fix_mut().set_codeset("sidecodeset")?;
/// registry.insert(side)?;
///
/// let set = registry.codeset_of(registry.field_by_tag(54)?).expect("the set");
/// assert_eq!(set.name(), "sidecodeset");
/// assert_eq!(set.code_value("buy"), Some("1"));
/// assert_eq!(set.code_name("2"), Some("Sell"));
/// assert_eq!(set.codes().count(), 2);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixCodeSet<'registry> {
    name: &'registry str,
    document: &'registry str,
}

impl<'registry> FixCodeSet<'registry> {
    /// The set under `name`, over the document the dictionary stores.
    pub(super) const fn new(name: &'registry str, document: &'registry str) -> Self {
        Self { name, document }
    }

    /// Returns the name the dictionary files this set under.
    #[must_use]
    pub const fn name(self) -> &'registry str {
        self.name
    }

    /// Returns the stored document, as the store writes it.
    #[must_use]
    pub const fn document(self) -> &'registry str {
        self.document
    }

    /// Walks the members, ordered by wire value.
    ///
    /// The iterator is lazy and allocates nothing: every spelling is a slice
    /// of the stored document, which the dictionary already owns.
    #[must_use]
    pub fn codes(self) -> FixCodes<'registry> {
        FixCodes::over(Some(self.document))
    }

    /// Returns the code one wire value stands for.
    ///
    /// The scan stops at the match: `value` leads each record, so this reads
    /// one key per code passed and no more.
    #[must_use]
    pub fn code(self, value: &str) -> Option<FixCodeValue<'registry>> {
        FixCodes::seek_value(self.document, value)
    }

    /// Returns the code one symbolic name or alias stands for, folded.
    ///
    /// This does **not** stop at the first match. Two codes folding to one
    /// spelling answer nothing rather than whichever the scan met first, so
    /// the whole set runs and exactly one match answers. It is affordable
    /// because [`Self::code`] is the hot path and a spelling lookup comes
    /// from human or JSON input.
    #[must_use]
    pub fn code_by_name(self, name: &str) -> Option<FixCodeValue<'registry>> {
        one_matching(self.document, |code| code.is_spelled(name))
    }

    /// Resolves any spelling of a code to its wire value.
    ///
    /// Composes the three tiers this module documents. An unresolved spelling
    /// answers `None` and the caller keeps its own text: a venue sends codes
    /// no dictionary lists, and refusing one would drop data.
    #[must_use]
    pub fn code_value(self, text: &str) -> Option<&'registry str> {
        translate(self.document, text)
    }

    /// Returns the symbolic name one wire value stands for.
    #[must_use]
    pub fn code_name(self, value: &str) -> Option<&'registry str> {
        self.code(value).map(FixCodeValue::name)
    }
}

impl FixRegistry {
    /// The code set held under `name`, folded, or nothing.
    ///
    /// The lenient door beside [`Self::codeset`], which raises absence: a
    /// caller asking whether a vocabulary is held asks this.
    #[must_use]
    pub fn get_codeset(&self, name: &str) -> Option<FixCodeSet<'_>> {
        // The stored key is the folded name and a field states it as the
        // store wrote it, so the exact hit is the ordinary one and costs no
        // allocation; a caller spelling it otherwise pays one fold.
        if let Some((held, document)) = self.codesets.get_key_value(name) {
            return Some(FixCodeSet::new(held.as_str(), document));
        }
        let folded = crate::normalized(name);
        self.codesets
            .get_key_value(folded.as_str())
            .map(|(held, document)| FixCodeSet::new(held.as_str(), document))
    }

    /// The code set held under `name`, raising absence.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the set when no such vocabulary is
    /// held.
    pub fn codeset(&self, name: &str) -> Result<FixCodeSet<'_>> {
        self.get_codeset(name)
            .ok_or_else(|| Error::absent("codesets", name))
    }

    /// The code set `field` draws its values from.
    ///
    /// The field states the name and the dictionary holds the members, so
    /// this is the one door between them. A field naming no set, and one
    /// this dictionary does not hold, both answer nothing - a held field
    /// never names a set this dictionary lacks, because registry validation
    /// refuses one at every door a field arrives through.
    #[must_use]
    pub fn codeset_of(&self, field: &Field) -> Option<FixCodeSet<'_>> {
        self.get_codeset(field.as_fix().codeset()?)
    }

    /// The document of the set `field` reads by, when this dictionary holds
    /// one.
    ///
    /// What the readers that only scan a set take, so a translation costs one
    /// name lookup rather than a set's worth of borrowing.
    pub(super) fn codes_document(&self, field: &Field) -> Option<&str> {
        self.codeset_of(field).map(FixCodeSet::document)
    }

    /// The shared document of the set `field` reads by.
    ///
    /// The memo retains this on its first ask, so the registry remains the
    /// document's one owner and remembering a field costs one reference count
    /// rather than a copy of the whole vocabulary.
    pub(super) fn codes_document_shared(&self, field: &Field) -> Option<Arc<str>> {
        let name = self.codeset_of(field)?.name();
        self.codesets.get(name).map(Arc::clone)
    }

    /// Walks every code set held, in name order.
    pub fn codesets(&self) -> impl ExactSizeIterator<Item = FixCodeSet<'_>> {
        self.codesets
            .iter()
            .map(|(name, document)| FixCodeSet::new(name.as_str(), document))
    }

    /// States the members of the code set `name`, replacing what it held.
    ///
    /// The set is filed under the folded name, which is the stem a store
    /// writes it as. Codes are rendered canonically, so one set is one text
    /// however it was built. Two names may share a value - that is an
    /// alias - but two codes may not share a name.
    ///
    /// An empty slice removes the set, exactly as an empty code list removed
    /// a field's own property before a set had a name; a set no field names
    /// is removed outright, and one a field still reads by is refused,
    /// because a field may not be left naming a vocabulary nothing states.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the name is not one a store can
    /// file, [`Error::Parse`] when two codes share a name or one states an
    /// empty value or name, and [`Error::Conflict`] when an empty slice would
    /// take away a set a held field names or any value would change the
    /// intrinsic MsgCat operation identifiers. Any of them leaves this
    /// dictionary exactly as it was.
    pub fn set_codeset(&mut self, name: &str, codes: &[FixCode]) -> Result<()> {
        let key = self.codeset_key(name)?;
        if codes.is_empty() {
            validate_intrinsic_codeset(&key, None)?;
            self.refuse_while_named(&key)?;
            if self.codesets.remove(&key).is_none() {
                return Ok(());
            }
        } else {
            let document = FixCodes::render(codes)?;
            validate_intrinsic_codeset(&key, Some(&document))?;
            if self
                .codesets
                .get(&key)
                .is_some_and(|held| held.as_ref() == document.as_str())
            {
                return Ok(());
            }
            self.codesets.insert(key, Arc::from(document.as_str()));
        }
        self.forget_codesets();
        Ok(())
    }

    /// The one code-set slot `name` currently holds, before a caller changes
    /// it as part of a larger atomic registry mutation.
    pub(super) fn codeset_checkpoint(&self, name: &str) -> Result<(SmolStr, Option<Arc<str>>)> {
        let key = self.codeset_key(name)?;
        Ok((key.clone(), self.codesets.get(&key).cloned()))
    }

    /// Restores one code-set slot from [`Self::codeset_checkpoint`].
    ///
    /// The document was already canonical when this registry held it, so a
    /// rollback neither parses nor validates it again. A slot that already
    /// answers the checkpoint is left alone, including its derived caches.
    pub(super) fn restore_codeset(&mut self, key: SmolStr, previous: Option<Arc<str>>) {
        if self.codesets.get(&key) == previous.as_ref() {
            return;
        }
        match previous {
            Some(document) => {
                self.codesets.insert(key, document);
            }
            None => {
                self.codesets.remove(&key);
            }
        }
        self.forget_codesets();
    }

    /// Folds `codes` into the code set `name`, keeping what it already held.
    ///
    /// The fold is keyed by wire value: a placeholder name yields to a real
    /// one and every surviving spelling remains an alias. So a venue's
    /// statement of a set enriches the one the dictionary holds rather than
    /// replacing it, and a set no dictionary held yet arrives whole.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set_codeset`] returns for the name and the
    /// render, including refusal to widen the intrinsic MsgCat operation
    /// identifiers, leaving this dictionary exactly as it was.
    pub fn merge_codeset(&mut self, name: &str, codes: &[FixCode]) -> Result<()> {
        let key = self.codeset_key(name)?;
        validate_intrinsic_merge(&key, codes)?;
        let incoming = FixCodes::render(codes)?;
        let held = self.codesets.get(&key).map(Arc::clone);
        let Some(merged) = FixCodes::merge(held.as_deref(), Some(incoming.as_str()))? else {
            return Ok(());
        };
        validate_intrinsic_codeset(&key, Some(&merged))?;
        self.codesets.insert(key, Arc::from(merged.as_str()));
        self.forget_codesets();
        Ok(())
    }

    /// Removes the code set `name`, answering the members it held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] naming the first field that reads by the
    /// set, because a field may not be left naming a vocabulary nothing
    /// states, and [`Error::Parse`] when the stored document does not parse,
    /// having already removed it: a document a reader refuses is one a caller
    /// asked to take away.
    pub fn remove_codeset(&mut self, name: &str) -> Result<Option<Vec<FixCode>>> {
        let key = self.codeset_key(name)?;
        validate_intrinsic_codeset(&key, None)?;
        self.refuse_while_named(&key)?;
        let Some(document) = self.codesets.remove(&key) else {
            return Ok(None);
        };
        self.forget_codesets();
        FixCodes::over(Some(&*document))
            .map(|code| code.map(FixCode::from))
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// Folds another dictionary's code sets in, name by name.
    ///
    /// Run before any field is folded, which is what keeps a merge from
    /// narrowing a vocabulary: a field keeps the set it already reads by
    /// ([`FixField::merge_with`](crate::FixField::merge_with)), so the
    /// members the other dictionary states have to be in that set by the
    /// time the field is looked at. A field only the other dictionary holds
    /// arrives naming its own set, which this fold has just added.
    ///
    /// A name both hold folds through [`FixCodes::merge`], the incoming
    /// winning a shared wire value and every surviving spelling kept, so
    /// what either side named, aliased or documented is named after the
    /// fold.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when either document does not parse.
    pub(super) fn merge_codesets(&mut self, other: &Self) -> Result<()> {
        for (name, incoming) in &other.codesets {
            match self.codesets.entry(name.clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(Arc::clone(incoming));
                }
                Entry::Occupied(mut slot) => {
                    if slot.get().as_ref() == incoming.as_ref() {
                        continue;
                    }
                    if let Some(merged) =
                        FixCodes::merge(Some(slot.get()), Some(incoming.as_ref()))?
                    {
                        validate_intrinsic_codeset(name, Some(&merged))?;
                        slot.insert(Arc::from(merged.as_str()));
                    }
                }
            }
        }
        self.forget_codesets();
        Ok(())
    }

    /// Folds the set an incoming field reads by into the one a stored field
    /// already reads by.
    ///
    /// Two dictionaries name one field's vocabulary differently - the
    /// specification calls it `SideCodeSet` and a venue's own file names it
    /// after the field - and the merged field keeps the name it already had
    /// ([`FixField::merge_with`](crate::FixField::merge_with)). Without this
    /// the members only the incoming set declared would be reachable from no
    /// field at all; with it they are in the set the field reads by before
    /// the field is folded, so a merge widens a vocabulary and never narrows
    /// one. Two fields naming one set, and a stored field naming none, are
    /// both nothing to do.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when either document does not parse.
    pub(super) fn unify_codeset(
        &mut self,
        stored: Option<&str>,
        incoming: Option<&str>,
    ) -> Result<()> {
        let (Some(stored), Some(incoming)) = (stored, incoming) else {
            return Ok(());
        };
        if folds_equal(stored, incoming) {
            return Ok(());
        }
        let Some(other) = self.get_codeset(incoming).map(FixCodeSet::document) else {
            return Ok(());
        };
        let held = self.get_codeset(stored).map(FixCodeSet::document);
        let Some(merged) = FixCodes::merge(held, Some(other))? else {
            return Ok(());
        };
        let key = self.codeset_key(stored)?;
        validate_intrinsic_codeset(&key, Some(&merged))?;
        self.codesets.insert(key, Arc::from(merged.as_str()));
        self.forget_codesets();
        Ok(())
    }

    /// Files one already-rendered document under `name`, as a store reads it.
    ///
    /// The loader's door: the document arrived canonical from a store, so it
    /// is filed rather than re-rendered, and the name is held to the one rule
    /// every stored name is held to.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the name is not one a store can
    /// file, and [`Error::Conflict`] when the name is already held with a
    /// different document. Re-reading the same canonical set is idempotent.
    pub(super) fn create_codeset(&mut self, name: &str, document: String) -> Result<()> {
        let key = self.codeset_key(name)?;
        validate_intrinsic_codeset(&key, Some(&document))?;
        match self.codesets.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert(Arc::from(document.as_str()));
                Ok(())
            }
            Entry::Occupied(slot) if slot.get().as_ref() == document.as_str() => Ok(()),
            Entry::Occupied(slot) => Err(Error::conflict(
                "one FIX code set per name",
                "a different code set under that name",
                slot.key().as_str(),
            )),
        }
    }

    /// The name a field's own set is filed under where it names none.
    ///
    /// A specification names its sets - `MsgTypeCodeSet` - and the shipped
    /// dictionary files them under those names; a field the generator met
    /// without one, and a dictionary built in memory, name the set after the
    /// field that reads by it, which is the one name the field itself
    /// supplies.
    #[must_use]
    pub fn derived_codeset_name(field: &Field) -> SmolStr {
        format_smolstr!("{}codeset", crate::normalized(field.name()))
    }

    /// The folded key `name` is filed under, refusing a name no store files.
    fn codeset_key(&self, name: &str) -> Result<SmolStr> {
        super::catalog::validate_definition_name(name)?;
        Ok(SmolStr::new(crate::normalized(name)))
    }

    /// Refuses taking away a set a held field still reads by.
    fn refuse_while_named(&self, key: &str) -> Result<()> {
        let named = self
            .scalars()
            .chain(self.catalog.all().map(|entry| entry.field.as_field()))
            .find(|field| {
                field
                    .as_fix()
                    .codeset()
                    .is_some_and(|name| folds_equal(name, key))
            });
        match named {
            Some(field) => Err(Error::conflict(
                "a FIX code set no field reads by",
                "one a field names",
                format_smolstr!("{key:?} on {:?}", field.name()),
            )),
            None => Ok(()),
        }
    }

    /// Forgets what was answered from the sets that just changed.
    ///
    /// The memo keys a translation by the document it read, and tag 35's set
    /// decides which message a bare code answers, so both are re-derived
    /// after an edit rather than answered from a set no longer held.
    fn forget_codesets(&mut self) {
        self.forget_derivations();
        self.refresh_msgtype_aliases();
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/mod_.rs` pins and a caller cannot reach.
    //!
    //! A caller states a set as codes and reads it back as codes; the document
    //! they are stored as is the store's business. These two are that document:
    //! the rendering a set persists as, and the door a load takes to file one
    //! already canonical.
    use super::FixCode;
    use crate::{FixRegistry, Result};

    /// The canonical document a set of codes persists as.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where two codes share a name, or one states an
    /// empty value or an empty name.
    pub fn render(codes: &[FixCode]) -> Result<String> {
        super::FixCodes::render(codes)
    }

    /// File an already canonical document under `name`, as a load does.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where the name is not one a store can file, or
    /// where it is already held with a different document.
    pub fn create_codeset(registry: &mut FixRegistry, name: &str, document: String) -> Result<()> {
        registry.create_codeset(name, document)
    }
}
