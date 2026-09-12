//! How a value of one field is restated at a later FIX version.
//!
//! The specification retires a field or a value and says what stands in for
//! it: `Rule80A(47)` became `OrderCapacity(528)` beside
//! `OrderRestrictions(529)`, the partial-fill values of `ExecType(150)`
//! folded into `Trade`, `ExecBroker(76)` became one `Parties` occurrence
//! with role `1`. Those rules are facts about the field being restated, so
//! they travel on it: a registry adds or edits one by editing metadata, and
//! nothing in Rust holds a table of them.
//!
//! `fix:replacements` is that document: one [canonical
//! document](super::document) of entries in **document order**, read
//! borrowed. Order is semantic: the first entry whose `when` matches a held
//! value answers, so a catch-all entry stating no `when` comes last. Each
//! entry states when the specification replaced the feature, where it
//! applies - which message types, inside which repeating groups, for which
//! held value - and the *fills*: the fields that take a value and what value
//! they take. A fill is the source field's own value re-typed, a constant,
//! another tag's value at the same level, the texts of several tags joined,
//! or one occurrence of a repeating group whose members are fills of their
//! own.
//!
//! What a reader does with an entry - matching `when`, refusing to overwrite
//! a stated value, creating a group occurrence - is the reader's; this
//! module owns what the document says and the one text it says it in.

use std::iter::FusedIterator;

use smol_str::{SmolStr, format_smolstr};

use super::document::{Cursor, Numbers, Refusal, Scan, Words, Writer, decode_text};
use crate::{Error, Result, Version};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix replacements";

/// The one array the document holds.
const REPLACEMENTS: &str = "replacements";

/// The version the specification replaced the feature at; required.
const SINCE: &str = "since";
/// The extension pack that dated the replacement.
const EP: &str = "ep";
/// The message types the entry applies to; absent is every message.
const MSGTYPES: &str = "msgtypes";
/// The repeating groups the source must sit inside; absent is anywhere.
const IN: &str = "in";
/// The held wire value the entry applies to; absent is any stated value.
const WHEN: &str = "when";
/// The fields filled and the values they take; required, at least one.
const FILLS: &str = "fills";
/// The specification's own wording of the mapping.
const DOC: &str = "doc";

/// The target tag of a field fill.
const TAG: &str = "tag";
/// A constant the target takes.
const VALUE: &str = "value";
/// Another tag at the same level whose value the target takes.
const FROM: &str = "from";
/// The tags whose texts, concatenated, the target takes.
const JOIN: &str = "join";
/// The repeating group one occurrence is filled of.
const GROUP: &str = "group";
/// The fills making up that occurrence.
const MEMBERS: &str = "members";

/// The keys one entry may state, in the order it states them.
///
/// `since` leads because it is what a future version filter keys on, and
/// `fills` sits after every condition so a reader that finds the entry does
/// not apply can stop before the one key that costs a nested walk.
const KEYS: [&str; 7] = [SINCE, EP, MSGTYPES, IN, WHEN, FILLS, DOC];

/// The keys one fill may state, in the order it states them.
///
/// Exactly one of `tag` and `group` is stated; `value`, `from` and `join`
/// travel only with `tag` and at most one of them, `members` only with
/// `group`.
const FILL_KEYS: [&str; 6] = [TAG, VALUE, FROM, JOIN, GROUP, MEMBERS];

/// Where a filled field's value comes from, as a caller states it.
///
/// The borrowed [`FixFillValue`] is what a read answers; this is what a
/// writer hands [`FixFill::Field`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixFillSource {
    /// The source field's own value, re-typed for the target.
    Source,
    /// A constant, as wire text.
    Constant(SmolStr),
    /// Another tag's value at the same level.
    From(i32),
    /// The texts of these tags concatenated, in this order.
    ///
    /// An integer part is spelled with two digits, which is how a day
    /// completes a month-year; every part must be stated for the join to
    /// answer.
    Join(Vec<i32>),
}

/// One field a replacement fills, as a caller states it.
///
/// The borrowed [`FixFillEntry`] is what a read answers; this is what a
/// writer hands [`FixReplacement::with_fills`]. A group fill nests: its
/// members are fills, and a member may itself be a group.
///
/// ```
/// use yggdryl::fix::{FixFill, FixFillSource};
///
/// let role = FixFill::Field { tag: 452, value: FixFillSource::Constant("1".into()) };
/// let party = FixFill::Group { name: "parties".into(), members: vec![
///     FixFill::Field { tag: 448, value: FixFillSource::Source },
///     role,
/// ] };
/// assert!(matches!(party, FixFill::Group { ref members, .. } if members.len() == 2));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixFill {
    /// One field at the level the entry applies to.
    Field {
        /// The target tag.
        tag: i32,
        /// Where its value comes from.
        value: FixFillSource,
    },
    /// One occurrence of a repeating group at that level.
    Group {
        /// The group's canonical folded name.
        name: SmolStr,
        /// The occurrence's members, fills of their own.
        members: Vec<FixFill>,
    },
}

impl FixFill {
    /// Holds this fill to what the document can state.
    fn validate(&self) -> Result<()> {
        match self {
            Self::Field { tag, value } => {
                if *tag < 0 {
                    return Err(refused(format_smolstr!("expected a FIX tag, got {tag}")));
                }
                match value {
                    FixFillSource::Source => {}
                    FixFillSource::Constant(constant) => word(VALUE, constant)?,
                    FixFillSource::From(from) => {
                        if *from < 0 {
                            return Err(refused(format_smolstr!("expected a FIX tag, got {from}")));
                        }
                    }
                    FixFillSource::Join(tags) => {
                        if tags.len() < 2 {
                            return Err(refused(format_smolstr!(
                                "expected {JOIN:?} to name at least two tags, got {}",
                                tags.len()
                            )));
                        }
                        if let Some(tag) = tags.iter().find(|tag| **tag < 0) {
                            return Err(refused(format_smolstr!("expected a FIX tag, got {tag}")));
                        }
                    }
                }
                Ok(())
            }
            Self::Group { name, members } => {
                word(GROUP, name)?;
                if members.is_empty() {
                    return Err(refused(format_smolstr!(
                        "expected every group fill to state at least one member, {name:?} states none"
                    )));
                }
                members.iter().try_for_each(Self::validate)
            }
        }
    }

    /// Renders this fill into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        match self {
            Self::Field { tag, value } => {
                writer.number(true, TAG, tag);
                match value {
                    FixFillSource::Source => {}
                    FixFillSource::Constant(constant) => writer.text(false, VALUE, constant)?,
                    FixFillSource::From(from) => writer.number(false, FROM, from),
                    FixFillSource::Join(tags) => writer.numbers(false, JOIN, tags.iter().copied()),
                }
            }
            Self::Group { name, members } => {
                writer.text(true, GROUP, name)?;
                writer.open_list(false, MEMBERS);
                for member in members {
                    member.write_into(writer)?;
                }
                writer.close_list();
            }
        }
        writer.close_element();
        Ok(())
    }
}

/// One rule restating a value of the field carrying it, as a caller states
/// it.
///
/// The borrowed [`FixReplacementEntry`] is what a read answers; this is what
/// a writer hands [`FixFieldMut::set_replacements`](crate::FixFieldMut).
///
/// ```
/// use yggdryl::fix::{FixFill, FixFillSource, FixReplacement};
/// use yggdryl::Version;
///
/// # fn main() -> yggdryl::Result<()> {
/// // Rule80A(47) `A` is an agency order: OrderCapacity(528) takes `A`.
/// let agency = FixReplacement::new("4.3".parse::<Version>()?)
///     .with_when("A")
///     .with_fills([FixFill::Field { tag: 528, value: FixFillSource::Constant("A".into()) }]);
/// assert_eq!(agency.since(), "4.3".parse::<Version>()?);
/// assert_eq!(agency.when(), Some("A"));
/// assert_eq!(agency.fills().len(), 1);
/// assert!(agency.msgtypes().is_empty(), "every message");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixReplacement {
    since: Version,
    ep: Option<u32>,
    msgtypes: Vec<SmolStr>,
    in_groups: Vec<SmolStr>,
    when: Option<SmolStr>,
    fills: Vec<FixFill>,
    doc: Option<SmolStr>,
}

impl FixReplacement {
    /// Builds one rule from the version the specification replaced the
    /// feature at.
    ///
    /// It states no fill yet, and a rule stating none is refused when
    /// written: [`Self::with_fills`] is what makes it one.
    #[must_use]
    pub const fn new(since: Version) -> Self {
        Self {
            since,
            ep: None,
            msgtypes: Vec::new(),
            in_groups: Vec::new(),
            when: None,
            fills: Vec::new(),
            doc: None,
        }
    }

    /// Sets the extension pack that dated the replacement.
    #[must_use]
    pub const fn with_ep(mut self, ep: u32) -> Self {
        self.ep = Some(ep);
        self
    }

    /// Restricts the rule to messages whose `MsgType(35)` is one of these.
    ///
    /// An empty list is every message, which is also what an absent key
    /// means, so the two have one stored form.
    #[must_use]
    pub fn with_msgtypes<I, S>(mut self, msgtypes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        self.msgtypes = msgtypes.into_iter().map(Into::into).collect();
        self
    }

    /// Restricts the rule to a source sitting inside an occurrence of one of
    /// these repeating groups, by canonical folded name.
    ///
    /// An empty list is wherever the field sits.
    #[must_use]
    pub fn with_in<I, S>(mut self, groups: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        self.in_groups = groups.into_iter().map(Into::into).collect();
        self
    }

    /// Restricts the rule to one held value, as wire text.
    ///
    /// A rule stating none applies to any stated value, so it is the
    /// catch-all and comes last in the document.
    #[must_use]
    pub fn with_when(mut self, when: impl Into<SmolStr>) -> Self {
        self.when = Some(when.into());
        self
    }

    /// States the fields this rule fills and the values they take.
    #[must_use]
    pub fn with_fills<I>(mut self, fills: I) -> Self
    where
        I: IntoIterator<Item = FixFill>,
    {
        self.fills = fills.into_iter().collect();
        self
    }

    /// Sets the specification's own wording of the mapping.
    #[must_use]
    pub fn with_doc(mut self, doc: impl Into<SmolStr>) -> Self {
        self.doc = Some(doc.into());
        self
    }

    /// Returns the version the specification replaced the feature at.
    #[must_use]
    pub const fn since(&self) -> Version {
        self.since
    }

    /// Returns the extension pack that dated the replacement.
    #[must_use]
    pub const fn ep(&self) -> Option<u32> {
        self.ep
    }

    /// Returns the message types the rule applies to; empty is every message.
    #[must_use]
    pub fn msgtypes(&self) -> &[SmolStr] {
        &self.msgtypes
    }

    /// Returns the repeating groups the source must sit inside; empty is
    /// wherever it sits.
    #[must_use]
    pub fn in_groups(&self) -> &[SmolStr] {
        &self.in_groups
    }

    /// Returns the held wire value the rule applies to; `None` is any.
    #[must_use]
    pub fn when(&self) -> Option<&str> {
        self.when.as_deref()
    }

    /// Returns the fields this rule fills.
    #[must_use]
    pub fn fills(&self) -> &[FixFill] {
        &self.fills
    }

    /// Returns the specification's own wording of the mapping.
    #[must_use]
    pub fn doc(&self) -> Option<&str> {
        self.doc.as_deref()
    }

    /// Holds this rule to what the document can state.
    fn validate(&self) -> Result<()> {
        if self.fills.is_empty() {
            return Err(refused(format_smolstr!(
                "expected every entry to state at least one fill, the one since {} states none",
                self.since
            )));
        }
        for msgtype in &self.msgtypes {
            super::msgtype::validate_code(msgtype)?;
            word(MSGTYPES, msgtype)?;
        }
        for group in &self.in_groups {
            word(IN, group)?;
        }
        if let Some(when) = self.when() {
            word(WHEN, when)?;
        }
        self.fills.iter().try_for_each(FixFill::validate)
    }

    /// Renders this rule into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, SINCE, &self.since.to_string())?;
        if let Some(ep) = self.ep {
            writer.number(false, EP, ep);
        }
        if !self.msgtypes.is_empty() {
            writer.words(false, MSGTYPES, self.msgtypes.iter().map(SmolStr::as_str))?;
        }
        if !self.in_groups.is_empty() {
            writer.words(false, IN, self.in_groups.iter().map(SmolStr::as_str))?;
        }
        if let Some(when) = self.when() {
            writer.text(false, WHEN, when)?;
        }
        writer.open_list(false, FILLS);
        for fill in &self.fills {
            fill.write_into(writer)?;
        }
        writer.close_list();
        if let Some(doc) = self.doc() {
            writer.text(false, DOC, doc)?;
        }
        writer.close_element();
        Ok(())
    }
}

/// A refusal the writer raises, before anything is written.
fn refused(reason: SmolStr) -> Error {
    Error::Parse {
        target: TARGET,
        position: 0,
        reason,
    }
}

/// Holds one text to what the reader reads back as a word.
///
/// A message type, a group name, a held value and a constant are read with
/// [`Cursor::read_word`], which refuses an escape, so a text the writer would
/// have to escape is refused here instead of being stored unreadable.
fn word(key: &'static str, text: &str) -> Result<()> {
    if text.is_empty()
        || text
            .bytes()
            .any(|byte| byte < 0x20 || matches!(byte, b'"' | b'\\'))
    {
        return Err(refused(format_smolstr!(
            "expected {key:?} to hold a non-empty word without quote, backslash or control character, got {text:?}"
        )));
    }
    Ok(())
}

/// Where a filled field's value comes from, borrowed from the stored
/// document.
#[derive(Clone, Debug)]
pub enum FixFillValue<'field> {
    /// The source field's own value, re-typed for the target.
    Source,
    /// A constant, as wire text.
    Constant(&'field str),
    /// Another tag's value at the same level.
    From(i32),
    /// The texts of these tags concatenated, in this order.
    Join(Numbers<'field>),
}

/// One field a replacement fills, borrowed from the stored document.
///
/// Answered by [`FixFills`]. A group fill hands back its members as a walk
/// of their own over the slice of the document holding them, so a nested
/// occurrence costs nothing until it is read.
#[derive(Clone, Debug)]
pub enum FixFillEntry<'field> {
    /// One field at the level the entry applies to.
    Field {
        /// The target tag.
        tag: i32,
        /// Where its value comes from.
        value: FixFillValue<'field>,
    },
    /// One occurrence of a repeating group at that level.
    Group {
        /// The group's canonical folded name.
        name: &'field str,
        /// The occurrence's members, fills of their own.
        members: FixFills<'field>,
    },
}

/// One rule restating a value of the field carrying it, borrowed from the
/// stored document.
///
/// Every key beyond [`Self::since`] and [`Self::fills`] is optional, and the
/// borrowed spellings are slices of the field's own stored document, so
/// reading them allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixReplacementEntry<'field> {
    since: Version,
    ep: Option<u32>,
    msgtypes: &'field str,
    in_groups: &'field str,
    when: Option<&'field str>,
    fills: &'field str,
    doc: &'field str,
}

impl<'field> FixReplacementEntry<'field> {
    /// Returns the version the specification replaced the feature at.
    #[must_use]
    pub const fn since(self) -> Version {
        self.since
    }

    /// Returns the extension pack that dated the replacement.
    #[must_use]
    pub const fn ep(self) -> Option<u32> {
        self.ep
    }

    /// Walks the message types the rule applies to; nothing is every message.
    #[must_use]
    pub const fn msgtypes(self) -> Words<'field> {
        Words::over(self.msgtypes)
    }

    /// Walks the repeating groups the source must sit inside; nothing is
    /// wherever it sits.
    #[must_use]
    pub const fn in_groups(self) -> Words<'field> {
        Words::over(self.in_groups)
    }

    /// Returns the held wire value the rule applies to; `None` is any.
    #[must_use]
    pub const fn when(self) -> Option<&'field str> {
        self.when
    }

    /// Walks the fields this rule fills.
    ///
    /// The entry's own read already held every fill to the grammar, so this
    /// walk never refuses; it keeps the fallible shape every document walk
    /// has, and [`FixFills::next_ok`] is the infallible reader.
    #[must_use]
    pub const fn fills(self) -> FixFills<'field> {
        FixFills::over(self.fills)
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
}

/// The fills one entry or one group occurrence states, in document order.
///
/// Answered by [`FixReplacementEntry::fills`] and by a group
/// [`FixFillEntry`]. It walks the slice of the stored document holding the
/// fills and hands back slices of it, so nothing is allocated. Byte
/// positions in a refusal count from the start of that slice.
#[derive(Clone, Debug)]
pub struct FixFills<'field> {
    cursor: Cursor<'field>,
    started: bool,
    done: bool,
}

impl<'field> FixFills<'field> {
    /// Walks one list body a reader answered.
    pub(super) const fn over(body: &'field str) -> Self {
        Self {
            cursor: Cursor::new(body),
            started: false,
            done: false,
        }
    }

    /// Advances one step: the next fill, the list's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixFillEntry<'field>>> {
        if !self.started {
            self.started = true;
            if self.cursor.is_done() {
                return Ok(None);
            }
        } else if self.cursor.is_done() {
            return Ok(None);
        } else {
            self.cursor.expect(b',')?;
        }
        read_fill(&mut self.cursor).map(Some)
    }

    /// The next fill, answering nothing where the list does not parse.
    ///
    /// A list an entry handed back was read whole when the entry was, so
    /// this only ever answers `None` at its end; a list built over a hand
    /// edit degrades to no answer rather than a wrong one, without
    /// allocating.
    ///
    /// ```
    /// use yggdryl::fix::{FixFill, FixFillEntry, FixFillSource, FixReplacement};
    /// use yggdryl::{DataType, Version};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut field = DataType::utf8().nullable_field("relatdsym");
    /// field.as_fix_mut().set_tag(46)?;
    /// field.as_fix_mut().set_replacements(&[FixReplacement::new("4.3".parse::<Version>()?)
    ///     .with_fills([FixFill::Field { tag: 55, value: FixFillSource::Source }])])?;
    ///
    /// let entry = field.as_fix().replacements().next_ok().expect("one rule");
    /// let mut fills = entry.fills();
    /// assert!(matches!(fills.next_ok(), Some(FixFillEntry::Field { tag: 55, .. })));
    /// assert!(fills.next_ok().is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn next_ok(&mut self) -> Option<FixFillEntry<'field>> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(fill)) => Some(fill),
            Ok(None) | Err(_) => {
                self.done = true;
                None
            }
        }
    }
}

impl<'field> Iterator for FixFills<'field> {
    type Item = Result<FixFillEntry<'field>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(fill)) => Some(Ok(fill)),
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
        // Every fill costs at least `{"tag":1}` plus its separator, which
        // bounds how many the remaining bytes can hold.
        (0, Some(self.cursor.document().len() / 10 + 1))
    }
}

impl FusedIterator for FixFills<'_> {}

/// Reads one tag-valued key.
fn read_tag<'doc>(cursor: &mut Cursor<'doc>, key: &'static str) -> Scan<i32> {
    let at = cursor.position();
    i32::try_from(cursor.read_number(key)?).map_err(|_| {
        cursor.seek(at);
        Refusal::TooWide(key)
    })
}

/// Reads one list of fills, handing back its body once every fill in it has
/// been read.
///
/// Fills are read here rather than skipped so a hand-edited document is
/// refused at its own byte by the entry's read, and the walk an entry hands
/// back afterwards never fails. A list stating no fill is refused too: an
/// entry filling nothing restates nothing, and the writer never produces one.
fn read_fills<'doc>(cursor: &mut Cursor<'doc>, key: &'static str) -> Scan<&'doc str> {
    let opening = cursor.position();
    let body = cursor.read_list()?;
    let mut walk = FixFills::over(body);
    let mut count = 0_usize;
    loop {
        match walk.step() {
            Ok(Some(_)) => count += 1,
            Ok(None) => break,
            Err(refusal) => {
                cursor.seek(opening + 1 + walk.cursor.position());
                return Err(refusal);
            }
        }
    }
    if count == 0 {
        cursor.seek(opening);
        return Err(Refusal::Short(key, 1));
    }
    Ok(body)
}

/// Reads the one fill starting at the cursor.
fn read_fill<'doc>(cursor: &mut Cursor<'doc>) -> Scan<FixFillEntry<'doc>> {
    cursor.expect(b'{')?;
    let mut tag = None;
    let mut value = None;
    let mut from = None;
    let mut join = None;
    let mut group = None;
    let mut members = None;
    let mut next = 0;
    loop {
        match FILL_KEYS[cursor.read_key(&FILL_KEYS, &mut next)?] {
            TAG => tag = Some(read_tag(cursor, TAG)?),
            VALUE => value = Some(cursor.read_word(VALUE)?),
            FROM => from = Some(read_tag(cursor, FROM)?),
            JOIN => {
                let at = cursor.position();
                let body = cursor.read_numbers(JOIN)?;
                if Numbers::over(body).count() < 2 {
                    cursor.seek(at);
                    return Err(Refusal::Short(JOIN, 2));
                }
                join = Some(body);
            }
            GROUP => group = Some(cursor.read_word(GROUP)?),
            _ => members = Some(read_fills(cursor, MEMBERS)?),
        }
        if !cursor.next_property()? {
            break;
        }
    }
    match (tag, group) {
        (Some(_), Some(_)) => Err(Refusal::Together(TAG, GROUP)),
        (None, None) => Err(Refusal::MissingKey(TAG)),
        (Some(tag), None) => {
            if members.is_some() {
                return Err(Refusal::Together(TAG, MEMBERS));
            }
            let value = match (value, from, join) {
                (None, None, None) => FixFillValue::Source,
                (Some(constant), None, None) => FixFillValue::Constant(constant),
                (None, Some(from), None) => FixFillValue::From(from),
                (None, None, Some(join)) => FixFillValue::Join(Numbers::over(join)),
                (Some(_), Some(_), _) => return Err(Refusal::Together(VALUE, FROM)),
                (Some(_), None, Some(_)) => return Err(Refusal::Together(VALUE, JOIN)),
                (None, Some(_), Some(_)) => return Err(Refusal::Together(FROM, JOIN)),
            };
            Ok(FixFillEntry::Field { tag, value })
        }
        (None, Some(name)) => {
            for (held, key) in [
                (value.is_some(), VALUE),
                (from.is_some(), FROM),
                (join.is_some(), JOIN),
            ] {
                if held {
                    return Err(Refusal::Together(GROUP, key));
                }
            }
            let Some(members) = members else {
                return Err(Refusal::MissingKey(MEMBERS));
            };
            Ok(FixFillEntry::Group {
                name,
                members: FixFills::over(members),
            })
        }
    }
}

/// A field's replacement rules, in document order.
///
/// Answered by [`FixField::replacements`](crate::FixField). It walks the
/// stored document as it goes and hands back slices of it, so nothing is
/// parsed ahead of the entry being asked for and nothing is allocated. An
/// absent property yields nothing, which is what a field the specification
/// never replaced answers.
#[derive(Clone, Debug)]
pub struct FixReplacements<'field> {
    cursor: Cursor<'field>,
    started: bool,
    done: bool,
}

impl<'field> FixReplacements<'field> {
    /// Walks one stored `fix:replacements` value, or nothing for an absent
    /// one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Renders rules into the one canonical document they have.
    ///
    /// Order is kept, because it is what the document says: the first entry
    /// whose `when` matches answers.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when an entry states no fill, a group fill
    /// states no member, a join names fewer than two tags, a tag is
    /// negative, or a message type, group name, held value or constant is
    /// not a word the reader reads back.
    pub(super) fn render(entries: &[FixReplacement]) -> Result<String> {
        entries.iter().try_for_each(FixReplacement::validate)?;
        let mut writer = Writer::open_array(REPLACEMENTS);
        for entry in entries {
            entry.write_into(&mut writer)?;
        }
        writer.close_array();
        Ok(writer.finish())
    }

    /// Advances one step: the next entry, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixReplacementEntry<'field>>> {
        if !self.started {
            self.started = true;
            if !self.cursor.open_array(REPLACEMENTS)? {
                self.cursor.expect(b'}')?;
                if !self.cursor.is_done() {
                    return Err(Refusal::Trailing);
                }
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
    fn read_entry(&mut self) -> Scan<FixReplacementEntry<'field>> {
        self.cursor.expect(b'{')?;
        let mut since = None;
        let mut ep = None;
        let mut msgtypes = "";
        let mut in_groups = "";
        let mut when = None;
        let mut fills = None;
        let mut doc = "";
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                SINCE => since = Some(self.cursor.read_version(SINCE)?),
                EP => ep = Some(self.cursor.read_number(EP)?),
                MSGTYPES => msgtypes = self.cursor.read_words(MSGTYPES)?,
                IN => in_groups = self.cursor.read_words(IN)?,
                WHEN => when = Some(self.cursor.read_word(WHEN)?),
                FILLS => fills = Some(read_fills(&mut self.cursor, FILLS)?),
                _ => doc = self.cursor.read_string()?,
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let (Some(since), Some(fills)) = (since, fills) else {
            return Err(Refusal::MissingKey(if since.is_none() {
                SINCE
            } else {
                FILLS
            }));
        };
        Ok(FixReplacementEntry {
            since,
            ep,
            msgtypes,
            in_groups,
            when,
            fills,
            doc,
        })
    }

    /// The next entry, answering nothing where the document does not parse.
    ///
    /// This is what every infallible reader walks, so a malformed document
    /// resolves to no answer rather than to a wrong one - the same
    /// degradation a name-digest collision takes on a registry read - and it
    /// spends no allocation doing it. The [`Iterator`] the same walk
    /// implements is the fallible door, naming the byte a hand edit stopped
    /// it at.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut field = DataType::utf8().nullable_field("rule80a");
    /// field.set_metadata([("fix:replacements", r#"{"replacements":[{"fills":[{"tag":528}]}]}"#)])?;
    /// // The entry states no `since`, which the grammar requires.
    /// assert!(field.as_fix().replacements().next_ok().is_none());
    /// assert!(field.as_fix().replacements().next().expect("a refusal").is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn next_ok(&mut self) -> Option<FixReplacementEntry<'field>> {
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

impl<'field> Iterator for FixReplacements<'field> {
    type Item = Result<FixReplacementEntry<'field>>;

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
        // Every entry costs at least `{"since":"0","fills":[{"tag":1}]}`
        // plus its separator, which bounds how many the remaining bytes can
        // hold.
        (0, Some(self.cursor.document().len() / 34 + 1))
    }
}

impl FusedIterator for FixReplacements<'_> {}

impl From<FixFillEntry<'_>> for FixFill {
    /// Owns what a borrowed fill holds, members included.
    fn from(fill: FixFillEntry<'_>) -> Self {
        match fill {
            FixFillEntry::Field { tag, value } => Self::Field {
                tag,
                value: match value {
                    FixFillValue::Source => FixFillSource::Source,
                    FixFillValue::Constant(constant) => {
                        FixFillSource::Constant(SmolStr::new(constant))
                    }
                    FixFillValue::From(from) => FixFillSource::From(from),
                    FixFillValue::Join(tags) => FixFillSource::Join(tags.collect()),
                },
            },
            FixFillEntry::Group { name, mut members } => {
                let mut owned = Vec::new();
                while let Some(member) = members.next_ok() {
                    owned.push(Self::from(member));
                }
                Self::Group {
                    name: SmolStr::new(name),
                    members: owned,
                }
            }
        }
    }
}

impl From<FixReplacementEntry<'_>> for FixReplacement {
    /// Owns what a borrowed entry holds, for a caller taking the rules away.
    ///
    /// The wording is decoded here rather than kept escaped, because an owned
    /// value has no document behind it to decode against later; a body the
    /// codec refuses keeps its escaped text, which is what arrived.
    fn from(entry: FixReplacementEntry<'_>) -> Self {
        let mut owned = Self::new(entry.since());
        owned.ep = entry.ep();
        owned.msgtypes = entry.msgtypes().map(SmolStr::new).collect();
        owned.in_groups = entry.in_groups().map(SmolStr::new).collect();
        owned.when = entry.when().map(SmolStr::new);
        let mut fills = entry.fills();
        while let Some(fill) = fills.next_ok() {
            owned.fills.push(FixFill::from(fill));
        }
        owned.doc = entry.doc().map(|doc| {
            entry
                .parse_doc()
                .ok()
                .flatten()
                .map_or_else(|| SmolStr::new(doc), SmolStr::new)
        });
        owned
    }
}
