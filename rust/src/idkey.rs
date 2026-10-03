//! The key of an identifier: who gave it and what type of name it is.
//!
//! An [`IdKey`] is a source and a type, the unique key an
//! [`Identifiers`](crate::Identifiers) map holds one value under. It is
//! spelled `src:type`, except under [`IdSource::Base`], which is never
//! spelled: the base key of a type is the type alone, so `isin` is the key
//! `base:isin` and `ullink:isin` the ISIN `ullink` stated. Every text and
//! Arrow form writes that spelling, keys order by it, and a key reads back
//! from it exactly (its [`FromStr`](std::str::FromStr)).
//!
//! A key both of whose words the crate names is spelled once per process in
//! one static table, so writing it is one borrowed string and reading its
//! canonical spelling back one binary search.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use smol_str::{SmolStr, format_smolstr};

use crate::identifier::{WORD_PAIR_WIDTH, fold_into};
use crate::{Error, IdSource, IdType, Result};

/// The key of an identifier: its source and its type.
///
/// Spelled `src:type`, a key under [`IdSource::Base`] as its type alone.
/// Keys order by that spelling - `cusip` < `derived:cusip` < `isin` <
/// `oms:instrumentid` < `ullink:isin` - which is the order a map holds,
/// lays out and digests its identifiers in.
///
/// ```
/// use yggdryl::{IdKey, IdSource, IdType};
///
/// let isin = IdKey::base(IdType::Isin);
/// assert_eq!(isin.to_string(), "isin");
/// assert_eq!("BASE:ISIN".parse::<IdKey>().unwrap(), isin);
/// let bridged: IdKey = "ullink:isin".parse().unwrap();
/// assert_eq!(bridged.src(), "ullink");
/// assert!(isin < bridged);
/// assert_eq!(IdKey::new(IdSource::Derived, IdType::Cusip).to_string(), "derived:cusip");
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct IdKey {
    src: IdSource,
    kind: IdType,
}

impl IdKey {
    /// The key of `kind` from `src`.
    #[must_use]
    pub const fn new(src: IdSource, kind: IdType) -> Self {
        Self { src, kind }
    }

    /// The base key of `kind`: the key under no named source, spelled as the
    /// type alone, whose value is the type's answer in a map.
    #[must_use]
    pub const fn base(kind: IdType) -> Self {
        Self::new(IdSource::Base, kind)
    }

    /// The source.
    #[must_use]
    pub fn src(&self) -> &IdSource {
        &self.src
    }

    /// The type.
    #[must_use]
    pub fn kind(&self) -> &IdType {
        &self.kind
    }

    /// Whether this is a base key, spelled as its type alone.
    #[must_use]
    pub fn is_base(&self) -> bool {
        self.src == IdSource::Base
    }

    /// The key of `kind` from the same source.
    #[must_use]
    pub fn with_kind(&self, kind: IdType) -> Self {
        Self::new(self.src.clone(), kind)
    }

    /// The static spelling of a key both of whose words the crate names.
    pub(crate) fn known(&self) -> Option<&'static str> {
        let at = self.src.ordinal()? * TYPES.len() + self.kind.ordinal()?;
        Some(KNOWN_KEYS.text_of(at))
    }

    /// The key the canonical spelling `text` of two words the crate names
    /// is, found without folding anything.
    pub(crate) fn from_known(text: &str) -> Option<Self> {
        let keys = &*KNOWN_KEYS;
        let at = keys
            .sorted
            .binary_search_by(|at| keys.text_of(usize::from(*at)).cmp(text))
            .ok()?;
        let at = usize::from(keys.sorted[at]);
        Some(Self::new(
            SOURCES[at / TYPES.len()].clone(),
            TYPES[at % TYPES.len()].clone(),
        ))
    }

    /// Writes the spelling into `out`: one static string for a key of two
    /// member words, else at most three writes.
    pub(crate) fn write_into(&self, out: &mut impl fmt::Write) -> fmt::Result {
        if let Some(text) = self.known() {
            return out.write_str(text);
        }
        if !self.is_base() {
            out.write_str(self.src.as_str())?;
            out.write_char(':')?;
        }
        out.write_str(self.kind.as_str())
    }

    /// The order of this key against the key `src:kind`, compared without
    /// building it.
    pub(crate) fn cmp_parts(&self, src: &IdSource, kind: &IdType) -> Ordering {
        if self.src == *src {
            return self.kind.as_str().cmp(kind.as_str());
        }
        spelled(&self.src, &self.kind).cmp(spelled(src, kind))
    }

    /// The key a FIX entry's name or a bridge's field names, inferred: its
    /// explicit `src:type`, else a whole name a security type is spelled by
    /// from the base source, else the type an identifier name at the end of
    /// it spells - one of `names`, stepped back over a parentage word - under
    /// the source the rest of it names, the base source where nothing is
    /// left or where that rest folds to a source the crate reserves. What
    /// [`Identifier::from_key`](crate::Identifier::from_key) reads a key by.
    pub(crate) fn infer<'name>(
        key: &str,
        names: impl IntoIterator<Item = &'name str>,
    ) -> Option<Self> {
        let key = key.trim();
        if let Some((src, kind)) = key.split_once(':') {
            return Some(Self::new(src.parse().ok()?, kind.parse().ok()?));
        }
        if let Some(kind) = IdType::from_field_name(key) {
            return Some(Self::base(kind));
        }
        // The source and the type are each bounded as a word where they are
        // read, so the key folds as wide as the two together.
        let mut buffer = [0_u8; WORD_PAIR_WIDTH];
        let folded = fold_into(key, &mut buffer).ok()?;
        let (at, kind) = IdType::from_key_end(folded, names)?;
        let src = IdSource::from_namespace(&folded[..at])
            .ok()?
            .unwrap_or(IdSource::Base);
        Some(Self::new(src, kind))
    }
}

/// The bytes of the key `src:kind` as it is spelled, one at a time: the type
/// alone under the base source.
fn spelled<'key>(src: &'key IdSource, kind: &'key IdType) -> impl Iterator<Item = u8> + 'key {
    let (src, colon): (&[u8], &[u8]) = if *src == IdSource::Base {
        (b"", b"")
    } else {
        (src.as_str().as_bytes(), b":")
    };
    src.iter()
        .chain(colon)
        .chain(kind.as_str().as_bytes())
        .copied()
}

/// The refusal a text no key reads from earns.
pub(crate) fn key_refusal(text: &str) -> Error {
    Error::InvalidDataType {
        kind: "identifier",
        reason: format_smolstr!(
            "expected an identifier key src:type or type, got {:?}",
            crate::text::elide_to(text, 64)
        ),
    }
}

impl FromStr for IdKey {
    type Err = Error;

    /// Reads a key exactly: `src:type` is that source and that type, each
    /// folded, and a bare word the type it folds to from the base source -
    /// `ISIN`, `ISIN_Number`, `base:isin`, `BASE:ISIN` and `fix:isin` are
    /// all `isin`, and `marketorderid` is that type from the base source.
    /// Nothing is inferred: an inferred reading is
    /// [`Identifier::from_key`](crate::Identifier::from_key)'s.
    ///
    /// # Errors
    ///
    /// An empty half, a second `:`, or a word no fold reads.
    fn from_str(text: &str) -> Result<Self> {
        if let Some(key) = Self::from_known(text) {
            return Ok(key);
        }
        let refused = || key_refusal(text);
        match text.split_once(':') {
            Some((_, kind)) if kind.contains(':') => Err(refused()),
            Some((src, kind)) => Ok(Self::new(
                src.parse().map_err(|_| refused())?,
                kind.parse().map_err(|_| refused())?,
            )),
            None => Ok(Self::base(text.parse().map_err(|_| refused())?)),
        }
    }
}

impl PartialOrd for IdKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IdKey {
    /// By the key as it is spelled, a base key as its type alone.
    fn cmp(&self, other: &Self) -> Ordering {
        self.cmp_parts(&other.src, &other.kind)
    }
}

impl PartialEq<str> for IdKey {
    /// Whether `other` is the key's spelling, compared without building it.
    fn eq(&self, other: &str) -> bool {
        spelled(&self.src, &self.kind).eq(other.bytes())
    }
}

impl PartialEq<&str> for IdKey {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

impl fmt::Display for IdKey {
    /// `src:type`, a base key as its type alone.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_into(formatter)
    }
}

impl From<&IdKey> for SmolStr {
    /// The spelling: static for a key of two member words, inline within
    /// `SmolStr`'s width, one allocation past it.
    fn from(key: &IdKey) -> Self {
        if let Some(text) = key.known() {
            return Self::new_static(text);
        }
        let mut buffer = [0_u8; WORD_PAIR_WIDTH];
        let mut len = 0;
        for (slot, byte) in buffer.iter_mut().zip(spelled(&key.src, &key.kind)) {
            *slot = byte;
            len += 1;
        }
        Self::new(std::str::from_utf8(&buffer[..len]).expect("ASCII words"))
    }
}

/// The sources the crate names, in declaration order.
static SOURCES: [IdSource; IdSource::KNOWN.len()] = IdSource::KNOWN;

/// The types the crate names, in declaration order.
static TYPES: [IdType; IdType::KNOWN.len()] = IdType::KNOWN;

/// Every key both of whose words the crate names, spelled once per process:
/// the pair `src` x `type` sits at `src.ordinal() * TYPES.len() +
/// type.ordinal()`, a base pair spelled as its type alone.
struct KnownKeys {
    /// Every pair's spelling, one after another in pair order.
    text: Box<str>,
    /// Where each pair's spelling ends in `text`.
    ends: Box<[u32]>,
    /// The pairs in the order of their spellings.
    sorted: Box<[u16]>,
}

impl KnownKeys {
    fn text_of(&self, at: usize) -> &str {
        let start = at
            .checked_sub(1)
            .map_or(0, |before| self.ends[before] as usize);
        &self.text[start..self.ends[at] as usize]
    }
}

static KNOWN_KEYS: LazyLock<KnownKeys> = LazyLock::new(|| {
    let base = IdSource::Base.ordinal();
    let pairs = || {
        IdSource::SPELLINGS
            .iter()
            .enumerate()
            .flat_map(move |(at, src)| {
                let src = (Some(at) != base).then_some(*src);
                IdType::SPELLINGS.iter().map(move |kind| (src, *kind))
            })
    };
    // Sized before it is written, so each table is one allocation.
    let len = pairs()
        .map(|(src, kind)| src.map_or(0, |src| src.len() + 1) + kind.len())
        .sum();
    let mut text = String::with_capacity(len);
    let mut ends = Vec::with_capacity(SOURCES.len() * TYPES.len());
    for (src, kind) in pairs() {
        if let Some(src) = src {
            text.push_str(src);
            text.push(':');
        }
        text.push_str(kind);
        ends.push(u32::try_from(text.len()).expect("a table of short words"));
    }
    let mut keys = KnownKeys {
        text: text.into_boxed_str(),
        ends: ends.into_boxed_slice(),
        sorted: Box::default(),
    };
    let mut sorted: Box<[u16]> = (0..keys.ends.len())
        .map(|at| u16::try_from(at).expect("fewer pairs than a u16 counts"))
        .collect();
    sorted.sort_unstable_by(|left, right| {
        keys.text_of(usize::from(*left))
            .cmp(keys.text_of(usize::from(*right)))
    });
    keys.sorted = sorted;
    keys
});

/// The most distinct key texts a [`KeyReader`] remembers.
const KEY_READER_BOUND: usize = 1024;

/// Reads the keys of one stream of maps: a key of two member words from the
/// static table, any other from what it read before, else parsed once and
/// remembered - a text no key reads included - up to [`KEY_READER_BOUND`]
/// texts, past which it parses without remembering.
#[derive(Debug, Default)]
pub(crate) struct KeyReader {
    others: HashMap<SmolStr, Option<IdKey>>,
}

impl KeyReader {
    /// The key `text` reads as, or `None` where it reads as none.
    pub(crate) fn read(&mut self, text: &str) -> Option<IdKey> {
        if let Some(key) = IdKey::from_known(text) {
            return Some(key);
        }
        if let Some(read) = self.others.get(text) {
            return read.clone();
        }
        let read = text.parse::<IdKey>().ok();
        if self.others.len() < KEY_READER_BOUND {
            self.others.insert(SmolStr::new(text), read.clone());
        }
        read
    }
}
