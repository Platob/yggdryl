//! The rules that name a code of tag 385's set from the prose in front of a
//! payload, as the dictionary states them.
//!
//! A transport writes which way a message moved in front of it -
//! `sending >>`, `recv <<`, `[OUT]`, a Jolokia `Response:` - and which words
//! mean which code is a fact about the code set, so it travels on the field
//! that declares the set (decision 15). `fix:directions` is that document: one
//! [canonical document](super::document) with an entry per code, in the
//! order the dictionary lists them, each holding the `regex::bytes` patterns
//! applied to the prefix. A code matches where any of its patterns matches;
//! exactly one matching code names the direction, and two or none name
//! nothing. Nothing in Rust holds the table but the defaults
//! [`MsgDirection`](super::MsgDirection) answers where the field carries no
//! property, and those are data a caller can read.
//!
//! What a reader does with the rules - compiling them once, scanning a
//! prefix - is the reader's; this module owns what the document says and the
//! one text it says it in.

use std::iter::FusedIterator;

use smol_str::{SmolStr, format_smolstr};

use super::document::{Cursor, Refusal, Scan, Writer, decode_text};
use crate::{Error, Result};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix directions";

/// The one array the document holds.
const DIRECTIONS: &str = "directions";

/// The code of tag 385's set the entry names; required, and the key every
/// lookup keys on.
const CODE: &str = "code";
/// The `regex::bytes` patterns that name it; required, at least one.
const PATTERNS: &str = "patterns";

/// The keys one entry states, in the order it states them.
const KEYS: [&str; 2] = [CODE, PATTERNS];

/// One rule naming a code of tag 385's set, as a caller states it.
///
/// The borrowed [`FixDirectionEntry`] is what a read answers; this is what
/// a writer hands [`FixFieldMut::set_directions`](crate::FixFieldMut) and
/// what [`MsgDirection::directions`](super::MsgDirection::directions)
/// answers as the rules in force.
///
/// ```
/// use yggdryl::fix::FixDirection;
///
/// let sent = FixDirection::new("S", ["^TX ", r"(?i)\bsent\b"]);
/// assert_eq!(sent.code(), "S");
/// assert_eq!(sent.patterns().len(), 2);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixDirection {
    code: SmolStr,
    patterns: Vec<SmolStr>,
}

impl FixDirection {
    /// Builds one rule from the code it names and the patterns that name it.
    ///
    /// The code is any spelling of a code of the set - the value or the
    /// name - and the reading resolves it once; a rule stating no pattern is
    /// refused when written.
    #[must_use]
    pub fn new<I, S>(code: impl Into<SmolStr>, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        Self {
            code: code.into(),
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns the code this rule names, as stated.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the patterns that name it, in the order they were stated.
    #[must_use]
    pub fn patterns(&self) -> &[SmolStr] {
        &self.patterns
    }

    /// Holds this rule to what the document can state.
    fn validate(&self) -> Result<()> {
        word(CODE, &self.code)?;
        if self.patterns.is_empty() {
            return Err(refused(format_smolstr!(
                "expected every entry to state at least one pattern, {:?} states none",
                self.code
            )));
        }
        for pattern in &self.patterns {
            compile(pattern)?;
        }
        Ok(())
    }

    /// Renders this rule into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, CODE, &self.code)?;
        writer.words(false, PATTERNS, self.patterns.iter().map(SmolStr::as_str))?;
        writer.close_element();
        Ok(())
    }
}

/// Compiles one pattern as the reading will, so the setter refuses what the
/// reading could not use.
///
/// # Errors
///
/// Returns [`Error::Parse`] quoting the pattern and the regex crate's own
/// reason when the pattern is empty or does not compile.
pub(super) fn compile(pattern: &str) -> Result<regex::bytes::Regex> {
    if pattern.is_empty() {
        return Err(refused(SmolStr::new_static(
            "expected every pattern to hold a byte regex, got an empty one",
        )));
    }
    regex::bytes::Regex::new(pattern).map_err(|error| {
        refused(format_smolstr!(
            "expected a valid byte regex, got {pattern:?}: {error}"
        ))
    })
}

/// The refusal a rule naming no code of the set earns: the setter's, and
/// the reading's warning where a hand edit slipped one past the door.
pub(super) fn outside_set<'a>(code: &str, set: impl Iterator<Item = &'a str>) -> Error {
    let codes: Vec<&str> = set.collect();
    refused(format_smolstr!(
        "expected a code of the set, one of {}, got {code:?}",
        codes.join(", ")
    ))
}

/// The refusal a second rule naming a code already named earns, under any
/// spelling: the setter's, and the reading's warning past the door.
pub(super) fn repeated(spelling: &str, code: &str) -> Error {
    refused(format_smolstr!(
        "expected each code once, got {spelling:?} naming {code:?} twice"
    ))
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
/// A code is read with [`Cursor::read_word`], which refuses an escape, so a
/// text the writer would have to escape is refused here instead of being
/// stored unreadable.
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

/// One rule naming a code of tag 385's set, borrowed from the stored
/// document.
///
/// The code is a slice of the field's own stored document and the patterns
/// a walk over another, so reading an entry allocates nothing; a pattern is
/// decoded only where a caller asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixDirectionEntry<'field> {
    code: &'field str,
    patterns: &'field str,
}

impl<'field> FixDirectionEntry<'field> {
    /// Returns the code this rule names, as stated.
    #[must_use]
    pub const fn code(self) -> &'field str {
        self.code
    }

    /// Walks the patterns that name it, each still escaped as stored.
    ///
    /// The entry's own read already held the list to the grammar, so the
    /// walk never stops short.
    #[must_use]
    pub const fn patterns(self) -> FixPatterns<'field> {
        FixPatterns::over(self.patterns)
    }

    /// Decodes the patterns that name it, in the order they were stated.
    ///
    /// # Errors
    ///
    /// Returns the JSON codec's refusal when a stored pattern is not a legal
    /// string body, which the writer never produces.
    pub fn parse_patterns(self) -> Result<Vec<String>> {
        self.patterns()
            .map(|pattern| {
                decode_text(TARGET, PATTERNS, Some(pattern)).map(Option::unwrap_or_default)
            })
            .collect()
    }
}

/// The patterns one entry states, borrowed and still escaped.
///
/// Answered by [`FixDirectionEntry::patterns`]. A pattern is the one text
/// these documents hold that carries backslashes as a matter of course -
/// `\s`, `\[` - so the walk steps over escapes as the string reader does,
/// and hands back the body between the quotes exactly as stored.
#[derive(Clone, Debug)]
pub struct FixPatterns<'field> {
    cursor: Cursor<'field>,
    started: bool,
}

impl<'field> FixPatterns<'field> {
    /// Walks one list body a reader answered.
    pub(super) const fn over(body: &'field str) -> Self {
        Self {
            cursor: Cursor::new(body),
            started: false,
        }
    }

    /// Advances one step: the next pattern, the list's end, or a refusal.
    fn step(&mut self) -> Scan<Option<&'field str>> {
        if self.started {
            if self.cursor.is_done() {
                return Ok(None);
            }
            self.cursor.expect(b',')?;
        } else {
            self.started = true;
            if self.cursor.is_done() {
                return Ok(None);
            }
        }
        self.cursor.read_string().map(Some)
    }
}

impl<'field> Iterator for FixPatterns<'field> {
    type Item = &'field str;

    fn next(&mut self) -> Option<Self::Item> {
        self.step().ok().flatten()
    }
}

impl FusedIterator for FixPatterns<'_> {}

/// Reads one list of patterns, handing back its body once every pattern in
/// it has been read.
///
/// Patterns are read here rather than skipped so a hand-edited document is
/// refused at its own byte by the entry's read, and the walk an entry hands
/// back afterwards never fails. A list stating no pattern is refused too: a
/// rule matching nothing names nothing, and the writer never produces one.
fn read_patterns<'doc>(cursor: &mut Cursor<'doc>) -> Scan<&'doc str> {
    let opening = cursor.position();
    let body = cursor.read_list()?;
    let mut walk = FixPatterns::over(body);
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
        return Err(Refusal::Short(PATTERNS, 1));
    }
    Ok(body)
}

/// A field's direction rules, in document order.
///
/// Answered by [`FixField::directions`](crate::FixField). It walks the
/// stored document as it goes and hands back slices of it, so nothing is
/// parsed ahead of the entry being asked for and nothing is allocated. An
/// absent property yields nothing, which is what a field reading by the
/// defaults answers.
#[derive(Clone, Debug)]
pub struct FixDirections<'field> {
    cursor: Cursor<'field>,
    stated: bool,
    started: bool,
    done: bool,
}

impl<'field> FixDirections<'field> {
    /// Walks one stored `fix:directions` value, or nothing for an absent one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            stated: stored.is_some(),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Whether the field carries the property at all.
    ///
    /// An absent property and a table stating no rule both yield nothing,
    /// and only this tells them apart: the reading answers its defaults for
    /// the absent one and reads by the table for the other.
    #[must_use]
    pub const fn is_stated(&self) -> bool {
        self.stated
    }

    /// Renders rules into the one canonical document they have.
    ///
    /// Order is kept, because it is what the document says: the order the
    /// rules were stated in, which is the order they are reported in. That
    /// each code is named once, under any spelling, is the setter's to hold
    /// against the set the field declares; this renders what it was given.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when an entry states an empty code or one
    /// the reader would not read back as a word, an entry states no
    /// pattern, or a pattern is empty or one the regex crate refuses.
    pub(super) fn render(directions: &[FixDirection]) -> Result<String> {
        directions.iter().try_for_each(FixDirection::validate)?;
        let mut writer = Writer::open_array(DIRECTIONS);
        for direction in directions {
            direction.write_into(&mut writer)?;
        }
        writer.close_array();
        Ok(writer.finish())
    }

    /// Advances one step: the next entry, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixDirectionEntry<'field>>> {
        if !self.started {
            self.started = true;
            if !self.cursor.open_array(DIRECTIONS)? {
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
        self.read_direction().map(Some)
    }

    /// Reads the one entry starting at the cursor.
    fn read_direction(&mut self) -> Scan<FixDirectionEntry<'field>> {
        self.cursor.expect(b'{')?;
        let mut code = None;
        let mut patterns = None;
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                CODE => code = Some(self.cursor.read_word(CODE)?),
                _ => patterns = Some(read_patterns(&mut self.cursor)?),
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let (Some(code), Some(patterns)) = (code, patterns) else {
            return Err(Refusal::MissingKey(if code.is_none() {
                CODE
            } else {
                PATTERNS
            }));
        };
        Ok(FixDirectionEntry { code, patterns })
    }

    /// The next entry, answering nothing where the document does not parse.
    ///
    /// This is what every infallible reader walks, so a malformed document
    /// resolves to no answer rather than to a wrong one, and it spends no
    /// allocation doing it. The [`Iterator`] the same walk implements is the
    /// fallible door, naming the byte a hand edit stopped it at.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut field = DataType::utf8().nullable_field("msgdirection");
    /// field.set_metadata([("fix:directions", r#"{"directions":[{"code":"S"}]}"#)])?;
    /// // The entry states no `patterns`, which the grammar requires.
    /// assert!(field.as_fix().directions().next_ok().is_none());
    /// assert!(field.as_fix().directions().next().expect("a refusal").is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn next_ok(&mut self) -> Option<FixDirectionEntry<'field>> {
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

impl<'field> Iterator for FixDirections<'field> {
    type Item = Result<FixDirectionEntry<'field>>;

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
        // Every entry costs at least `{"code":"","patterns":[""]}` plus its
        // separator - the grammar admits the empty texts the setter refuses
        // - which bounds how many the remaining bytes can hold, the refusal
        // that may follow the last one included.
        (0, Some(self.cursor.document().len() / 28 + 1))
    }
}

impl FusedIterator for FixDirections<'_> {}

impl From<FixDirectionEntry<'_>> for FixDirection {
    /// Owns what a borrowed entry holds, for a caller taking the rules away.
    ///
    /// The patterns are decoded here rather than kept escaped, because an
    /// owned value has no document behind it to decode against later; a
    /// body the codec refuses keeps its escaped text, which is what arrived.
    fn from(entry: FixDirectionEntry<'_>) -> Self {
        let patterns = entry.patterns().map(|pattern| {
            decode_text(TARGET, PATTERNS, Some(pattern))
                .ok()
                .flatten()
                .map_or_else(|| SmolStr::new(pattern), SmolStr::new)
        });
        Self::new(entry.code(), patterns)
    }
}
