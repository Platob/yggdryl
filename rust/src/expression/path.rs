//! The one path into a nested schema or value.
//!
//! Every surface that addresses a child by path resolves it here, once, and
//! carries the resolved [`FieldPath`] afterwards. Before this module each
//! surface split a string at `.` in its own way: the schema lookup backtracked
//! so a field genuinely named `a.b` resolved, the scalar navigator did not so
//! the same path selected a column and then failed to select its value, the
//! FIX navigator read a decimal segment as a repeating-group index, and none of
//! the plain splitters could address a list or a map at all.
//!
//! One grammar answers all of it: `.name` for a struct child, `[0]` and `[-1]`
//! for a list element, `['key']` for a map entry. A name the bare spelling
//! cannot carry is quoted, so `a.b` has exactly one spelling and it is not two
//! levels. A trailing `as name`, spelled the way SQL spells it, says what to
//! call what the path reached - the one thing a selector cannot say by itself.
//!
//! This is a *selector*: it says which child a caller wants. It is not the
//! crate-private `Path` cons-list a recursive walk carries to report where a
//! failure happened. The two never merge - one is caller input resolved once,
//! the other is walker state rendered only on error.

use std::fmt::{self, Write as _};
use std::str::FromStr;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::Literal;
use crate::{Error, Result, Scalar};

/// What a parse failure names itself as.
const TARGET: &str = "field path";

/// The quote a text key is written in.
const QUOTE: u8 = b'\'';

/// One step of a path into a nested schema or value.
///
/// Written once in the grammar, resolved once against the container's
/// datatype, and applied identically by every walk that takes a path.
#[derive(Clone, Debug, Eq, PartialEq, Hash, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSegment {
    /// `.name` - a struct child, resolved ASCII case-insensitively the way
    /// every cast in this crate resolves a name.
    Field(SmolStr),
    /// `[0]`, `[-1]` - one list element by position, 0-based, a negative index
    /// counting back from the end. Out of range is null rather than an error,
    /// because absence is not a failure on the read path anywhere else here.
    Index(i64),
    /// `['k']` - one map entry by key, the key read once through the map's own
    /// key type. A struct child may also be reached this way when the key is
    /// text, which is the spelling JSON tooling already uses.
    Key(Literal),
}

impl FieldSegment {
    /// Name a struct child.
    #[must_use]
    pub fn field(name: impl Into<SmolStr>) -> Self {
        Self::Field(name.into())
    }

    /// Name a list element by position.
    #[must_use]
    pub const fn index(position: i64) -> Self {
        Self::Index(position)
    }

    /// Name a map entry by text key.
    ///
    /// A path key is text, because text is what a path can write down and read
    /// back unambiguously. A whole number in brackets is a list position, and
    /// a map keyed by anything else is reached through the expression
    /// grammar's own accessor, which takes a computed key and never has to
    /// render it.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is not text, or when it and the
    /// datatype it is paired with disagree.
    pub fn key(value: Scalar) -> Result<Self> {
        if value.as_str().is_none() {
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: format_smolstr!("expected a text map key, got {}", value.kind()),
            });
        }
        Ok(Self::Key(Literal::infer(value)?))
    }

    /// The name this segment addresses a child by, where it addresses one.
    ///
    /// A text key names a child too: `['price']` and `.price` reach the same
    /// struct member, and a caller asking for the name should not have to know
    /// which spelling arrived.
    #[must_use]
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Field(name) => Some(name.as_str()),
            Self::Key(key) => key.value().as_str(),
            Self::Index(_) => None,
        }
    }

    /// The position this segment addresses an element by, where it addresses
    /// one.
    ///
    /// Only a position segment answers. A key that happens to hold a number is
    /// a key: reading it as a position is the exact ambiguity this one grammar
    /// exists to remove.
    #[must_use]
    pub const fn as_index(&self) -> Option<i64> {
        match self {
            Self::Index(position) => Some(*position),
            Self::Field(_) | Self::Key(_) => None,
        }
    }
}

impl Ord for FieldSegment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Field(left), Self::Field(right)) => left.cmp(right),
            (Self::Index(left), Self::Index(right)) => left.cmp(right),
            (Self::Key(left), Self::Key(right)) => left.cmp(right),
            (left, right) => left.rank().cmp(&right.rank()),
        }
    }
}

impl PartialOrd for FieldSegment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl FieldSegment {
    /// The order kinds sort in when two segments are not the same kind.
    const fn rank(&self) -> u8 {
        match self {
            Self::Field(_) => 0,
            Self::Index(_) => 1,
            Self::Key(_) => 2,
        }
    }
}

impl fmt::Display for FieldSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(name) => {
                formatter.write_char('.')?;
                write_identifier(formatter, name)
            }
            Self::Index(index) => write!(formatter, "[{index}]"),
            Self::Key(key) => {
                formatter.write_char('[')?;
                write_key(formatter, key)?;
                formatter.write_char(']')
            }
        }
    }
}

/// One resolved path into a nested schema or value, and what to call what it
/// reaches.
///
/// Cloning shares the segments rather than copying them, so a path hoisted out
/// of a loop and handed to each iteration costs one reference count.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FieldPath {
    segments: Arc<[FieldSegment]>,
    alias: Option<SmolStr>,
}

impl FieldPath {
    /// The empty path, which selects the value it is applied to.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// Build a path from segments already resolved.
    ///
    /// The way to build a path from parts: nothing is rendered to text and
    /// nothing is parsed back.
    pub fn new(segments: impl IntoIterator<Item = FieldSegment>) -> Self {
        Self {
            segments: segments.into_iter().collect(),
            alias: None,
        }
    }

    /// Parse one path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position and what was expected.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Borrow the resolved segments.
    #[must_use]
    pub fn segments(&self) -> &[FieldSegment] {
        &self.segments
    }

    /// The number of segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether this is the empty path.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Whether this path selects the value it is applied to.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// The first segment.
    #[must_use]
    pub fn first(&self) -> Option<&FieldSegment> {
        self.segments.first()
    }

    /// The last segment.
    #[must_use]
    pub fn last(&self) -> Option<&FieldSegment> {
        self.segments.last()
    }

    /// The path without its last segment.
    ///
    /// The alias is not carried up: it names what the whole path reached, and
    /// the parent reaches something else.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let (_, head) = self.segments.split_last()?;
        Some(Self::new(head.iter().cloned()))
    }

    /// This path with one more segment.
    ///
    /// The alias is dropped for the same reason [`Self::parent`] drops it.
    #[must_use]
    pub fn join(&self, segment: FieldSegment) -> Self {
        Self::new(
            self.segments
                .iter()
                .cloned()
                .chain(std::iter::once(segment)),
        )
    }

    /// What to call what this path reaches.
    ///
    /// Written the way SQL writes it - `order.line[0].price as price` - and it
    /// answers the one question a selector cannot: a path says which value to
    /// take, and an alias says what the column holding it is called. Without
    /// one, a caller naming a column from a path falls back to the last
    /// segment's own name.
    #[must_use]
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    /// Set or clear what to call what this path reaches.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for an empty alias, or for one on the root
    /// path: the root reaches the value it is applied to, so there is nothing
    /// there for a name to be about. Failure leaves the path unchanged.
    pub fn set_alias(&mut self, alias: Option<&str>) -> Result<()> {
        let Some(alias) = alias else {
            self.alias = None;
            return Ok(());
        };
        if alias.is_empty() {
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: SmolStr::new_static("expected a name after `as`, got an empty one"),
            });
        }
        if self.is_root() {
            return Err(Error::Parse {
                target: TARGET,
                position: 0,
                reason: SmolStr::new_static(
                    "expected a path to alias, got the root; the root reaches what it is applied to",
                ),
            });
        }
        self.alias = Some(SmolStr::new(alias));
        Ok(())
    }

    /// Return this path with an alias.
    ///
    /// # Errors
    ///
    /// Returns the same refusals as [`Self::set_alias`].
    pub fn try_with_alias(mut self, alias: &str) -> Result<Self> {
        self.set_alias(Some(alias))?;
        Ok(self)
    }

    /// The name this path gives what it reaches.
    ///
    /// The alias where one is written, and the last segment's own name
    /// otherwise. This is what a caller building a column from a path reads,
    /// so the fallback lives here rather than at each call site.
    #[must_use]
    pub fn column_name(&self) -> Option<&str> {
        self.alias()
            .or_else(|| self.last().and_then(FieldSegment::as_name))
    }

    /// The single name this path addresses, when it addresses exactly one.
    ///
    /// The common shape by a wide margin - one column, named - and the one a
    /// caller can answer without walking.
    #[must_use]
    pub fn as_name(&self) -> Option<&str> {
        match self.segments.as_ref() {
            [segment] => segment.as_name(),
            _ => None,
        }
    }

    /// A deterministic hash of the complete path.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, segment) in self.segments.iter().enumerate() {
            match segment {
                // The leading dot is written between steps, never in front of
                // the first one: a one-name path renders as that name, which
                // is what every caller writing one spells.
                FieldSegment::Field(name) if index == 0 => write_identifier(formatter, name)?,
                segment => write!(formatter, "{segment}")?,
            }
        }
        // An alias never sits on the root, so this never opens the rendering
        // with a space, and parsing it back is the exact inverse.
        if let Some(alias) = &self.alias {
            formatter.write_str(" as ")?;
            write_identifier(formatter, alias)?;
        }
        Ok(())
    }
}

impl FromIterator<FieldSegment> for FieldPath {
    fn from_iter<I: IntoIterator<Item = FieldSegment>>(segments: I) -> Self {
        Self::new(segments)
    }
}

impl From<FieldSegment> for FieldPath {
    fn from(segment: FieldSegment) -> Self {
        Self::new([segment])
    }
}

impl AsRef<[FieldSegment]> for FieldPath {
    fn as_ref(&self) -> &[FieldSegment] {
        &self.segments
    }
}

impl<'a> IntoIterator for &'a FieldPath {
    type Item = &'a FieldSegment;
    type IntoIter = std::slice::Iter<'a, FieldSegment>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments.iter()
    }
}

impl FromStr for FieldPath {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Parser::new(value).path()
    }
}

impl ::serde::Serialize for FieldPath {
    fn serialize<S: ::serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        // The canonical text, because that is the spelling every catalog,
        // metadata map and configuration file this path travels through
        // already holds.
        serializer.collect_str(self)
    }
}

impl<'de> ::serde::Deserialize<'de> for FieldPath {
    fn deserialize<D: ::serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let text = SmolStr::deserialize(deserializer)?;
        Self::from_str(&text).map_err(::serde::de::Error::custom)
    }
}

/// Write one identifier, quoting it only when the bare spelling would not come
/// back as itself.
fn write_identifier(formatter: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    if is_bare_identifier(name) {
        return formatter.write_str(name);
    }
    formatter.write_char('"')?;
    for character in name.chars() {
        if character == '"' {
            formatter.write_char('"')?;
        }
        formatter.write_char(character)?;
    }
    formatter.write_char('"')
}

/// Write one map key inside its brackets.
///
/// Total by construction: [`FieldSegment::key`] admits only the kinds this
/// writes, so rendering a path is always the exact inverse of parsing one.
fn write_key(formatter: &mut fmt::Formatter<'_>, key: &Literal) -> fmt::Result {
    if let Some(text) = key.value().as_str() {
        formatter.write_char('\'')?;
        for character in text.chars() {
            if character == '\'' {
                formatter.write_char('\'')?;
            }
            formatter.write_char(character)?;
        }
        return formatter.write_char('\'');
    }
    formatter.write_str("''")
}

/// Whether a name is spelled the way a bare segment is spelled.
///
/// Deliberately narrow: anything else is quoted rather than guessed at, so a
/// name carrying a dot, a bracket or a space round-trips instead of becoming
/// two segments.
fn is_bare_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// The one path parser.
struct Parser<'a> {
    bytes: &'a [u8],
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    const fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            text,
            at: 0,
        }
    }

    fn path(mut self) -> Result<FieldPath> {
        let mut segments: Vec<FieldSegment> = Vec::new();
        let mut alias = None;
        self.skip_space();
        if self.at == self.bytes.len() {
            return Ok(FieldPath::root());
        }
        loop {
            self.skip_space();
            match self.peek() {
                Some(b'[') => segments.push(self.bracketed()?),
                Some(b'.') => {
                    self.at += 1;
                    segments.push(FieldSegment::Field(self.name()?));
                }
                // A leading dot is optional, so the first step may be a bare
                // name. A later one may not: two names in a row with nothing
                // between them is a typo, not a path.
                Some(_) if segments.is_empty() => {
                    segments.push(FieldSegment::Field(self.name()?));
                }
                Some(_) => {
                    return Err(self.fail("expected `.` or `[` between path segments"));
                }
                None => break,
            }
            self.skip_space();
            if self.at == self.bytes.len() {
                break;
            }
            if self.eat_as() {
                alias = Some(self.alias_name()?);
                self.skip_space();
                if self.at != self.bytes.len() {
                    return Err(self.fail("expected the end of the path after its alias"));
                }
                break;
            }
        }
        Ok(FieldPath {
            segments: segments.into(),
            alias,
        })
    }

    /// Take the `as` keyword, when that is what comes next.
    ///
    /// A boundary is required after it, so `assets` stays one name rather than
    /// `as` followed by `sets`. The keyword is only looked for once a path has
    /// something to alias, which is what leaves `as` usable as a segment name.
    fn eat_as(&mut self) -> bool {
        let rest = &self.bytes[self.at..];
        if rest.len() < 2 || !rest[..2].eq_ignore_ascii_case(b"as") {
            return false;
        }
        match rest.get(2) {
            Some(byte) if byte.is_ascii_whitespace() || *byte == b'"' || *byte == QUOTE => {}
            _ => return false,
        }
        self.at += 2;
        true
    }

    /// Read one `[...]` step: a position, or a constant key.
    fn bracketed(&mut self) -> Result<FieldSegment> {
        self.at += 1;
        self.skip_space();
        let segment = match self.peek() {
            Some(b'\'') => FieldSegment::Key(text_key(self.quoted(b'\'')?)?),
            Some(b'"') => FieldSegment::Key(text_key(self.quoted(b'"')?)?),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.position()?,
            _ => return Err(self.fail("expected a position or a quoted key inside `[`")),
        };
        self.skip_space();
        if self.peek() != Some(b']') {
            return Err(self.fail("expected `]` closing a path step"));
        }
        self.at += 1;
        Ok(segment)
    }

    /// Read one list position.
    fn position(&mut self) -> Result<FieldSegment> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at += 1;
        }
        let text = &self.text[start..self.at];
        text.parse::<i64>()
            .map(FieldSegment::Index)
            .map_err(|_| Error::Parse {
                target: TARGET,
                position: start,
                reason: format_smolstr!("expected a position that fits in 64 bits, got {text:?}"),
            })
    }

    /// Read one alias, bare or quoted either way.
    ///
    /// Wider at intake than a segment name is, because an alias is written by
    /// hand and both quotes are spellings people reach for. It still renders
    /// back one way.
    fn alias_name(&mut self) -> Result<SmolStr> {
        self.skip_space();
        if self.peek() == Some(QUOTE) {
            return Ok(SmolStr::new(self.quoted(QUOTE)?));
        }
        self.name()
    }

    /// Read one segment name, bare or double-quoted.
    fn name(&mut self) -> Result<SmolStr> {
        self.skip_space();
        if self.peek() == Some(b'"') {
            return Ok(SmolStr::new(self.quoted(b'"')?));
        }
        let start = self.at;
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.at += 1;
        }
        if self.at == start {
            return Err(self.fail("expected a segment name"));
        }
        Ok(SmolStr::new(&self.text[start..self.at]))
    }

    /// Read one quoted run, with the quote doubled to mean itself.
    fn quoted(&mut self, quote: u8) -> Result<String> {
        let opened = self.at;
        self.at += 1;
        let mut value = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(Error::Parse {
                    target: TARGET,
                    position: opened,
                    reason: format_smolstr!(
                        "expected a closing {:?}, got the end of the path",
                        char::from(quote)
                    ),
                });
            };
            if byte == quote {
                if self.bytes.get(self.at + 1) == Some(&quote) {
                    value.push(char::from(quote));
                    self.at += 2;
                    continue;
                }
                self.at += 1;
                return Ok(value);
            }
            let rest = &self.text[self.at..];
            let character = rest.chars().next().unwrap_or_default();
            value.push(character);
            self.at += character.len_utf8();
        }
    }

    const fn peek(&self) -> Option<u8> {
        if self.at < self.bytes.len() {
            Some(self.bytes[self.at])
        } else {
            None
        }
    }

    const fn skip_space(&mut self) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn fail(&self, reason: &'static str) -> Error {
        Error::Parse {
            target: TARGET,
            position: self.at,
            reason: SmolStr::new_static(reason),
        }
    }
}

/// Pair one text key with the datatype it is read at.
fn text_key(value: String) -> Result<Literal> {
    Literal::infer(Scalar::from(value))
}

#[cfg(test)]
mod tests;
