//! `rust/src/graph/market.rs`: the `Market` and `Operation` traits and the
//! readings they provide an implementor that is also an `Event` - the
//! digests, following and merging a dated market or operation runs.

use std::borrow::Cow;

use smol_str::SmolStr;
use yggdryl::graph::{
    Element, Event, ExecutionEvent, FxRates, Market, Operation, Order, OrderEvent,
};
use yggdryl::securityid::{SecType, SecurityId, SecurityIds};
use yggdryl::{Ccy, Cfi, Decimal, Mic, Side, TimeInForce, Uuid};

/// The identifier set holding `ids`, each `(key, code)` validated.
fn securityids(ids: &[(&str, &str)]) -> SecurityIds {
    let mut set = SecurityIds::default();
    for (key, code) in ids {
        set.insert(SecurityId::new(SecType::read(key).unwrap(), code).unwrap());
    }
    set
}

/// Every identifier an element holds, as `KEY:code`, in source order.
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
    order.set_price(Some(Decimal::from_int(80)));
    order.finalize();
    order
}

/// The market event digest feeds the market's facts and nothing an
/// operation states; the operation event digest feeds both.
#[test]
fn the_operation_event_digest_feeds_what_the_market_event_digest_does_not() {
    let plain = order(1);
    let mut standing = plain.clone();
    standing.set_tif(TimeInForce::from_spelling("GoodTillCancel"));
    assert_eq!(
        plain.digest_market_event().as_u64(),
        standing.digest_market_event().as_u64(),
    );
    assert_ne!(
        plain.digest_operation_event().as_u64(),
        standing.digest_operation_event().as_u64(),
    );
    let mut priced = plain.clone();
    priced.set_price(Some(Decimal::from_int(81)));
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
    let mut previous = order(1);
    previous.set_ticker(Some(SmolStr::new("BRN")));
    previous.set_tif(TimeInForce::from_spelling("GoodTillCancel"));
    previous.finalize();
    let next = order(2);

    let market = next
        .clone()
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(market.get_ticker(), Some("BRN"));
    assert_eq!(market.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(
        market.get_tif(),
        None,
        "the market reading leaves the operation"
    );

    let operation = next
        .clone()
        .following_operation(&previous)
        .expect("a later event follows");
    assert_eq!(operation.get_ticker(), Some("BRN"));
    assert_eq!(operation.get_tif(), previous.get_tif());

    assert!(previous.clone().following_market(&next).is_none());
    assert!(previous.clone().following_operation(&next).is_none());
    assert!(next.clone().following_market(&next).is_none());
}

/// Merging another statement of the same event takes what the market
/// reading merges - its sources - and, only for the operation reading,
/// the operation's facts; a stranger merges with neither.
#[test]
fn merging_takes_the_market_and_only_the_operation_reading_takes_the_operation() {
    let this = order(1);
    let mut restated = this.clone();
    restated.set_srcuuids(vec![Uuid::from_v8(9)]);
    restated.set_tif(TimeInForce::from_spelling("GoodTillCancel"));

    let market = this
        .clone()
        .merging_market_event(&restated)
        .expect("the sources move");
    assert_eq!(market.get_srcuuids(), [Uuid::from_v8(9)]);
    assert_eq!(market.get_tif(), None);

    let operation = this
        .clone()
        .merging_operation_event(&restated)
        .expect("the sources move");
    assert_eq!(operation.get_srcuuids(), [Uuid::from_v8(9)]);
    assert_eq!(operation.get_tif(), restated.get_tif());

    let mut stranger = OrderEvent::at(1);
    stranger.set_crosscode("OTHER".to_owned());
    stranger.finalize();
    assert!(this.clone().merging_market_event(&stranger).is_none());
    assert!(this.clone().merging_operation_event(&stranger).is_none());
}

/// A chain carries its instrument's identifiers to the step that states
/// none, what the ISIN implied included; a step naming another ISIN is
/// another instrument and takes none of them.
#[test]
fn following_carries_the_identifiers_only_of_the_same_instrument() {
    let mut previous = order(1);
    previous
        .set_securityids(securityids(&[("ISIN", "US0378331005"), ("RIC", "AAPL.O")]))
        .unwrap();
    previous.finalize();
    assert_eq!(
        ids(&previous),
        ["CUSIP:037833100", "ISIN:US0378331005", "RIC:AAPL.O"]
    );

    let silent = order(2)
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(ids(&silent), ids(&previous));

    let mut other = order(2);
    other
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]))
        .unwrap();
    other.finalize();
    let followed = other
        .clone()
        .following_market(&previous)
        .expect("a later event follows");
    assert_eq!(ids(&followed), ["ISIN:GB0002634946", "SEDOL:0263494"]);
}

/// Two statements of one element naming different ISINs name two
/// instruments: the leading statement's identifiers stand whole, never a
/// key of the other beside them.
#[test]
fn merging_statements_naming_different_isins_keeps_the_leading_identifiers() {
    let mut this = Order::new();
    this.set_crosscode("ORDER".to_owned());
    this.set_securityids(securityids(&[("ISIN", "US0378331005"), ("RIC", "AAPL.O")]))
        .unwrap();
    this.finalize();
    let mut restated = this.clone();
    restated
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]))
        .unwrap();
    restated.set_price(Some(Decimal::from_int(81)));

    // This statement leads, so it keeps its identifiers and nothing moves:
    // the other's SEDOL does not join them.
    assert!(this.clone().merging_market(&restated).is_none());

    // Where the other statement leads, its identifiers replace these whole.
    let mut first = order(1);
    first
        .set_securityids(securityids(&[("ISIN", "US0378331005"), ("RIC", "AAPL.O")]))
        .unwrap();
    first.finalize();
    let mut recorded = first.clone();
    recorded.set_recdunix(Some(5));
    recorded
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]))
        .unwrap();
    let merged = first
        .clone()
        .merging_market_event(&recorded)
        .expect("the later recording leads");
    assert_eq!(ids(&merged), ["ISIN:GB0002634946", "SEDOL:0263494"]);

    // The same instrument restated fills what this statement left open.
    let mut same = this.clone();
    same.set_securityids(securityids(&[
        ("ISIN", "US0378331005"),
        ("FIGI", "BBG000BLNQ16"),
    ]))
    .unwrap();
    let merged = this.clone().merging_market(&same).expect("the FIGI fills");
    assert_eq!(
        ids(&merged),
        [
            "CUSIP:037833100",
            "FIGI:BBG000BLNQ16",
            "ISIN:US0378331005",
            "RIC:AAPL.O"
        ]
    );
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// An execution's price stays the one it states - none - through any
/// number of finalizes: what it last executed is its `lastpx`, never its
/// price.
#[test]
fn an_execution_price_is_never_what_it_last_executed() {
    let mut fill = ExecutionEvent::at(10);
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::read("Buy").unwrap());
    fill.set_lastpx(Some(dec("100.5")));
    fill.set_lastqty(Some(Decimal::from_int(5)));
    fill.finalize();
    let once = fill.clone();
    fill.finalize();
    assert_eq!(fill, once, "finalizing twice changes nothing");
    assert_eq!((fill.get_price(), fill.get_quantity()), (None, None));
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
    let mut previous = order(1);
    previous.set_fxrates(rates(&[("USD", "1.08"), ("JPY", "150")]));
    previous.finalize();
    let mut next = order(2);
    next.set_fxrates(rates(&[("USD", "1.09")]));
    next.finalize();
    let followed = next
        .clone()
        .with_previous(&previous)
        .expect("a later order");
    assert_eq!(followed.get_fxrates(), &rates(&[("USD", "1.09")]));

    let mut restated = next.clone();
    restated.set_fxrates(rates(&[("USD", "1.10"), ("GBP", "0.86")]));
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
    empty.set_fxrates(FxRates::new());
    empty.finalize();
    assert_eq!(empty.get_curruuid(), order(3).get_curruuid());
}

/// A `ZZ` ISIN names no country's instrument: it yields to a real one on
/// follow and on merge, the national code the real one carries deriving
/// afresh, and it never makes two statements two instruments.
#[test]
fn an_unknown_isin_yields_to_a_real_one() {
    let mut previous = order(1);
    previous
        .set_securityids(securityids(&[
            ("ISIN", "US0378331005"),
            ("BLOOMBERG", "BBG000B9XRY4"),
        ]))
        .unwrap();
    previous.finalize();
    assert_eq!(previous.get_securityids().get("CUSIP"), Some("037833100"));
    let mut next = order(2);
    next.set_securityids(securityids(&[("ISIN", "ZZ0000000008")]))
        .unwrap();
    next.finalize();
    let followed = next
        .clone()
        .with_previous(&previous)
        .expect("a later order");
    assert_eq!(followed.get_isincode(), Some("US0378331005"));
    assert_eq!(followed.get_securityids().get("CUSIP"), Some("037833100"));
    assert_eq!(
        followed.get_securityids().get("BLOOMBERG"),
        Some("BBG000B9XRY4"),
        "a ZZ ISIN is no other instrument"
    );

    let mut restated = next.clone();
    restated
        .set_securityids(securityids(&[("ISIN", "US0378331005")]))
        .unwrap();
    let merged = next
        .clone()
        .merging_operation_event(&restated)
        .expect("the same order restated");
    assert_eq!(merged.get_isincode(), Some("US0378331005"));
    assert_eq!(merged.get_securityids().get("CUSIP"), Some("037833100"));
    // And the other way round: a real ISIN never takes a ZZ one, leading or
    // not.
    let merged = restated
        .clone()
        .merging_operation_event(&next)
        .unwrap_or_else(|| restated.clone());
    assert_eq!(merged.get_isincode(), Some("US0378331005"));
}

/// The ISIN is borrowed from the identifiers: no copy, no second store.
#[test]
fn get_isincode_borrows_the_isin_identifier() {
    let mut listed = order(1);
    listed
        .set_securityids(securityids(&[("ISIN", "US0378331005")]))
        .unwrap();
    let projected = listed.get_isincode().unwrap();
    let stored = listed.get_securityids().get("ISIN").unwrap();
    assert!(std::ptr::eq(projected, stored));
    assert_eq!(order(2).get_isincode(), None);
}

/// A book's key is the ticker where one is stated, else the category.
#[test]
fn book_crosscode_is_the_ticker_else_the_market_and_classification() {
    let mut ticker = order(1);
    ticker.set_ticker(Some(SmolStr::new("ACME")));
    assert!(matches!(ticker.book_crosscode(), Cow::Borrowed("ACME")));
    let blank = order(2);
    assert_eq!(blank.book_crosscode(), "XXXX:XXXXXX");
    let mut classified = order(3);
    classified.set_miccode(Some(Mic::new("XPAR").unwrap()));
    classified.set_cficode(Some(Cfi::new("ESVUFR").unwrap()));
    assert_eq!(classified.book_crosscode(), "XPAR:ESVUFR");
}
