//! `rust/src/graph/market.rs`: the `Market` and `Operation` traits and the
//! readings they provide an implementor that is also an `Event` - the
//! digests, following and merging a dated market or operation runs.

use std::borrow::Cow;
use std::hash::Hasher;

use smol_str::SmolStr;
use yggdryl::graph::{
    BookEvent, Element, Event, ExecutionEvent, FxRates, Market, Operation, Order, OrderEvent,
    QuoteEvent, SnapshotEvent, TradeEvent,
};
use yggdryl::xxhash::Xxh3;
use yggdryl::{
    Ccy, Cfi, Decimal, IdSource, IdType, Identifier, Identifiers, Mic, Side, TimeInForce, Uuid,
};

/// The identifier set holding `ids`, each `(kind, code)` - a type's name or
/// its FIX source code - validated by its type and stated from `base`.
fn securityids(ids: &[(&str, &str)]) -> Identifiers {
    ids.iter()
        .map(|(kind, code)| {
            Identifier::new(
                IdSource::Base,
                IdType::from_security_source(kind).unwrap(),
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
    assert_eq!(market.get_prevuuid(), Some(previous.get_curruuid()));
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

/// A chain carries its instrument's identifiers to the step that states
/// none, what the ISIN implied included; a step naming another ISIN is
/// another instrument and takes none of them.
#[test]
fn following_carries_the_identifiers_only_of_the_same_instrument() {
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
            "base:isin=US0378331005",
            "base:ric=AAPL.O",
            "derived:cusip=037833100"
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
        ["base:isin=GB0002634946", "derived:sedol=0263494"]
    );
}

/// Two statements of one element naming different ISINs name two
/// instruments: the leading statement's identifiers stand whole, never a
/// key of the other beside them.
#[test]
fn merging_statements_naming_different_isins_keeps_the_leading_identifiers() {
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

    // This statement leads, so it keeps its identifiers and nothing moves:
    // the other's SEDOL does not join them.
    assert!(this.clone().merging_market(&restated).is_none());

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
    recorded.set_recdunix(Some(5));
    recorded
        .set_securityids(securityids(&[("ISIN", "GB0002634946")]), true)
        .unwrap();
    let merged = first
        .clone()
        .merging_market_event(&recorded)
        .expect("the later recording leads");
    assert_eq!(
        ids(&merged),
        ["base:isin=GB0002634946", "derived:sedol=0263494"]
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
            "base:figi=BBG000BLNQ16",
            "base:isin=US0378331005",
            "base:ric=AAPL.O",
            "derived:cusip=037833100"
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
    fill.set_side(Side::read("Buy").unwrap(), true);
    fill.set_lastpx(Some(dec("100.5")), true);
    fill.set_lastqty(Some(Decimal::from_int(5)), true);
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
    assert_eq!(empty.get_curruuid(), order(3).get_curruuid());
}

/// A `ZZ` ISIN names no country's instrument: it yields to a real one on
/// follow and on merge, the national code the real one carries deriving
/// afresh, and it never makes two statements two instruments.
#[test]
fn an_unknown_isin_yields_to_a_real_one() {
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

/// The ISIN is borrowed from the identifiers: no copy, no second store.
#[test]
fn get_isincode_borrows_the_isin_identifier() {
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
/// of the category it is filed under and, for a sided one - an order, a
/// quote, an execution - the code of the side it takes, `0` for any other
/// kind and for a side nobody stated. The cross hash and the cross element
/// follow the stored text.
#[test]
fn every_element_stores_its_cross_code_under_its_category_and_side() {
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
    assert!(quote.is_sided());
    assert_eq!(quote.get_crosscode(), "14:1:Q-1");

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
        quote.get_crosscode(),
        "14:2:ORD-1",
        "the side travels, the kind is its own"
    );
}

/// A code already carrying another category's or side's prefix has it
/// replaced, never stacked; one carrying this element's own is answered as
/// it is, borrowed; and an empty code stays empty whatever the element
/// takes.
#[test]
fn a_prefix_is_replaced_never_stacked_and_an_empty_code_stays_empty() {
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
    let book = BookEvent::new(1, "");
    assert_eq!(book.get_crosscode(), "");
    let snapshot = SnapshotEvent::snapshot(&empty, None);
    assert_eq!(snapshot.get_crosscode(), "");
}

/// A copy of an element into another leaf kind takes the base code and
/// states it under its own prefix: the facts travel, the category is the
/// leaf's.
#[test]
fn a_copy_into_another_kind_takes_the_base_under_its_own_prefix() {
    let mut order = OrderEvent::at(1);
    order.set_crosscode("ORD-1".to_owned());
    order.set_side(Side::Buy, true);
    order.finalize();
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");

    let quote = QuoteEvent::from(&order);
    assert_eq!(quote.get_crosscode(), "14:1:ORD-1");
    let fill = ExecutionEvent::from(&order);
    assert_eq!(fill.get_crosscode(), "8:1:ORD-1");
    let undated = order.clone().into_element();
    assert_eq!(undated.get_crosscode(), "10:1:ORD-1");
    let quoted = QuoteEvent::from(&fill);
    assert_eq!(quoted.get_crosscode(), "14:1:ORD-1");
    assert_ne!(quote.get_crosshashcode(), order.get_crosshashcode());
    assert_ne!(quote.get_crossuuid(), fill.get_crossuuid());
    assert_eq!(quote.get_crosshashcode(), quoted.get_crosshashcode());
    assert_eq!(quote.get_crosshashcode(), crosshash("14:1:ORD-1"));

    // The same copy of an order that stated no side states side 0 in each.
    let mut sideless = OrderEvent::at(1);
    sideless.set_crosscode("ORD-1".to_owned());
    assert_eq!(QuoteEvent::from(&sideless).get_crosscode(), "14:0:ORD-1");
    assert_eq!(ExecutionEvent::from(&sideless).get_crosscode(), "8:0:ORD-1");
}

/// A book's key is the ticker where one is stated, else the category.
#[test]
fn book_crosscode_is_the_ticker_else_the_market_and_classification() {
    let mut ticker = order(1);
    ticker.set_ticker(Some(SmolStr::new("ACME")), true);
    assert!(matches!(ticker.book_crosscode(), Cow::Borrowed("ACME")));
    let blank = order(2);
    assert_eq!(blank.book_crosscode(), "XXXX:XXXXXX");
    let mut classified = order(3);
    classified.set_miccode(Some(Mic::new("XPAR").unwrap()), true);
    classified.set_cficode(Some(Cfi::new("ESVUFR").unwrap()), true);
    assert_eq!(classified.book_crosscode(), "XPAR:ESVUFR");
}

/// Without `overwrite` a setter fills only a fact the element states
/// nothing under - `None`, `UNKN`, a currency or unit of none, an empty map -
/// and with it states the value, `None` clearing; an equal value changes
/// nothing either way.
#[test]
fn a_setter_fills_an_unstated_fact_and_overwrites_only_when_told() {
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
    order.set_marketdatatype(yggdryl::MarketDataType::OrdLimit, false);
    order.set_marketdatatype(yggdryl::MarketDataType::OrdMarket, false);
    assert_eq!(
        order.get_marketdatatype(),
        yggdryl::MarketDataType::OrdLimit
    );
    order.set_marketdatatype(yggdryl::MarketDataType::OrdMarket, true);
    assert_eq!(
        order.get_marketdatatype(),
        yggdryl::MarketDataType::OrdMarket
    );
    order.set_timeinforce(TimeInForce::from_spelling("DAY"), false);
    order.set_timeinforce(TimeInForce::from_spelling("IOC"), false);
    assert_eq!(
        order.get_timeinforce().map(|held| held.as_str()),
        Some("DAY")
    );

    // A map fills only the keys it lacks without `overwrite`, and is
    // replaced with it.
    let mut first = yggdryl::graph::Metadata::new();
    first.insert("desk".into(), "A".into());
    order.set_metadata(Some(first), false);
    let mut second = yggdryl::graph::Metadata::new();
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

/// An iceberg's hidden part is the quantity past the shown one, kept in
/// step with both; a hidden part stated fills the shown one, or the
/// quantity the two make together.
#[test]
fn an_iceberg_keeps_its_hidden_part_in_step() {
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
    use yggdryl::Unit;
    use yggdryl::graph::Element as _;

    let pair = |currency: Option<&str>| {
        let mut order = OrderEvent::at(1);
        order
            .insert_securityid(Identifier::new(IdSource::Base, IdType::Forex, "EURUSD").unwrap())
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
        .insert_securityid(Identifier::new(IdSource::Base, IdType::Isin, "CH0012214059").unwrap())
        .unwrap();
    stated.set_ticker(Some("US0378331005".into()), true);
    stated.finalize();
    assert_eq!(
        stated.get_securityids().get(&IdType::Isin),
        Some("CH0012214059")
    );
}
