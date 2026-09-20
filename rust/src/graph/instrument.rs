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

    #[cfg(test)]
    fn with_budget(byte_budget: usize) -> Self {
        Self {
            byte_budget,
            ..Self::default()
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{MarketElementData, MarketEventData};

    fn apple() -> MarketEventData {
        let mut event = MarketEventData::at(1);
        event.set_isincode(Some(IsinCode::new("US0378331005").unwrap()));
        event
    }

    fn numbered(number: usize) -> MarketEventData {
        let body = format!("FR{number:09}");
        let digit = IsinCode::closing_digit(&body).unwrap();
        let mut event = MarketEventData::at(number as i64);
        event.set_isincode(Some(IsinCode::new(format!("{body}{digit}")).unwrap()));
        event
    }

    #[test]
    fn learned_codes_are_local_validated_and_ambiguous_defaults_are_silent() {
        let mut codes = InstrumentCodes::default();
        let mut first = apple();
        first.set_cficode(Some(CfiCode::new("ESXXXX").unwrap()));
        first.set_bloombergcode(Some(BloombergCode::new("AAPL US Equity").unwrap()));
        codes.enrich(&mut first);
        let mut next = apple();
        codes.enrich(&mut next);
        assert!(
            next.get_cficode().is_none(),
            "coarse classifications are not learned"
        );
        assert_eq!(next.get_bloombergcode(), first.get_bloombergcode());

        let mut precise = apple();
        precise.set_cficode(Some(CfiCode::new("ESVUFR").unwrap()));
        precise.set_sedolcode(Some(SedolCode::new("2046251").unwrap()));
        codes.enrich(&mut precise);
        let mut later = apple();
        codes.enrich(&mut later);
        assert_eq!(later.get_cficode(), precise.get_cficode());
        assert_eq!(later.get_sedolcode(), precise.get_sedolcode());
        assert_eq!(
            first.get_cficode().unwrap().as_str(),
            "ESXXXX",
            "earlier snapshots stay unchanged"
        );
        let mut coarse = apple();
        coarse.set_cficode(Some(CfiCode::new("ESXXXX").unwrap()));
        codes.enrich(&mut coarse);
        assert_eq!(coarse.get_cficode(), precise.get_cficode());

        let mut conflict = apple();
        conflict.set_bloombergcode(Some(BloombergCode::new("AAPL LN Equity").unwrap()));
        codes.enrich(&mut conflict);
        assert_eq!(
            conflict.get_bloombergcode().unwrap().as_str(),
            "AAPL LN Equity"
        );
        let mut after_conflict = apple();
        codes.enrich(&mut after_conflict);
        assert!(after_conflict.get_bloombergcode().is_none());

        let mut independent = apple();
        InstrumentCodes::default().enrich(&mut independent);
        assert!(independent.get_cficode().is_none());
        assert!(independent.get_bloombergcode().is_none());
    }

    #[test]
    fn invalid_default_codes_cannot_seed_associations() {
        let mut codes = InstrumentCodes::default();
        let mut empty = MarketElementData::default();
        empty.set_isincode(Some(IsinCode::default()));
        codes.enrich(&mut empty);
        assert!(codes.by_isin.is_empty());
        assert_eq!(codes.reserved_bytes, 0);
        let mut observed = apple();
        observed.set_cficode(Some(CfiCode::new("XXXXXX").unwrap()));
        observed.set_cusipcode(Some(CusipCode::default()));
        observed.set_sedolcode(Some(SedolCode::default()));
        observed.set_bloombergcode(Some(BloombergCode::default()));
        codes.enrich(&mut observed);
        let mut later = apple();
        codes.enrich(&mut later);
        assert!(later.get_cficode().is_none());
        assert!(later.get_sedolcode().is_none());
        assert!(later.get_bloombergcode().is_none());
    }

    #[test]
    fn the_byte_budget_bounds_new_instruments_and_keeps_learning_known_ones() {
        let mut codes = InstrumentCodes::with_budget(2 * ENTRY_CHARGE);
        let mut first = apple();
        first.set_bloombergcode(Some(BloombergCode::new("AAPL US Equity").unwrap()));
        codes.enrich(&mut first);
        assert_eq!(
            (codes.by_isin.len(), codes.reserved_bytes),
            (1, ENTRY_CHARGE)
        );

        let mut same = apple();
        same.set_sedolcode(Some(SedolCode::new("2046251").unwrap()));
        codes.enrich(&mut same);
        assert_eq!(
            (codes.by_isin.len(), codes.reserved_bytes),
            (1, ENTRY_CHARGE),
            "a known ISIN consumes no second reservation"
        );

        codes.enrich(&mut numbered(1));
        assert_eq!(
            (codes.by_isin.len(), codes.reserved_bytes),
            (2, 2 * ENTRY_CHARGE)
        );
        for number in 2..128 {
            codes.enrich(&mut numbered(number));
        }
        assert_eq!(
            (codes.by_isin.len(), codes.reserved_bytes),
            (2, 2 * ENTRY_CHARGE),
            "repeated unseen instruments cannot grow a full registry"
        );

        let mut learned_at_cap = apple();
        learned_at_cap.set_cficode(Some(CfiCode::new("ESVUFR").unwrap()));
        codes.enrich(&mut learned_at_cap);
        let mut later = apple();
        codes.enrich(&mut later);
        assert_eq!(later.get_cficode(), learned_at_cap.get_cficode());
        assert_eq!(later.get_sedolcode(), same.get_sedolcode());
        assert_eq!(later.get_bloombergcode(), first.get_bloombergcode());
        assert_eq!(codes.reserved_bytes, 2 * ENTRY_CHARGE);

        let mut overflow = InstrumentCodes {
            by_isin: HashMap::new(),
            reserved_bytes: usize::MAX,
            byte_budget: usize::MAX,
        };
        overflow.enrich(&mut apple());
        assert!(
            overflow.by_isin.is_empty(),
            "reservation arithmetic is checked"
        );
    }
}
