//! Identifiers: what a thing is called, by whom, as what.
//!
//! One [`Identifier`] is a value under a key, an [`IdKey`]: the source that
//! gave it and the type of name it is - `isin=US0378331005`,
//! `proprietary:executingtrader=trader1`,
//! `oms:instrumentid=dbi;CH0012214059_XSWX_CHF` - a key from the base source
//! spelled as its type alone. [`Identifiers`] is the map, sorted by the key
//! as spelled, of one value per key a market element states three of - its
//! `securityids`, its own `identifiers` and its `partyids` - so a party, an
//! ISIN and a client order identifier are read, written, merged and digested
//! alike, and laid out as one sorted Arrow `map<utf8, utf8>`.
//!
//! The base key of a type is the type's answer: a named source stating a
//! type fills it where it is empty, so `ullink:isin=X` alone is also
//! `isin=X`, and the map answers `isin` whichever source stated it; the
//! whole rule is [`Identifiers`]'.
//!
//! A source is an [`IdSource`] and a type an [`IdType`]: words folded to
//! lower case, the ones the crate names held as members that cost nothing,
//! any other held as an `Other` word. A key is read from its spelling
//! exactly; a name that spells no key - a FIX entry no dictionary maps -
//! names one by the identifier name it ends with and the source the rest of
//! it spells ([`Identifier::from_key`]).
//!
//! Parentage is a relation between types, never a part of a value: a type
//! has a list of parent types, nearest first ([`IdType::parents`]) -
//! `orderid`'s are `parentorderid`, the value it held before it last
//! changed, then `origorderid`, the value its chain first stated, and
//! `clordid`'s is `origclordid` alone, FIX's previous client order
//! identifier. Along a chain a follower takes the parents of each type it
//! states from its predecessor ([`Identifiers::follow_parents`]), and an
//! element stating a parent but not the type it is a parent of takes that
//! type from its nearest parent once it finalizes
//! ([`Identifiers::fill_parents`]).

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::code::is_null_like;
use crate::{DataType, Error, Field, IdKey, IdSource, IdType, Map, Result, Scalar, StructType};

/// The most bytes a type or a source may be once folded: room for the
/// longest name a FIX code set gives a party role or an identifier source.
pub const IDENTIFIER_WORD_WIDTH: usize = 64;

/// The most bytes a value may be once trimmed.
pub const IDENTIFIER_VALUE_WIDTH: usize = 64;

/// The most bytes two words spell with one byte between them: a key's
/// `src:type`, or a spelling folded whole before the word it ends with is
/// split off it.
pub(crate) const WORD_PAIR_WIDTH: usize = 2 * IDENTIFIER_WORD_WIDTH + 1;

/// A word no [`IdType`] or [`IdSource`] member names, folded: lower-case
/// ASCII letters, digits and `.`, one to [`IDENTIFIER_WORD_WIDTH`] bytes.
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
/// `executing_trader` are `executingtrader`. A word folds into
/// [`IDENTIFIER_WORD_WIDTH`] bytes, a word pair into [`WORD_PAIR_WIDTH`].
///
/// # Errors
///
/// What the fold expected, where `word` folds to nothing, past the
/// buffer's bytes, or holds a byte other than an ASCII letter, a digit or
/// `.`. The width the refusal names is a word's: a word pair's fold is read
/// only for whether it held.
pub(crate) fn fold_into<'buffer>(
    word: &str,
    buffer: &'buffer mut [u8],
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
    folded_len(word).is_some_and(|len| (1..=IDENTIFIER_WORD_WIDTH).contains(&len))
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

            /// Where a word the crate names stands in [`Self::KNOWN`], or
            /// `None` for an `Other` word.
            pub(crate) const fn ordinal(&self) -> Option<usize> {
                /// One unit variant per member, numbered as it is declared.
                #[allow(dead_code)]
                enum Ordinal {
                    $( $variant, )*
                }
                match self {
                    $( Self::$variant => Some(Ordinal::$variant as usize), )*
                    Self::Other(_) => None,
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
        }

        impl std::str::FromStr for $name {
            type Err = $crate::Error;

            /// Folds `text` and reads the member it names, else the word.
            fn from_str(text: &str) -> $crate::Result<Self> {
                let mut buffer = [0_u8; $crate::identifier::IDENTIFIER_WORD_WIDTH];
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

/// One identifier: a value under its key, the source that gave it and the
/// type of name it is.
///
/// The key is an [`IdKey`]: `isin` where nothing names the source,
/// `ullink:isin` from `ullink`. The value is trimmed text that states
/// something - an empty or null-like value (`null`, `none`, `n/a`) is no
/// identifier - held as its type stores it: an ISIN must close on its check
/// digit and is upper-cased, a pair is canonical
/// ([`IdType::max_value_width`] bounds every type).
///
/// Identifiers order by their key as it is spelled, then by value, which is
/// the order [`Identifiers`] holds them in, lays them out in and digests them
/// by.
///
/// ```
/// use yggdryl::{IdKey, IdType, Identifier};
///
/// let isin = Identifier::new(IdKey::base(IdType::Isin), " us0378331005 ").unwrap();
/// assert_eq!(isin.to_string(), "isin=US0378331005");
/// let bridged = Identifier::new("ullink:isin".parse().unwrap(), "US0378331005").unwrap();
/// assert_eq!(bridged.to_string(), "ullink:isin=US0378331005");
/// assert!(Identifier::new(IdKey::base(IdType::Isin), "US0378331006").is_err(), "a check digit");
/// assert!(Identifier::new(IdKey::base(IdType::OrderId), "n/a").is_err());
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Identifier {
    key: IdKey,
    value: SmolStr,
}

impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Identifier {
    /// By the key as spelled, then by value.
    fn cmp(&self, other: &Self) -> Ordering {
        self.key
            .cmp(&other.key)
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
    pub fn new(key: IdKey, value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() || is_null_like(value) {
            return Err(refusal(
                "an identifier value",
                value,
                "text stating something",
            ));
        }
        let mut buffer = [0_u8; IDENTIFIER_VALUE_WIDTH];
        let value = SmolStr::new(key.kind().value_into(value, &mut buffer)?);
        Ok(Self { key, value })
    }

    /// This identifier's value under another type of the same source - a
    /// parent's type, a type's parent - held as that type stores it.
    ///
    /// # Errors
    ///
    /// A value `kind` refuses.
    pub fn with_kind(&self, kind: IdType) -> Result<Self> {
        Self::new(self.key.with_kind(kind), &self.value)
    }

    /// The identifier a name no key spells names: its explicit `src:type`,
    /// else the type an identifier name at the name's end spells under the
    /// source the rest of the name names, the base source where nothing is
    /// left or where that rest folds to a source the crate reserves -
    /// `base`, which `fix` spells too, or `derived` - which names no
    /// namespace: `Derived_ISIN` and `FIX.ISIN` are `isin`, so a namespace
    /// spelled before the name never files a code under `derived`. An
    /// explicit `src:type` keeps the source it spells, `derived:isin`
    /// included.
    ///
    /// The name folds as a word does - the source and the type each held to
    /// a word's width, never the key they spell together - so the source
    /// keeps its dots and loses the separators at its ends:
    /// `firm.x.ParentOrderID` is `firm.x:parentorderid`, `OMS_InstrumentID`
    /// `oms:instrumentid`, `marketorderid` `market:orderid`. An identifier
    /// name is a type the crate names whose spelling ends with `id`, the
    /// account, an ISIN, a CUSIP, a SEDOL or a FIGI; the longest one the name
    /// ends with answers, and a parentage word spelled before it - `parent`,
    /// `orig`, `origin`, `original` - stays part of the type. A whole name a
    /// security type is spelled by - `ISINCode`, `security_cusip` - is that
    /// type from the base source, and a security type is never read off a
    /// name that names another instrument's - `leg`, `underlying`, `contra`,
    /// `related` or `benchmark` opening the name or spelled just before the
    /// type, after any namespace: `underlyingisin`, `OMS_UnderlyingISIN`,
    /// `FIX.LegISIN`. `None` where the name names no identifier, the value
    /// states nothing or the type refuses it.
    ///
    /// A key's own spelling is read exactly by [`IdKey`]'s
    /// [`FromStr`](std::str::FromStr); this reading is for the names a source gives
    /// its fields.
    ///
    /// ```
    /// use yggdryl::Identifier;
    ///
    /// let keyed = Identifier::from_key("firm.x.ParentOrderID", "P-1").unwrap();
    /// assert_eq!(keyed.to_string(), "firm.x:parentorderid=P-1");
    /// let bridged = Identifier::from_key("OMS_InstrumentID", "dbi;X").unwrap();
    /// assert_eq!(bridged.to_string(), "oms:instrumentid=dbi;X");
    /// assert_eq!(Identifier::from_key("fix:clordid", "C-1").unwrap().to_string(), "clordid=C-1");
    /// assert_eq!(Identifier::from_key("Derived_ISIN", "US0378331005").unwrap().to_string(), "isin=US0378331005");
    /// assert_eq!(Identifier::from_key("ISINCode", "US0378331005").unwrap().to_string(), "isin=US0378331005");
    /// assert!(Identifier::from_key("underlyingisin", "US0378331005").is_none());
    /// assert!(Identifier::from_key("OMS_UnderlyingISIN", "US0378331005").is_none());
    /// assert!(Identifier::from_key("transversalkey", "K-1").is_none());
    /// ```
    #[must_use]
    pub fn from_key(key: &str, value: &str) -> Option<Self> {
        Self::new(IdKey::infer(key, IdType::identifier_names())?, value).ok()
    }

    /// The key: its source and its type.
    ///
    /// ```
    /// use yggdryl::{IdKey, IdType, Identifier};
    ///
    /// let id = Identifier::new(IdKey::base(IdType::ClOrdId), "C-1").unwrap();
    /// assert_eq!(id.key(), "clordid");
    /// ```
    #[must_use]
    pub fn key(&self) -> &IdKey {
        &self.key
    }

    /// The source that gave it: `oms`, `proprietary`, [`IdSource::Base`].
    #[must_use]
    pub fn src(&self) -> &IdSource {
        self.key.src()
    }

    /// The type of name this is: `isin`, `executingtrader`, `clordid`.
    #[must_use]
    pub fn kind(&self) -> &IdType {
        self.key.kind()
    }

    /// The value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The same value under the base key of its type.
    fn into_base(self) -> Self {
        Self {
            key: IdKey::base(self.key.kind().clone()),
            value: self.value,
        }
    }
}

impl fmt::Display for Identifier {
    /// `key=value`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}={}", self.key, self.value)
    }
}

/// A sorted map of identifiers, one value per key.
///
/// Held sorted by the key as it is spelled, so a lookup by key is a binary
/// search, an insertion one shifted slice, a digest one walk in a canonical
/// order, two maps equal exactly when they hold the same identifiers, and
/// the Arrow layout - a sorted `map<utf8, utf8>` from the key to the value
/// ([`Self::dtype`]) - is written in the order it is held. The map is one
/// vector, three words wide: an empty one - what most market data states -
/// holds no backing.
///
/// The base key of a type is the type's answer ([`Self::get`]), and one
/// rule keeps it, for every map:
///
/// | A statement | What moves |
/// | --- | --- |
/// | a named source states a type | the base key is filled where it is empty: `ullink:isin=X` alone is also `isin=X` |
/// | anything but a derivation states a type | the derivation of it, `derived:T`, is taken back first, and the base key it filled leaves with it |
/// | a derivation, `derived:T` | it lands only where nothing of its type is held, and fills the base key; replaced or removed, the base key follows it while that is all the base key holds |
/// | the base key itself | it moves only through its own key: [`Self::insert`] fills it, [`Self::set`] replaces it, and removing it removes every key of its type |
///
/// So wherever a type is held, its base key is, and replacing or removing
/// a named source leaves the base key as it was: a bridge restating the
/// wire's code under its own name never overwrites or erases the wire's. A
/// map read back from its scalar is closed by the same rule - a type with no
/// base key takes its first named source's value in key order, else its
/// derivation's - and a map the crate wrote comes back unchanged.
///
/// ```
/// use yggdryl::{IdKey, IdType, Identifier, Identifiers};
///
/// let id = |key: &str, value: &str| Identifier::new(key.parse().unwrap(), value).unwrap();
/// let mut ids = Identifiers::new();
/// assert!(ids.insert(id("ullink:isin", "US0378331005")));
/// assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"), "the source fills the base key");
/// assert!(!ids.insert(id("isin", "CH0012214059")), "fill only");
/// assert!(ids.insert(id("oms:instrumentid", "dbi;X")));
/// assert_eq!(ids.get_from(&"oms:instrumentid".parse().unwrap()), Some("dbi;X"));
/// assert_eq!(
///     ids.to_string(),
///     "[instrumentid=dbi;X, isin=US0378331005, oms:instrumentid=dbi;X, ullink:isin=US0378331005]"
/// );
/// assert!(ids.remove(&IdKey::base(IdType::Isin)).is_some(), "the whole type");
/// assert_eq!(ids.len(), 2);
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Identifiers(Vec<Identifier>);

impl Identifiers {
    /// An empty map.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// An empty map with room for `capacity` identifiers.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// How many identifiers the map holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the map holds none.
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

    /// Where the identifier keyed `key` is, or where it would go.
    fn position(&self, key: &IdKey) -> std::result::Result<usize, usize> {
        self.0.binary_search_by(|held| held.key.cmp(key))
    }

    /// Where the identifier keyed `src:kind` is, or where it would go,
    /// found without building its key.
    fn position_of(&self, src: &IdSource, kind: &IdType) -> std::result::Result<usize, usize> {
        self.0
            .binary_search_by(|held| held.key.cmp_parts(src, kind))
    }

    /// The value keyed `src:kind`.
    fn value_of(&self, src: &IdSource, kind: &IdType) -> Option<&str> {
        self.position_of(src, kind)
            .ok()
            .map(|at| self.0[at].value())
    }

    /// Holds `id` under its key, replacing what was held there.
    fn put(&mut self, id: Identifier) {
        match self.position(&id.key) {
            Ok(at) => self.0[at] = id,
            Err(at) => self.0.insert(at, id),
        }
    }

    /// Holds `id`, and its value under the base key of its type where that
    /// is empty.
    fn put_filling(&mut self, id: Identifier) {
        let fill = self
            .position_of(&IdSource::Base, id.kind())
            .is_err()
            .then(|| id.clone().into_base());
        self.put(id);
        if let Some(fill) = fill {
            self.put(fill);
        }
    }

    /// The named sources of `kind` - neither the base source nor a
    /// derivation - in key order.
    fn named<'map>(&'map self, kind: &'map IdType) -> impl Iterator<Item = &'map Identifier> {
        self.0.iter().filter(move |held| {
            held.kind() == kind && !matches!(held.src(), IdSource::Base | IdSource::Derived)
        })
    }

    /// The value of `kind`'s base key: the type's answer, whichever source
    /// stated it.
    #[must_use]
    pub fn get(&self, kind: &IdType) -> Option<&str> {
        self.value_of(&IdSource::Base, kind)
    }

    /// The value held under exactly `key`.
    #[must_use]
    pub fn get_from(&self, key: &IdKey) -> Option<&str> {
        self.position(key).ok().map(|at| self.0[at].value())
    }

    /// Whether the map holds `kind`: its base key, which it holds wherever
    /// it holds any key of the type.
    #[must_use]
    pub fn contains_kind(&self, kind: &IdType) -> bool {
        self.position_of(&IdSource::Base, kind).is_ok()
    }

    /// Every identifier of `kind`, its base key included, in key order.
    pub fn of_kind<'map>(&'map self, kind: &'map IdType) -> impl Iterator<Item = &'map Identifier> {
        self.0.iter().filter(move |held| held.kind() == kind)
    }

    /// Whether `kind`'s base key holds only a derivation: the value of
    /// `derived:kind`, which no named source of the type states.
    #[must_use]
    pub fn is_derived(&self, kind: &IdType) -> bool {
        let (Some(answer), Some(derived)) = (
            self.value_of(&IdSource::Base, kind),
            self.value_of(&IdSource::Derived, kind),
        ) else {
            return false;
        };
        answer == derived && !self.named(kind).any(|held| held.value() == answer)
    }

    /// Whether `id` is a base key holding only a derivation: an echo of a
    /// statement the map holds under `derived`, never one of its own.
    fn is_echo(&self, id: &Identifier) -> bool {
        id.key.is_base() && self.is_derived(id.kind())
    }

    /// Whether anything but a derivation states `kind`: a named source, or
    /// a base key holding more than a derivation.
    fn states(&self, kind: &IdType) -> bool {
        self.named(kind).next().is_some() || (self.contains_kind(kind) && !self.is_derived(kind))
    }

    /// Takes back the derivation of `kind`, answering it. A base key that
    /// held only it leaves with it, unless a named source of the type is
    /// still held, whose value it takes.
    fn take_derivation(&mut self, kind: &IdType) -> Option<Identifier> {
        let at = self.position_of(&IdSource::Derived, kind).ok()?;
        let echoed = self.is_derived(kind);
        let taken = self.0.remove(at);
        if echoed {
            let answer = self
                .position_of(&IdSource::Base, kind)
                .expect("a derivation's echo is held");
            let named = self.named(kind).next().map(|held| held.value.clone());
            match named {
                Some(value) => self.0[answer].value = value,
                None => drop(self.0.remove(answer)),
            }
        }
        Some(taken)
    }

    /// Adds `id` where nothing is held under its key, filling the base key
    /// of its type where that is empty; whether `id` landed.
    ///
    /// A statement - any source but a derivation - takes back the
    /// derivation of its type first, and a base `id` over a base key that
    /// holds only a derivation replaces it. A derivation lands only where
    /// nothing of its type is held.
    pub fn insert(&mut self, id: Identifier) -> bool {
        match id.src() {
            IdSource::Derived => {
                if self.contains_kind(id.kind()) {
                    return false;
                }
                self.put_filling(id);
            }
            IdSource::Base => {
                if self.position(&id.key).is_ok() && !self.is_derived(id.kind()) {
                    return false;
                }
                self.take_derivation(id.kind());
                self.put(id);
            }
            _ => {
                if self.position(&id.key).is_ok() {
                    return false;
                }
                self.take_derivation(id.kind());
                self.put_filling(id);
            }
        }
        true
    }

    /// Holds `id`, replacing the value held under its key, and fills the
    /// base key of its type where that is empty; whether anything moved.
    ///
    /// A base `id` replaces the type's answer and leaves its named sources
    /// alone; a named one leaves the answer as it was. A statement takes
    /// back the derivation of its type first, and a derivation is refused
    /// where anything else states its type - replacing one moves the base key
    /// with it while that is all the base key holds.
    pub fn set(&mut self, id: Identifier) -> bool {
        if *id.src() == IdSource::Derived {
            if self.states(id.kind()) || self.get_from(&id.key) == Some(id.value()) {
                return false;
            }
            let echo = id.clone().into_base();
            self.put(id);
            self.put(echo);
            return true;
        }
        let derived = self.position_of(&IdSource::Derived, id.kind()).is_ok();
        if !derived && self.get_from(&id.key) == Some(id.value()) {
            return false;
        }
        self.take_derivation(id.kind());
        self.put_filling(id);
        true
    }

    /// Removes what `key` holds, answering it. A named source's key removes
    /// that identifier alone; `derived:T` removes the derivation, its echo
    /// in the base key following it; and a base key removes every key of
    /// its type, answering the base one.
    pub fn remove(&mut self, key: &IdKey) -> Option<Identifier> {
        match key.src() {
            IdSource::Base => {
                let answer = self.position(key).ok().map(|at| self.0.remove(at));
                self.0.retain(|held| held.kind() != key.kind());
                answer
            }
            IdSource::Derived => self.take_derivation(key.kind()),
            _ => {
                let at = self.position(key).ok()?;
                Some(self.0.remove(at))
            }
        }
    }

    /// Takes every key `other` holds and this map does not; where both hold
    /// a key with two values, `other`'s when it is `later`, else this map's.
    /// A derivation of `other`'s lands only where this map holds nothing of
    /// its type, and a base key of `other`'s that only echoes its derivation
    /// comes with it rather than as a statement. Whether anything moved.
    pub fn merge(&mut self, other: &Self, later: bool) -> bool {
        let mut moved = false;
        for id in other.iter().filter(|id| !other.is_echo(id)) {
            let replaced = later
                && *id.src() != IdSource::Derived
                && self
                    .get_from(&id.key)
                    .is_some_and(|held| held != id.value());
            moved |= if replaced {
                self.set(id.clone())
            } else {
                self.insert(id.clone())
            };
        }
        moved
    }

    /// Removes every identifier.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Carries each identifier `previous` - the same element's map one step
    /// earlier in its chain - holds under a key this map does not, where
    /// `carried` admits it, through [`Self::insert`]: a base key lands only
    /// where this map holds nothing of its type, and one that only echoes a
    /// derivation comes with its derivation. Whether any was.
    pub fn carry(&mut self, previous: &Self, carried: impl Fn(&Identifier) -> bool) -> bool {
        let mut moved = false;
        for id in previous.iter() {
            if !previous.is_echo(id) && carried(id) && self.position(&id.key).is_err() {
                moved |= self.insert(id.clone());
            }
        }
        moved
    }

    /// The parents each type this map states takes from `previous`, its
    /// chain's statement one step earlier, under the same source and only
    /// where the map holds none under that parent's key. `parents_of` names
    /// a type's parent types, nearest first, and `parent_of` says which type
    /// a parent type is a parent of, so a parent type is never given parents
    /// of its own. A base key that only echoes a derivation takes none.
    ///
    /// Where the type kept its value, each parent is the previous one.
    /// Where it changed, the first parent is the type's previous value, each
    /// middle one the previous parent one step nearer, and the last of two
    /// or more the chain's first value - the previous last parent, else the
    /// farthest previous parent stated, else the previous value - so
    /// `orderid` A, B, C, D carries `parentorderid` C and `origorderid` A,
    /// and a one-parent list (`clordid`'s `origclordid`) the previous value.
    /// The base source's parents are held first, nearest first, so a parent
    /// type's base key follows the base source's chain before a named
    /// source's parent can fill it. Whether anything moved.
    ///
    /// ```
    /// use yggdryl::{IdKey, IdType, Identifier, Identifiers};
    ///
    /// let id = |kind: &str, value: &str| {
    ///     Identifier::new(IdKey::base(kind.parse().unwrap()), value).unwrap()
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
        let mut fills: Vec<(bool, usize, IdKey, &str)> = Vec::new();
        for id in self
            .iter()
            .filter(|id| parent_of(id.kind()).is_none() && !self.is_echo(id))
        {
            let Some(before) = previous.get_from(&id.key) else {
                continue;
            };
            let parents = parents_of(id.kind());
            let last = parents.len().saturating_sub(1);
            let held = |at: usize| previous.value_of(id.src(), &parents[at]);
            for (at, parent) in parents.iter().enumerate() {
                if self.position_of(id.src(), parent).is_ok() {
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
                fills.extend(value.map(|value| {
                    (
                        !id.key.is_base(),
                        at,
                        id.key.with_kind(parent.clone()),
                        value,
                    )
                }));
            }
        }
        fills.sort_by_key(|(named, at, ..)| (*named, *at));
        let mut moved = false;
        for (_, _, key, value) in fills {
            if let Ok(id) = Identifier::new(key, value) {
                moved |= self.insert(id);
            }
        }
        moved
    }

    /// Fills each type a parent identifier is a parent of and the map does
    /// not state under the parent's source, with the value of the nearest
    /// parent it states: an element stating where it came from but not what
    /// it is now is what it came from. `parent_of` says which type a parent
    /// type is a parent of and its place among that type's parents. The base
    /// source's are filled first, nearest first, and a base key that only
    /// echoes a derivation fills nothing. Whether anything moved.
    ///
    /// ```
    /// use yggdryl::{IdKey, IdType, Identifier, Identifiers};
    ///
    /// let id = |kind: &str, value: &str| {
    ///     Identifier::new(IdKey::base(kind.parse().unwrap()), value).unwrap()
    /// };
    /// let mut ids: Identifiers = [id("origorderid", "A"), id("parentorderid", "C")].into_iter().collect();
    /// assert!(ids.fill_parents(IdType::parent_of));
    /// assert_eq!(ids.get(&IdType::OrderId), Some("C"), "the nearest parent");
    /// ```
    pub fn fill_parents(&mut self, parent_of: impl Fn(&IdType) -> Option<(IdType, usize)>) -> bool {
        let mut fills: Vec<(bool, usize, Identifier)> = self
            .iter()
            .filter(|id| !self.is_echo(id))
            .filter_map(|id| {
                let (kind, at) = parent_of(id.kind())?;
                let filled = self.position_of(id.src(), &kind).is_err();
                filled
                    .then(|| id.with_kind(kind).ok())
                    .flatten()
                    .map(|filled| (!id.key.is_base(), at, filled))
            })
            .collect();
        fills.sort_by_key(|(named, at, _)| (*named, *at));
        let mut moved = false;
        for (_, _, id) in fills {
            moved |= self.insert(id);
        }
        moved
    }

    /// Closes a map read raw: each type with no base key takes its first
    /// named source's value in key order, else its derivation's. A map the
    /// rule kept is closed already and does not move.
    pub(crate) fn close(&mut self) {
        for derivations in [false, true] {
            let mut at = 0;
            while at < self.0.len() {
                let held = &self.0[at];
                let source =
                    !held.key.is_base() && (*held.src() == IdSource::Derived) == derivations;
                if source && let Err(slot) = self.position_of(&IdSource::Base, held.kind()) {
                    let fill = held.clone().into_base();
                    self.0.insert(slot, fill);
                    if slot <= at {
                        at += 1;
                    }
                }
                at += 1;
            }
        }
    }

    /// Reads one entry as it was written, raw - the base rule is
    /// [`Self::close`]'s once every entry is read: `text` the key as spelled,
    /// `key` what it reads as, none where it reads as no key, and `value` its
    /// value. The same value twice under one key is one identifier.
    ///
    /// # Errors
    ///
    /// Located `$['text']`: a key that reads as none, a value that states
    /// nothing or that its type refuses, and a second value under a key
    /// already held - two spellings of one key.
    pub(crate) fn read_entry(&mut self, text: &str, key: Option<IdKey>, value: &str) -> Result<()> {
        let path = || format_smolstr!("$['{}']", crate::text::elide_to(text, 64));
        let key = key.ok_or_else(|| located(path(), crate::idkey::key_refusal(text)))?;
        let id = Identifier::new(key, value).map_err(|error| located(path(), error))?;
        match self.position(&id.key) {
            Ok(at) if self.0[at].value == id.value => Ok(()),
            Ok(at) => Err(Error::InvalidRecord {
                path: path(),
                reason: format_smolstr!(
                    "expected one value under {}, got {:?} and {:?}",
                    id.key,
                    crate::text::elide_to(self.0[at].value(), 64),
                    crate::text::elide_to(id.value(), 64)
                ),
            }),
            Err(at) => {
                self.0.insert(at, id);
                Ok(())
            }
        }
    }

    /// Where the identifier keyed `key` and holding `value` is.
    pub(crate) fn held_at(&self, key: &IdKey, value: &str) -> Option<usize> {
        self.position(key)
            .ok()
            .filter(|at| self.0[*at].value() == value)
    }

    /// The Arrow datatype a map lays out: a sorted map from the required
    /// text key, spelled as [`IdKey`] spells it, to the required text value.
    #[must_use]
    pub fn dtype() -> DataType {
        let entries = StructType::from_unique_fields(vec![
            Field::new("key", DataType::utf8(), false),
            Field::new("value", DataType::utf8(), false),
        ]);
        DataType::map(
            Field::new("entries", DataType::Struct(entries), false),
            true,
        )
        .expect("a text key beside a text value is a map's entries")
    }

    /// The map as its sorted entries, each key's spelling beside its value:
    /// a key of two member words is a static string, and a value is shared.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        if self.0.is_empty() {
            return match Scalar::from_mapping([]).expect("no entries") {
                Scalar::Map(entries) => Scalar::SortedMap(entries),
                empty => empty,
            };
        }
        let entries: Arc<[(Scalar, Scalar)]> = self
            .0
            .iter()
            .map(|id| {
                (
                    Scalar::from(SmolStr::from(&id.key)),
                    Scalar::from(id.value.clone()),
                )
            })
            .collect();
        Scalar::SortedMap(Map::new(entries))
    }

    /// Reads a map back from its entries, in any order, or from a sequence
    /// of such maps - their union - each key read exactly as [`IdKey`]
    /// spells it, then closed by the base rule.
    ///
    /// # Errors
    ///
    /// Anything but a map or a sequence of maps, and, located on its key -
    /// `$['k']`, `$[i]['k']` in a sequence - a key that is no text or reads
    /// as no key, a value that is no text, states nothing or that its type
    /// refuses, and two spellings of one key with two values.
    pub fn from_scalar(scalar: &Scalar) -> Result<Self> {
        if let Some(entries) = scalar.as_mapping() {
            let mut map = Self(Vec::with_capacity(2 * entries.len()));
            map.read_entries(entries)?;
            map.close();
            return Ok(map);
        }
        let Some(rows) = scalar.sequence_rows() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    "a map of identifier keys to values, or a sequence of such maps",
                    scalar.kind(),
                ),
            });
        };
        let held: usize = rows
            .iter()
            .filter_map(Scalar::as_mapping)
            .map(<[_]>::len)
            .sum();
        let mut map = Self(Vec::with_capacity(2 * held));
        for (at, row) in rows.iter().enumerate() {
            let Some(entries) = row.as_mapping() else {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$[{at}]"),
                    reason: crate::text::expected_got(
                        "a map of identifier keys to values",
                        row.kind(),
                    ),
                });
            };
            map.read_entries(entries).map_err(|error| match error {
                Error::InvalidRecord { path, reason } => Error::InvalidRecord {
                    path: format_smolstr!("$[{at}]{}", path.strip_prefix('$').unwrap_or(&path)),
                    reason,
                },
                other => other,
            })?;
        }
        map.close();
        Ok(map)
    }

    /// Reads one map's entries raw, each through [`Self::read_entry`].
    fn read_entries(&mut self, entries: &[(Scalar, Scalar)]) -> Result<()> {
        for (at, (key, value)) in entries.iter().enumerate() {
            let Some(text) = key.as_str() else {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        format_args!("a text key at entry {at}"),
                        key.kind(),
                    ),
                });
            };
            let Some(value) = value.as_str() else {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$['{}']", crate::text::elide_to(text, 64)),
                    reason: crate::text::expected_got("a text value", value.kind()),
                });
            };
            self.read_entry(text, text.parse().ok(), value)?;
        }
        Ok(())
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

impl<'map> IntoIterator for &'map Identifiers {
    type Item = &'map Identifier;
    type IntoIter = std::slice::Iter<'map, Identifier>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<Identifier> for Identifiers {
    /// The map the identifiers make through [`Identifiers::insert`], the
    /// first of one key kept.
    fn from_iter<I: IntoIterator<Item = Identifier>>(ids: I) -> Self {
        let mut map = Self::new();
        for id in ids {
            map.insert(id);
        }
        map
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
