//! Which identifier map a field's value names a message by, and under which
//! key.
//!
//! A message goes by the names its fields state - the order's own and its
//! parent's identifiers, the quote's, the execution's - and the
//! [`Identifiers`](yggdryl_market::Identifiers) an
//! [`Operation`](yggdryl_market::graph::Operation) answers holds them, each a type
//! from the `fix` source. Which field states which key is a fact about the field,
//! so it travels on the field: `FIX:idmap` is one [canonical
//! document](super::document) of entries, read borrowed, and the registry
//! compiles every field's once into the table a message rebuilds its maps
//! from.
//!
//! ```text
//! OrderID(37)   [{"map":"identifiers","key":"orderid","follow":true}]
//! ```
//!
//! An entry states the map and the key, whether an operation that follows
//! another carries the key forward, and, on `PartyID(448)`, the
//! `PartyRole(452)` code of the `Parties` occurrence that states it.
//!
//! Two readings are the crate's own rather than a field's: a message's
//! parties and its `Account(1)` are its
//! [`partyids`](yggdryl_market::graph::Operation::get_partyids), every `Parties(453)`
//! and `RootParties(1116)` occurrence's identifier typed by its role's name -
//! `executingtrader`, `customeraccount` - from its `PartyIDSource(447)`'s,
//! and the account typed `account` from its `AcctIDSource(660)`'s, and its
//! regulatory trade identifiers are identifiers typed by their
//! `RegulatoryTradeIDType(1906)` - `regtradeid`, `tvtic`. A side's own parties, account and identifiers come first, so an
//! execution a trade's parse split off states its side's, and a book
//! entry's own parties lead the message's on its leaf. A leaf lifts, too,
//! every scalar of its metadata whose key ends with one of the
//! `FIX:identifiers` its message's type declares into its alternate
//! identifiers; the message's own maps are the ones its fields state.

use std::fmt;
use std::iter::FusedIterator;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use yggdryl_market::IdType;

use super::document::{Cursor, Refusal, Scan, Writer};
use yggdryl::{Error, Result};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix idmap";

/// The map the value lands in.
const MAP: &str = "map";
/// The identifier type it lands under.
const KEY: &str = "key";
/// Whether a following operation carries it.
const FOLLOW: &str = "follow";
/// The `PartyRole(452)` code of the occurrence stating it.
const ROLE: &str = "role";

/// The keys one entry may state, in the order it states them.
pub(super) const KEYS: [&str; 4] = [MAP, KEY, FOLLOW, ROLE];

/// What a role is: a `PartyRole(452)` code, ASCII letters and digits.
const ROLE_CODE: &str = "a PartyRole code";

/// Whether `role` is a `PartyRole(452)` code the document can state.
fn is_role_code(role: &str) -> bool {
    !role.is_empty() && role.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The identifier map an operation answers that a field's value names a
/// message by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixIdMapKind {
    /// `identifiers`: the identifiers it goes by.
    Identifiers,
}

impl FixIdMapKind {
    /// Every map, in the order an operation states them.
    pub const ALL: [Self; 1] = [Self::Identifiers];

    /// The map's name, as an operation's accessor spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identifiers => "identifiers",
        }
    }
}

impl fmt::Display for FixIdMapKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for FixIdMapKind {
    type Err = Error;

    /// Reads a map by its name, ASCII case folded.
    fn from_str(text: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(text.trim()))
            .ok_or_else(|| {
                refused(format_smolstr!(
                    "expected {MAP:?} to be identifiers, got {text:?}"
                ))
            })
    }
}

/// One field stating one key of one identifier map.
///
/// ```
/// use yggdryl_market::IdType;
/// use yggdryl_fix::{FixIdMapKind, FixIdSource};
///
/// # fn main() -> yggdryl::Result<()> {
/// #     yggdryl_fix::install().unwrap();
/// let order = FixIdSource::new(FixIdMapKind::Identifiers, IdType::OrderId).with_follow(true);
/// assert_eq!(order.map(), FixIdMapKind::Identifiers);
/// assert_eq!(order.key(), &IdType::OrderId);
/// assert!(order.follows());
/// assert_eq!(order.role(), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FixIdSource {
    map: FixIdMapKind,
    key: IdType,
    follow: bool,
    role: Option<SmolStr>,
}

impl FixIdSource {
    /// Builds one source stating `key` in `map`, followed by nothing and
    /// read off the field itself.
    #[must_use]
    pub fn new(map: FixIdMapKind, key: IdType) -> Self {
        Self {
            map,
            key,
            follow: false,
            role: None,
        }
    }

    /// Sets whether an operation that follows another carries this key.
    #[must_use]
    pub const fn with_follow(mut self, follow: bool) -> Self {
        self.follow = follow;
        self
    }

    /// Sets the `PartyRole(452)` code of the `Parties` occurrence stating it.
    #[must_use]
    pub fn with_role(mut self, role: impl Into<SmolStr>) -> Self {
        self.role = Some(role.into());
        self
    }

    /// The map the value lands in.
    #[must_use]
    pub const fn map(&self) -> FixIdMapKind {
        self.map
    }

    /// The identifier type it lands under.
    #[must_use]
    pub fn key(&self) -> &IdType {
        &self.key
    }

    /// Whether an operation that follows another carries this key.
    #[must_use]
    pub const fn follows(&self) -> bool {
        self.follow
    }

    /// The `PartyRole(452)` code of the occurrence stating it, where a
    /// `Parties` occurrence states it rather than the field alone.
    #[must_use]
    pub fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    /// Holds this source to what the document can state: a role that is a
    /// code of ASCII letters and digits.
    fn validate(&self) -> Result<()> {
        if let Some(role) = self.role().filter(|role| !is_role_code(role)) {
            return Err(refused(format_smolstr!(
                "expected {ROLE:?} to be {ROLE_CODE}, got {role:?}"
            )));
        }
        Ok(())
    }

    /// Renders this source into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, MAP, self.map.as_str())?;
        writer.text(false, KEY, self.key.as_str())?;
        if self.follow {
            writer.flag(false, FOLLOW, true);
        }
        if let Some(role) = self.role() {
            writer.text(false, ROLE, role)?;
        }
        writer.close_element();
        Ok(())
    }
}

/// A refusal the writer or the reader raises about what an entry states.
fn refused(reason: SmolStr) -> Error {
    Error::Parse {
        target: TARGET,
        position: 0,
        reason,
    }
}

/// Walks the sources one field carries, in document order.
pub struct FixIdSources<'field> {
    cursor: Cursor<'field>,
    started: bool,
    done: bool,
}

impl<'field> FixIdSources<'field> {
    /// Walks one stored `FIX:idmap` value, or nothing for an absent one.
    pub(super) fn over(stored: Option<&'field str>) -> Self {
        Self {
            cursor: Cursor::new(stored.unwrap_or_default()),
            started: false,
            done: stored.is_none(),
        }
    }

    /// Renders sources into the one canonical document they have.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when a source states a role that is not a
    /// code of letters and digits, or a key twice.
    pub(super) fn render(sources: &[FixIdSource]) -> Result<String> {
        sources.iter().try_for_each(FixIdSource::validate)?;
        for (index, source) in sources.iter().enumerate() {
            if sources[..index]
                .iter()
                .any(|held| held.map == source.map && held.key == source.key)
            {
                return Err(refused(format_smolstr!(
                    "expected each key once, {}:{} is stated twice",
                    source.map,
                    source.key
                )));
            }
        }
        let mut writer = Writer::open_array();
        for source in sources {
            source.write_into(&mut writer)?;
        }
        Ok(writer.finish())
    }

    /// Advances one step: the next entry, the document's end, or a refusal.
    fn step(&mut self) -> Scan<Option<FixIdSource>> {
        if !self.cursor.next_entry(&mut self.started)? {
            return Ok(None);
        }
        self.read_entry().map(Some)
    }

    /// Reads the one entry starting at the cursor.
    fn read_entry(&mut self) -> Scan<FixIdSource> {
        self.cursor.expect(b'{')?;
        let (mut map, mut key, mut follow, mut role) = (None, None, false, None);
        let mut next = 0;
        loop {
            match KEYS[self.cursor.read_key(&KEYS, &mut next)?] {
                MAP => {
                    map = Some(self.read_as(MAP, "an identifier map", |word| word.parse().ok())?);
                }
                // A stored key is the folded word its type spells, no alias
                // of it.
                KEY => {
                    key = Some(self.read_as(
                        KEY,
                        "the folded word of an identifier type",
                        |word| {
                            word.parse::<IdType>()
                                .ok()
                                .filter(|kind| kind.as_str() == word)
                        },
                    )?);
                }
                FOLLOW => follow = self.cursor.read_flag(FOLLOW)?,
                _ => {
                    role = Some(
                        self.read_as(ROLE, ROLE_CODE, |word| is_role_code(word).then_some(word))?,
                    )
                }
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let map = map.ok_or(Refusal::MissingKey(MAP))?;
        let key = key.ok_or(Refusal::MissingKey(KEY))?;
        let mut source = FixIdSource::new(map, key).with_follow(follow);
        if let Some(role) = role {
            source = source.with_role(role);
        }
        Ok(source)
    }

    /// Reads the word `key` holds as what `read` makes of it, refused as
    /// `expected` - the cursor back on the word, which the refusal quotes -
    /// where it makes nothing.
    fn read_as<T>(
        &mut self,
        key: &'static str,
        expected: &'static str,
        read: impl FnOnce(&'field str) -> Option<T>,
    ) -> Scan<T> {
        let at = self.cursor.position();
        let word = self.cursor.read_word(key)?;
        read(word).ok_or_else(|| {
            self.cursor.seek(at);
            Refusal::Unexpected(key, expected)
        })
    }
}

impl Iterator for FixIdSources<'_> {
    type Item = Result<FixIdSource>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(None) => {
                self.done = true;
                None
            }
            Ok(Some(source)) => Some(Ok(source)),
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
}

impl FusedIterator for FixIdSources<'_> {}
