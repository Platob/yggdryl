//! Which identifier map a field's value names a message by, and under which
//! key.
//!
//! A message goes by the names its fields state - the account it is booked
//! to, the user who entered it, the order's own and its parent's
//! identifiers - and the three [`IdMap`](crate::IdMap)s an
//! [`Operation`](crate::graph::Operation) answers hold them, each under an
//! upper-cased key. Which field states which key is a fact about the field,
//! so it travels on the field: `FIX:idmap` is one [canonical
//! document](super::document) of entries, read borrowed, and the registry
//! compiles every field's once into the table a message rebuilds its maps
//! from.
//!
//! ```text
//! OrderID(37)   [{"map":"altids","key":"ORDERID","follow":true}]
//! PartyID(448)  [{"map":"accountids","key":"CUSTOMERACCOUNT","role":"24"}, ...]
//! ```
//!
//! An entry states the map and the key, whether an operation that follows
//! another carries the key forward - an alternate identifier only, because
//! an account or a user always follows - and, on `PartyID(448)`, the
//! `PartyRole(452)` code of the `Parties` occurrence that states it.

use std::fmt;
use std::iter::FusedIterator;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use super::document::{Cursor, Refusal, Scan, Writer};
use crate::{Error, Result};

/// What the document is called for every refusal it raises.
const TARGET: &str = "fix idmap";

/// The map the value lands in.
const MAP: &str = "map";
/// The upper-cased key it lands under.
const KEY: &str = "key";
/// Whether a following operation carries it.
const FOLLOW: &str = "follow";
/// The `PartyRole(452)` code of the occurrence stating it.
const ROLE: &str = "role";

/// The keys one entry may state, in the order it states them.
pub(super) const KEYS: [&str; 4] = [MAP, KEY, FOLLOW, ROLE];

/// The most bytes a key may be: the width an [`IdMap`](crate::IdMap) key
/// holds.
const KEY_WIDTH: usize = 32;

/// One of the three identifier maps an operation answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FixIdMapKind {
    /// `accountids`: the accounts a message is booked to.
    Accounts,
    /// `userids`: the users and traders who handled it.
    Users,
    /// `altids`: the alternate identifiers it goes by.
    Alts,
}

impl FixIdMapKind {
    /// Every map, in the order an operation states them.
    pub const ALL: [Self; 3] = [Self::Accounts, Self::Users, Self::Alts];

    /// The map's name, as an operation's accessor spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accounts => "accountids",
            Self::Users => "userids",
            Self::Alts => "altids",
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
                    "expected {MAP:?} to be accountids, userids or altids, got {text:?}"
                ))
            })
    }
}

/// One field stating one key of one identifier map.
///
/// ```
/// use yggdryl::fix::{FixIdMapKind, FixIdSource};
///
/// # fn main() -> yggdryl::Result<()> {
/// let order = FixIdSource::new(FixIdMapKind::Alts, "ORDERID").with_follow(true);
/// assert_eq!(order.map(), FixIdMapKind::Alts);
/// assert_eq!(order.key(), "ORDERID");
/// assert!(order.follows());
/// assert_eq!(order.role(), None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FixIdSource {
    map: FixIdMapKind,
    key: SmolStr,
    follow: bool,
    role: Option<SmolStr>,
}

impl FixIdSource {
    /// Builds one source stating `key` in `map`, followed by nothing and
    /// read off the field itself.
    #[must_use]
    pub fn new(map: FixIdMapKind, key: impl Into<SmolStr>) -> Self {
        Self {
            map,
            key: key.into(),
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

    /// The key it lands under.
    #[must_use]
    pub fn key(&self) -> &str {
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

    /// Holds this source to what the document can state: a key of one to
    /// 32 upper-case ASCII letters and digits, a follow flag on an alternate
    /// identifier only - an account and a user always follow - and a role
    /// that is a code of ASCII letters and digits.
    fn validate(&self) -> Result<()> {
        let key = self.key.as_str();
        if key.is_empty()
            || key.len() > KEY_WIDTH
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(refused(format_smolstr!(
                "expected {KEY:?} to be 1 to {KEY_WIDTH} upper-case ASCII letters or digits, \
                 got {key:?}"
            )));
        }
        if self.follow && self.map != FixIdMapKind::Alts {
            return Err(refused(format_smolstr!(
                "expected {FOLLOW:?} on altids only, {} always follows",
                self.map
            )));
        }
        if let Some(role) = self.role() {
            if role.is_empty() || !role.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                return Err(refused(format_smolstr!(
                    "expected {ROLE:?} to be a PartyRole code, got {role:?}"
                )));
            }
        }
        Ok(())
    }

    /// Renders this source into the document being written.
    fn write_into(&self, writer: &mut Writer) -> Result<()> {
        writer.open_element();
        writer.text(true, MAP, self.map.as_str())?;
        writer.text(false, KEY, &self.key)?;
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
    /// Returns [`Error::Parse`] when a source states a key that is not one
    /// to 32 upper-case letters or digits, a follow flag on a map that is not
    /// `altids`, a role that is not a code of letters and digits, or a key
    /// twice.
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
                MAP => map = Some(self.cursor.read_word(MAP)?),
                KEY => key = Some(self.cursor.read_word(KEY)?),
                FOLLOW => follow = self.cursor.read_flag(FOLLOW)?,
                _ => role = Some(self.cursor.read_word(ROLE)?),
            }
            if !self.cursor.next_property()? {
                break;
            }
        }
        let map = map.ok_or(Refusal::MissingKey(MAP))?;
        let key = key.ok_or(Refusal::MissingKey(KEY))?;
        let map = map
            .parse::<FixIdMapKind>()
            .map_err(|_| Refusal::NotAWord(MAP))?;
        let mut source = FixIdSource::new(map, key).with_follow(follow);
        if let Some(role) = role {
            source = source.with_role(role);
        }
        source.validate().map_err(|_| Refusal::NotAWord(KEY))?;
        Ok(source)
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
