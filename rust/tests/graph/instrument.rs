//! `rust/src/graph/instrument.rs`: the learned associations no caller can name.
//!
//! The registry is a lifecycle's own - nothing above it names one, and what it
//! learned is only ever read off the events it filled - so what it learns,
//! what it refuses to learn, and what it costs are reached through
//! `yggdryl::internals`.

use yggdryl::graph::{MarketElement, MarketElementData, MarketEventData};
use yggdryl::internals::graph_instrument::{ENTRY_CHARGE, InstrumentCodes};
use yggdryl::{BloombergCode, CfiCode, CusipCode, IsinCode, SedolCode};

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
    assert_eq!(codes.instruments(), 0);
    assert_eq!(codes.reserved_bytes(), 0);
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
        (codes.instruments(), codes.reserved_bytes()),
        (1, ENTRY_CHARGE)
    );

    let mut same = apple();
    same.set_sedolcode(Some(SedolCode::new("2046251").unwrap()));
    codes.enrich(&mut same);
    assert_eq!(
        (codes.instruments(), codes.reserved_bytes()),
        (1, ENTRY_CHARGE),
        "a known ISIN consumes no second reservation"
    );

    codes.enrich(&mut numbered(1));
    assert_eq!(
        (codes.instruments(), codes.reserved_bytes()),
        (2, 2 * ENTRY_CHARGE)
    );
    for number in 2..128 {
        codes.enrich(&mut numbered(number));
    }
    assert_eq!(
        (codes.instruments(), codes.reserved_bytes()),
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
    assert_eq!(codes.reserved_bytes(), 2 * ENTRY_CHARGE);

    let mut overflow = InstrumentCodes::saturated();
    overflow.enrich(&mut apple());
    assert_eq!(
        overflow.instruments(),
        0,
        "reservation arithmetic is checked"
    );
}
