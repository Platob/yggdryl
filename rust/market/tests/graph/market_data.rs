//! `rust/market/src/graph/market_data.rs`: one value over every leaf, which leaf it
//! is, the borrows and conversions to each, and the element readings it
//! delegates by variant.

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event};
use yggdryl::{Decimal, Error, State};
use yggdryl_market::Side;
use yggdryl_market::graph::{
    BookEvent, ExecutionEvent, Market, MarketData, Order, OrderEvent, QuoteEvent, SnapshotEvent,
    TradeEvent,
};

fn order(unix: i64, code: &str) -> OrderEvent {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some(SmolStr::new("ACME")), true);
    order.set_instcode(Some(yggdryl::Str::new("ACME")), true);
    order.set_side(Side::read("Buy").unwrap(), true);
    order.set_price(Some(Decimal::from_int(100)), true);
    order.set_quantity(Some(Decimal::from_int(2)), true);
    order.finalize();
    order
}

fn execution(unix: i64, code: &str) -> ExecutionEvent {
    let mut execution = ExecutionEvent::from(&order(unix, code));
    execution.set_state(State::read("Filled").unwrap());
    execution.finalize();
    execution
}

#[test]
fn a_leaf_converts_back_and_another_kind_is_refused_at_the_kind() {
    crate::install::installed();
    let leaf = order(1, "O-1");
    let value = MarketData::from(leaf.clone());
    assert_eq!(OrderEvent::try_from(value.clone()).unwrap(), leaf);
    let error = QuoteEvent::try_from(value.clone()).unwrap_err();
    assert!(
        matches!(&error, Error::InvalidRecord { path, .. } if path == "$.kind"),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("expected quote_event, got order_event"),
        "{message}"
    );
    let book = BookEvent::keyed(9, "ACME");
    assert_eq!(
        BookEvent::try_from(MarketData::from(book.clone())).unwrap(),
        book
    );
    assert!(BookEvent::try_from(value).is_err());
}

#[test]
fn only_two_dated_values_order_and_only_one_variant_merges() {
    crate::install::installed();
    // A millisecond apart: two instants in one millisecond stating the same
    // content are one identity, which follows nothing.
    let first = MarketData::from(order(1_000_000, "O-1"));
    let second = MarketData::from(order(2_000_000, "O-1"));
    assert!(second.is_after(&first));
    assert!(!first.is_after(&second));
    let undated = MarketData::from(Order::new());
    assert!(!undated.is_after(&first) && !first.is_after(&undated));

    let followed = second
        .clone()
        .with_previous(&first)
        .expect("a later order follows");
    let leaf = followed.as_order_event().unwrap();
    assert_eq!(leaf.get_prevuuid(), Some(first.get_uuid()));
    assert_eq!(
        followed,
        MarketData::from(
            order(2_000_000, "O-1")
                .with_previous(first.as_order_event().unwrap())
                .unwrap()
        ),
        "the leaf's own following"
    );

    let quote = MarketData::from(QuoteEvent::from(&order(2_000_000, "O-1")));
    assert!(
        quote.clone().with_previous(&first).is_some(),
        "an operation event follows another kind"
    );
    assert!(
        quote.merge_with(&first).is_none(),
        "a merge never crosses variants"
    );
    assert!(undated.clone().with_previous(&first).is_none());

    let mut restated = order(1_000_000, "O-1");
    restated.set_srcuuids(vec![yggdryl::Uuid::from_v8(9)]);
    let merged = first
        .clone()
        .merge_with(&MarketData::from(restated))
        .expect("another statement of the same order adds its source");
    assert_eq!(merged.get_srcuuids(), [yggdryl::Uuid::from_v8(9)]);
}

/// The enum is its widest inline leaf, a trade; the book is boxed. The book
/// was the wider of the two when the box was written, so the one value every
/// boundary crosses as did not carry a book's width; it is the narrower now
/// (see the last sentence). Pinned when the enum replaced the generic
/// envelope; a moved number is a
/// design answer, never one to re-pin from a whole run. It moved from 1072
/// when the market facts began to know which identifiers they only derived:
/// one `u64` mask, padded to sixteen bytes' alignment. It moved back to
/// 1072 when an event's `State` became an `i32` member rather than a
/// twenty-four-byte code string, padded to the same alignment. It moved to
/// 1088 when the dated operation facts moved from 1040 to 1056 - the market
/// facts' FX rates, sixteen, less the `marketoperationid` they no longer
/// hold, eight, padded - the trade's executions taking it to 1080, padded
/// to 1088. It moved to 960 when the dated operation facts moved from 1056
/// to 928 - the rates a map in the market facts' padding, and two
/// `IdMap`s fewer - the trade's executions taking it to 952, padded to 960.
/// It moved to 944 when the dated operation facts moved from 928 to 912 -
/// the two boxed lanes they no longer hold - the trade's executions taking
/// it to 936, padded to 944. It moved to 960 when the market facts gained
/// the boxed bid and ask, the dated operation facts sixteen wider. It moved
/// to 1008 when the operation facts gained `accountids`, one `IdMap` of
/// forty-eight. It moved to 992 when the time in force became an enum: an
/// `Option<TimeInForce>` of one byte where a twenty-four-byte string code
/// stood, sixteen fewer after padding. It moved to 912 when the identifiers
/// became `Identifiers`, one 24-byte sorted vector each: the dated operation
/// holds three sets where it held a 56-byte `SecurityIds` with its derived
/// mask and two 56-byte `IdMap`s, 96 fewer, so the trade is 896, and the
/// snapshot - whose market facts hold the one set, 32 fewer after padding -
/// is the widest inline leaf at 912. The book was 992, the wider again,
/// until a side became one shared store and the deltas one list across both
/// sides: a side holds two reference counts where it held a price map and
/// its own deltas, so the book is 912 - the snapshot's width - and the box
/// keeps a book's move a pointer's. The book moved to 880 when its two
/// sides became one optional pair - none on a delta book - each side its
/// two reference counts without the digest it no longer keeps, a side being
/// digested only at a snapshot, and the walk's replaced scopes left the
/// book, every group replacing membership being a snapshot: 40 fewer, 32
/// after padding. It moved to 896 when its one list became two - its
/// `delta`, the orders and quotes its instant applied, and its `events`,
/// every other event its instant recorded - one 24-byte vector more, 16
/// after padding, still narrower than the snapshot.
/// Both moved by sixteen when `instcode` joined `MarketFacts`, D42: one
/// `Option<Str>` of twenty-four, eight of them the padding the facts
/// carried, so the book is 912 and the snapshot 928.
#[test]
fn the_enum_is_the_size_of_its_widest_inline_leaf() {
    crate::install::installed();
    use std::mem::size_of;
    assert_eq!(size_of::<MarketData>(), size_of::<SnapshotEvent>());
    assert!(size_of::<TradeEvent>() < size_of::<MarketData>());
    assert_eq!(size_of::<BookEvent>(), 912);
    assert_eq!(size_of::<MarketData>(), 928);
}

/// The origin currency is the `marketdata` row's column right after the
/// currency - sixty-five columns - written only where a leaf holds one and
/// null otherwise, never the currency it defaults to at read, and read back
/// as stated.
#[test]
fn the_origin_currency_cell_is_written_only_where_held() {
    crate::install::installed();
    use arrow_array::Array;
    use yggdryl::Ccy;

    let root = MarketData::field().expect("the marketdata row");
    let names: Vec<&str> = root.fields().iter().map(|field| field.name()).collect();
    assert_eq!(names.len(), 66);
    let at = names
        .iter()
        .position(|name| *name == "origccy")
        .expect("a column");
    assert_eq!(names[at - 1], "currency");
    assert_eq!(names[at + 1], "quantity");

    let mut issued = order(1, "O-1");
    issued.set_currency(Ccy::new("EUR").unwrap(), true);
    issued.set_origccy(Ccy::new("USD").unwrap(), true);
    issued.finalize();
    let mut listed = order(2, "O-2");
    listed.set_currency(Ccy::new("EUR").unwrap(), true);
    listed.finalize();
    let expected = vec![MarketData::from(issued), MarketData::from(listed)];
    let mut encoded = MarketData::arrow_reader(expected.clone(), None, None).expect("two rows");
    let batch = encoded.next().expect("a batch").expect("a written batch");
    assert!(encoded.next().is_none());
    let column = batch.column_by_name("origccy").expect("the column");
    assert!(column.is_valid(0), "held, written");
    assert!(column.is_null(1), "unheld, null - never the currency");

    let read: Vec<MarketData> = MarketData::from_arrow_reader(
        MarketData::arrow_reader(expected.clone(), None, None).expect("two rows"),
    )
    .expect("the row's schema")
    .collect::<yggdryl::Result<_>>()
    .expect("the rows read");
    assert_eq!(read, expected);
    assert_eq!(read[0].get_origccy().as_str(), "USD");
    assert!(read[1].get_origccy().is_none());
    assert_eq!(read[1].origin_currency().as_str(), "EUR");
}

/// A book states no sources in its row, and neither does an entry nested in
/// its `alive`: that entry is the very one the `delta` of the book that
/// applied it holds, whose row writes its sources - as `events` does, and a
/// trade's `executions`, and a row standing alone. Read back, the book and
/// its alive entries state none, whatever their cells hold, and the book is
/// the one written by identity; the delta and the events laid out of the
/// book rows carry every source their entries were read with.
#[test]
fn a_book_row_writes_no_sources_of_its_own_nor_of_its_alive_entries() {
    crate::install::installed();
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, ListArray, RecordBatch, StructArray};
    use yggdryl::{ArrowCastOptions, Serie, StreamChunkedSerie, Uuid};

    let source = |payload: u128| vec![Uuid::from_v8(payload)];
    let new = |unix: i64, code: &str| {
        let mut entry = order(unix, code);
        entry.set_state(State::New);
        entry.finalize();
        entry
    };
    let sourced = |mut value: MarketData, payload: u128| {
        value.set_srcuuids(source(payload));
        value
    };
    // A complete book: an order and a quote alive and in its delta, an
    // execution among its events, each read from a line of its own.
    let mut quote = QuoteEvent::from(&new(1, "Q-1"));
    quote.finalize();
    let mut book = BookEvent::keyed(1, "ACME");
    book.add_operations([
        sourced(MarketData::from(new(1, "O-1")), 70),
        sourced(MarketData::from(quote), 71),
        sourced(MarketData::from(execution(1, "E-1")), 72),
    ])
    .unwrap();
    assert_eq!((book.alive().count(), book.delta().len()), (2, 2));
    assert_eq!(book.events().len(), 1);
    let mut fill = execution(8, "E-8");
    fill.set_srcuuids(source(73));
    let trade = TradeEvent::from_parts(&new(8, "T-8"), vec![fill]).unwrap();
    let alone = sourced(MarketData::from(new(9, "O-9")), 74);
    let written = vec![
        MarketData::from(book.clone()),
        MarketData::from(trade),
        alone,
    ];
    let rows = |values: Vec<MarketData>| -> RecordBatch {
        let mut encoded = MarketData::arrow_reader(values, None, None).unwrap();
        let batch = encoded.next().unwrap().unwrap();
        assert!(encoded.next().is_none());
        batch
    };
    let batch = rows(written.clone());
    let list = |batch: &RecordBatch, name: &str| -> ListArray {
        let column = batch.column_by_name(name).unwrap();
        column.as_any().downcast_ref::<ListArray>().unwrap().clone()
    };
    let items = |list: &ListArray| -> StructArray {
        let values = list.values();
        values
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap()
            .clone()
    };
    let stated = |column: &ArrayRef| {
        (0..column.len())
            .map(|at| column.is_valid(at))
            .collect::<Vec<_>>()
    };
    let sources_cell =
        |list: &ListArray| Arc::clone(items(list).column_by_name("srcuuids").unwrap());

    let own = batch.column_by_name("srcuuids").unwrap();
    assert_eq!(
        stated(own),
        [false, false, true],
        "the book's, the trade's, the order's"
    );
    assert_eq!(
        stated(&sources_cell(&list(&batch, "alive"))),
        [false, false]
    );
    assert_eq!(stated(&sources_cell(&list(&batch, "delta"))), [true, true]);
    assert_eq!(stated(&sources_cell(&list(&batch, "events"))), [true]);
    assert_eq!(stated(&sources_cell(&list(&batch, "executions"))), [true]);

    let read_all = |batch: RecordBatch| {
        let source = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        MarketData::from_arrow_reader(source)
            .unwrap()
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
    };
    let sources_of = |entries: Vec<&MarketData>| {
        entries
            .into_iter()
            .map(|entry| entry.get_srcuuids().to_vec())
            .collect::<Vec<_>>()
    };
    let uuids = |book: &BookEvent| book.alive().map(Element::get_uuid).collect::<Vec<_>>();
    let check = |read: &BookEvent| {
        assert!(read.get_srcuuids().is_empty());
        assert_eq!(read.get_uuid(), book.get_uuid());
        assert_eq!(uuids(read), uuids(&book));
        assert!(read.alive().all(|entry| entry.get_srcuuids().is_empty()));
        assert_eq!(sources_of(read.delta().collect()), [source(70), source(71)]);
        assert_eq!(sources_of(read.events().collect()), [source(72)]);
    };
    let read = read_all(batch.clone());
    check(read[0].as_book_event().expect("a book"));
    let executions = read[1].as_trade_event().expect("a trade").executions();
    assert_eq!(executions[0].get_srcuuids(), source(73));
    assert_eq!(read[2].get_srcuuids(), source(74));

    // A book row stating sources in its own cell and in its alive entries'
    // reads back as the same book, stating none.
    let book_row = rows(written[..1].to_vec());
    let order_row = rows(written[2..].to_vec());
    let alive = list(&book_row, "alive");
    let alive_items = items(&alive);
    let delta_sources = sources_cell(&list(&book_row, "delta"));
    assert_eq!(delta_sources.len(), alive_items.len());
    let at = alive_items
        .column_names()
        .iter()
        .position(|name| *name == "srcuuids")
        .unwrap();
    let mut columns = alive_items.columns().to_vec();
    columns[at] = delta_sources;
    let stated_items = StructArray::try_new(
        alive_items.fields().clone(),
        columns,
        alive_items.nulls().cloned(),
    )
    .unwrap();
    let (field, offsets, _, nulls) = alive.into_parts();
    let stated_alive = ListArray::try_new(field, offsets, Arc::new(stated_items), nulls).unwrap();
    let mut replaced = book_row.columns().to_vec();
    let schema = book_row.schema();
    replaced[schema.index_of("alive").unwrap()] = Arc::new(stated_alive);
    replaced[schema.index_of("srcuuids").unwrap()] =
        Arc::clone(order_row.column_by_name("srcuuids").unwrap());
    let stating = RecordBatch::try_new(schema, replaced).unwrap();
    check(read_all(stating)[0].as_book_event().expect("a book"));

    // The delta and the events laid out of the book rows keep every source.
    let table = Serie::from_arrow_reader(
        None,
        MarketData::arrow_reader(written[..1].to_vec(), None, None).unwrap(),
        ArrowCastOptions::new(),
    )
    .unwrap();
    let laid_out = |serie: StreamChunkedSerie| {
        MarketData::from_arrow_reader(serie.into_arrow_reader())
            .unwrap()
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
    };
    let delta = laid_out(MarketData::delta_serie(table.clone(), None).unwrap());
    assert_eq!(sources_of(delta.iter().collect()), [source(70), source(71)]);
    let events = laid_out(MarketData::events_serie(table, None).unwrap());
    assert_eq!(sources_of(events.iter().collect()), [source(72)]);
}

/// An execution follows the order it fills across kinds, through the facts
/// both hold: it follows the same place rule as within one kind, keeps its
/// own kind and digests as an execution; a trade still follows nothing of
/// another variant.
#[test]
fn an_operation_event_follows_one_of_another_kind_through_their_facts() {
    crate::install::installed();
    let first = MarketData::from(order(1_000_000, "O-1"));
    let fill = execution(2_000_000, "O-1");
    let followed = MarketData::from(fill.clone())
        .with_previous(&first)
        .expect("an execution follows the order it fills");
    let leaf = followed.as_execution_event().expect("the kind is kept");
    assert_eq!(leaf.get_prevuuid(), Some(first.get_uuid()));
    // A later instant keeps its own place.
    assert_eq!(leaf.get_seqnum(), 0);
    assert_eq!(leaf.get_prevpx(), first.get_price());
    assert!(leaf.is_execution());
    assert_ne!(leaf.get_uuid(), fill.get_uuid(), "finalized once more");

    let trade = MarketData::from(
        TradeEvent::from_parts(&order(3_000_000, "T-3"), vec![execution(3_000_000, "E-3")])
            .unwrap(),
    );
    assert!(trade.with_previous(&first).is_none());
}

/// A message is market data as it is, held whole: what a FIX message
/// answers through it is `rust/fix/tests/graph/market_data.rs`'s.
mod message {
    /// The variant holds the message as a trait object, two words wide, so
    /// it adds nothing to the enum's size: `the_enum_is_the_size_of_its_widest_inline_leaf`
    /// pins that size, with this variant among the ones it is held against.
    #[test]
    fn a_held_message_is_a_two_word_pointer() {
        crate::install::installed();
        use std::mem::size_of;
        use yggdryl_market::graph::market_data::MarketMessage;

        assert_eq!(size_of::<Box<dyn MarketMessage>>(), 2 * size_of::<usize>());
    }
}
