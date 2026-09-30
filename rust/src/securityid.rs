//! Security identifiers: the national number an ISIN embeds, the shape a
//! symbol reads as, and the associations one ordered lifecycle learns
//! between an instrument's identifiers.

use std::collections::HashMap;
use std::mem::{align_of, size_of};

use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::graph::{Element, Market};
use crate::identifier::IDENTIFIER_VALUE_WIDTH;
use crate::{Cfi, Cusip, Figi, IdSource, IdType, Identifier, Identifiers, Isin, Ric, Sedol};

// ---------------------------------------------------------------------------
// embedded: the national number an ISIN carries.
// ---------------------------------------------------------------------------

/// The identifier a canonical ISIN embeds as its national number, where its
/// country's scheme is one this crate checks: at most one.
///
/// `US` and `CA` carry a CUSIP in positions 2 to 11; `GB`, `IE`, `GG`, `JE`
/// and `IM` a SEDOL in positions 4 to 11 behind `00`; `DE` a WKN in positions
/// 5 to 11 behind `000`; `CH` and `LI` a Valor number in positions 2 to 11
/// with its leading zeros dropped. The embedded code must close on its own
/// check too: neither an arbitrary national number nor an unchecked ISIN is
/// enough to name another identifier.
///
/// ```
/// use yggdryl::Isin;
/// use yggdryl::securityid::embedded;
///
/// let apple = Isin::new("US0378331005").unwrap();
/// let cusip = embedded(&apple).next().unwrap();
/// assert_eq!(cusip.to_string(), "derived:cusip=037833100");
/// assert!(embedded(&Isin::new("XS0203470157").unwrap()).next().is_none());
/// ```
pub fn embedded(isin: &Isin) -> impl Iterator<Item = Identifier> {
    let text = isin.as_str();
    let found = if !Isin::is_canonical(text) {
        None
    } else {
        let (kind, code) = match &text[..2] {
            "US" | "CA" => (Some(IdType::Cusip), &text[2..11]),
            "GB" | "IE" | "GG" | "JE" | "IM" if &text[2..4] == "00" => {
                (Some(IdType::Sedol), &text[4..11])
            }
            "DE" if &text[2..5] == "000" => (Some(IdType::Wkn), &text[5..11]),
            "CH" | "LI" => (Some(IdType::Valor), text[2..11].trim_start_matches('0')),
            _ => (None, ""),
        };
        kind.and_then(|kind| Identifier::new(IdSource::Derived, kind, code).ok())
    };
    found.into_iter()
}

// ---------------------------------------------------------------------------
// SymbolCode: what a symbol's own shape says it is.
// ---------------------------------------------------------------------------

/// What a symbol is, read off its shape: a ticker a venue gives is often an
/// identifier a standard gives, and its length alone says which one to try
/// before any check runs.
///
/// | length | shape | reads as |
/// | --- | --- | --- |
/// | 21 | `{ISIN}_{MIC}_{CCY}` | [`Self::Instrument`], each part its own type's |
/// | 12 | `BBG` then nine | [`Self::Figi`] |
/// | 12 | any other | [`Self::Isin`] |
/// | 9 | | [`Self::Cusip`] |
/// | 7 | | [`Self::Sedol`] |
/// | 6 | upper-case letters, a detailed classification | [`Self::Cfi`] |
/// | any | a dot between a code and a venue - `AAPL.OQ` | [`Self::Ric`] |
/// | any | a Bloomberg yellow key last - `AAPL US Equity` | [`Self::Bloomberg`] |
///
/// Every candidate closes on its own type's check - an ISIN's, a FIGI's, a
/// CUSIP's and a SEDOL's check digit, a CFI's grammar past its category
/// and group - so a symbol that is
/// only the length of one reads as nothing, and a symbol reading as nothing
/// is a ticker and nothing more. A currency pair is the FIX layer's
/// [`FxSymbol`](crate::FxSymbol) reading, which knows its tenors.
///
/// ```
/// use yggdryl::securityid::SymbolCode;
///
/// assert!(matches!(SymbolCode::from_symbol("US0378331005"), Some(SymbolCode::Isin(_))));
/// assert!(matches!(SymbolCode::from_symbol("BBG000BLNQ16"), Some(SymbolCode::Figi(_))));
/// assert!(matches!(
///     SymbolCode::from_symbol("CH0012214059_XSWX_CHF"),
///     Some(SymbolCode::Instrument { .. })
/// ));
/// assert!(matches!(SymbolCode::from_symbol("ESVUFR"), Some(SymbolCode::Cfi(_))));
/// assert!(matches!(SymbolCode::from_symbol("AAPL.OQ"), Some(SymbolCode::Ric(_))));
/// assert!(matches!(SymbolCode::from_symbol("HOLN SW Equity"), Some(SymbolCode::Bloomberg(_))));
/// assert_eq!(SymbolCode::from_symbol("AAPL"), None);
/// assert_eq!(SymbolCode::from_symbol("US0378331006"), None);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolCode {
    /// An instrument key: its ISIN, the MIC it trades on and the currency
    /// it trades in, `_` between them - twelve, four and three characters,
    /// each part read by its own type and `None` where that type refuses
    /// it, so one bad part leaves the others answering.
    Instrument {
        /// The instrument.
        isin: Option<Isin>,
        /// Where it trades: an ISO 10383 MIC, or the Reuters mnemonic FIX
        /// 4.2 spelled one in.
        mic: Option<crate::Mic>,
        /// What it trades in.
        ccy: Option<crate::Ccy>,
    },
    /// An ISIN.
    Isin(Isin),
    /// A FIGI.
    Figi(Figi),
    /// A CUSIP.
    Cusip(Cusip),
    /// A SEDOL.
    Sedol(Sedol),
    /// A CFI code.
    Cfi(Cfi),
    /// A Reuters instrument code.
    Ric(Ric),
    /// A Bloomberg identifier ending in its yellow key.
    Bloomberg(crate::Bbg),
}

/// The Bloomberg yellow keys a terminal identifier ends with.
const YELLOW_KEYS: [&str; 9] = [
    "Equity", "Comdty", "Curncy", "Index", "Govt", "Corp", "Mtge", "Muni", "Pfd",
];

impl SymbolCode {
    /// What `symbol` is, by its length first and its type's check second,
    /// or `None` where it is no identifier this crate checks.
    #[must_use]
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        let text = symbol.trim();
        let bytes = text.as_bytes();
        let upper = |part: &[u8]| part.iter().all(u8::is_ascii_uppercase);
        let alphanumeric = |part: &[u8]| part.iter().all(u8::is_ascii_alphanumeric);
        let found = match bytes.len() {
            21 if bytes[12] == b'_' && bytes[17] == b'_' => Self::instrument(text),
            12 if text.starts_with("BBG") => Figi::new(text).ok().map(Self::Figi),
            12 if alphanumeric(bytes) => Isin::new(text)
                .ok()
                .filter(|isin| Isin::is_canonical(isin.as_str()))
                .map(Self::Isin),
            9 if alphanumeric(bytes) => Cusip::new(text).ok().map(Self::Cusip),
            7 if alphanumeric(bytes) => Sedol::new(text).ok().map(Self::Sedol),
            6 if upper(bytes) && Cfi::is_detailed(text) => Cfi::new(text).ok().map(Self::Cfi),
            _ => None,
        };
        found.or_else(|| {
            let yellow = text.contains(' ')
                && text.rsplit(' ').next().is_some_and(|key| {
                    YELLOW_KEYS
                        .iter()
                        .any(|yellow| yellow.eq_ignore_ascii_case(key))
                });
            if yellow {
                return crate::Bbg::new(text).ok().map(Self::Bloomberg);
            }
            let (code, venue) = text.split_once('.')?;
            (!code.is_empty()
                && !venue.is_empty()
                && !venue.contains('.')
                && alphanumeric(venue.as_bytes())
                && !text.contains(' '))
            .then(|| Ric::new(text).ok().map(Self::Ric))
            .flatten()
        })
    }

    /// The instrument key `{ISIN}_{MIC}_{CCY}` spells - twelve, four and
    /// three ASCII letters and digits, `_` between them - each part read by
    /// its own type, or `None` where the text is not that shape or no part
    /// reads.
    #[must_use]
    pub fn instrument(text: &str) -> Option<Self> {
        let mut parts = text.split('_');
        let (isin, mic, ccy) = (parts.next()?, parts.next()?, parts.next()?);
        let shaped = |part: &str, width: usize| {
            part.len() == width && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        };
        if parts.next().is_some() || !shaped(isin, 12) || !shaped(mic, 4) || !shaped(ccy, 3) {
            return None;
        }
        let isin = Isin::new(isin)
            .ok()
            .filter(|held| Isin::is_canonical(held.as_str()));
        let mic = crate::Mic::from_market(mic);
        let ccy = crate::Ccy::new(ccy).ok();
        (isin.is_some() || mic.is_some() || ccy.is_some()).then_some(Self::Instrument {
            isin,
            mic,
            ccy,
        })
    }
}

// ---------------------------------------------------------------------------
// SecurityIdRegistry: what one ordered lifecycle learns about an instrument.
// ---------------------------------------------------------------------------

/// One lifecycle reserves at most 32 MiB for learned associations. Each first
/// valid ISIN is charged conservatively for its full key set and hash-table
/// growth; a known ISIN needs no further reservation.
const MAX_REGISTRY_BYTES: usize = 32 * 1024 * 1024;
const ENTRY_CHARGE: usize = 2048;
const HASH_CONTROL_BYTES: usize = 1;
/// The heap one retained value may take: the widest value any identifier
/// type admits, behind its `Arc` counters and allocator rounding. A learned
/// key is a type the crate names, a static word with no heap of its own.
const MAX_VALUE_HEAP_ALLOWANCE: usize =
    IDENTIFIER_VALUE_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// The most sources one instrument's associations are learned under.
const MAX_KEYS_PER_INSTRUMENT: usize = 8;

// Four buckets per entry cover the supported standard HashMap's load slack
// plus old and new tables during growth. Each learned key is charged its
// slot and, once, the widest retained value's heap payload, Arc counters and
// allocator rounding.
const _: () = assert!(
    ENTRY_CHARGE
        >= 4 * (size_of::<(SmolStr, Learned)>() + HASH_CONTROL_BYTES)
            + MAX_KEYS_PER_INSTRUMENT * (size_of::<Slot>() + MAX_VALUE_HEAP_ALLOWANCE)
);
const _: () = assert!(MAX_REGISTRY_BYTES / ENTRY_CHARGE == 16_384);

/// A second invariant cap independent of byte-accounting constants.
const MAX_INSTRUMENTS: usize = 65_536;

#[derive(Default)]
enum Association<T> {
    #[default]
    Missing,
    Known(T),
    Ambiguous,
}

impl<T> Association<T> {
    fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Missing | Self::Ambiguous => None,
        }
    }
}

impl Association<SmolStr> {
    /// Learn `value`: a first sighting is known, a second different one
    /// makes the association ambiguous for good. The value is built only
    /// where it is first learned, so seeing a known one again costs nothing.
    fn observe(&mut self, value: &str) {
        *self = match self {
            Self::Missing => Self::Known(SmolStr::new(value)),
            Self::Known(known) if known == value => return,
            Self::Known(_) | Self::Ambiguous => Self::Ambiguous,
        };
    }
}

impl Association<Cfi> {
    fn classify(&mut self, value: Option<&Cfi>) {
        let Some(value) = value else { return };
        match self {
            Self::Ambiguous => return,
            Self::Known(known) if known == value => return,
            _ => {}
        }
        if !Cfi::is_classified(value.as_str())
            || value.as_str().as_bytes()[2..]
                .iter()
                .all(|byte| *byte == b'X')
        {
            return;
        }
        if let Self::Known(known) = self {
            if known
                .as_str()
                .bytes()
                .zip(value.as_str().bytes())
                .any(|(left, right)| left != right && left != b'X' && right != b'X')
            {
                *self = Self::Ambiguous;
            } else if let Some(merged) = Cfi::merged(known.as_str(), value.as_str()) {
                *known = Cfi::new(merged).expect("merged validated CFI codes");
            }
        } else {
            *self = Self::Known(value.clone());
        }
    }
}

/// One type's association for one instrument.
type Slot = (IdType, Association<SmolStr>);

#[derive(Default)]
struct Learned {
    cfi: Association<Cfi>,
    keys: SmallVec<[Slot; 2]>,
}

/// The associations one ordered lifecycle learns between an instrument's
/// ISIN, its other identifiers and its classification, never by a codec.
pub(crate) struct SecurityIdRegistry {
    by_isin: HashMap<SmolStr, Learned>,
    reserved_bytes: usize,
    byte_budget: usize,
    /// Types whose value names one listing of an instrument - its ISIN with
    /// a market and a currency - rather than the instrument the ISIN
    /// numbers. One ISIN has as many listings as markets, so what one
    /// message states under such a type is no association to fill onto
    /// another, and none is learned.
    listings: SmallVec<[IdType; 2]>,
}

/// The types a registry fills for one instrument, each with the code it is
/// known by, borrowed from the registry: four held inline.
type Fills<'learned> = SmallVec<[(&'learned IdType, &'learned str); 4]>;

impl Default for SecurityIdRegistry {
    fn default() -> Self {
        Self {
            by_isin: HashMap::new(),
            reserved_bytes: 0,
            byte_budget: MAX_REGISTRY_BYTES,
            listings: SmallVec::new(),
        }
    }
}

impl SecurityIdRegistry {
    /// A registry that never learns an association under one of
    /// `listings`, the types naming a listing rather than an instrument.
    pub(crate) fn with_listings(listings: impl IntoIterator<Item = IdType>) -> Self {
        Self {
            listings: listings.into_iter().collect(),
            ..Self::default()
        }
    }

    /// The retained slot for `isin`, registering it only after a checked
    /// reservation. A rejected registration does not clone the key or ask
    /// the map to reserve.
    fn learned_for(&mut self, isin: &str) -> Option<&mut Learned> {
        if self.by_isin.contains_key(isin) {
            return self.by_isin.get_mut(isin);
        }
        if self.by_isin.len() >= MAX_INSTRUMENTS {
            return None;
        }
        let reserved_bytes = self.reserved_bytes.checked_add(ENTRY_CHARGE)?;
        if reserved_bytes > self.byte_budget {
            return None;
        }
        self.by_isin.insert(SmolStr::new(isin), Learned::default());
        self.reserved_bytes = reserved_bytes;
        self.by_isin.get_mut(isin)
    }

    /// Learn what `ids` and `cfi` state about the instrument `ids` names by
    /// its ISIN: one association per stated type the crate names but a
    /// listing's and the ISIN's own, at most `MAX_KEYS_PER_INSTRUMENT` of
    /// them, and the classification where it is detailed. Nothing without an
    /// ISIN; nothing the crate derived, which is no statement; nothing under
    /// a word the crate does not name - a private source's code, `100` - whose
    /// spelling is heap the entry charge does not count.
    pub(crate) fn learn(&mut self, ids: &Identifiers, cfi: Option<&Cfi>) {
        let Some(isin) = ids.get(&IdType::Isin) else {
            return;
        };
        let isin = SmolStr::new(isin);
        let learnable: SmallVec<[&Identifier; 8]> = ids
            .iter()
            .filter(|id| {
                id.kind() != &IdType::Isin
                    && id.kind().is_known()
                    && id.src() != &IdSource::Derived
                    && !self.listings.contains(id.kind())
            })
            .collect();
        let Some(learned) = self.learned_for(&isin) else {
            return;
        };
        learned.cfi.classify(cfi);
        for id in learnable {
            let position = learned.keys.iter().position(|(key, _)| key == id.kind());
            let slot = match position {
                Some(position) => &mut learned.keys[position],
                None if learned.keys.len() < MAX_KEYS_PER_INSTRUMENT => {
                    learned.keys.push((id.kind().clone(), Association::Missing));
                    learned.keys.last_mut().expect("just pushed")
                }
                None => continue,
            };
            slot.1.observe(id.value());
        }
    }

    /// What `ids` and `cfi` leave unstated about the instrument `ids`
    /// names by its ISIN, from what was learned: each absent type with the
    /// code it is known by, borrowed, and the classification where the
    /// learned one refines the stated one. Planned off the borrowed set,
    /// so filling costs nothing where there is nothing to fill.
    fn fills<'learned>(
        &'learned self,
        ids: &Identifiers,
        cfi: Option<&Cfi>,
    ) -> (Fills<'learned>, Option<Cfi>) {
        let mut derived = SmallVec::new();
        let Some(learned) = ids
            .get(&IdType::Isin)
            .and_then(|isin| self.by_isin.get(isin))
        else {
            return (derived, None);
        };
        let classified = learned.cfi.known().and_then(|known| match cfi {
            None => Some(known.clone()),
            Some(stated) if stated != known => Cfi::merged(stated.as_str(), known.as_str())
                .filter(|merged| {
                    // Unknown positions can fill; a stated classification
                    // attribute is never overwritten by a learned default.
                    stated
                        .as_str()
                        .bytes()
                        .zip(merged.bytes())
                        .all(|(old, new)| old == b'X' || old == new)
                        && merged.as_str() != stated.as_str()
                })
                .and_then(|merged| Cfi::new(merged).ok()),
            Some(_) => None,
        });
        for (kind, association) in &learned.keys {
            if let Some(known) = association.known().filter(|_| !ids.contains_kind(kind)) {
                derived.push((kind, known.as_str()));
            }
        }
        (derived, classified)
    }
}

/// Learn what `event` states about its instrument - its security identifiers
/// and its CFI - then fill what it left unstated: each absent type the
/// registry knows for the ISIN is derived onto the event, never stated, and
/// the element is finalized where anything moved. The event's set is read
/// where it lies, never copied.
pub(crate) fn enrich<E: Market + Element>(registry: &mut SecurityIdRegistry, event: &mut E) {
    registry.learn(event.get_securityids(), event.get_cficode());
    let (derived, classified) = registry.fills(event.get_securityids(), event.get_cficode());
    let mut changed = false;
    if let Some(code) = classified {
        event.set_cficode(Some(code), true);
        changed = true;
    }
    for (kind, known) in derived {
        changed |= event.derive_securityid(kind, known);
    }
    if changed {
        event.finalize();
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/securityid.rs` pins and a caller cannot reach.
    //!
    //! The registry is a lifecycle's own: nothing above it names one, and what
    //! it learned is only ever read off the events it filled. What it costs -
    //! one reservation per first valid ISIN, none for a known one, and a full
    //! registry that still learns about the instruments it already holds - is
    //! read here through a wrapper, so the registry, its budget and its
    //! reservation stay exactly as private as they were.
    use std::collections::HashMap;

    use crate::graph::{Element, Market};
    use crate::{Cfi, Identifiers};

    /// The instrument associations one ordered lifecycle learns.
    #[derive(Default)]
    pub struct SecurityIdRegistry(super::SecurityIdRegistry);

    impl SecurityIdRegistry {
        /// A registry that may reserve at most `byte_budget` bytes.
        pub fn with_budget(byte_budget: usize) -> Self {
            Self(super::SecurityIdRegistry {
                by_isin: HashMap::new(),
                reserved_bytes: 0,
                byte_budget,
                listings: Default::default(),
            })
        }

        /// A registry whose reservation arithmetic already stands at the top
        /// of `usize`, so the next charge can only overflow.
        pub fn saturated() -> Self {
            Self(super::SecurityIdRegistry {
                by_isin: HashMap::new(),
                reserved_bytes: usize::MAX,
                byte_budget: usize::MAX,
                listings: Default::default(),
            })
        }

        /// Learn what `event` states, then fill what it left unstated.
        pub fn enrich<E: Market + Element>(&mut self, event: &mut E) {
            super::enrich(&mut self.0, event);
        }

        /// Learn what `ids` and `cfi` state.
        pub fn learn(&mut self, ids: &Identifiers, cfi: Option<&Cfi>) {
            self.0.learn(ids, cfi);
        }

        /// Fill what `ids` and `cfi` leave unstated, each absent type as a
        /// derived identifier; whether anything was.
        pub fn fill(&self, ids: &mut Identifiers, cfi: &mut Option<Cfi>) -> bool {
            let (derived, classified) = self.0.fills(ids, cfi.as_ref());
            let mut changed = classified.is_some();
            if classified.is_some() {
                *cfi = classified;
            }
            for (kind, known) in derived {
                if let Ok(id) =
                    crate::Identifier::new(crate::IdSource::Derived, kind.clone(), known)
                {
                    changed |= ids.insert(id);
                }
            }
            changed
        }

        /// How many instruments the registry holds.
        pub fn instruments(&self) -> usize {
            self.0.by_isin.len()
        }

        /// The bytes it has reserved for them.
        pub fn reserved_bytes(&self) -> usize {
            self.0.reserved_bytes
        }
    }

    /// What one instrument's first sighting reserves.
    pub const ENTRY_CHARGE: usize = super::ENTRY_CHARGE;

    /// The most sources one instrument's associations are learned under.
    pub const MAX_KEYS_PER_INSTRUMENT: usize = super::MAX_KEYS_PER_INSTRUMENT;
}
