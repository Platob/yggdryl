//! `rust/src/graph/market.rs`: the `Market` and `Operation` traits and the
//! readings they provide an implementor that is also an `Event` - the
//! digests, following and merging a dated market or operation runs.

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event, Market, Operation, OrderEvent};
use yggdryl::{Decimal, TimeInForce, Uuid};

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
