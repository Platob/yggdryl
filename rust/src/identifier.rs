//! Identifiers: what a thing is called, by whom, as what.
//!
//! One [`Identifier`] is a value beside the source that gave it and the type
//! of name it is: `base:isin=US0378331005`,
//! `proprietary:executingtrader=trader1`,
//! `oms:instrumentid=dbi;CH0012214059_XSWX_CHF`. Its source and its type
//! are its unique key, `src:type`: [`Identifiers`] is the map, sorted by
//! that key, of one identifier per key a market element states three of -
//! its `securityids`, its own `identifiers` and its `partyids` - so a party,
//! an ISIN and a client order identifier are read, written, merged and
//! digested alike, and laid out as one sorted Arrow map from the key to the
//! `struct<src, type, value>` it names.
//!
//! A source is an [`IdSource`] and a type an [`IdType`]: words folded to
//! lower case, the ones the crate names held as members that cost nothing,
//! any other held as an `Other` word. A key that is no `src:type` names
//! one by the identifier name it ends with and the source the rest of it
//! spells ([`Identifier::from_key`]): a FIX entry no dictionary maps is read
//! that way.
//!
//! Parentage is a relation between types, never a part of a value: a base
//! type has a list of parent types, nearest first ([`IdType::parents`]) -
//! `orderid`'s are `parentorderid`, the value it held before it last
//! changed, then `origorderid`, the value its chain first stated, and
//! `clordid`'s is `origclordid` alone, FIX's previous client order
//! identifier. Along a chain a follower takes the parents of each base it
//! states from its predecessor ([`Identifiers::follow_parents`]), and an
//! element stating a parent but not its base takes the base from its
//! nearest parent once it finalizes ([`Identifiers::fill_parents`]).

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use crate::code::is_null_like;
use crate::{DataType, Error, Field, IdSource, IdType, Result, Scalar, StructType};

/// The most bytes a type or a source may be once folded: room for the
/// longest name a FIX code set gives a party role or an identifier source.
pub const IDENTIFIER_KEY_WIDTH: usize = 64;

/// The most bytes a value may be once trimmed.
pub const IDENTIFIER_VALUE_WIDTH: usize = 64;

/// A word no [`IdType`] or [`IdSource`] member names, folded: lower-case
/// ASCII letters, digits and `.`, one to [`IDENTIFIER_KEY_WIDTH`] bytes.
/// Only a fold builds one, so a word a member names is never one.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct IdWord(pub(crate) SmolStr);

impl IdWord {
    /// The folded word.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `byte` is one a spelling breaks a word with, dropped by a fold.
const fn is_break(byte: u8) -> bool {
    matches!(byte, b'_' | b'-' | b' ' | b'#')
}

/// `word` folded into `buffer`: trimmed, lower-cased and without the `_`,
/// `-`, space and `#` a spelling breaks it with - `Executing Trader` and
/// `executing_trader` are `executingtrader`.
///
/// # Errors
///
/// What the fold expected, where `word` folds to nothing, past
/// [`IDENTIFIER_KEY_WIDTH`] bytes, or holds a byte other than an ASCII
/// letter, a digit or `.`.
pub(crate) fn fold_into<'buffer>(
    word: &str,
    buffer: &'buffer mut [u8; IDENTIFIER_KEY_WIDTH],
) -> std::result::Result<&'buffer str, &'static str> {
    let mut len = 0;
    for byte in word.trim().bytes() {
        if is_break(byte) {
            continue;
        }
        if !(byte.is_ascii_alphanumeric() || byte == b'.') {
            return Err("ASCII letters, digits and '.'");
        }
        let Some(slot) = buffer.get_mut(len) else {
            return Err("at most 64 bytes");
        };
        *slot = byte.to_ascii_lowercase();
        len += 1;
    }
    if len == 0 {
        return Err("a word");
    }
    Ok(std::str::from_utf8(&buffer[..len]).expect("ASCII"))
}

/// The refusal a spelling of `what` earns.
pub(crate) fn word_refusal(what: &'static str, actual: &str, expected: &str) -> Error {
    refusal(what, actual, expected)
}

/// Whether a type or a source folds from `word`, decided without building
/// the refusal it would raise: a reading choosing between spellings asks
/// this first.
pub(crate) fn is_word(word: &str) -> bool {
    folded_len(word).is_some_and(|len| (1..=IDENTIFIER_KEY_WIDTH).contains(&len))
}

/// How many bytes `word` folds to, or none where it holds a byte no word
/// does; counted, never built.
pub(crate) fn folded_len(word: &str) -> Option<usize> {
    let mut len = 0_usize;
    for byte in word.trim().bytes() {
        if is_break(byte) {
            continue;
        }
        if !(byte.is_ascii_alphanumeric() || byte == b'.') {
            return None;
        }
        len += 1;
    }
    Some(len)
}

/// Declares a word vocabulary - [`IdType`] and [`IdSource`]: an enum of the
/// words the crate names, each with its folded spelling and the aliases
/// that fold to it, beside the [`IdWord`] any other folded spelling is.
/// Words order, compare and display by their spelling.
macro_rules! id_vocabulary {
    (
        $(#[$meta:meta])*
        $name:ident, $what:literal {
            $( $(#[$variant_meta:meta])* $variant:ident => $spelling:literal $(| $alias:literal)* ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub enum $name {
            $( $(#[$variant_meta])* $variant, )*
            /// Any other word, folded.
            Other($crate::identifier::IdWord),
        }

        impl $name {
            /// Every word the crate names, in declaration order.
            pub const KNOWN: [Self; [$(id_vocabulary!(@unit $variant)),*].len()] =
                [$(Self::$variant),*];

            /// The spelling of every word the crate names, in declaration
            /// order.
            #[allow(dead_code)] // An `IdSource` never names a key's end.
            pub(crate) const SPELLINGS: [&'static str; [$(id_vocabulary!(@unit $variant)),*].len()] =
                [$($spelling),*];

            /// The folded spelling.
            #[must_use]
            pub fn as_str(&self) -> &str {
                match self {
                    $( Self::$variant => $spelling, )*
                    Self::Other(word) => word.as_str(),
                }
            }

            /// The member a folded spelling names, an alias included.
            pub(crate) fn from_folded(folded: &str) -> Option<Self> {
                match folded {
                    $( $spelling $(| $alias)* => Some(Self::$variant), )*
                    _ => None,
                }
            }

            /// Whether the word is one the crate names.
            #[must_use]
            pub const fn is_known(&self) -> bool {
                !matches!(self, Self::Other(_))
            }

            /// Whether `text` reads as this word - folded on the stack, an
            /// alias included - without building a copy of it.
            pub(crate) fn is_spelled(&self, text: &str) -> bool {
                let mut buffer = [0_u8; $crate::identifier::IDENTIFIER_KEY_WIDTH];
                $crate::identifier::fold_into(text, &mut buffer).is_ok_and(|folded| {
                    Self::from_folded(folded)
                        .map_or_else(|| self.as_str() == folded, |known| known == *self)
                })
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::Error;

            /// Folds `text` and reads the member it names, else the word.
            fn from_str(text: &str) -> $crate::Result<Self> {
                let mut buffer = [0_u8; $crate::identifier::IDENTIFIER_KEY_WIDTH];
                let folded = $crate::identifier::fold_into(text, &mut buffer)
                    .map_err(|expected| $crate::identifier::word_refusal($what, text, expected))?;
                Ok(Self::from_folded(folded).unwrap_or_else(|| {
                    Self::Other($crate::identifier::IdWord(smol_str::SmolStr::new(folded)))
                }))
            }
        }

        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $name {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.as_str().cmp(other.as_str())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.as_str() == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<$name> for smol_str::SmolStr {
            /// The spelling, a member's held statically.
            fn from(word: $name) -> Self {
                match word {
                    $( $name::$variant => smol_str::SmolStr::new_static($spelling), )*
                    $name::Other(word) => word.0,
                }
            }
        }
    };
    (@unit $variant:ident) => { () };
}

pub(crate) use id_vocabulary;

/// The refusal a type, a source or a value earns.
fn refusal(what: &'static str, actual: &str, expected: impl fmt::Display) -> Error {
    Error::InvalidDataType {
        kind: "identifier",
        reason: format_smolstr!("expected {what} of {expected}, got {actual:?}"),
    }
}

/// One identifier: the value a source gave a thing, as a type of name.
///
/// The source and the type are words, [`IdSource::Base`] where nothing names
/// the source, and together they are the identifier's unique key,
/// `src:type`. The value is trimmed text that states something - an empty or
/// null-like value (`null`, `none`, `n/a`) is no identifier - held as its
/// type stores it: an ISIN must close on its check digit and is upper-cased,
/// a pair is canonical ([`IdType::max_value_width`] bounds every type).
///
/// Identifiers order by their unique key as it is spelled, `src:type`,
/// then by value, which is the order [`Identifiers`] holds them in, lays
/// them out in and digests them by.
///
/// ```
/// use yggdryl::{IdSource, IdType, Identifier};
///
/// let isin = Identifier::new(IdSource::Base, IdType::Isin, " us0378331005 ").unwrap();
/// assert_eq!(isin.to_string(), "base:isin=US0378331005");
/// assert!(Identifier::new(IdSource::Fix, IdType::Isin, "US0378331006").is_err(), "a check digit");
/// assert!(Identifier::new(IdSource::Base, IdType::OrderId, "n/a").is_err());
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Identifier {
    src: IdSource,
    kind: IdType,
    value: SmolStr,
}

/// The order of two unique keys as the text `src:type` they spell, compared
/// without building it: a source ending where another goes on with a `.` or
/// a digit sorts after it, as its `:` does.
fn key_order(src: &IdSource, kind: &IdType, other_src: &IdSource, other_kind: &IdType) -> Ordering {
    spelled_key(src, kind).cmp(spelled_key(other_src, other_kind))
}

/// The bytes of the unique key `src:type`, one at a time.
fn spelled_key<'key>(src: &'key IdSource, kind: &'key IdType) -> impl Iterator<Item = u8> + 'key {
    let (src, kind) = (src.as_str().as_bytes(), kind.as_str().as_bytes());
    src.iter().chain(b":").chain(kind).copied()
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Identifier {
    /// By the unique key as spelled, then by value.
    fn cmp(&self, other: &Self) -> Ordering {
        key_order(&self.src, &self.kind, &other.src, &other.kind)
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl Identifier {
    /// Validates and builds one identifier, its value held as its type
    /// stores it.
    ///
    /// # Errors
    ///
    /// A value that states nothing, or one its type refuses.
    pub fn new(src: IdSource, kind: IdType, value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() || is_null_like(value) {
            return Err(refusal(
                "an identifier value",
                value,
                "text stating something",
            ));
        }
        let mut buffer = [0_u8; IDENTIFIER_VALUE_WIDTH];
        let value = SmolStr::new(kind.value_into(value, &mut buffer)?);
        Ok(Self { src, kind, value })
    }

    /// This identifier's value under another type of the same source - a
    /// parent's base, a base's parent - held as that type stores it.
    ///
    /// # Errors
    ///
    /// A value `kind` refuses.
    pub fn with_kind(&self, kind: IdType) -> Result<Self> {
        Self::new(self.src.clone(), kind, &self.value)
    }

    /// The identifier a key names: its unique key `src:type`, else the
    /// type an identifier name at the key's end spells under the source the
    /// rest of the key names, [`IdSource::Base`] where nothing is left.
    ///
    /// The key folds as a word does, so the source keeps its dots and loses
    /// the separators at its ends: `firm.x.ParentOrderID` is
    /// `firm.x:parentorderid`, `OMS_InstrumentID` `oms:instrumentid`,
    /// `marketorderid` `market:orderid`. An identifier name is a type the
    /// crate names whose spelling ends with `id`, the account, an ISIN, a
    /// CUSIP, a SEDOL or a FIGI; the longest one the key ends with answers,
    /// and a parentage word spelled before it - `parent`, `orig`, `origin`,
    /// `original` - stays part of the type. A whole name a security type is
    /// spelled by - `ISINCode`, `security_cusip` - is that type from
    /// [`IdSource::Base`], and a security type is never read off a key that
    /// names another instrument's: `underlyingisin`, `legisin`. `None`
    /// where the key names no identifier, the value states nothing or the
    /// type refuses it.
    ///
    /// ```
    /// use yggdryl::Identifier;
    ///
    /// let keyed = Identifier::from_key("firm.x.ParentOrderID", "P-1").unwrap();
    /// assert_eq!(keyed.to_string(), "firm.x:parentorderid=P-1");
    /// let bridged = Identifier::from_key("OMS_InstrumentID", "dbi;X").unwrap();
    /// assert_eq!(bridged.to_string(), "oms:instrumentid=dbi;X");
    /// assert_eq!(Identifier::from_key("fix:clordid", "C-1").unwrap().to_string(), "fix:clordid=C-1");
    /// assert_eq!(Identifier::from_key("ISINCode", "US0378331005").unwrap().to_string(), "base:isin=US0378331005");
    /// assert!(Identifier::from_key("underlyingisin", "US0378331005").is_none());
    /// assert!(Identifier::from_key("transversalkey", "K-1").is_none());
    /// ```
    #[must_use]
    pub fn from_key(key: &str, value: &str) -> Option<Self> {
        let (src, kind) = Self::key_parts(key, IdType::identifier_names())?;
        Self::new(src, kind, value).ok()
    }

    /// The source and the type a key names, as [`Self::from_key`] reads
    /// them, against `names` - the crate's identifier names, a FIX
    /// component's declared ones, or both.
    pub(crate) fn key_parts<'name>(
        key: &str,
        names: impl IntoIterator<Item = &'name str>,
    ) -> Option<(IdSource, IdType)> {
        let key = key.trim();
        if let Some((src, kind)) = key.split_once(':') {
            return Some((src.parse().ok()?, kind.parse().ok()?));
        }
        if let Some(kind) = IdType::from_field_name(key) {
            return Some((IdSource::Base, kind));
        }
        let mut buffer = [0_u8; IDENTIFIER_KEY_WIDTH];
        let folded = fold_into(key, &mut buffer).ok()?;
        let (at, kind) = IdType::from_key_end(folded, names)?;
        let src = folded[..at].trim_matches('.');
        let src = if src.is_empty() {
            IdSource::Base
        } else {
            src.parse().ok()?
        };
        Some((src, kind))
    }

    /// The source that gave it: `oms`, `proprietary`, [`IdSource::Base`].
    #[must_use]
    pub fn src(&self) -> &IdSource {
        &self.src
    }

    /// The type of name this is - the `type` column: `isin`,
    /// `executingtrader`, `clordid`.
    #[must_use]
    pub fn kind(&self) -> &IdType {
        &self.kind
    }

    /// The value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether this identifier's key is `src:kind`.
    #[must_use]
    pub fn is_of(&self, src: &IdSource, kind: &IdType) -> bool {
        self.src == *src && self.kind == *kind
    }

    /// The unique key, `src:type`, as the map an [`Identifiers`] lays out
    /// keys it.
    ///
    /// ```
    /// use yggdryl::{IdSource, IdType, Identifier};
    ///
    /// let id = Identifier::new(IdSource::Fix, IdType::ClOrdId, "C-1").unwrap();
    /// assert_eq!(id.key(), "fix:clordid");
    /// ```
    #[must_use]
    pub fn key(&self) -> SmolStr {
        // Two ASCII words of at most `IDENTIFIER_KEY_WIDTH` bytes each,
        // spelled on the stack and copied once: inline within `SmolStr`'s
        // width, one allocation past it.
        let mut buffer = [0_u8; 2 * IDENTIFIER_KEY_WIDTH + 1];
        let mut len = 0;
        for (slot, byte) in buffer.iter_mut().zip(spelled_key(&self.src, &self.kind)) {
            *slot = byte;
            len += 1;
        }
        SmolStr::new(std::str::from_utf8(&buffer[..len]).expect("ASCII words"))
    }

    /// The identifier's Arrow row: `struct<src, type, value>`, each required
    /// text.
    #[must_use]
    pub fn dtype() -> DataType {
        DataType::Struct(StructType::from_unique_fields(vec![
            Field::new("src", DataType::utf8(), false),
            Field::new("type", DataType::utf8(), false),
            Field::new("value", DataType::utf8(), false),
        ]))
    }

    /// The identifier as its row.
    #[must_use]
    pub fn into_scalar(self) -> Scalar {
        Scalar::from_sequence([
            Scalar::from(SmolStr::from(self.src)),
            Scalar::from(SmolStr::from(self.kind)),
            Scalar::from(self.value),
        ])
    }

    /// Reads one identifier back from its row.
    ///
    /// # Errors
    ///
    /// Anything but three text cells, or cells [`Self::new`] refuses.
    pub fn from_scalar(scalar: &Scalar) -> Result<Self> {
        let refused = || {
            refusal(
                "an identifier row",
                scalar.kind(),
                "three text cells: src, type, value",
            )
        };
        if scalar.get(3).is_some() {
            return Err(refused());
        }
        let cell = |at: usize| {
            scalar
                .get(at)
                .and_then(|cell| cell.as_str().map(str::to_owned))
                .ok_or_else(refused)
        };
        Self::new(cell(0)?.parse()?, cell(1)?.parse()?, &cell(2)?)
    }
}

impl fmt::Display for Identifier {
    /// `src:type=value`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}={}", self.src, self.kind, self.value)
    }
}

/// A sorted map of identifiers, one per unique key `src:type`.
///
/// Held sorted by the key as it is spelled, so a lookup by key is a binary
/// search, an insertion one shifted slice, a digest one walk in a canonical
/// order, two maps equal exactly when they hold the same identifiers, and
/// the Arrow layout - a sorted map from the key to the identifier
/// ([`Self::dtype`]) - is written in the order it is held. The map is one
/// vector, three words wide: an empty one - what most market data states -
/// holds no backing, and a stated one holds its identifiers in one
/// allocation, however many there are; the key is spelled only where it is
/// written out. A second value under a key already held is a statement of
/// the same name: [`Self::insert`] keeps the first, [`Self::set`] replaces
/// it.
///
/// ```
/// use yggdryl::{IdSource, IdType, Identifier, Identifiers};
///
/// let mut ids = Identifiers::new();
/// assert!(ids.insert(Identifier::new(IdSource::Base, IdType::Isin, "US0378331005").unwrap()));
/// assert!(!ids.insert(Identifier::new(IdSource::Base, IdType::Isin, "CH0012214059").unwrap()), "fill only");
/// let venue: IdSource = "venue".parse().unwrap();
/// assert!(ids.insert(Identifier::new(venue.clone(), IdType::InstrumentId, "dbi;X").unwrap()));
/// assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"));
/// assert_eq!(ids.get_from(&venue, &IdType::InstrumentId), Some("dbi;X"));
/// assert_eq!(ids.to_string(), "[base:isin=US0378331005, venue:instrumentid=dbi;X]");
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Identifiers(Vec<Identifier>);

impl Identifiers {
    /// An empty set.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// How many identifiers the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the set holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every identifier, sorted by its key.
    pub fn iter(&self) -> std::slice::Iter<'_, Identifier> {
        self.0.iter()
    }

    /// The identifiers held, sorted.
    #[must_use]
    pub fn as_slice(&self) -> &[Identifier] {
        &self.0
    }

    /// Where the identifier keyed `src:kind` is, or where it would go.
    fn position(&self, src: &IdSource, kind: &IdType) -> std::result::Result<usize, usize> {
        self.0
            .binary_search_by(|held| key_order(held.src(), held.kind(), src, kind))
    }

    /// The value [`get_identifier`](Self::get_identifier) answers.
    #[must_use]
    pub fn get(&self, kind: &IdType) -> Option<&str> {
        self.get_identifier(kind).map(Identifier::value)
    }

    /// The first identifier of `kind` a named source stated, else the one
    /// stated under no source (`base`), else the one the crate derived.
    #[must_use]
    pub fn get_identifier(&self, kind: &IdType) -> Option<&Identifier> {
        let rank = |held: &Identifier| match held.src() {
            IdSource::Base => 1,
            IdSource::Derived => 2,
            _ => 0,
        };
        let mut best: Option<&Identifier> = None;
        for held in self.0.iter().filter(|held| held.kind() == kind) {
            if rank(held) == 0 {
                return Some(held);
            }
            if best.is_none_or(|best| rank(held) < rank(best)) {
                best = Some(held);
            }
        }
        best
    }

    /// The value of the identifier keyed `src:kind`.
    #[must_use]
    pub fn get_from(&self, src: &IdSource, kind: &IdType) -> Option<&str> {
        self.position(src, kind).ok().map(|at| self.0[at].value())
    }

    /// Whether the set holds an identifier of `kind`.
    #[must_use]
    pub fn contains_kind(&self, kind: &IdType) -> bool {
        self.0.iter().any(|held| held.kind() == kind)
    }

    /// Every identifier of `kind`, one per source.
    pub fn of_kind<'set>(&'set self, kind: &'set IdType) -> impl Iterator<Item = &'set Identifier> {
        self.0.iter().filter(move |held| held.kind() == kind)
    }

    /// Adds `id` where no identifier of its key is held; whether it was
    /// added.
    pub fn insert(&mut self, id: Identifier) -> bool {
        match self.position(id.src(), id.kind()) {
            Ok(_) => false,
            Err(at) => {
                self.0.insert(at, id);
                true
            }
        }
    }

    /// Holds `id`, replacing the value held under its key; whether anything
    /// moved.
    pub fn set(&mut self, id: Identifier) -> bool {
        match self.position(id.src(), id.kind()) {
            Ok(at) if self.0[at] == id => false,
            Ok(at) => {
                self.0[at] = id;
                true
            }
            Err(at) => {
                self.0.insert(at, id);
                true
            }
        }
    }

    /// Removes the identifier keyed `src:kind`, answering it.
    pub fn remove(&mut self, src: &IdSource, kind: &IdType) -> Option<Identifier> {
        let at = self.position(src, kind).ok()?;
        Some(self.0.remove(at))
    }

    /// Removes every identifier of `kind`, answering how many.
    pub fn remove_kind(&mut self, kind: &IdType) -> usize {
        let before = self.0.len();
        self.0.retain(|held| held.kind() != kind);
        before - self.0.len()
    }

    /// Adds every identifier of `other` whose key this set does not hold;
    /// whether any was.
    pub fn merge(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for id in other.iter() {
            changed |= self.insert(id.clone());
        }
        changed
    }

    /// Removes every identifier.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Carries each identifier `previous` - the same element's set one step
    /// earlier in its chain - holds under a key this set does not, where
    /// `carried` admits it; whether any was.
    pub fn carry(&mut self, previous: &Self, carried: impl Fn(&Identifier) -> bool) -> bool {
        let mut moved = false;
        for id in previous.iter() {
            if carried(id) && self.position(id.src(), id.kind()).is_err() {
                moved |= self.insert(id.clone());
            }
        }
        moved
    }

    /// The parents each base identifier this map states takes from
    /// `previous`, its chain's statement one step earlier, under the same
    /// source and only where the map holds none under that parent's key.
    /// `parents_of` names a base's parent types, nearest first, and
    /// `parent_of` says which base a type is a parent of, so a parent type
    /// is never taken for a base of its own.
    ///
    /// Where the base kept its value, each parent is the previous one.
    /// Where it changed, the first parent is the base's previous value,
    /// each middle one the previous parent one step nearer, and the last of
    /// two or more the chain's first value - the previous last parent, else
    /// the farthest previous parent stated, else the previous value - so
    /// `orderid` A, B, C, D carries `parentorderid` C and `origorderid` A,
    /// and a one-parent list (`clordid`'s `origclordid`) the previous value.
    /// Whether anything moved.
    ///
    /// ```
    /// use yggdryl::{IdSource, IdType, Identifier, Identifiers};
    ///
    /// let id = |kind: &str, value: &str| {
    ///     Identifier::new(IdSource::Fix, kind.parse().unwrap(), value).unwrap()
    /// };
    /// let mut chain: Identifiers = [id("orderid", "A")].into_iter().collect();
    /// for value in ["B", "C", "D"] {
    ///     let mut next: Identifiers = [id("orderid", value)].into_iter().collect();
    ///     next.follow_parents(&chain, IdType::parents, IdType::parent_of);
    ///     chain = next;
    /// }
    /// assert_eq!(chain.get(&"parentorderid".parse().unwrap()), Some("C"));
    /// assert_eq!(chain.get(&"origorderid".parse().unwrap()), Some("A"));
    /// ```
    pub fn follow_parents<'list>(
        &mut self,
        previous: &Self,
        parents_of: impl Fn(&IdType) -> Cow<'list, [IdType]>,
        parent_of: impl Fn(&IdType) -> Option<(IdType, usize)>,
    ) -> bool {
        let mut fills: Vec<(IdSource, IdType, &str)> = Vec::new();
        for id in self.iter().filter(|id| parent_of(id.kind()).is_none()) {
            let Some(before) = previous.get_from(id.src(), id.kind()) else {
                continue;
            };
            let parents = parents_of(id.kind());
            let last = parents.len().saturating_sub(1);
            let held = |at: usize| previous.get_from(id.src(), &parents[at]);
            for (at, parent) in parents.iter().enumerate() {
                if self.get_from(id.src(), parent).is_some() {
                    continue;
                }
                let value = if before == id.value() {
                    held(at)
                } else if at == 0 {
                    Some(before)
                } else if at == last {
                    (0..=at).rev().find_map(held).or(Some(before))
                } else {
                    held(at - 1)
                };
                fills.extend(value.map(|value| (id.src().clone(), parent.clone(), value)));
            }
        }
        let mut moved = false;
        for (src, kind, value) in fills {
            if let Ok(id) = Identifier::new(src, kind, value) {
                moved |= self.insert(id);
            }
        }
        moved
    }

    /// Fills each base a parent identifier names and the map does not
    /// state, under the parent's source, with the value of the nearest
    /// parent it states: an element stating where it came from but not
    /// what it is now is what it came from. `parent_of` says which base a
    /// type is a parent of and its place among the base's parents. Whether
    /// anything moved.
    ///
    /// ```
    /// use yggdryl::{IdSource, IdType, Identifier, Identifiers};
    ///
    /// let id = |kind: &str, value: &str| {
    ///     Identifier::new(IdSource::Fix, kind.parse().unwrap(), value).unwrap()
    /// };
    /// let mut ids: Identifiers = [id("origorderid", "A"), id("parentorderid", "C")].into_iter().collect();
    /// assert!(ids.fill_parents(IdType::parent_of));
    /// assert_eq!(ids.get(&IdType::OrderId), Some("C"), "the nearest parent");
    /// ```
    pub fn fill_parents(&mut self, parent_of: impl Fn(&IdType) -> Option<(IdType, usize)>) -> bool {
        let mut fills: Vec<(usize, Identifier)> = self
            .iter()
            .filter_map(|id| {
                let (base, at) = parent_of(id.kind())?;
                let filled = self.get_from(id.src(), &base).is_none();
                filled
                    .then(|| id.with_kind(base).ok())
                    .flatten()
                    .map(|base| (at, base))
            })
            .collect();
        fills.sort_by_key(|(at, _)| *at);
        let mut moved = false;
        for (_, id) in fills {
            moved |= self.insert(id);
        }
        moved
    }

    /// The Arrow datatype a map lays out: a sorted map from the required
    /// text key `src:type` to the required [`Identifier::dtype`] it names,
    /// the value field named `item` - `securityid`, `identifier`,
    /// `partyid`.
    #[must_use]
    pub fn dtype(item: &str) -> DataType {
        let entries = StructType::from_unique_fields(vec![
            Field::new("key", DataType::utf8(), false),
            Field::new(item, Identifier::dtype(), false),
        ]);
        DataType::map(
            Field::new("entries", DataType::Struct(entries), false),
            true,
        )
        .expect("a text key beside an identifier row is a map's entries")
    }

    /// The map as its sorted entries, each key `src:type` beside its row.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        let entries = self
            .0
            .iter()
            .map(|id| (Scalar::from(id.key()), id.clone().into_scalar()));
        match Scalar::from_mapping(entries).expect("the keys of a map are unique") {
            Scalar::Map(entries) => Scalar::SortedMap(entries),
            empty => empty,
        }
    }

    /// Reads a map back from its entries, in any order, or from a
    /// sequence of identifier rows, each keyed by its own `src:type`.
    ///
    /// # Errors
    ///
    /// Anything but a map or a sequence of identifier rows, a row
    /// [`Identifier::from_scalar`] refuses, a key that is not the
    /// `src:type` of the row it keys, or two rows of a sequence stating
    /// one key with two values, each located on its key or its place.
    pub fn from_scalar(scalar: &Scalar) -> Result<Self> {
        let mut map = Self::new();
        if let Some(entries) = scalar.as_mapping() {
            for (key, row) in entries {
                let key = key.as_str().unwrap_or_default();
                let path = || format_smolstr!("$['{}']", crate::text::elide_to(key, 64));
                let id = Identifier::from_scalar(row).map_err(|error| located(path(), error))?;
                if id.key() != key {
                    return Err(Error::InvalidRecord {
                        path: path(),
                        reason: format_smolstr!(
                            "expected the key {}, got {:?}",
                            id.key(),
                            crate::text::elide_to(key, 64)
                        ),
                    });
                }
                map.insert(id);
            }
            return Ok(map);
        }
        let Some(rows) = scalar.sequence_rows() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    format_args!("a map or a sequence of identifiers"),
                    scalar.kind(),
                ),
            });
        };
        for (at, row) in rows.iter().enumerate() {
            let path = || format_smolstr!("$[{at}]");
            let id = Identifier::from_scalar(row).map_err(|error| located(path(), error))?;
            if let Some(held) = map
                .get_from(id.src(), id.kind())
                .filter(|held| *held != id.value())
            {
                return Err(Error::InvalidRecord {
                    path: path(),
                    reason: format_smolstr!(
                        "expected one value under {}, got {:?} and {:?}",
                        id.key(),
                        crate::text::elide_to(held, 64),
                        crate::text::elide_to(id.value(), 64)
                    ),
                });
            }
            map.insert(id);
        }
        Ok(map)
    }
}

/// `error` relocated onto `path`.
fn located(path: SmolStr, error: Error) -> Error {
    let reason = match error {
        Error::InvalidDataType { reason, .. } | Error::InvalidRecord { reason, .. } => reason,
        other => format_smolstr!("{other}"),
    };
    Error::InvalidRecord { path, reason }
}

impl<'set> IntoIterator for &'set Identifiers {
    type Item = &'set Identifier;
    type IntoIter = std::slice::Iter<'set, Identifier>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<Identifier> for Identifiers {
    /// The set the identifiers make, the first of one key kept.
    fn from_iter<I: IntoIterator<Item = Identifier>>(ids: I) -> Self {
        let mut set = Self::new();
        for id in ids {
            set.insert(id);
        }
        set
    }
}

impl fmt::Display for Identifiers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[")?;
        for (index, id) in self.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{id}")?;
        }
        formatter.write_str("]")
    }
}
