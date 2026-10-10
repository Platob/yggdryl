//! `rust/market/src/graph/market.rs`: the `Market` and `Operation` traits and the
//! readings they provide an implementor that is also an `Event` - the
//! digests, following and merging a dated market or operation runs.

use std::borrow::Cow;
use std::hash::Hasher;

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event};
use yggdryl::xxhash::Xxh3;
use yggdryl::{Ccy, Cfi, Decimal, Isin, Mic, Unit, Uuid};
use yggdryl_market::IdKey;
use yggdryl_market::graph::{
    BookEvent, ExecutionEvent, FxRates, Market, Operation, Order, OrderEvent, QuoteEvent,
    SnapshotEvent, TradeEvent,
};
use yggdryl_market::{IdType, Identifier, Identifiers, Side, TimeInForce};

/// The identifier set holding `ids`, each `(kind, code)` - a type's name or
/// its FIX source code - validated by its type and stated from `base`.
fn securityids(ids: &[(&str, &str)]) -> Identifiers {
    ids.iter()
        .map(|(kind, code)| {
            Identifier::new(
                IdKey::base(IdType::from_security_source(kind).unwrap()),
                code,
            )
            .unwrap()
        })
        .collect()
}

/// Every identifier an element holds, as `src:type=code`, in key order.
fn ids(element: &impl Market) -> Vec<String> {
    element
        .get_securityids()
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// One order at `unix` under the cross code `ORDER`, finalized.
fn order(unix: i64) -> OrderEvent {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode("ORDER".to_owned());
    order.set_price(Some(Decimal::from_int(80)), true);
    order.finalize();
    order
}

/// The market event digest feeds the market's facts and nothing an
/// operation states; the operation event digest feeds both.
#[test]
fn the_operation_event_digest_feeds_what_the_market_event_digest_does_not() {
    crate::install::installed();
    let plain = order(1);
    let mut standing = plain.clone();
    standing.set_timeinforce(TimeInForce::from_spelling("GoodTillCancel"), true);
    assert_eq!(
        plain.digest_market_event().as_u64(),
        standing.digest_market_event().as_u64(),
    );
    assert_ne!(
        plain.digest_operation_event().as_u64(),
        standing.digest_operation_event().as_u64(),
    );
    let mut priced = plain.clone();
    priced.set_price(Some(Decimal::from_int(81)), true);
    assert_ne!(
        plain.digest_market_event().as_u64(),
        priced.digest_market_event().as_u64(),
    );
}

/// Following a predecessor carries the market facts the chain shares -
/// here the ticker - and, only for the operation reading, the operation's
/// own - here the time in force. Neither follows itself or a later event.
#[test]
fn following_carries_the_market_and_only_the_operation_reading_carries_the_operation() {
    crate::install::installed();
    let mut previous = order(1);
    previous.set_ticker(Some(SmolStr::new("BRN")), true);
    previous.set_timeinforce(TimeInForce::from_spelling("GoodTillCancel"), true);
    previous.finalize();
    let next = order(2);

    let market = next
        .clone()
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(market.get_ticker(), Some("BRN"));
    assert_eq!(market.get_prevuuid(), Some(previous.get_uuid()));
    assert_eq!(
        market.get_timeinforce(),
        None,
        "the market reading leaves the operation"
    );

    let operation = next
        .clone()
        .following_operation(&previous)
        .expect("a later event follows");
    assert_eq!(operation.get_ticker(), Some("BRN"));
    assert_eq!(operation.get_timeinforce(), previous.get_timeinforce());

    assert!(previous.clone().following_market(&next).is_none());
    assert!(previous.clone().following_operation(&next).is_none());
    assert!(next.clone().following_market(&next).is_none());
}

/// Merging another statement of the same event takes what the market
/// reading merges - its sources - and, only for the operation reading,
/// the operation's facts; a stranger merges with neither.
#[test]
fn merging_takes_the_market_and_only_the_operation_reading_takes_the_operation() {
    crate::install::installed();
    let this = order(1);
    let mut restated = this.clone();
    restated.set_srcuuids(vec![Uuid::from_v8(9)]);
    restated.set_timeinforce(TimeInForce::from_spelling("GoodTillCancel"), true);

    let market = this
        .clone()
        .merging_market_event(&restated)
        .expect("the sources move");
    assert_eq!(market.get_srcuuids(), [Uuid::from_v8(9)]);
    assert_eq!(market.get_timeinforce(), None);

    let operation = this
        .clone()
        .merging_operation_event(&restated)
        .expect("the sources move");
    assert_eq!(operation.get_srcuuids(), [Uuid::from_v8(9)]);
    assert_eq!(operation.get_timeinforce(), restated.get_timeinforce());

    let mut stranger = OrderEvent::at(1);
    stranger.set_crosscode("OTHER".to_owned());
    stranger.finalize();
    assert!(this.clone().merging_market_event(&stranger).is_none());
    assert!(this.clone().merging_operation_event(&stranger).is_none());
}

/// Two statements of one event merge the price, the quantity and the unit
/// as they merge every other fact: the leading statement's where it states
/// one, the other's where it states none - never cleared because the
/// leading statement is silent.
#[test]
fn merging_takes_the_price_either_statement_states() {
    crate::install::installed();
    let mut priced = OrderEvent::at(1);
    priced.set_crosscode("ORD-1".to_owned());
    priced.set_price(Some(dec("100")), true);
    priced.set_unit(Unit::new("share").unwrap(), true);
    priced.finalize();
    // Recorded, so it leads whichever side of the merge it is on.
    let mut sized = OrderEvent::at(1);
    sized.set_crosscode("ORD-1".to_owned());
    sized.set_quantity(Some(dec("5")), true);
    sized.set_sendunix(Some(2));
    sized.finalize();
    sized.set_uuid(priced.get_uuid());
    for (this, other) in [(priced.clone(), &sized), (sized.clone(), &priced)] {
        let merged = this.merge_with(other).expect("one event stated twice");
        assert_eq!(merged.get_price(), Some(dec("100")));
        assert_eq!(merged.get_quantity(), Some(dec("5")));
        assert_eq!(merged.get_unit(), &Unit::new("share").unwrap());
    }

    let mut repriced = sized.clone();
    repriced.set_price(Some(dec("101")), true);
    repriced.set_uuid(priced.get_uuid());
    for (this, other) in [(priced.clone(), &repriced), (repriced.clone(), &priced)] {
        let merged = this.merge_with(other).expect("one event stated twice");
        assert_eq!(
            merged.get_price(),
            Some(dec("101")),
            "the leading statement's where both state one"
        );
    }
}

/// A chain carries its instrument's identifiers to the step that states
/// none, what the ISIN implied included; a step naming another ISIN is
/// another instrument and takes none of them.
#[test]
fn following_carries_the_identifiers_only_of_the_same_instrument() {
    crate::install::installed();
    let mut previous = order(1);
    previous
        .set_securityids(
            securityids(&[("ISIN", "US0378331005"), ("RIC", "AAPL.O")]),
            true,
        )
        .unwrap();
    previous.finalize();
    assert_eq!(
        ids(&previous),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005",
            "ric=AAPL.O"
        ]
    );

    let silent = order(2)
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(ids(&silent), ids(&previous));

    let mut other = order(2);
    other
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]), true)
        .unwrap();
    other.finalize();
    let followed = other
        .clone()
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(
        ids(&followed),
        [
            "derived:sedol=0263494",
            "isin=GB0002634946",
            "sedol=0263494"
        ]
    );
}

/// The strike of the option an element is about is an instrument fact: a
/// follower of the same instrument stating none takes its chain's, one
/// naming another ISIN takes none and a stated strike stands; a merge takes
/// the strike either statement states; and the digest feeds it only where
/// it is stated, so an element stating none digests as it did before.
#[test]
fn the_strike_follows_its_instrument_merges_and_digests_only_where_stated() {
    crate::install::installed();
    let isin = |code: &str| securityids(&[("ISIN", code)]);
    let mut previous = order(1);
    previous
        .set_securityids(isin("US0378331005"), true)
        .unwrap();
    previous.set_strikepx(Some(dec("4600.5")), true);
    previous.finalize();

    let silent = order(2)
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(silent.get_strikepx(), Some(dec("4600.5")));

    let mut stated = order(2);
    stated.set_strikepx(Some(dec("4700")), true);
    stated.finalize();
    let stated = stated
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(
        stated.get_strikepx(),
        Some(dec("4700")),
        "a stated strike stands"
    );

    let mut other = order(2);
    other.set_securityids(isin("GB0002634946"), true).unwrap();
    other.finalize();
    let other = other
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(other.get_strikepx(), None, "another instrument's strike");

    // Two statements of one event, each stating what the other does not:
    // the merge takes the strike either one states.
    let plain = order(1);
    let mut sized = plain.clone();
    sized.set_quantity(Some(dec("5")), true);
    sized.finalize();
    let mut struck = plain.clone();
    struck.set_strikepx(Some(dec("4600.5")), true);
    struck.set_sendunix(Some(2));
    struck.finalize();
    struck.set_uuid(sized.get_uuid());
    for (this, other) in [(sized.clone(), &struck), (struck.clone(), &sized)] {
        let merged = this.merge_with(other).expect("one event stated twice");
        assert_eq!(merged.get_strikepx(), Some(dec("4600.5")));
        assert_eq!(merged.get_quantity(), Some(dec("5")));
    }

    // Fed only where stated.
    let mut cleared = plain.clone();
    cleared.set_strikepx(Some(dec("4600.5")), true);
    assert_ne!(
        cleared.digest_market_event().as_u64(),
        plain.digest_market_event().as_u64()
    );
    cleared.set_strikepx(None, true);
    assert_eq!(
        cleared.digest_market_event().as_u64(),
        plain.digest_market_event().as_u64()
    );
}

/// The currency an instrument originates in is held only where stated or
/// filled: stating the currency leaves it unheld, and the origin read
/// answers the currency there; a stated origin stands over a later
/// currency and the currency is never read off it; a fill lands only where
/// none is held. A follower of the same instrument holding none takes its
/// chain's, one naming another ISIN takes none; a merge takes the origin
/// either statement holds; and the digest feeds it only where held, so an
/// element holding none digests as it did before the fact existed.
#[test]
fn the_origin_currency_is_held_only_where_stated_and_read_as_the_currency_else() {
    crate::install::installed();
    let ccy = |code: &str| Ccy::new(code).unwrap();
    let isin = |code: &str| securityids(&[("ISIN", code)]);

    let mut listed = order(1);
    listed.set_currency(ccy("EUR"), true);
    assert!(
        listed.get_origccy().is_none(),
        "the currency states no origin"
    );
    assert_eq!(listed.origin_currency(), &ccy("EUR"));
    listed.set_origccy(ccy("USD"), true);
    listed.set_currency(ccy("GBP"), true);
    assert_eq!(listed.get_origccy(), &ccy("USD"), "a stated origin stands");
    assert_eq!(listed.origin_currency(), &ccy("USD"));
    assert_eq!(
        listed.get_currency(),
        &ccy("GBP"),
        "never read off the origin"
    );
    // A fill lands only where none is held; a statement replaces it.
    listed.set_origccy(ccy("CHF"), false);
    assert_eq!(listed.get_origccy(), &ccy("USD"));
    let mut unheld = order(1);
    unheld.set_origccy(ccy("CHF"), false);
    assert_eq!(unheld.get_origccy(), &ccy("CHF"));
    unheld.set_origccy(Ccy::none(), true);
    assert!(unheld.get_origccy().is_none(), "none clears it");
    let mut none = order(1);
    assert!(none.origin_currency().is_none(), "neither stated");
    none.set_origccy(Ccy::none(), false);
    assert!(none.get_origccy().is_none());

    // Followed along the instrument.
    let mut previous = order(1);
    previous
        .set_securityids(isin("US0378331005"), true)
        .unwrap();
    previous.set_origccy(ccy("USD"), true);
    previous.finalize();
    let silent = order(2)
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(silent.get_origccy(), &ccy("USD"));
    let mut stated = order(2);
    stated.set_origccy(ccy("JPY"), true);
    stated.finalize();
    let stated = stated
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(stated.get_origccy(), &ccy("JPY"), "a stated origin stands");
    let mut other = order(2);
    other.set_securityids(isin("GB0002634946"), true).unwrap();
    other.finalize();
    let other = other
        .following_market(&previous)
        .expect("a later event follows");
    assert!(other.get_origccy().is_none(), "another instrument's origin");

    // Two statements of one event: the merge takes the origin either holds.
    let plain = order(1);
    let mut sized = plain.clone();
    sized.set_quantity(Some(dec("5")), true);
    sized.finalize();
    let mut issued = plain.clone();
    issued.set_origccy(ccy("USD"), true);
    issued.set_sendunix(Some(2));
    issued.finalize();
    issued.set_uuid(sized.get_uuid());
    for (this, other) in [(sized.clone(), &issued), (issued.clone(), &sized)] {
        let merged = this.merge_with(other).expect("one event stated twice");
        assert_eq!(merged.get_origccy(), &ccy("USD"));
        assert_eq!(merged.get_quantity(), Some(dec("5")));
    }

    // Fed only where held.
    let mut cleared = plain.clone();
    cleared.set_origccy(ccy("USD"), true);
    assert_ne!(
        cleared.digest_market_event().as_u64(),
        plain.digest_market_event().as_u64()
    );
    cleared.set_origccy(Ccy::none(), true);
    assert_eq!(
        cleared.digest_market_event().as_u64(),
        plain.digest_market_event().as_u64()
    );
}

/// A follower that derived its instrument's real number - a registry's
/// fill - keeps it over the masked number its chain stated before it,
/// under the base key or under a named source, which it carries as
/// evidence: a derivation ranks like a statement.
#[test]
fn following_keeps_a_derived_real_isin_over_the_chains_masked_statement() {
    crate::install::installed();
    let derived = |unix: i64| {
        let mut next = order(unix);
        next.insert_securityid(
            Identifier::new("derived:isin".parse().unwrap(), "US0378331005").unwrap(),
        )
        .unwrap();
        next.finalize();
        next
    };
    let mut previous = order(1);
    previous
        .set_securityids(securityids(&[("ISIN", "XX0000000001")]), true)
        .unwrap();
    previous.finalize();
    let followed = derived(2)
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(followed.get_isincode(), Some("US0378331005"));
    assert_eq!(followed.book_crosscode(), "US0378331005");
    assert!(followed.get_securityids().is_derived(&IdType::Isin));

    let mut previous = order(1);
    previous
        .insert_securityid(Identifier::new("ullink:isin".parse().unwrap(), "XX0000000001").unwrap())
        .unwrap();
    previous.finalize();
    assert_eq!(previous.get_isincode(), Some("XX0000000001"));
    let followed = derived(2)
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(followed.get_isincode(), Some("US0378331005"));
    assert!(followed.get_securityids().is_derived(&IdType::Isin));
    assert_eq!(
        followed
            .get_securityids()
            .get_from(&"ullink:isin".parse().unwrap()),
        Some("XX0000000001"),
        "the chain's named source is carried as evidence"
    );
}

/// A statement holding a derived real ISIN beside a masked named source it
/// carries as evidence keeps the derivation when merged with a restatement
/// whose named source is a typo - outranking the evidence, not the
/// derivation - whichever statement leads.
#[test]
fn merging_keeps_a_derived_real_isin_over_a_restated_named_source_whichever_leads() {
    crate::install::installed();
    let named = |value: &str| Identifier::new("ullink:isin".parse().unwrap(), value).unwrap();
    let mut derived = order(1);
    derived
        .insert_securityid(
            Identifier::new("derived:isin".parse().unwrap(), "US0378331005").unwrap(),
        )
        .unwrap();
    derived.insert_securityid(named("XX0000000001")).unwrap();
    assert_eq!(derived.get_isincode(), Some("US0378331005"));
    // The same order restated - one identity - naming its instrument by a
    // typo under the bridge's source.
    let mut typo = derived.clone();
    typo.set_securityids([named("US0378331006")].into_iter().collect(), true)
        .unwrap();
    assert_eq!(typo.get_isincode(), Some("US0378331006"));
    for (this, other) in [(&derived, &typo), (&typo, &derived)] {
        let merged = this
            .clone()
            .merging_operation_event(other)
            .unwrap_or_else(|| this.clone());
        assert_eq!(merged.get_isincode(), Some("US0378331005"));
        assert_eq!(merged.book_crosscode(), "US0378331005");
        assert!(merged.get_securityids().is_derived(&IdType::Isin));
    }
}

/// Two statements of one element naming different ISINs name two
/// instruments: the leading statement's identifiers stand whole, never a
/// key of the other beside them.
#[test]
fn merging_statements_naming_different_isins_keeps_the_leading_identifiers() {
    crate::install::installed();
    let mut this = Order::new();
    this.set_crosscode("ORDER".to_owned());
    this.set_securityids(
        securityids(&[("ISIN", "US0378331005"), ("RIC", "AAPL.O")]),
        true,
    )
    .unwrap();
    this.finalize();
    let mut restated = this.clone();
    restated
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]), true)
        .unwrap();
    restated.set_price(Some(Decimal::from_int(81)), true);

    // This statement leads, so it keeps its identifiers: the other's SEDOL
    // does not join them. The price it states none of is the other's, as
    // every fact one statement leaves open is.
    let merged = this
        .clone()
        .merging_market(&restated)
        .expect("the price fills");
    assert_eq!(ids(&merged), ids(&this));
    assert_eq!(merged.get_price(), Some(Decimal::from_int(81)));

    // Where the other statement leads, its identifiers replace these whole.
    let mut first = order(1);
    first
        .set_securityids(
            securityids(&[("ISIN", "US0378331005"), ("RIC", "AAPL.O")]),
            true,
        )
        .unwrap();
    first.finalize();
    let mut recorded = first.clone();
    recorded.set_sendunix(Some(5));
    recorded
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]), true)
        .unwrap();
    let merged = first
        .clone()
        .merging_market_event(&recorded)
        .expect("the statement sent later leads");
    assert_eq!(
        ids(&merged),
        [
            "derived:sedol=0263494",
            "isin=GB0002634946",
            "sedol=0263494"
        ]
    );

    // The same instrument restated fills what this statement left open.
    let mut same = this.clone();
    same.set_securityids(
        securityids(&[("ISIN", "US0378331005"), ("FIGI", "BBG000BLNQ16")]),
        true,
    )
    .unwrap();
    let merged = this.clone().merging_market(&same).expect("the FIGI fills");
    assert_eq!(
        ids(&merged),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "figi=BBG000BLNQ16",
            "isin=US0378331005",
            "ric=AAPL.O"
        ]
    );
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// An execution's price stays the one it states - none - through any
/// number of finalizes: what it last executed is its `lastpx`, never its
/// price. Its quantity is another matter: what executed is what an
/// execution is about, so its `lastqty` is its quantity (decision 27).
#[test]
fn an_execution_price_is_never_what_it_last_executed() {
    crate::install::installed();
    let mut fill = ExecutionEvent::at(10);
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::read("Buy").unwrap(), true);
    fill.set_lastpx(Some(dec("100.5")), true);
    fill.set_lastqty(Some(Decimal::from_int(5)), true);
    fill.finalize();
    let once = fill.clone();
    fill.finalize();
    assert_eq!(fill, once, "finalizing twice changes nothing");
    assert_eq!(
        (fill.get_price(), fill.get_quantity()),
        (None, Some(Decimal::from_int(5)))
    );
    assert_eq!(fill.get_lastpx(), Some(dec("100.5")));
    assert_eq!(fill.get_lastqty(), Some(Decimal::from_int(5)));
}

fn rates(stated: &[(&str, &str)]) -> FxRates {
    stated
        .iter()
        .map(|(target, rate)| (Ccy::new(target).unwrap(), dec(rate)))
        .collect()
}

/// Rates are keyed by target currency and never carried: a follower states
/// only its own, and a merge takes the union, the leading statement's rate
/// where both state one.
#[test]
fn fxrates_are_never_followed_and_merge_per_target() {
    crate::install::installed();
    let mut previous = order(1);
    previous.set_fxrates(rates(&[("USD", "1.08"), ("JPY", "150")]), true);
    previous.finalize();
    let mut next = order(2);
    next.set_fxrates(rates(&[("USD", "1.09")]), true);
    next.finalize();
    let followed = next
        .clone()
        .with_previous(&previous)
        .expect("a later order");
    assert_eq!(followed.get_fxrates(), &rates(&[("USD", "1.09")]));

    let mut restated = next.clone();
    restated.set_fxrates(rates(&[("USD", "1.10"), ("GBP", "0.86")]), true);
    let merged = next
        .clone()
        .merging_operation_event(&restated)
        .expect("the same order restated");
    assert_eq!(
        merged.get_fxrates(),
        &rates(&[("USD", "1.09"), ("GBP", "0.86")]),
        "the leading statement keeps its rate for the target both state"
    );
    assert!(
        order(3).get_fxrates().is_empty(),
        "an element stating no rate states none"
    );
    // An empty map states none, and digests as none.
    let mut empty = order(3);
    empty.set_fxrates(FxRates::new(), true);
    empty.finalize();
    assert_eq!(empty.get_uuid(), order(3).get_uuid());
}

/// A `ZZ` ISIN names no country's instrument: it yields to a real one on
/// follow and on merge, the national code the real one carries deriving
/// afresh, and it never makes two statements two instruments.
#[test]
fn an_unknown_isin_yields_to_a_real_one() {
    crate::install::installed();
    let mut previous = order(1);
    previous
        .set_securityids(
            securityids(&[("ISIN", "US0378331005"), ("BLOOMBERG", "BBG000B9XRY4")]),
            true,
        )
        .unwrap();
    previous.finalize();
    assert_eq!(
        previous.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    let mut next = order(2);
    next.set_securityids(securityids(&[("ISIN", "ZZ0000000008")]), true)
        .unwrap();
    next.finalize();
    let followed = next
        .clone()
        .with_previous(&previous)
        .expect("a later order");
    assert_eq!(followed.get_isincode(), Some("US0378331005"));
    assert_eq!(
        followed.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    assert_eq!(
        followed.get_securityids().get(&IdType::Bloomberg),
        Some("BBG000B9XRY4"),
        "a ZZ ISIN is no other instrument"
    );

    let mut restated = next.clone();
    restated
        .set_securityids(securityids(&[("ISIN", "US0378331005")]), true)
        .unwrap();
    let merged = next
        .clone()
        .merging_operation_event(&restated)
        .expect("the same order restated");
    assert_eq!(merged.get_isincode(), Some("US0378331005"));
    assert_eq!(
        merged.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    // And the other way round: a real ISIN never takes a ZZ one, leading or
    // not.
    let merged = restated
        .clone()
        .merging_operation_event(&next)
        .unwrap_or_else(|| restated.clone());
    assert_eq!(merged.get_isincode(), Some("US0378331005"));
}

/// A number ranking below another - a masked one, a typo its check digit
/// does not close - yields to it on follow and on merge whichever leads,
/// as a `ZZ` number does, and two numbers of one rank are two instruments
/// only where both are real.
#[test]
fn a_lower_ranked_isin_yields_to_a_higher_one_whichever_leads() {
    crate::install::installed();
    let mut real = order(1);
    real.set_securityids(securityids(&[("ISIN", "US0378331005")]), true)
        .unwrap();
    real.finalize();
    for lower in ["XX0000000001", "US0378331006"] {
        let mut next = order(2);
        next.set_securityids(securityids(&[("ISIN", lower)]), true)
            .unwrap();
        next.finalize();
        assert!(
            next.get_securityids().get(&IdType::Cusip).is_none(),
            "{lower}: nothing derives off a number that does not close"
        );
        let followed = next.clone().with_previous(&real).expect("a later order");
        assert_eq!(followed.get_isincode(), Some("US0378331005"), "{lower}");
        assert_eq!(
            followed.get_securityids().get(&IdType::Cusip),
            Some("037833100"),
            "{lower}: the real number's national code derives afresh"
        );
        // Merging, whichever statement leads.
        let mut restated = next.clone();
        restated
            .set_securityids(securityids(&[("ISIN", "US0378331005")]), true)
            .unwrap();
        let merged = next
            .clone()
            .merging_operation_event(&restated)
            .expect("the same order restated");
        assert_eq!(merged.get_isincode(), Some("US0378331005"), "{lower}");
        let merged = restated
            .clone()
            .merging_operation_event(&next)
            .unwrap_or_else(|| restated.clone());
        assert_eq!(merged.get_isincode(), Some("US0378331005"), "{lower}");
    }
    // A masked number yields to a typo too: one reading beats none.
    let mut typo = order(1);
    typo.set_securityids(securityids(&[("ISIN", "US0378331006")]), true)
        .unwrap();
    typo.finalize();
    let mut masked = order(2);
    masked
        .set_securityids(securityids(&[("ISIN", "XX0000000001")]), true)
        .unwrap();
    masked.finalize();
    let followed = masked.clone().with_previous(&typo).expect("a later order");
    assert_eq!(followed.get_isincode(), Some("US0378331006"));
    // Two masked numbers are not two instruments, and the leading one keeps.
    let mut other = order(3);
    other
        .set_securityids(securityids(&[("ISIN", "XX0000000003")]), true)
        .unwrap();
    other.finalize();
    let followed = other.clone().with_previous(&masked).expect("a later order");
    assert_eq!(followed.get_isincode(), Some("XX0000000003"));
}

/// The ISIN is borrowed from the identifiers: no copy, no second store.
#[test]
fn get_isincode_borrows_the_isin_identifier() {
    crate::install::installed();
    let mut listed = order(1);
    listed
        .set_securityids(securityids(&[("ISIN", "US0378331005")]), true)
        .unwrap();
    let projected = listed.get_isincode().unwrap();
    let stored = listed.get_securityids().get(&IdType::Isin).unwrap();
    assert!(std::ptr::eq(projected, stored));
    assert_eq!(order(2).get_isincode(), None);
}

/// The XXH3-64 of one stored cross code, as the element derives its cross
/// hash.
fn crosshash(stored: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(stored.as_bytes());
    state.as_u64()
}

/// Every element stores its cross code as `{kind}:{side}:{base}`: the code
/// of the category it is filed under and, for a sided one - an order, an
/// execution - the code of the side it takes, `0` for any other kind - a
/// quote, whose side is a tag - and for a side nobody stated. The cross hash
/// and the cross element follow the stored text.
#[test]
fn every_element_stores_its_cross_code_under_its_category_and_side() {
    crate::install::installed();
    let stated = |side: Side| {
        let mut buy = OrderEvent::at(1);
        buy.set_crosscode("ORD-1".to_owned());
        buy.set_side(side, true);
        buy.finalize();
        buy
    };
    let buy = stated(Side::Buy);
    assert!(buy.is_sided());
    assert_eq!(buy.get_crosscode(), "10:1:ORD-1");
    assert_eq!(stated(Side::Sell).get_crosscode(), "10:2:ORD-1");
    assert_eq!(stated(Side::Unknown).get_crosscode(), "10:0:ORD-1");
    assert_eq!(buy.get_crosshashcode(), crosshash("10:1:ORD-1"));
    assert_eq!(
        buy.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("10:1:ORD-1")))
    );

    let mut quote = QuoteEvent::at(1);
    quote.set_crosscode("Q-1".to_owned());
    quote.set_side(Side::Buy, true);
    quote.finalize();
    assert!(!quote.is_sided());
    assert_eq!(quote.get_crosscode(), "14:0:Q-1");

    let mut fill = ExecutionEvent::at(1);
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::Sell, true);
    fill.finalize();
    assert!(fill.is_sided());
    assert_eq!(fill.get_crosscode(), "8:2:E-1");

    // A book is not sided: it states side 0 whatever side it takes, and a
    // `BUYS:` in its name is the name's own, never a prefix.
    let mut book = BookEvent::new(1, "AAPL");
    book.set_side(Side::Buy, true);
    book.finalize();
    assert!(!book.is_sided());
    assert_eq!(book.get_crosscode(), "3:0:AAPL");
    assert_eq!(book.stored_crosscode("AAPL"), "3:0:AAPL");
    let mut keyed = BookEvent::new(1, "XNAS:ESVUFR");
    keyed.finalize();
    assert_eq!(keyed.get_crosscode(), "3:0:XNAS:ESVUFR");
    let mut named = BookEvent::new(1, "BUYS:AAPL");
    named.set_side(Side::Sell, true);
    assert_eq!(named.get_crosscode(), "3:0:BUYS:AAPL");

    // A snapshot control is a book: the code of the element it was read
    // from, under the book's prefix and no side, whatever side that stated.
    let snapshot = SnapshotEvent::snapshot(&buy, None);
    assert!(!snapshot.is_sided());
    assert_eq!(
        (snapshot.get_crosscode(), snapshot.get_side()),
        ("3:0:ORD-1", Side::Buy)
    );

    // A trade states side 0 as a book does; its executions keep their own.
    let trade = TradeEvent::from_parts(&buy, vec![fill]).unwrap();
    assert!(!trade.is_sided());
    assert_eq!(trade.get_crosscode(), "21:0:ORD-1");
    assert_eq!(trade.executions()[0].get_crosscode(), "8:2:E-1");
    assert_eq!(trade.get_crosshashcode(), crosshash("21:0:ORD-1"));
    let mut unnamed = BookEvent::new(1, "ORD-1");
    unnamed.finalize();
    assert_ne!(
        trade.get_crosshashcode(),
        unnamed.get_crosshashcode(),
        "a trade and a book under one name are two chains"
    );

    // An order over the trade's facts is sided again, under its side.
    let again = OrderEvent::from(&trade);
    assert_eq!(again.get_crosscode(), "10:1:ORD-1");
    assert_eq!(again.get_crosshashcode(), buy.get_crosshashcode());
}

/// The prefix is decided in one place, so the order in which the code, the
/// side and the stamped kind are stated never matters: each converges on
/// the same stored code, the same cross hash and the same cross element.
#[test]
fn the_prefix_follows_the_side_and_the_kind_whichever_is_stated_last() {
    crate::install::installed();
    let mut code_first = OrderEvent::at(1);
    code_first.set_crosscode("ORD-1".to_owned());
    assert_eq!(code_first.get_crosscode(), "10:0:ORD-1");
    code_first.set_side(Side::Buy, true);
    assert_eq!(
        code_first.get_crosscode(),
        "10:1:ORD-1",
        "the side moves it"
    );
    assert_eq!(code_first.get_crosshashcode(), crosshash("10:1:ORD-1"));
    assert_eq!(code_first.get_crossuuid(), code_first.cross_uuid());

    let mut side_first = OrderEvent::at(1);
    side_first.set_side(Side::Buy, true);
    side_first.set_crosscode("ORD-1".to_owned());
    assert_eq!(side_first.get_crosscode(), "10:1:ORD-1");
    assert_eq!(
        side_first.get_crosshashcode(),
        code_first.get_crosshashcode()
    );
    assert_eq!(side_first.get_crossuuid(), code_first.get_crossuuid());

    // The side changing again reprefixes again, and back, the same code.
    code_first.set_side(Side::Sell, true);
    assert_eq!(code_first.get_crosscode(), "10:2:ORD-1");
    assert_ne!(
        code_first.get_crosshashcode(),
        side_first.get_crosshashcode()
    );
    code_first.set_side(Side::Buy, true);
    assert_eq!(code_first.get_crosscode(), "10:1:ORD-1");
    assert_eq!(
        code_first.get_crosshashcode(),
        side_first.get_crosshashcode()
    );

    // A kind stamped after the code moves it too: a leaf built over another
    // one's facts reprefixes the code it was given.
    let sell = {
        let mut sell = OrderEvent::at(1);
        sell.set_crosscode("ORD-1".to_owned());
        sell.set_side(Side::Sell, true);
        sell
    };
    let snapshot = SnapshotEvent::snapshot(&sell, None);
    assert_eq!(snapshot.get_crosscode(), "3:0:ORD-1");
    assert_eq!(snapshot.get_crosshashcode(), crosshash("3:0:ORD-1"));
    assert_eq!(snapshot.get_crossuuid(), snapshot.cross_uuid());
    let quote = QuoteEvent::from(&sell);
    assert_eq!(
        (quote.get_crosscode(), quote.get_side()),
        ("14:0:ORD-1", Side::Sell),
        "the side travels as a tag, the kind and its prefix are its own"
    );
}

/// A code already carrying another category's or side's prefix has it
/// replaced, never stacked; one carrying this element's own is answered as
/// it is, borrowed; and an empty code stays empty whatever the element
/// takes.
#[test]
fn a_prefix_is_replaced_never_stacked_and_an_empty_code_stays_empty() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_side(Side::Buy, true);
    assert!(
        matches!(
            order.stored_crosscode("10:1:ORD-1"),
            Cow::Borrowed("10:1:ORD-1")
        ),
        "idempotent: its own prefix is answered as it is"
    );
    assert_eq!(order.stored_crosscode("ORD-1"), "10:1:ORD-1");
    assert_eq!(
        order.stored_crosscode("10:2:ORD-1"),
        "10:1:ORD-1",
        "another side"
    );
    assert_eq!(
        order.stored_crosscode("14:1:ORD-1"),
        "10:1:ORD-1",
        "another kind"
    );
    assert_eq!(order.stored_crosscode("21:0:ORD-1"), "10:1:ORD-1");
    // A base that holds a colon of its own keeps it: `3:0:` is the prefix and
    // the rest the base.
    assert_eq!(
        order.stored_crosscode("3:0:XNAS:ESVUFR"),
        "10:1:XNAS:ESVUFR"
    );
    assert_eq!(order.stored_crosscode("XNAS:ESVUFR"), "10:1:XNAS:ESVUFR");

    // Stating a stored code is stating its base: the same element.
    order.set_crosscode("14:2:ORD-1".to_owned());
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");
    let hash = order.get_crosshashcode();
    order.set_crosscode("10:1:ORD-1".to_owned());
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");
    assert_eq!(order.get_crosshashcode(), hash);

    // An empty code names nothing: no prefix, no hash, whatever the side.
    let mut empty = OrderEvent::at(1);
    assert_eq!(empty.stored_crosscode(""), "");
    assert!(matches!(empty.stored_crosscode(""), Cow::Borrowed("")));
    empty.set_side(Side::Buy, true);
    assert_eq!(empty.get_crosscode(), "");
    assert_eq!(empty.get_crosshashcode(), 0);
    empty.finalize();
    assert_eq!(empty.get_crosscode(), "");
    assert_eq!(empty.get_crosshashcode(), 0);
    // A book is never codeless: an empty symbol keys it by the number that
    // states none.
    let book = BookEvent::new(1, "");
    assert_eq!(book.get_crosscode(), "3:0:XX0000000000");
    let snapshot = SnapshotEvent::snapshot(&empty, None);
    assert_eq!(snapshot.get_crosscode(), "");
}

/// A copy of an element into another leaf kind takes the base code and
/// states it under its own prefix: the facts travel, the category is the
/// leaf's.
#[test]
fn a_copy_into_another_kind_takes_the_base_under_its_own_prefix() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_crosscode("ORD-1".to_owned());
    order.set_side(Side::Buy, true);
    order.finalize();
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");

    let quote = QuoteEvent::from(&order);
    assert_eq!(quote.get_crosscode(), "14:0:ORD-1");
    let fill = ExecutionEvent::from(&order);
    assert_eq!(fill.get_crosscode(), "8:1:ORD-1");
    let undated = order.clone().into_element();
    assert_eq!(undated.get_crosscode(), "10:1:ORD-1");
    let quoted = QuoteEvent::from(&fill);
    assert_eq!(quoted.get_crosscode(), "14:0:ORD-1");
    assert_ne!(quote.get_crosshashcode(), order.get_crosshashcode());
    assert_ne!(quote.get_crossuuid(), fill.get_crossuuid());
    assert_eq!(quote.get_crosshashcode(), quoted.get_crosshashcode());
    assert_eq!(quote.get_crosshashcode(), crosshash("14:0:ORD-1"));

    // The same copy of an order that stated no side states side 0 in each.
    let mut sideless = OrderEvent::at(1);
    sideless.set_crosscode("ORD-1".to_owned());
    assert_eq!(QuoteEvent::from(&sideless).get_crosscode(), "14:0:ORD-1");
    assert_eq!(ExecutionEvent::from(&sideless).get_crosscode(), "8:0:ORD-1");
}

/// A book's key is the ticker where one is stated, else the category.
#[test]
fn book_crosscode_is_the_isin_else_the_ticker_else_the_default() {
    crate::install::installed();
    // The ISIN keys the book wherever one is held, whatever its rank, and
    // the key is borrowed from the identifier it is.
    let mut listed = order(1);
    listed.set_ticker(Some(SmolStr::new("ACME")), true);
    listed
        .set_securityids(securityids(&[("ISIN", "US0378331005")]), true)
        .unwrap();
    assert_eq!(listed.book_crosscode(), "US0378331005");
    assert!(std::ptr::eq(
        listed.book_crosscode(),
        listed.get_isincode().unwrap()
    ));
    let mut masked = order(1);
    masked.set_ticker(Some(SmolStr::new("ACME")), true);
    masked
        .set_securityids(securityids(&[("ISIN", "XX0000000001")]), true)
        .unwrap();
    assert_eq!(masked.book_crosscode(), "XX0000000001");
    // Else the ticker, where it states a non-empty one, borrowed.
    let mut ticker = order(1);
    ticker.set_ticker(Some(SmolStr::new("ACME")), true);
    assert_eq!(ticker.book_crosscode(), "ACME");
    assert!(std::ptr::eq(
        ticker.book_crosscode(),
        ticker.get_ticker().unwrap()
    ));
    // Else the number that states none: the market and the classification
    // key nothing.
    let blank = order(2);
    assert_eq!(blank.book_crosscode(), Isin::NONE);
    assert_eq!(blank.book_crosscode(), "XX0000000000");
    let mut classified = order(3);
    classified.set_miccode(Some(Mic::new("XPAR").unwrap()), true);
    classified.set_cficode(Some(Cfi::new("ESVUFR").unwrap()), true);
    classified.set_ticker(Some(SmolStr::new("")), true);
    assert_eq!(classified.book_crosscode(), Isin::NONE);
}

/// Without `overwrite` a setter fills only a fact the element states
/// nothing under - `None`, `UKNW`, a currency or unit of none, an empty map -
/// and with it states the value, `None` clearing; an equal value changes
/// nothing either way.
#[test]
fn a_setter_fills_an_unstated_fact_and_overwrites_only_when_told() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_price(Some(dec("10")), false);
    assert_eq!(order.get_price(), Some(dec("10")));
    order.set_price(Some(dec("11")), false);
    assert_eq!(order.get_price(), Some(dec("10")), "a stated price stands");
    order.set_price(Some(dec("11")), true);
    assert_eq!(order.get_price(), Some(dec("11")));
    order.set_price(None, false);
    assert_eq!(
        order.get_price(),
        Some(dec("11")),
        "a fill of none is no change"
    );
    order.set_price(None, true);
    assert_eq!(order.get_price(), None, "an overwrite of none clears");

    order.set_side(Side::Buy, false);
    order.set_side(Side::Sell, false);
    assert_eq!(order.get_side(), Side::Buy);
    order.set_currency(Ccy::new("EUR").unwrap(), false);
    order.set_currency(Ccy::new("USD").unwrap(), false);
    assert_eq!(order.get_currency().as_str(), "EUR");
    order.set_marketdatatype(yggdryl_market::MarketDataType::OrdLimit, false);
    order.set_marketdatatype(yggdryl_market::MarketDataType::OrdMarket, false);
    assert_eq!(
        order.get_marketdatatype(),
        yggdryl_market::MarketDataType::OrdLimit
    );
    order.set_marketdatatype(yggdryl_market::MarketDataType::OrdMarket, true);
    assert_eq!(
        order.get_marketdatatype(),
        yggdryl_market::MarketDataType::OrdMarket
    );
    order.set_timeinforce(TimeInForce::from_spelling("DAY"), false);
    order.set_timeinforce(TimeInForce::from_spelling("IOC"), false);
    assert_eq!(
        order.get_timeinforce().map(|held| held.as_str()),
        Some("DAY")
    );

    // A map fills only the keys it lacks without `overwrite`, and is
    // replaced with it.
    let mut first = yggdryl_market::graph::Metadata::new();
    first.insert("desk".into(), "A".into());
    order.set_metadata(Some(first), false);
    let mut second = yggdryl_market::graph::Metadata::new();
    second.insert("desk".into(), "B".into());
    second.insert("book".into(), "X".into());
    order.set_metadata(Some(second.clone()), false);
    assert_eq!(order.get_metadata()["desk"], "A");
    assert_eq!(order.get_metadata()["book"], "X");
    order.set_metadata(Some(second), true);
    assert_eq!(order.get_metadata()["desk"], "B");
}

/// A buyer's price and quantity are its bid and a seller's its ask, in its
/// currency; the price moving moves the bid it quoted, a bid stated apart
/// from the price stands, and a side left stops quoting.
#[test]
fn the_side_quotes_the_price_and_the_quote_follows_it() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_currency(Ccy::new("USD").unwrap(), true);
    order.set_price(Some(dec("100")), true);
    order.set_quantity(Some(dec("5")), true);
    assert_eq!(
        (order.get_bidpx(), order.get_askpx()),
        (None, None),
        "no side, no quote"
    );

    order.set_side(Side::Buy, false);
    assert_eq!(order.get_bidpx(), Some(dec("100")));
    assert_eq!(order.get_bidqty(), Some(dec("5")));
    assert_eq!(order.get_bidccy().map(Ccy::as_str), Some("USD"));
    assert_eq!(order.get_askpx(), None);

    order.set_price(Some(dec("101")), true);
    order.set_quantity(Some(dec("7")), true);
    assert_eq!(
        (order.get_bidpx(), order.get_bidqty()),
        (Some(dec("101")), Some(dec("7")))
    );
    order.set_currency(Ccy::new("EUR").unwrap(), true);
    assert_eq!(order.get_bidccy().map(Ccy::as_str), Some("EUR"));

    // A bid stated apart from the price stands when the price moves.
    order.set_bidpx(Some(dec("99")), true);
    assert_eq!(
        order.get_price(),
        Some(dec("101")),
        "a stated price is not the bid's"
    );
    order.set_price(Some(dec("102")), true);
    assert_eq!(order.get_bidpx(), Some(dec("99")));

    // Selling instead withdraws what the buy quoted and asks the price.
    let mut order = OrderEvent::at(1);
    order.set_price(Some(dec("100")), true);
    order.set_quantity(Some(dec("5")), true);
    order.set_side(Side::Buy, true);
    order.set_side(Side::Sell, true);
    assert_eq!((order.get_bidpx(), order.get_bidqty()), (None, None));
    assert_eq!(
        (order.get_askpx(), order.get_askqty()),
        (Some(dec("100")), Some(dec("5")))
    );
    // A side taking neither lane quotes nothing.
    order.set_side(Side::Cross, true);
    assert_eq!((order.get_askpx(), order.get_bidpx()), (None, None));
}

/// What a quote states fills back an element's own price and quantity
/// where it states none - never over one it states.
#[test]
fn a_quote_fills_the_price_and_quantity_it_quotes_back() {
    crate::install::installed();
    let mut quote = OrderEvent::at(1);
    quote.set_side(Side::Sell, true);
    quote.set_askpx(Some(dec("1.25")), true);
    quote.set_askqty(Some(dec("1000")), true);
    assert_eq!(quote.get_price(), Some(dec("1.25")));
    assert_eq!(quote.get_quantity(), Some(dec("1000")));
    // A bid on a seller is no price of its own.
    quote.set_bidpx(Some(dec("1.20")), true);
    assert_eq!(quote.get_price(), Some(dec("1.25")));

    // Stated before the side, the ask waits for the side to read it.
    let mut quote = OrderEvent::at(1);
    quote.set_askpx(Some(dec("2")), true);
    assert_eq!(quote.get_price(), None);
    quote.set_side(Side::Sell, true);
    assert_eq!(quote.get_price(), Some(dec("2")));
}

/// An order takes one side: leaving it withdraws what that side quoted of
/// its price and quantity, the side it takes quotes them, and its cross
/// code moves under it; a leg it states apart from its price stands.
#[test]
fn an_order_leaving_a_side_withdraws_that_sides_quote() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_crosscode("ORD-1".to_owned());
    order.set_price(Some(dec("100")), true);
    order.set_quantity(Some(dec("5")), true);
    order.set_side(Side::Buy, true);
    assert_eq!(
        (order.get_bidpx(), order.get_bidqty()),
        (Some(dec("100")), Some(dec("5")))
    );
    order.set_side(Side::Sell, true);
    assert_eq!((order.get_bidpx(), order.get_bidqty()), (None, None));
    assert_eq!(
        (order.get_askpx(), order.get_askqty()),
        (Some(dec("100")), Some(dec("5")))
    );
    assert_eq!(order.get_crosscode(), "10:2:ORD-1");

    order.set_bidpx(Some(dec("98")), true);
    order.set_side(Side::Buy, true);
    assert_eq!((order.get_askpx(), order.get_askqty()), (None, None));
    assert_eq!(
        (order.get_bidpx(), order.get_bidqty()),
        (Some(dec("98")), Some(dec("5"))),
        "a bid stated apart from the price stands"
    );
    assert_eq!(order.get_price(), Some(dec("100")));
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");
}

/// A quote holds its bid and its ask in one element and its side is a tag:
/// moving the tag moves the price and quantity it reads from the leg the old
/// tag took to the leg the new one takes - none where it takes neither - and
/// never withdraws a leg; no midpoint is ever invented. Facts that do not
/// contradict land alike in whatever order they are stated.
#[test]
fn a_two_sided_quote_keeps_both_legs_when_its_side_tag_moves() {
    crate::install::installed();
    let mut quote = QuoteEvent::at(1);
    quote.set_crosscode("Q-1".to_owned());
    quote.set_bidpx(Some(dec("99")), true);
    quote.set_bidqty(Some(dec("10")), true);
    quote.set_askpx(Some(dec("101")), true);
    quote.set_askqty(Some(dec("20")), true);
    let legs = |quote: &QuoteEvent| {
        (
            quote.get_bidpx(),
            quote.get_bidqty(),
            quote.get_askpx(),
            quote.get_askqty(),
        )
    };
    let both = (
        Some(dec("99")),
        Some(dec("10")),
        Some(dec("101")),
        Some(dec("20")),
    );
    assert_eq!(
        (quote.get_price(), quote.get_quantity()),
        (None, None),
        "a two-sided quote states no price of its own"
    );

    quote.set_side(Side::Buy, true);
    assert_eq!(
        (quote.get_price(), quote.get_quantity()),
        (Some(dec("99")), Some(dec("10")))
    );
    quote.set_side(Side::Sell, true);
    assert_eq!(legs(&quote), both, "the bid stays");
    assert_eq!(
        (quote.get_price(), quote.get_quantity()),
        (Some(dec("101")), Some(dec("20")))
    );
    assert_eq!(quote.get_crosscode(), "14:0:Q-1", "the tag moves no prefix");
    quote.set_side(Side::Unknown, true);
    assert_eq!(legs(&quote), both);
    assert_eq!((quote.get_price(), quote.get_quantity()), (None, None));

    // {price 100, side SELL, bid 99}, stated in each of the six orders.
    type Statement = fn(&mut QuoteEvent);
    let statements: [Statement; 3] = [
        |quote| quote.set_price(Some(dec("100")), true),
        |quote| quote.set_side(Side::Sell, true),
        |quote| quote.set_bidpx(Some(dec("99")), true),
    ];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut quote = QuoteEvent::at(1);
        for index in order {
            statements[index](&mut quote);
        }
        assert_eq!(
            (quote.get_price(), quote.get_askpx(), quote.get_bidpx()),
            (Some(dec("100")), Some(dec("100")), Some(dec("99"))),
            "stated in the order {order:?}"
        );
    }
}

/// A quote statement stating no side tag takes each leg of the quote it
/// follows that it states nothing of - the price, the quantity and the
/// currency of that leg as one - so updating one leg keeps the other; a leg
/// it states, a zero quantity withdrawing it included, is its own.
#[test]
fn a_quote_follower_takes_each_leg_it_states_nothing_of() {
    crate::install::installed();
    let mut first = QuoteEvent::at(1);
    first.set_crosscode("Q-1".to_owned());
    first.set_currency(Ccy::new("USD").unwrap(), true);
    first.set_bidpx(Some(dec("99")), true);
    first.set_bidqty(Some(dec("10")), true);
    first.set_bidccy(Some(Ccy::new("EUR").unwrap()), true);
    first.set_askpx(Some(dec("101")), true);
    first.set_askqty(Some(dec("20")), true);
    first.finalize();

    let mut second = QuoteEvent::at(2);
    second.set_crosscode("Q-1".to_owned());
    second.set_askpx(Some(dec("102")), true);
    second.set_askqty(Some(dec("5")), true);
    second.finalize();
    let second = second.with_previous(&first).expect("a later statement");
    assert_eq!(
        second.get_side(),
        Side::Both,
        "holding the bid it carried and the ask it states, it holds both sides"
    );
    assert_eq!(
        (second.get_bidpx(), second.get_bidqty()),
        (Some(dec("99")), Some(dec("10"))),
        "the bid it said nothing of is the chain's"
    );
    assert_eq!(
        second.get_bidccy().map(Ccy::as_str),
        Some("EUR"),
        "the leg's own currency travels with it"
    );
    assert_eq!(
        (second.get_askpx(), second.get_askqty()),
        (Some(dec("102")), Some(dec("5")))
    );
    assert_eq!(second.get_askccy().map(Ccy::as_str), Some("USD"));
    assert_eq!(
        (second.get_price(), second.get_quantity()),
        (None, None),
        "no price is carried"
    );

    let mut third = QuoteEvent::at(3);
    third.set_crosscode("Q-1".to_owned());
    third.set_bidqty(Some(dec("0")), true);
    third.finalize();
    let third = third.with_previous(&second).expect("a later statement");
    assert_eq!(
        (third.get_bidpx(), third.get_bidqty()),
        (None, Some(dec("0"))),
        "a bid withdrawn is stated, never carried over"
    );
    assert_eq!(
        (third.get_askpx(), third.get_askqty()),
        (Some(dec("102")), Some(dec("5")))
    );
}

/// A quote tagging no side that quotes both its legs holds both sides: it
/// states `BOTH` once finalized, quoting nothing onto one leg and storing
/// its cross code under side `0`. A quote stating one leg, or tagging a
/// side, is left as it is.
#[test]
fn a_quote_tagging_no_side_that_quotes_both_legs_states_both_sides() {
    crate::install::installed();
    let two_sided = || {
        let mut quote = QuoteEvent::at(1);
        quote.set_crosscode("Q-1".to_owned());
        quote.set_bidpx(Some(dec("99")), true);
        quote.set_bidqty(Some(dec("10")), true);
        quote.set_askpx(Some(dec("101")), true);
        quote.set_askqty(Some(dec("20")), true);
        quote
    };
    let legs = |quote: &QuoteEvent| {
        (
            quote.get_bidpx(),
            quote.get_bidqty(),
            quote.get_askpx(),
            quote.get_askqty(),
        )
    };
    let both = (
        Some(dec("99")),
        Some(dec("10")),
        Some(dec("101")),
        Some(dec("20")),
    );

    let mut quote = two_sided();
    assert_eq!(quote.get_side(), Side::Unknown, "until it is finalized");
    quote.finalize();
    assert_eq!(quote.get_side(), Side::Both);
    assert_eq!(quote.get_crosscode(), "14:0:Q-1");
    assert_eq!(legs(&quote), both);
    assert_eq!((quote.get_price(), quote.get_quantity()), (None, None));

    // Stated as none again, the next finalize states it once more.
    quote.set_side(Side::Unknown, true);
    assert_eq!(quote.get_side(), Side::Unknown);
    quote.finalize();
    assert_eq!(quote.get_side(), Side::Both);
    assert_eq!(legs(&quote), both);

    // A tag it states stands.
    let mut tagged = two_sided();
    tagged.set_side(Side::Buy, true);
    tagged.finalize();
    assert_eq!(tagged.get_side(), Side::Buy);
    assert_eq!(tagged.get_price(), Some(dec("99")));

    // A quote stating one leg tags none.
    let mut bid = QuoteEvent::at(1);
    bid.set_crosscode("Q-2".to_owned());
    bid.set_bidpx(Some(dec("99")), true);
    bid.set_bidqty(Some(dec("10")), true);
    bid.finalize();
    assert_eq!(bid.get_side(), Side::Unknown);
    assert_eq!(bid.get_crosscode(), "14:0:Q-2");
}

/// A sided element takes its chain's side where it states none, but never
/// both sides at once: an order continuing an element that holds both - a
/// two-sided quote's book entry - takes no side from it.
#[test]
fn a_sided_follower_takes_no_side_from_a_predecessor_holding_both() {
    crate::install::installed();
    let mut first = OrderEvent::at(1);
    first.set_crosscode("ORD-1".to_owned());
    first.set_ticker(Some(SmolStr::new("AAPL")), true);
    first.set_side(Side::Both, true);
    first.finalize();
    assert_eq!(first.get_side(), Side::Both);
    let mut next = OrderEvent::at(2);
    next.set_crosscode("ORD-1".to_owned());
    next.finalize();
    let next = next.with_previous(&first).expect("a later statement");
    assert_eq!(next.get_ticker(), Some("AAPL"), "the chain's facts carry");
    assert_eq!(next.get_side(), Side::Unknown);
    assert_eq!(next.get_crosscode(), "10:0:ORD-1");
}

/// A quote statement tagging a side over a quote holding both legs - a
/// fill reported on the leg that traded - updates the leg its tag takes and
/// keeps the other, the price of its own leg where it states a quantity
/// alone; over a tagged entry quoting one leg - a book level - it restates
/// that entry whole, moving to the other side included.
#[test]
fn a_tagged_quote_restates_a_level_whole_and_updates_a_quote_leg_by_leg() {
    crate::install::installed();
    let mut first = QuoteEvent::at(1);
    first.set_crosscode("Q-1".to_owned());
    first.set_ticker(Some(SmolStr::new("AAPL")), true);
    first.set_bidpx(Some(dec("99")), true);
    first.set_bidqty(Some(dec("10")), true);
    first.set_askpx(Some(dec("101")), true);
    first.set_askqty(Some(dec("20")), true);
    first.finalize();

    let mut level = QuoteEvent::at(2);
    level.set_crosscode("Q-1".to_owned());
    level.set_side(Side::Buy, true);
    level.set_price(Some(dec("98")), true);
    level.set_quantity(Some(dec("7")), true);
    level.finalize();
    let level = level.with_previous(&first).expect("a later statement");
    assert_eq!(level.get_side(), Side::Buy);
    assert_eq!(
        (level.get_bidpx(), level.get_bidqty()),
        (Some(dec("98")), Some(dec("7")))
    );
    assert_eq!(
        (level.get_askpx(), level.get_askqty()),
        (Some(dec("101")), Some(dec("20"))),
        "the leg its tag does not take is the quote's"
    );

    // A fill on the bid stating what is left of it alone keeps its price.
    let mut fill = QuoteEvent::at(3);
    fill.set_crosscode("Q-1".to_owned());
    fill.set_side(Side::Buy, true);
    fill.set_quantity(Some(dec("6")), true);
    fill.finalize();
    let fill = fill.with_previous(&first).expect("a later statement");
    assert_eq!(
        (fill.get_bidpx(), fill.get_bidqty()),
        (Some(dec("99")), Some(dec("6")))
    );
    assert_eq!(fill.get_price(), Some(dec("99")));
    assert_eq!(
        (fill.get_askpx(), fill.get_askqty()),
        (Some(dec("101")), Some(dec("20")))
    );

    // A tagged level, then a statement of it on the other side: whole.
    let mut bid = QuoteEvent::at(1);
    bid.set_crosscode("E-1".to_owned());
    bid.set_side(Side::Buy, true);
    bid.set_price(Some(dec("99")), true);
    bid.set_quantity(Some(dec("5")), true);
    bid.finalize();
    let mut moved = QuoteEvent::at(2);
    moved.set_crosscode("E-1".to_owned());
    moved.set_side(Side::Sell, true);
    moved.set_price(Some(dec("101")), true);
    moved.set_quantity(Some(dec("6")), true);
    moved.finalize();
    let moved = moved.with_previous(&bid).expect("a later statement");
    assert_eq!(
        (moved.get_bidpx(), moved.get_bidqty()),
        (None, None),
        "a level restated takes no leg of the one it was"
    );
    assert_eq!(
        (moved.get_askpx(), moved.get_askqty()),
        (Some(dec("101")), Some(dec("6")))
    );

    // No leg crosses to another instrument.
    let mut other = QuoteEvent::at(2);
    other.set_crosscode("Q-1".to_owned());
    other.set_ticker(Some(SmolStr::new("MSFT")), true);
    other.set_bidpx(Some(dec("199")), true);
    other.set_bidqty(Some(dec("5")), true);
    other.finalize();
    let other = other.with_previous(&first).expect("a later statement");
    assert_eq!((other.get_askpx(), other.get_askqty()), (None, None));
}

/// Two statements of one quote tagging two sides merge leg by leg, the same
/// whichever is merged into the other: the leading statement's tag, price
/// and quantity over both legs as each statement quoted them.
#[test]
fn two_statements_of_a_quote_tagging_two_sides_merge_leg_by_leg() {
    crate::install::installed();
    let statement = |side: Side, price: &str, quantity: &str, recorded: i64| {
        let mut quote = QuoteEvent::at(1);
        quote.set_crosscode("Q-1".to_owned());
        quote.set_side(side, true);
        quote.set_price(Some(dec(price)), true);
        quote.set_quantity(Some(dec(quantity)), true);
        quote.set_sendunix(Some(recorded));
        quote.finalize();
        quote
    };
    let bid = statement(Side::Buy, "99", "10", 1);
    let mut ask = statement(Side::Sell, "101", "5", 2);
    ask.set_uuid(bid.get_uuid());
    for merged in [
        bid.clone().merge_with(&ask).expect("a merge"),
        ask.clone().merge_with(&bid).expect("a merge"),
    ] {
        assert_eq!(merged.get_side(), Side::Sell);
        assert_eq!(
            (merged.get_bidpx(), merged.get_bidqty()),
            (Some(dec("99")), Some(dec("10")))
        );
        assert_eq!(
            (merged.get_askpx(), merged.get_askqty()),
            (Some(dec("101")), Some(dec("5")))
        );
        assert_eq!(
            (merged.get_price(), merged.get_quantity()),
            (Some(dec("101")), Some(dec("5")))
        );
    }
}

/// A side is part of a sided element's identity, so an order statement
/// stating none takes its chain's; an unsided element's side is its own
/// tag, so a quote statement stating none states none.
#[test]
fn an_unsided_follower_takes_no_side_from_its_chain() {
    crate::install::installed();
    let mut offer = QuoteEvent::at(1);
    offer.set_crosscode("Q-1".to_owned());
    offer.set_side(Side::Sell, true);
    offer.set_price(Some(dec("101")), true);
    offer.set_quantity(Some(dec("20")), true);
    offer.finalize();
    let mut ack = QuoteEvent::at(2);
    ack.set_crosscode("Q-1".to_owned());
    ack.finalize();
    let ack = ack.with_previous(&offer).expect("a later statement");
    assert_eq!(ack.get_side(), Side::Unknown);
    assert_eq!(ack.get_crosscode(), "14:0:Q-1");
    assert_eq!(
        (ack.get_askpx(), ack.get_askqty()),
        (Some(dec("101")), Some(dec("20"))),
        "the quote it acknowledges is the chain's"
    );

    let mut buy = OrderEvent::at(1);
    buy.set_crosscode("ORD-1".to_owned());
    buy.set_side(Side::Buy, true);
    buy.finalize();
    let mut status = OrderEvent::at(2);
    status.set_crosscode("ORD-1".to_owned());
    status.finalize();
    let status = status.with_previous(&buy).expect("a later statement");
    assert_eq!(status.get_side(), Side::Buy);
    assert_eq!(status.get_crosscode(), "10:1:ORD-1");
}

/// An iceberg's hidden part is the quantity past the shown one, kept in
/// step with both; a hidden part stated fills the shown one, or the
/// quantity the two make together.
#[test]
fn an_iceberg_keeps_its_hidden_part_in_step() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_quantity(Some(dec("10")), true);
    order.set_displayqty(Some(dec("4")), true);
    assert_eq!(order.get_hiddenqty(), Some(dec("6")));
    order.set_quantity(Some(dec("12")), true);
    assert_eq!(order.get_hiddenqty(), Some(dec("8")));
    order.set_displayqty(Some(dec("12")), true);
    assert_eq!(order.get_hiddenqty(), None, "nothing kept back");

    let mut shown = OrderEvent::at(1);
    shown.set_quantity(Some(dec("10")), true);
    shown.set_hiddenqty(Some(dec("7")), true);
    assert_eq!(shown.get_displayqty(), Some(dec("3")));

    let mut total = OrderEvent::at(1);
    total.set_side(Side::Buy, true);
    total.set_displayqty(Some(dec("2")), true);
    total.set_hiddenqty(Some(dec("8")), true);
    assert_eq!(total.get_quantity(), Some(dec("10")));
    assert_eq!(
        total.get_bidqty(),
        Some(dec("10")),
        "the quantity it made quotes too"
    );
}

/// `LastPx` is the spot rate plus the forward points: two stated fill the
/// third, and three stated are left as they are.
#[test]
fn an_fx_triple_fills_the_part_it_leaves_out() {
    crate::install::installed();
    let mut fill = OrderEvent::at(1);
    fill.set_lastpx(Some(dec("1.0862")), true);
    fill.set_forwardpoints(Some(dec("0.0012")), true);
    assert_eq!(fill.get_spotrate(), Some(dec("1.085")));

    let mut parts = OrderEvent::at(1);
    parts.set_spotrate(Some(dec("1.1")), true);
    parts.set_forwardpoints(Some(dec("0.01")), true);
    assert_eq!(parts.get_lastpx(), Some(dec("1.11")));
}

/// A sided kind stores its cross code under its side: setting the side
/// moves the prefix, and an unsided kind states side 0 whatever it takes.
#[test]
fn the_side_moves_a_sided_cross_code() {
    crate::install::installed();
    let mut order = OrderEvent::at(1);
    order.set_crosscode("ORD-1".to_owned());
    assert_eq!(order.get_crosscode(), "10:0:ORD-1");
    order.set_side(Side::Buy, false);
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");
    order.set_side(Side::Sell, false);
    assert_eq!(order.get_crosscode(), "10:1:ORD-1", "a stated side stands");
    order.set_side(Side::Sell, true);
    assert_eq!(order.get_crosscode(), "10:2:ORD-1");
    order.finalize();
    assert_eq!(order.get_crosscode(), "10:2:ORD-1");

    let mut book = BookEvent::new(1, "AAPL");
    book.set_side(Side::Buy, true);
    assert_eq!(book.get_crosscode(), "3:0:AAPL");
}

/// What an order ordered, traded and has left fill one another against its
/// state - FIX's `LeavesQty = OrderQty - CumQty` while it works, nothing
/// left once it is done - and what is left is the quantity it is about.
#[test]
fn an_orders_quantities_fill_one_another_by_its_state() {
    crate::install::installed();
    use yggdryl::State;

    // Working: any two fill the third, and the quantity is what is left.
    let mut working = OrderEvent::at(1);
    working.set_state(State::PartiallyFilled);
    working.set_ordqty(Some(dec("100")), true);
    working.set_cumqty(Some(dec("40")), true);
    assert_eq!(working.get_leavesqty(), Some(dec("60")));
    assert_eq!(working.get_quantity(), Some(dec("60")));
    // Another fill moves what is left, and the quantity with it.
    working.set_cumqty(Some(dec("70")), true);
    working.set_leavesqty(Some(dec("30")), true);
    assert_eq!(working.get_quantity(), Some(dec("30")));

    let mut ordered = OrderEvent::at(1);
    ordered.set_state(State::PartiallyFilled);
    ordered.set_cumqty(Some(dec("25")), true);
    ordered.set_leavesqty(Some(dec("75")), true);
    assert_eq!(ordered.get_ordqty(), Some(dec("100")));
    // What was ordered is never the quantity: what is left is.
    assert_eq!(ordered.get_quantity(), Some(dec("75")));

    // Fresh - asked for or new - nothing traded, so all of it is left.
    let mut fresh = OrderEvent::at(1);
    fresh.set_state(State::New);
    fresh.set_ordqty(Some(dec("10")), true);
    assert_eq!(fresh.get_leavesqty(), Some(dec("10")));
    assert_eq!(fresh.get_cumqty(), None, "nothing traded is stated as none");
    // Fresh and stating less left than it ordered, the rest traded -
    // whichever is stated first, and whether the state comes first or last.
    for state_first in [true, false] {
        for ordered_first in [true, false] {
            let mut fresh = OrderEvent::at(1);
            if state_first {
                fresh.set_state(State::New);
            }
            if ordered_first {
                fresh.set_ordqty(Some(dec("100")), true);
                fresh.set_leavesqty(Some(dec("60")), true);
            } else {
                fresh.set_leavesqty(Some(dec("60")), true);
                fresh.set_ordqty(Some(dec("100")), true);
            }
            if !state_first {
                fresh.set_state(State::New);
            }
            assert_eq!(
                (
                    fresh.get_ordqty(),
                    fresh.get_cumqty(),
                    fresh.get_leavesqty()
                ),
                (Some(dec("100")), Some(dec("40")), Some(dec("60"))),
                "state first {state_first}, ordered first {ordered_first}"
            );
        }
    }
    // What is left, with nothing else stated, is all it ordered.
    let mut left = OrderEvent::at(1);
    left.set_state(State::PendingNew);
    left.set_leavesqty(Some(dec("10")), true);
    assert_eq!(left.get_ordqty(), Some(dec("10")));

    // Filled: nothing left, all of it traded.
    let mut filled = OrderEvent::at(1);
    filled.set_ordqty(Some(dec("10")), true);
    filled.set_state(State::Filled);
    assert_eq!(
        (filled.get_leavesqty(), filled.get_cumqty()),
        (Some(dec("0")), Some(dec("10")))
    );

    // Canceled: nothing left, and what was not traded was canceled.
    let mut canceled = OrderEvent::at(1);
    canceled.set_ordqty(Some(dec("10")), true);
    canceled.set_cumqty(Some(dec("4")), true);
    canceled.set_state(State::Canceled);
    assert_eq!(canceled.get_leavesqty(), Some(dec("0")));
    assert_eq!(canceled.get_cxlqty(), Some(dec("6")));
    // Rejected: nothing left, and nothing canceled by anyone.
    let mut rejected = OrderEvent::at(1);
    rejected.set_ordqty(Some(dec("10")), true);
    rejected.set_state(State::Rejected);
    assert_eq!(
        (rejected.get_leavesqty(), rejected.get_cxlqty()),
        (Some(dec("0")), None)
    );

    // An execution stating its last fill alone states nothing left.
    let mut fill = ExecutionEvent::at(1);
    fill.set_state(State::Filled);
    fill.set_lastqty(Some(dec("15")), true);
    assert_eq!(fill.get_leavesqty(), None);

    // One fill is its own average.
    let mut first = OrderEvent::at(1);
    first.set_lastpx(Some(dec("10.5")), true);
    first.set_lastqty(Some(dec("3")), true);
    first.set_cumqty(Some(dec("3")), true);
    assert_eq!(first.get_avgpx(), Some(dec("10.5")));
}

/// An iceberg's hidden part carries along its chain where the next
/// statement states none: less what traded since, never below nothing.
#[test]
fn a_hidden_part_carries_along_its_chain_less_what_traded() {
    crate::install::installed();
    let mut first = OrderEvent::at(1);
    first.set_crosscode("ICE".to_owned());
    first.set_quantity(Some(dec("100")), true);
    first.set_displayqty(Some(dec("10")), true);
    first.set_cumqty(Some(dec("0")), true);
    first.finalize();
    assert_eq!(first.get_hiddenqty(), Some(dec("90")));

    let mut next = OrderEvent::at(2);
    next.set_crosscode("ICE".to_owned());
    next.set_cumqty(Some(dec("25")), true);
    next.finalize();
    let next = next.following_operation(&first).expect("it follows");
    assert_eq!(next.get_hiddenqty(), Some(dec("65")));

    // Stated, it is its own word.
    let mut stated = OrderEvent::at(3);
    stated.set_crosscode("ICE".to_owned());
    stated.set_hiddenqty(Some(dec("50")), true);
    stated.finalize();
    let stated = stated.following_operation(&next).expect("it follows");
    assert_eq!(stated.get_hiddenqty(), Some(dec("50")));
}

/// A currency pair states its quantity in the currency dealt: an element
/// trading one takes as its unit its stated currency where that is a leg of
/// the pair, else the pair's base - and never over a unit it states.
#[test]
fn a_currency_pair_fills_the_unit_its_quantity_is_dealt_in() {
    crate::install::installed();
    use yggdryl::Unit;
    use yggdryl::graph::Element as _;

    let pair = |currency: Option<&str>| {
        let mut order = OrderEvent::at(1);
        order
            .insert_securityid(Identifier::new(IdKey::base(IdType::Forex), "EURUSD").unwrap())
            .unwrap();
        if let Some(currency) = currency {
            order.set_currency(Ccy::new(currency).unwrap(), true);
        }
        order.finalize();
        order
    };
    assert_eq!(pair(None).get_unit().as_str(), "EUR");
    assert_eq!(pair(Some("USD")).get_unit().as_str(), "USD");
    assert_eq!(pair(Some("JPY")).get_unit().as_str(), "EUR");
    let mut stated = pair(Some("EUR"));
    stated.set_unit(Unit::new("Lots").unwrap(), true);
    stated.finalize();
    assert_eq!(stated.get_unit().as_str(), "Lots");
}

/// A ticker of an identifier's own shape names it: the identifier derived,
/// an instrument key's market and currency filled, a CFI code taken as the
/// classification - each only where the element states none.
#[test]
fn a_ticker_of_an_identifiers_shape_names_it() {
    crate::install::installed();
    use yggdryl::graph::Element as _;

    let ticker = |symbol: &str| {
        let mut order = OrderEvent::at(1);
        order.set_ticker(Some(symbol.into()), true);
        order.finalize();
        order
    };
    let apple = ticker("US0378331005");
    assert_eq!(
        apple.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    // What the ISIN embeds follows it.
    assert_eq!(
        apple.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    let holcim = ticker("CH0012214059_XSWX_CHF");
    assert_eq!(
        holcim.get_securityids().get(&IdType::Isin),
        Some("CH0012214059")
    );
    assert_eq!(holcim.get_miccode().map(Mic::as_str), Some("XSWX"));
    assert_eq!(holcim.get_currency().as_str(), "CHF");
    assert_eq!(
        ticker("BBG000BLNQ16").get_securityids().get(&IdType::Figi),
        Some("BBG000BLNQ16")
    );
    assert_eq!(
        ticker("ESVUFR").get_cficode().map(Cfi::as_str),
        Some("ESVUFR")
    );
    // A ticker that is only an identifier's length is a ticker.
    let plain = ticker("US0378331006");
    assert!(plain.get_securityids().is_empty());
    // A stated identifier is never replaced by the ticker's.
    let mut stated = OrderEvent::at(1);
    stated
        .insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059").unwrap())
        .unwrap();
    stated.set_ticker(Some("US0378331005".into()), true);
    stated.finalize();
    assert_eq!(
        stated.get_securityids().get(&IdType::Isin),
        Some("CH0012214059")
    );
}

/// An iceberg is three quantities - the whole, the part it shows and the
/// part it keeps back, the whole the two together - and any two of them
/// stated, in either order, fill the third.
#[test]
fn an_iceberg_completes_from_any_two_of_its_three_quantities() {
    crate::install::installed();
    let iceberg = |order: &OrderEvent| {
        (
            order.get_quantity(),
            order.get_displayqty(),
            order.get_hiddenqty(),
        )
    };
    let whole = (Some(dec("10")), Some(dec("2")), Some(dec("8")));
    type Statement = fn(&mut OrderEvent);
    let quantity: Statement = |order| order.set_quantity(Some(dec("10")), false);
    let display: Statement = |order| order.set_displayqty(Some(dec("2")), false);
    let hidden: Statement = |order| order.set_hiddenqty(Some(dec("8")), false);
    for (first, second) in [
        (quantity, display),
        (display, quantity),
        (quantity, hidden),
        (hidden, quantity),
        (display, hidden),
        (hidden, display),
    ] {
        let mut order = OrderEvent::at(1);
        first(&mut order);
        second(&mut order);
        assert_eq!(iceberg(&order), whole);
    }

    // The whole it made quotes, as a stated one does.
    let mut buy = OrderEvent::at(1);
    buy.set_side(Side::Buy, true);
    buy.set_hiddenqty(Some(dec("8")), true);
    buy.set_displayqty(Some(dec("2")), true);
    assert_eq!(buy.get_bidqty(), Some(dec("10")));
}

/// A fact a setter states is the setter's word: what the other quantities
/// imply never writes it back - a stated `leavesqty` stands over `ordqty`
/// less `cumqty`, and a fact stated as none stays none whatever the others
/// imply.
#[test]
fn a_stated_leavesqty_stands_over_what_the_other_quantities_imply() {
    crate::install::installed();
    use yggdryl::State;

    let mut order = OrderEvent::at(1);
    order.set_state(State::PartiallyFilled);
    order.set_ordqty(Some(dec("10")), true);
    order.set_cumqty(Some(dec("4")), true);
    assert_eq!(order.get_leavesqty(), Some(dec("6")));
    order.set_leavesqty(Some(dec("10")), true);
    assert_eq!(order.get_leavesqty(), Some(dec("10")), "the stated value");
    assert_eq!(order.get_quantity(), Some(dec("10")));

    // Filled from nothing, it is the setter's word too.
    let mut fresh = OrderEvent::at(1);
    fresh.set_state(State::PartiallyFilled);
    fresh.set_ordqty(Some(dec("10")), true);
    fresh.set_cumqty(Some(dec("4")), true);
    fresh.set_leavesqty(None, true);
    assert_eq!(fresh.get_leavesqty(), None, "a leavesqty stated as none");
    fresh.set_leavesqty(Some(dec("10")), false);
    assert_eq!(fresh.get_leavesqty(), Some(dec("10")));

    // Cleared, a fact the others imply stays clear.
    let mut cleared = OrderEvent::at(1);
    cleared.set_state(State::PartiallyFilled);
    cleared.set_ordqty(Some(dec("10")), true);
    cleared.set_leavesqty(Some(dec("6")), true);
    assert_eq!(cleared.get_cumqty(), Some(dec("4")));
    cleared.set_cumqty(None, true);
    assert_eq!(cleared.get_cumqty(), None);
    let mut fx = OrderEvent::at(1);
    fx.set_spotrate(Some(dec("1.1")), true);
    fx.set_forwardpoints(Some(dec("0.01")), true);
    assert_eq!(fx.get_lastpx(), Some(dec("1.11")));
    fx.set_lastpx(None, true);
    assert_eq!(fx.get_lastpx(), None);
    let mut iceberg = OrderEvent::at(1);
    iceberg.set_quantity(Some(dec("10")), true);
    iceberg.set_hiddenqty(Some(dec("8")), true);
    assert_eq!(iceberg.get_displayqty(), Some(dec("2")));
    iceberg.set_displayqty(None, true);
    assert_eq!(iceberg.get_displayqty(), None);
}

/// An order that ended - filled, canceled, done for the day, expired,
/// rejected, removed, failed - has nothing left whatever else it states,
/// and what it ordered, traded and canceled fill one another: filled, all
/// it ordered traded; ended any other way, what it ordered is what traded
/// plus what was canceled.
#[test]
fn a_closed_order_leaves_nothing_whatever_it_states() {
    crate::install::installed();
    use yggdryl::State;

    for state in [
        State::Filled,
        State::Canceled,
        State::DoneForDay,
        State::Expired,
        State::Rejected,
        State::Removed,
        State::Failed,
        State::Calculated,
    ] {
        let mut bare = OrderEvent::at(1);
        bare.set_state(state);
        assert_eq!(
            (bare.get_leavesqty(), bare.get_quantity()),
            (Some(dec("0")), Some(dec("0"))),
            "{state:?} states nothing else"
        );
        let mut priced = OrderEvent::at(1);
        priced.set_price(Some(dec("10")), true);
        priced.set_lastqty(Some(dec("3")), true);
        priced.set_state(state);
        assert_eq!(priced.get_leavesqty(), Some(dec("0")), "{state:?}");
    }

    // Someone ended it: the rest of what it ordered was canceled.
    for state in [
        State::Canceled,
        State::Removed,
        State::DoneForDay,
        State::Expired,
    ] {
        let mut ended = OrderEvent::at(1);
        ended.set_ordqty(Some(dec("10")), true);
        ended.set_cumqty(Some(dec("4")), true);
        ended.set_state(state);
        assert_eq!(
            (ended.get_leavesqty(), ended.get_cxlqty()),
            (Some(dec("0")), Some(dec("6"))),
            "{state:?}"
        );
    }
    // It could not be done: nothing left, nothing canceled by anyone.
    let mut failed = OrderEvent::at(1);
    failed.set_ordqty(Some(dec("10")), true);
    failed.set_cumqty(Some(dec("4")), true);
    failed.set_state(State::Failed);
    assert_eq!(
        (failed.get_leavesqty(), failed.get_cxlqty()),
        (Some(dec("0")), None)
    );

    // Filled, what traded is what it ordered, either way round.
    let mut filled = OrderEvent::at(1);
    filled.set_state(State::Filled);
    filled.set_cumqty(Some(dec("10")), true);
    assert_eq!(filled.get_ordqty(), Some(dec("10")));
    // Canceled, any two of what it ordered, traded and canceled fill the
    // third.
    let mut canceled = OrderEvent::at(1);
    canceled.set_state(State::Canceled);
    canceled.set_cumqty(Some(dec("4")), true);
    canceled.set_cxlqty(Some(dec("6")), true);
    assert_eq!(canceled.get_ordqty(), Some(dec("10")));
    let mut canceled = OrderEvent::at(1);
    canceled.set_state(State::Canceled);
    canceled.set_ordqty(Some(dec("10")), true);
    canceled.set_cxlqty(Some(dec("6")), true);
    assert_eq!(canceled.get_cumqty(), Some(dec("4")));

    // A quote or any other element states nothing left of an order it
    // never stated.
    let mut quote = QuoteEvent::at(1);
    quote.set_state(State::Canceled);
    assert_eq!(quote.get_leavesqty(), None);
}

/// What an execution or a trade states as its quantity is its own - the
/// fill it reports, its `lastqty` (decision 27) - never what the order it
/// fills has left; a quantity stated stands over it.
#[test]
fn an_execution_quantity_is_never_its_orders_leaves() {
    crate::install::installed();
    use yggdryl::State;

    let mut fill = ExecutionEvent::at(1);
    fill.set_state(State::PartiallyFilled);
    fill.set_ordqty(Some(dec("100")), true);
    fill.set_cumqty(Some(dec("40")), true);
    assert_eq!(fill.get_leavesqty(), Some(dec("60")));
    assert_eq!(fill.get_quantity(), None, "no fill reported yet");
    fill.set_lastqty(Some(dec("40")), true);
    assert_eq!(fill.get_quantity(), Some(dec("40")), "what executed");
    assert_eq!(fill.get_bidqty(), None, "unsided");
    fill.set_quantity(Some(dec("50")), true);
    fill.set_leavesqty(Some(dec("50")), true);
    fill.set_lastqty(Some(dec("45")), true);
    assert_eq!(
        fill.get_quantity(),
        Some(dec("50")),
        "a stated quantity stands"
    );
    let mut sided = ExecutionEvent::at(1);
    sided.set_side(yggdryl_market::Side::Buy, true);
    sided.set_lastqty(Some(dec("30")), true);
    assert_eq!(
        (sided.get_quantity(), sided.get_bidqty()),
        (Some(dec("30")), Some(dec("30"))),
        "a sided execution quotes what executed on its side"
    );

    let mut ended = ExecutionEvent::at(1);
    ended.set_state(State::Filled);
    ended.set_leavesqty(Some(dec("0")), true);
    assert_eq!(ended.get_quantity(), None);
}

/// A detailed classification is never replaced by a coarser one: a code
/// saying nothing past its category and group states nothing, and a
/// detailed one stated over another describing the same instrument takes
/// what that one says where it says nothing.
#[test]
fn a_coarse_cfi_never_clears_a_detailed_one() {
    crate::install::installed();
    let cfi = |code: &str| Cfi::new(code).unwrap();
    let mut order = OrderEvent::at(1);
    order.set_cficode(Some(cfi("ESVUFR")), true);
    order.set_cficode(Some(cfi("ESXXXX")), true);
    assert_eq!(order.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    order.set_cficode(Some(cfi("ESXXXX")), false);
    assert_eq!(order.get_cficode().map(Cfi::as_str), Some("ESVUFR"));

    let mut refined = OrderEvent::at(1);
    refined.set_cficode(Some(cfi("ESXUFR")), true);
    refined.set_cficode(Some(cfi("ESVXXX")), true);
    assert_eq!(refined.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    // Another instrument's code is stated over it whole.
    refined.set_cficode(Some(cfi("ESNUFR")), true);
    assert_eq!(refined.get_cficode().map(Cfi::as_str), Some("ESNUFR"));
    // Without `overwrite` a stated code stands, and none clears it with.
    refined.set_cficode(Some(cfi("ESVUFR")), false);
    assert_eq!(refined.get_cficode().map(Cfi::as_str), Some("ESNUFR"));
    refined.set_cficode(None, true);
    assert_eq!(refined.get_cficode(), None);
}

/// An operation's follower stating none of them takes what its chain
/// filled, `cumqty` and `avgpx` beside `ordqty`, so an acknowledgement of a
/// cancel after a partial fill canceled the rest and no more.
#[test]
fn an_operation_follower_takes_the_cumulative_fill_of_its_chain() {
    crate::install::installed();
    use yggdryl::State;

    let mut partial = OrderEvent::at(1);
    partial.set_crosscode("ORD-1".to_owned());
    partial.set_state(State::PartiallyFilled);
    partial.set_ordqty(Some(dec("100")), true);
    partial.set_cumqty(Some(dec("40")), true);
    partial.set_avgpx(Some(dec("10.5")), true);
    partial.finalize();

    let mut canceled = OrderEvent::at(2);
    canceled.set_crosscode("ORD-1".to_owned());
    canceled.set_state(State::Canceled);
    canceled.finalize();
    let canceled = canceled.with_previous(&partial).expect("a later statement");
    assert_eq!(canceled.get_ordqty(), Some(dec("100")));
    assert_eq!(canceled.get_cumqty(), Some(dec("40")));
    assert_eq!(canceled.get_avgpx(), Some(dec("10.5")));
    assert_eq!(
        (canceled.get_leavesqty(), canceled.get_cxlqty()),
        (Some(dec("0")), Some(dec("60")))
    );
    // A follower stating its own keeps it.
    let mut filled = OrderEvent::at(3);
    filled.set_crosscode("ORD-1".to_owned());
    filled.set_state(State::Filled);
    filled.set_cumqty(Some(dec("100")), true);
    filled.finalize();
    let filled = filled.with_previous(&partial).expect("a later statement");
    assert_eq!(filled.get_cumqty(), Some(dec("100")));
    assert_eq!(filled.get_lastqty(), None, "no fill is invented");
    // Its own cumulative fill is another average than the chain's, which a
    // follower stating none is never given.
    assert_eq!(filled.get_avgpx(), None);
    let mut more = OrderEvent::at(3);
    more.set_crosscode("ORD-1".to_owned());
    more.set_state(State::PartiallyFilled);
    more.set_cumqty(Some(dec("70")), true);
    more.finalize();
    let more = more.with_previous(&partial).expect("a later statement");
    assert_eq!(more.get_cumqty(), Some(dec("70")));
    assert_eq!(more.get_avgpx(), None);
    // A follower reporting a fill of its own traded past the chain's
    // cumulative fill: the chain's is not what traded now. A bare follower
    // holds no ledger - the lifecycle walk counts the chain's fills and
    // writes the total (`rust/market/tests/graph/iterator.rs`).
    let mut fill = OrderEvent::at(3);
    fill.set_crosscode("ORD-1".to_owned());
    fill.set_state(State::PartiallyFilled);
    fill.set_lastqty(Some(dec("30")), true);
    fill.set_lastpx(Some(dec("11")), true);
    fill.finalize();
    let fill = fill.with_previous(&partial).expect("a later statement");
    assert_eq!(fill.get_ordqty(), Some(dec("100")));
    assert_eq!(fill.get_cumqty(), None);
    assert_eq!(fill.get_avgpx(), None);
    assert_eq!(fill.get_leavesqty(), None);
}

/// What one consistent statement of an element's facts lands on, whatever
/// order its facts are stated in: every implication is a fill or a follow,
/// none recursive, so facts that do not contradict converge.
#[derive(Clone, Default)]
struct Facts {
    state: Option<yggdryl::State>,
    side: Option<Side>,
    currency: Option<&'static str>,
    price: Option<&'static str>,
    quantity: Option<&'static str>,
    displayqty: Option<&'static str>,
    hiddenqty: Option<&'static str>,
    bidpx: Option<&'static str>,
    bidqty: Option<&'static str>,
    bidccy: Option<&'static str>,
    askpx: Option<&'static str>,
    askqty: Option<&'static str>,
    askccy: Option<&'static str>,
    ordqty: Option<&'static str>,
    cumqty: Option<&'static str>,
    leavesqty: Option<&'static str>,
    cxlqty: Option<&'static str>,
    lastpx: Option<&'static str>,
    lastqty: Option<&'static str>,
    avgpx: Option<&'static str>,
    spotrate: Option<&'static str>,
    forwardpoints: Option<&'static str>,
    cficode: Option<&'static str>,
}

/// One fact stated on an element, under `overwrite`.
type Statement<E> = Box<dyn Fn(&mut E, bool)>;

impl Facts {
    /// Each fact the statement states, as the setter stating it, named.
    fn statements<E: Event + Operation + 'static>(&self) -> Vec<(&'static str, Statement<E>)> {
        let mut held: Vec<(&'static str, Statement<E>)> = Vec::new();
        macro_rules! number {
            ($fact:ident, $set:ident) => {
                if let Some(text) = self.$fact {
                    held.push((
                        stringify!($fact),
                        Box::new(move |e: &mut E, over| e.$set(Some(dec(text)), over)),
                    ));
                }
            };
        }
        macro_rules! code {
            ($fact:ident, $set:ident) => {
                if let Some(text) = self.$fact {
                    held.push((
                        stringify!($fact),
                        Box::new(move |e: &mut E, over| {
                            e.$set(Some(Ccy::new(text).unwrap()), over)
                        }),
                    ));
                }
            };
        }
        if let Some(state) = self.state {
            held.push(("state", Box::new(move |e: &mut E, _| e.set_state(state))));
        }
        if let Some(side) = self.side {
            held.push((
                "side",
                Box::new(move |e: &mut E, over| e.set_side(side, over)),
            ));
        }
        if let Some(text) = self.currency {
            held.push((
                "currency",
                Box::new(move |e: &mut E, over| e.set_currency(Ccy::new(text).unwrap(), over)),
            ));
        }
        if let Some(text) = self.cficode {
            held.push((
                "cficode",
                Box::new(move |e: &mut E, over| e.set_cficode(Some(Cfi::new(text).unwrap()), over)),
            ));
        }
        number!(price, set_price);
        number!(quantity, set_quantity);
        number!(displayqty, set_displayqty);
        number!(hiddenqty, set_hiddenqty);
        number!(bidpx, set_bidpx);
        number!(bidqty, set_bidqty);
        code!(bidccy, set_bidccy);
        number!(askpx, set_askpx);
        number!(askqty, set_askqty);
        code!(askccy, set_askccy);
        number!(ordqty, set_ordqty);
        number!(cumqty, set_cumqty);
        number!(leavesqty, set_leavesqty);
        number!(cxlqty, set_cxlqty);
        number!(lastpx, set_lastpx);
        number!(lastqty, set_lastqty);
        number!(avgpx, set_avgpx);
        number!(spotrate, set_spotrate);
        number!(forwardpoints, set_forwardpoints);
        held
    }
}

/// Every market and operation fact an element answers, as text.
fn reading<E: Event + Operation>(element: &E) -> Vec<String> {
    let numbers = [
        element.get_price(),
        element.get_quantity(),
        element.get_displayqty(),
        element.get_hiddenqty(),
        element.get_bidpx(),
        element.get_bidqty(),
        element.get_askpx(),
        element.get_askqty(),
        element.get_ordqty(),
        element.get_cumqty(),
        element.get_leavesqty(),
        element.get_cxlqty(),
        element.get_lastpx(),
        element.get_lastqty(),
        element.get_avgpx(),
        element.get_spotrate(),
        element.get_forwardpoints(),
    ];
    let mut read: Vec<String> = numbers.iter().map(|held| format!("{held:?}")).collect();
    read.push(format!("{:?}", element.get_state()));
    read.push(element.get_side().as_str().to_owned());
    read.push(element.get_currency().as_str().to_owned());
    read.push(format!("{:?}", element.get_bidccy().map(Ccy::as_str)));
    read.push(format!("{:?}", element.get_askccy().map(Ccy::as_str)));
    read.push(format!("{:?}", element.get_cficode().map(Cfi::as_str)));
    read
}

/// The orders of `count` statements a case tries: as listed, reversed, and
/// a fixed run of shuffles, so a failure names an order it can replay.
fn orders(count: usize) -> Vec<Vec<usize>> {
    let mut held = vec![(0..count).collect::<Vec<_>>(), (0..count).rev().collect()];
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64 ^ count as u64;
    for _ in 0..32 {
        let mut order: Vec<usize> = (0..count).collect();
        for at in (1..count).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            order.swap(at, (seed % (at as u64 + 1)) as usize);
        }
        held.push(order);
    }
    held
}

/// Every order of every sub-statement of `facts` lands one element, under
/// either `overwrite`, and that element states each fact as stated.
fn converges<E: Event + Operation + 'static>(name: &str, at: fn(i64) -> E, facts: &Facts) {
    let all = facts.statements::<E>();
    // The whole statement and each one with a single fact left out - never
    // the state, which says which of the facts imply one another.
    let mut subsets: Vec<Vec<usize>> = vec![(0..all.len()).collect()];
    for left in (0..all.len()).filter(|at| all[*at].0 != "state") {
        subsets.push((0..all.len()).filter(|at| *at != left).collect());
    }
    for overwrite in [false, true] {
        for subset in &subsets {
            let mut landed: Option<(Vec<usize>, Vec<String>)> = None;
            for order in orders(subset.len()) {
                let order: Vec<usize> = order.into_iter().map(|at| subset[at]).collect();
                let mut element = at(1);
                for &index in &order {
                    (all[index].1)(&mut element, overwrite);
                }
                let read = reading(&element);
                match &landed {
                    None => landed = Some((order, read)),
                    Some((first, expected)) => assert_eq!(
                        &read,
                        expected,
                        "{name}, overwrite {overwrite}: {:?} against {:?}",
                        order.iter().map(|at| all[*at].0).collect::<Vec<_>>(),
                        first.iter().map(|at| all[*at].0).collect::<Vec<_>>(),
                    ),
                }
            }
            if subset.len() == all.len() {
                // The whole statement reads back as stated.
                let mut stated = at(1);
                for (_, statement) in &all {
                    statement(&mut stated, true);
                }
                let expected = landed.expect("one order at least").1;
                assert_eq!(reading(&stated), expected, "{name}");
            }
        }
    }
}

/// Facts that do not contradict give the same element whatever order they
/// are stated in, under either `overwrite`: an order, a quote and an
/// execution, working, filled, ended, two-sided and tagged.
#[test]
fn market_implications_are_confluent() {
    crate::install::installed();
    use yggdryl::State;

    let working = Facts {
        state: Some(State::PartiallyFilled),
        side: Some(Side::Buy),
        currency: Some("USD"),
        price: Some("100"),
        quantity: Some("60"),
        displayqty: Some("10"),
        hiddenqty: Some("50"),
        bidpx: Some("100"),
        bidqty: Some("60"),
        bidccy: Some("USD"),
        ordqty: Some("100"),
        cumqty: Some("40"),
        leavesqty: Some("60"),
        lastpx: Some("100.5"),
        lastqty: Some("40"),
        avgpx: Some("100.5"),
        spotrate: Some("100.25"),
        forwardpoints: Some("0.25"),
        cficode: Some("ESVUFR"),
        ..Facts::default()
    };
    converges("a working order", OrderEvent::at, &working);
    let filled = Facts {
        state: Some(State::Filled),
        side: Some(Side::Sell),
        currency: Some("EUR"),
        price: Some("50"),
        quantity: Some("0"),
        askpx: Some("50"),
        askqty: Some("0"),
        askccy: Some("EUR"),
        ordqty: Some("10"),
        cumqty: Some("10"),
        leavesqty: Some("0"),
        lastpx: Some("51"),
        lastqty: Some("4"),
        avgpx: Some("50.8"),
        ..Facts::default()
    };
    converges("a filled order", OrderEvent::at, &filled);
    let canceled = Facts {
        state: Some(State::Canceled),
        side: Some(Side::Buy),
        price: Some("10"),
        quantity: Some("0"),
        bidpx: Some("10"),
        bidqty: Some("0"),
        ordqty: Some("100"),
        cumqty: Some("40"),
        cxlqty: Some("60"),
        leavesqty: Some("0"),
        ..Facts::default()
    };
    converges("a canceled order", OrderEvent::at, &canceled);
    let fresh = Facts {
        state: Some(State::New),
        side: Some(Side::Sell),
        price: Some("20"),
        quantity: Some("100"),
        askpx: Some("20"),
        askqty: Some("100"),
        ordqty: Some("100"),
        leavesqty: Some("100"),
        ..Facts::default()
    };
    converges("a new order", OrderEvent::at, &fresh);
    let rejected = Facts {
        state: Some(State::Rejected),
        ordqty: Some("100"),
        leavesqty: Some("0"),
        quantity: Some("0"),
        ..Facts::default()
    };
    converges("a rejected order", OrderEvent::at, &rejected);
    let expired = Facts {
        state: Some(State::Expired),
        ordqty: Some("30"),
        cumqty: Some("0"),
        cxlqty: Some("30"),
        leavesqty: Some("0"),
        quantity: Some("0"),
        ..Facts::default()
    };
    converges("an expired order", OrderEvent::at, &expired);

    let two_sided = Facts {
        state: Some(State::Active),
        currency: Some("USD"),
        bidpx: Some("99"),
        bidqty: Some("10"),
        bidccy: Some("USD"),
        askpx: Some("101"),
        askqty: Some("20"),
        askccy: Some("USD"),
        ..Facts::default()
    };
    converges("a two-sided quote", QuoteEvent::at, &two_sided);
    let tagged = Facts {
        state: Some(State::Active),
        side: Some(Side::Buy),
        currency: Some("USD"),
        price: Some("99"),
        quantity: Some("10"),
        displayqty: Some("4"),
        hiddenqty: Some("6"),
        bidpx: Some("99"),
        bidqty: Some("10"),
        bidccy: Some("USD"),
        askpx: Some("101"),
        askqty: Some("20"),
        askccy: Some("USD"),
        ..Facts::default()
    };
    converges("a quote tagged buy", QuoteEvent::at, &tagged);

    let fill = Facts {
        state: Some(State::Filled),
        side: Some(Side::Buy),
        price: Some("10"),
        quantity: Some("40"),
        bidpx: Some("10"),
        bidqty: Some("40"),
        ordqty: Some("100"),
        cumqty: Some("40"),
        leavesqty: Some("60"),
        lastpx: Some("10"),
        lastqty: Some("40"),
        avgpx: Some("10"),
        spotrate: Some("9.5"),
        forwardpoints: Some("0.5"),
        ..Facts::default()
    };
    converges("an execution", ExecutionEvent::at, &fill);
}

/// `Operation::follow_identity`, what a lifecycle stands every element it
/// states as a chain's under: a sided operation stating no side takes the
/// live statement's - so its price quotes that side - and then the live
/// stored cross code, its cross hash and cross element derived from it; a
/// live statement stating no code lends the side alone, and one already
/// shared moves nothing. A conflict noted on a typed leaf records nothing.
#[test]
fn following_an_identity_lends_the_side_and_forces_the_code() {
    crate::install::installed();
    let crosshash = |code: &str| {
        let mut state = Xxh3::new();
        state.write(code.as_bytes());
        state.as_u64()
    };
    let mut live = OrderEvent::at(1);
    live.set_crosscode("J".to_owned());
    live.set_side(Side::Buy, true);
    live.finalize();
    assert_eq!(live.get_crosscode(), "10:1:J");
    let unsided = || {
        let mut event = OrderEvent::at(2);
        event.set_crosscode("K".to_owned());
        event.set_price(Some(Decimal::from_int(99)), true);
        event.set_quantity(Some(Decimal::from_int(5)), true);
        event.finalize();
        event
    };

    let mut event = unsided();
    assert_eq!(event.get_crosscode(), "10:0:K");
    assert!(event.follow_identity(&live));
    assert_eq!(event.get_side(), Side::Buy);
    assert_eq!(event.get_crosscode(), "10:1:J");
    assert_eq!(event.get_crosshashcode(), crosshash("10:1:J"));
    assert_eq!(
        event.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("10:1:J")))
    );
    assert_eq!(event.get_crossuuid(), live.get_crossuuid());
    assert_eq!(
        (event.get_bidpx(), event.get_bidqty()),
        (Some(Decimal::from_int(99)), Some(Decimal::from_int(5))),
        "the side lent quotes the price and the quantity"
    );
    // Moved, so the caller finalizes; settled, it moves nothing again.
    event.finalize();
    assert!(!event.follow_identity(&live), "one of equal side and code");
    assert_eq!(event.get_crosscode(), "10:1:J");

    // A live statement stating no code lends the side alone.
    let mut codeless = OrderEvent::at(1);
    codeless.set_side(Side::Sell, true);
    codeless.finalize();
    let mut event = unsided();
    assert!(event.follow_identity(&codeless));
    assert_eq!(event.get_side(), Side::Sell);
    assert_eq!(
        event.get_crosscode(),
        "10:2:K",
        "its own code, under the side"
    );

    // An unsided kind takes no side: a quote's is a tag.
    let mut quote = QuoteEvent::at(2);
    quote.set_crosscode("Q".to_owned());
    quote.finalize();
    let mut held = QuoteEvent::at(1);
    held.set_crosscode("Q".to_owned());
    held.set_side(Side::Buy, true);
    held.finalize();
    assert!(!quote.follow_identity(&held), "one code, no side lent");
    assert_eq!(quote.get_side(), Side::Unknown);

    // A conflict noted on a typed leaf records nothing.
    let mut noted = unsided();
    let before = noted.clone();
    noted.note_conflict("10:0:K cites 10:1:J by clordid=C1 and 10:2:L by its cross code");
    assert_eq!(noted, before);
}
