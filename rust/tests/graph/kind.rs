//! `rust/src/graph/kind.rs`: which leaf a `MarketData` value is, and the
//! word every schema states it under.

use yggdryl::graph::MarketKind;

#[test]
fn every_kind_spells_itself_once_in_declaration_order_and_reads_back_ignoring_case() {
    assert_eq!(
        MarketKind::ALL.map(MarketKind::as_str),
        [
            "order",
            "quote",
            "execution",
            "book_side",
            "order_event",
            "quote_event",
            "execution_event",
            "trade_event",
            "book_event",
            "snapshot_event",
        ]
    );
    for (stored, kind) in MarketKind::ALL.into_iter().enumerate() {
        assert_eq!(kind as usize, stored);
        assert_eq!(MarketKind::read(kind.as_str()), Some(kind));
        assert_eq!(
            MarketKind::read(&kind.as_str().to_ascii_uppercase()),
            Some(kind)
        );
    }
    assert_eq!(MarketKind::read(""), None);
    assert_eq!(MarketKind::read("trade"), None, "a trade is dated");
    assert_eq!(MarketKind::read("snapshot"), None);
    assert_eq!(MarketKind::read(" order "), None, "a spelling is exact");
    assert!(MarketKind::Order < MarketKind::SnapshotEvent);
}

#[test]
fn the_six_dated_kinds_are_the_events() {
    let events: Vec<MarketKind> = MarketKind::ALL
        .into_iter()
        .filter(|kind| kind.is_event())
        .collect();
    assert_eq!(
        events,
        [
            MarketKind::OrderEvent,
            MarketKind::QuoteEvent,
            MarketKind::ExecutionEvent,
            MarketKind::TradeEvent,
            MarketKind::BookEvent,
            MarketKind::SnapshotEvent,
        ]
    );
}
