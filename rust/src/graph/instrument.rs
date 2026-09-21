//! Instrument associations learned by one ordered lifecycle, never by a codec.

use std::collections::HashMap;
use std::mem::{align_of, size_of};

use super::MarketElement;
use crate::bloomberg_code::BLOOMBERG_WIDTH;
use crate::{BloombergCode, CfiCode, CusipCode, FIGICode, IsinCode, SedolCode};

/// One lifecycle reserves at most 32 MiB for learned associations. Each first
/// valid ISIN is charged conservatively for its full code set and hash-table
/// growth; a known ISIN needs no further reservation.
const MAX_REGISTRY_BYTES: usize = 32 * 1024 * 1024;
const ENTRY_CHARGE: usize = 1024;
const HASH_CONTROL_BYTES: usize = 1;
const MAX_BLOOMBERG_HEAP_ALLOWANCE: usize =
    BLOOMBERG_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

// Four buckets per entry cover the supported standard HashMap's load slack
// plus old and new tables during growth. The separate allowance covers the
// widest retained code's payload, Arc counters and allocator rounding.
const _: () = assert!(
    ENTRY_CHARGE
        >= 4 * (size_of::<(IsinCode, Codes)>() + HASH_CONTROL_BYTES) + MAX_BLOOMBERG_HEAP_ALLOWANCE
);
const _: () = assert!(MAX_REGISTRY_BYTES / ENTRY_CHARGE == 32_768);

/// A second invariant cap independent of byte-accounting constants.
const MAX_INSTRUMENTS: usize = 65_536;

#[derive(Default)]
enum Association<T> {
    #[default]
    Missing,
    Known(T),
    Ambiguous,
}

impl<T: Clone + PartialEq> Association<T> {
    fn observe(&mut self, value: Option<&T>, valid: impl FnOnce(&T) -> bool) {
        let Some(value) = value else { return };
        match self {
            Self::Ambiguous => return,
            Self::Known(known) if known == value => return,
            _ => {}
        }
        // A repeated association neither revalidates nor clones its code.
        if !valid(value) {
            return;
        }
        *self = match self {
            Self::Missing => Self::Known(value.clone()),
            Self::Known(_) | Self::Ambiguous => Self::Ambiguous,
        };
    }

    fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Missing | Self::Ambiguous => None,
        }
    }
}

impl Association<CfiCode> {
    fn classify(&mut self, value: Option<&CfiCode>) {
        let Some(value) = value else { return };
        match self {
            Self::Ambiguous => return,
            Self::Known(known) if known == value => return,
            _ => {}
        }
        if !CfiCode::is_classified(value.as_str())
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
            } else if let Some(merged) = CfiCode::merged(known.as_str(), value.as_str()) {
                *known = CfiCode::new(merged).expect("merged validated CFI codes");
            }
        } else {
            *self = Self::Known(value.clone());
        }
    }
}

#[derive(Default)]
struct Codes {
    cfi: Association<CfiCode>,
    cusip: Association<CusipCode>,
    sedol: Association<SedolCode>,
    bloomberg: Association<BloombergCode>,
    figi: Association<FIGICode>,
}

pub(crate) struct InstrumentCodes {
    by_isin: HashMap<IsinCode, Codes>,
    reserved_bytes: usize,
    byte_budget: usize,
}

impl Default for InstrumentCodes {
    fn default() -> Self {
        Self {
            by_isin: HashMap::new(),
            reserved_bytes: 0,
            byte_budget: MAX_REGISTRY_BYTES,
        }
    }
}

impl InstrumentCodes {
    /// The retained slot for `isin`, registering it only after validation and
    /// a checked reservation. A rejected registration does not clone the key
    /// or ask the map to reserve.
    fn codes_for(&mut self, isin: &IsinCode) -> Option<&mut Codes> {
        if self.by_isin.contains_key(isin) {
            return self.by_isin.get_mut(isin);
        }
        if !IsinCode::is_canonical(isin.as_str()) || self.by_isin.len() >= MAX_INSTRUMENTS {
            return None;
        }
        let reserved_bytes = self.reserved_bytes.checked_add(ENTRY_CHARGE)?;
        if reserved_bytes > self.byte_budget {
            return None;
        }
        self.by_isin.insert(isin.clone(), Codes::default());
        self.reserved_bytes = reserved_bytes;
        self.by_isin.get_mut(isin)
    }

    /// Learn stated associations before filling missing facts, so conflicting
    /// observations disable an ambiguous default instead of choosing a winner.
    pub(crate) fn enrich<E: MarketElement>(&mut self, event: &mut E) {
        let Some(isin) = event.get_isincode() else {
            return;
        };
        let Some(codes) = self.codes_for(isin) else {
            return;
        };
        codes.cfi.classify(event.get_cficode());
        codes.cusip.observe(event.get_cusipcode(), |code| {
            CusipCode::is_canonical(code.as_str())
        });
        codes.sedol.observe(event.get_sedolcode(), |code| {
            SedolCode::is_canonical(code.as_str())
        });
        codes.bloomberg.observe(event.get_bloombergcode(), |code| {
            let text = code.as_str();
            !text.is_empty()
                && !["null", "none", "[n/a]"]
                    .iter()
                    .any(|null| text.eq_ignore_ascii_case(null))
                && BloombergCode::is_canonical(text)
        });
        codes.figi.observe(event.get_figicode(), |code| {
            FIGICode::is_canonical(code.as_str())
        });
        let mut changed = false;
        macro_rules! fill {
            ($get:ident, $set:ident, $held:ident) => {
                if event.$get().is_none() {
                    if let Some(value) = codes.$held.known() {
                        event.$set(Some(value.clone()));
                        changed = true;
                    }
                }
            };
        }
        if let Some(known) = codes.cfi.known() {
            let replacement = match event.get_cficode() {
                None => Some(known.clone()),
                Some(stated) if stated != known => {
                    CfiCode::merged(stated.as_str(), known.as_str())
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
                        .and_then(|merged| CfiCode::new(merged).ok())
                }
                Some(_) => None,
            };
            if let Some(code) = replacement {
                event.set_cficode(Some(code));
                changed = true;
            }
        }
        fill!(get_cusipcode, set_cusipcode, cusip);
        fill!(get_sedolcode, set_sedolcode, sedol);
        fill!(get_bloombergcode, set_bloombergcode, bloomberg);
        fill!(get_figicode, set_figicode, figi);
        if changed {
            event.finalize();
        }
    }
}

/// US and Canadian ISINs carry their CUSIP in positions 3 through 11. The
/// CUSIP's own check digit must close too; neither an arbitrary NSIN nor an
/// unchecked/default ISIN is enough to name another identifier.
pub(super) fn embedded_cusip(isin: &IsinCode) -> Option<CusipCode> {
    let text = isin.as_str();
    if !matches!(text.get(..2)?, "US" | "CA") || !IsinCode::is_canonical(text) {
        return None;
    }
    CusipCode::new(text.get(2..11)?).ok()
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/graph/instrument.rs` pins and a caller cannot reach.
    //!
    //! The registry is a lifecycle's own: nothing above it names one, and what
    //! it learned is only ever read off the events it filled. What it costs -
    //! one reservation per first valid ISIN, none for a known one, and a full
    //! registry that still learns about the instruments it already holds - is
    //! read here through a wrapper, so the registry, its budget and its
    //! reservation stay exactly as private as they were.
    use std::collections::HashMap;

    use crate::graph::MarketElement;

    /// The instrument associations one ordered lifecycle learns.
    #[derive(Default)]
    pub struct InstrumentCodes(super::InstrumentCodes);

    impl InstrumentCodes {
        /// A registry that may reserve at most `byte_budget` bytes.
        pub fn with_budget(byte_budget: usize) -> Self {
            Self(super::InstrumentCodes {
                by_isin: HashMap::new(),
                reserved_bytes: 0,
                byte_budget,
            })
        }

        /// A registry whose reservation arithmetic already stands at the top
        /// of `usize`, so the next charge can only overflow.
        pub fn saturated() -> Self {
            Self(super::InstrumentCodes {
                by_isin: HashMap::new(),
                reserved_bytes: usize::MAX,
                byte_budget: usize::MAX,
            })
        }

        /// Learn what `event` states, then fill what it left unstated.
        pub fn enrich<E: MarketElement>(&mut self, event: &mut E) {
            self.0.enrich(event);
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
}
