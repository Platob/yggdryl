//! `rust/src/graph/market.rs`: the `Market` and `Operation` traits and the
//! readings they provide an implementor that is also an `Event` - the
//! digests, following and merging a dated market or operation runs.

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event, Market, Operation, Order, OrderEvent};
use yggdryl::securityid::{SecType, SecurityId, SecurityIds};
use yggdryl::{Decimal, TimeInForce, Uuid};

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
