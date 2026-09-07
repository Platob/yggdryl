//! The FIX code set a field carries, and every spelling that reaches a value.
//!
//! Most FIX fields with a vocabulary are `int`, `Boolean` or `String` and
//! carry their members as a *code set*: a wire value, a symbolic name, and
//! usually a sentence of documentation. `Side(54)` is `1` for `Buy`; a
//! message may spell that `1`, `Buy`, `buy` or `BUY`, and a JSON or human
//! caller routinely spells it the long way. One field answers all of them.
//!
//! `fix:codes` is that vocabulary: one [canonical document](super::document)
//! ordered by wire value, read borrowed. It is a second key beside
//! [`AsciiEnum`](crate::AsciiEnum) rather than a second copy of it - that
//! type is name to ASCII value packed through the field's own width, so it
//! accepts only ASCII-width and coded datatypes and carries no description or
//! pedigree. A field may carry both and neither derives from the other.
//!
//! # Resolving a spelling
//!
//! [`FixField::code_value`](crate::FixField) composes three tiers, and a
//! spelling that reaches none falls through unchanged rather than failing:
//! a venue sends codes no dictionary lists exactly as it sends fields no
//! dictionary names.
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

use std::iter::FusedIterator;

use smol_str::{SmolStr, format_smolstr};

use super::document::{Cursor, Refusal, Scan, Words, Writer, decode_text};
use crate::types::folds_equal;
use crate::{Error, Result, Version};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix codes";

/// The one array the document holds.
const CODES: &str = "codes";

/// The wire value, and the key every lookup keys on.
const VALUE: &str = "value";
/// The symbolic name.
const NAME: &str = "name";
/// The version the specification added this code at.
const SINCE: &str = "since";
/// The extension pack that dated the addition.
const EP: &str = "ep";
/// The version the specification deprecated this code at.
const DEPRECATED: &str = "deprecated";
/// The presentation rank the specification gives this code.
const SORT: &str = "sort";
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
/// `value` leads because it is the key every lookup keys on and the one a
/// code set is ordered by, so tier 1 reads one key per record and stops.
const KEYS: [&str; 9] = [
    VALUE, NAME, SINCE, EP, DEPRECATED, SORT, GROUP, ALIASES, DOC,
];

/// One member of a FIX code set, as a caller states it.
///
/// The borrowed [`FixCodeValue`] is what a read answers; this is what a writer
/// hands [`FixFieldMut::set_codes`](crate::FixFieldMut).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixCode {
    name: SmolStr,
    value: SmolStr,
    description: Option<SmolStr>,
    aliases: Vec<SmolStr>,
    since: Option<Version>,
    deprecated: Option<Version>,
    ep: Option<u32>,
    sort: Option<u32>,
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
            since: None,
            deprecated: None,
            ep: None,
            sort: None,
            group: None,
        }
    }

    /// Sets the specification's own wording for this code.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<SmolStr>) -> Self {
        self.description = Some(description.into());
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

    /// Sets when the specification added this code.
    ///
    /// Many codes are dated by extension pack alone - `BasisPoints` is "Added
    /// EP208" rather than added in a version - so the pair is stored as real
    /// numbers and a moving label never becomes one.
    #[must_use]
    pub const fn with_since(mut self, since: Version, ep: Option<u32>) -> Self {
        self.since = Some(since);
        self.ep = ep;
        self
    }

    /// Sets when the specification deprecated this code.
    #[must_use]
    pub const fn with_deprecated(mut self, deprecated: Version) -> Self {
        self.deprecated = Some(deprecated);
        self
    }

    /// Sets the presentation rank the specification gives this code.
    #[must_use]
    pub const fn with_sort(mut self, sort: u32) -> Self {
        self.sort = Some(sort);
        self
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

    /// Whether `text` is this code's name or one of its aliases, folded.
    ///
    /// The same fold a stored set answers spellings by, so a set answers the
    /// same ones before it is rendered as after. A
    /// caller assembling one asks this to keep a spelling from reaching two
    /// codes. It is deliberately stricter than rendering refuses - the whole
    /// fold rather than ASCII case - because rendering only has to keep a
    /// document readable, while two codes one spelling reaches resolve to
    /// nothing rather than to whichever was met first.
    #[must_use]
    pub fn is_spelled(&self, text: &str) -> bool {
        folds_equal(&self.name, text) || self.aliases.iter().any(|alias| folds_equal(alias, text))
    }

    /// Returns the version the specification added this code at.
    #[must_use]
    pub const fn since(&self) -> Option<Version> {
        self.since
    }

    /// Returns the extension pack that dated the addition.
    #[must_use]
    pub const fn ep(&self) -> Option<u32> {
        self.ep
    }

    /// Returns the version the specification deprecated this code at.
    #[must_use]
    pub const fn deprecated(&self) -> Option<Version> {
        self.deprecated
    }

    /// Returns the presentation rank the specification gives this code.
    #[must_use]
    pub const fn sort(&self) -> Option<u32> {
        self.sort
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
        if let Some(since) = self.since {
            writer.text(false, SINCE, &since.to_string())?;
        }
        if let Some(ep) = self.ep {
            writer.number(false, EP, ep);
        }
        if let Some(deprecated) = self.deprecated {
            writer.text(false, DEPRECATED, &deprecated.to_string())?;
        }
        if let Some(sort) = self.sort {
            writer.number(false, SORT, sort);
        }
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
    since: Option<Version>,
    deprecated: Option<Version>,
    ep: Option<u32>,
    sort: Option<u32>,
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

    /// Returns the version the specification added this code at.
    #[must_use]
    pub const fn since(self) -> Option<Version> {
        self.since
    }

    /// Returns the extension pack that dated the addition.
    #[must_use]
    pub const fn ep(self) -> Option<u32> {
        self.ep
    }

    /// Returns the version the specification deprecated this code at.
    #[must_use]
    pub const fn deprecated(self) -> Option<Version> {
        self.deprecated
    }

    /// Returns the presentation rank the specification gives this code.
    #[must_use]
    pub const fn sort(self) -> Option<u32> {
        self.sort
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

    /// Whether this code exists at `at`.
    ///
    /// A code added later, and one deprecated at or before, are both outside
    /// the version asked for: a 4.2 message cannot resolve a name added in
    /// 4.4, and a deprecated code stops answering where the specification
    /// stopped declaring it.
    #[must_use]
    pub fn defined_at(self, at: Version) -> bool {
        self.since.is_none_or(|since| since <= at)
            && self.deprecated.is_none_or(|deprecated| at < deprecated)
    }

    /// Whether `text` is this code's name or one of its aliases, folded.
    pub(super) fn is_spelled(self, text: &str) -> bool {
        folds_equal(self.name, text) || self.aliases().any(|alias| folds_equal(alias, text))
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
/// Answered by [`FixField::codes`](crate::FixField). It walks the stored
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
    /// Walks one stored `fix:codes` value, or nothing for an absent one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Renders codes into the one canonical document they have.
    ///
    /// Ordering is by wire value, so one code set is one text however it was
    /// built. Two names may share a value - that is an alias, the rule
    /// `AsciiEnum` already states - but two codes may not share a name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when two codes share a name, or when one
    /// states an empty value or an empty name.
    pub(super) fn render(codes: &[FixCode]) -> Result<String> {
        let mut ordered: Vec<&FixCode> = codes.iter().collect();
        ordered.sort_by(|left, right| {
            left.value
                .cmp(&right.value)
                .then(left.name.cmp(&right.name))
        });
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
            if ordered[..index]
                .iter()
                .any(|held| held.name.eq_ignore_ascii_case(&code.name))
            {
                return Err(Error::Parse {
                    target: TARGET,
                    position: 0,
                    reason: format_smolstr!("expected each name once, got {:?} twice", code.name()),
                });
            }
        }
        let mut writer = Writer::open_array(CODES);
        for code in ordered {
            code.write_into(&mut writer)?;
        }
        writer.close_array();
        Ok(writer.finish())
    }

    /// Advances one step: the next code, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixCodeValue<'field>>> {
        if !self.started {
            self.started = true;
            if !self.cursor.open_array(CODES)? {
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
        let mut since = None;
        let mut deprecated = None;
        let mut ep = None;
        let mut sort = None;
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                VALUE => value = Some(self.cursor.read_word(VALUE)?),
                NAME => name = Some(self.cursor.read_word(NAME)?),
                SINCE => since = Some(self.cursor.read_version(SINCE)?),
                EP => ep = Some(self.cursor.read_number(EP)?),
                DEPRECATED => deprecated = Some(self.cursor.read_version(DEPRECATED)?),
                SORT => sort = Some(self.cursor.read_number(SORT)?),
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
            since,
            deprecated,
            ep,
            sort,
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
        owned.group = code.group().map(SmolStr::new);
        owned.since = code.since();
        owned.deprecated = code.deprecated();
        owned.ep = code.ep();
        owned.sort = code.sort();
        owned
    }
}
