//! `rust/src/graph/view.rs`: the named readings of a `marketdata` stream,
//! each one plan over the rows every leaf is written in.

use arrow_array::{
    Array, ArrayRef, FixedSizeBinaryArray, ListArray, RecordBatch, StringArray,
    TimestampNanosecondArray,
};
use arrow_buffer::NullBuffer;
use smol_str::SmolStr;
use std::sync::Arc;
use yggdryl::IdKey;
use yggdryl::arrow::BatchReader;
use yggdryl::graph::{
    BookEvent, BookIterator, Element, Event, ExecutionEvent, Market, MarketData, MarketView,
    Operation, OperationEvent, OperationKind, OrderEvent, OrderKind, QuoteEvent, SnapshotEvent,
    TradeEvent,
};
use yggdryl::{Decimal, Field, FieldPath, IdType, Identifier, MarketDataKind, Plan, Side, State};

/// The nested columns of the root row: what a flat view drops.
const NESTED: [&str; 6] = [
    "alive",
    "delta",
    "events",
    "executions",
    "bidlimits",
    "asklimits",
];

const ISIN: &str = "US0378331005";

/// One operation, finalized, dated at `unix` under `code`.
fn operation<K: OperationKind>(
    unix: i64,
    code: &str,
    side: &str,
    state: &str,
) -> OperationEvent<K> {
    let mut operation = OperationEvent::<K>::at(unix);
    operation.set_crosscode(code.to_owned());
    operation.set_seqnum(u64::try_from(unix).unwrap());
    operation.set_price(Some(Decimal::from_int(100 + unix)), true);
    operation.set_quantity(Some(Decimal::from_int(10 + unix)), true);
    operation.set_side(Side::read(side).unwrap(), true);
    operation.set_ticker(Some(SmolStr::new("ACME")), true);
    operation.set_state(State::read(state).unwrap());
    operation.finalize();
    operation
}

/// An order that states an ISIN among its security identifiers.
fn identified_order(unix: i64, code: &str) -> OrderEvent {
    let mut order: OrderEvent = operation(unix, code, "Buy", "New");
    order
        .insert_securityid(Identifier::new(IdKey::base(IdType::Isin), ISIN).unwrap())
        .unwrap();
    order.finalize();
    order
}

fn execution(unix: i64, code: &str, side: &str) -> ExecutionEvent {
    operation(unix, code, side, "Filled")
}

/// A trade of `executions` executions.
fn trade(unix: i64, code: &str, executions: usize) -> TradeEvent {
    let mut root = OrderEvent::at(unix);
    root.set_crosscode(code.to_owned());
    root.set_ticker(Some(SmolStr::new("ACME")), true);
    root.set_state(State::read("Filled").unwrap());
    root.finalize();
    let parts = (0..executions)
        .map(|index| {
            let side = if index % 2 == 0 { "Sell" } else { "Buy" };
            execution(unix, &format!("{code}-{index}"), side)
        })
        .collect();
    TradeEvent::from_parts(&root, parts).unwrap()
}

/// A book of one order and one quote.
fn book(unix: i64) -> BookEvent {
    let mut book = BookEvent::new(unix, "ACME");
    book.add_operations([
        MarketData::from(operation::<yggdryl::graph::OrderKind>(
            unix,
            &format!("O-{unix}"),
            "Buy",
            "New",
        )),
        MarketData::from(operation::<yggdryl::graph::QuoteKind>(
            unix,
            &format!("Q-{unix}"),
            "Sell",
            "New",
        )),
    ])
    .unwrap();
    book
}

/// One element's chain of three orders under `code`, the predecessor of
/// each named by its `prevuuid`.
fn chain(code: &str) -> [OrderEvent; 3] {
    let first: OrderEvent = operation(10, code, "Buy", "New");
    let mut second = operation::<yggdryl::graph::OrderKind>(20, code, "Buy", "New")
        .with_previous(&first)
        .unwrap();
    second.finalize();
    let mut third = operation::<yggdryl::graph::OrderKind>(30, code, "Buy", "Filled")
        .with_previous(&second)
        .unwrap();
    third.finalize();
    [first, second, third]
}

/// One element's chain of `leaves` orders under `code`, each naming its
/// predecessor by its `prevuuid`: every leaf at one instant but the last,
/// which follows a tick later.
fn tied_chain(code: &str, leaves: usize) -> Vec<OrderEvent> {
    let mut chain: Vec<OrderEvent> = Vec::with_capacity(leaves);
    for index in 0..leaves {
        let unix = if index + 1 == leaves { 20 } else { 10 };
        let mut order: OrderEvent = operation(unix, code, "Buy", "New");
        order.set_quantity(
            Some(Decimal::from_int(i64::try_from(1 + index).unwrap())),
            true,
        );
        order.finalize();
        let order = match chain.last() {
            Some(previous) => order.with_previous(previous).unwrap(),
            None => order,
        };
        chain.push(order);
    }
    chain
}

/// Every kind of leaf, the chain's out of order.
fn leaves() -> Vec<MarketData> {
    let [first, second, third] = chain("C-1");
    let undated = |unix, code: &str| {
        let mut element =
            operation::<yggdryl::graph::QuoteKind>(unix, code, "Sell", "New").into_element();
        element.finalize();
        element
    };
    vec![
        MarketData::from(third),
        MarketData::from(identified_order(5, "O-5")),
        MarketData::from(operation::<yggdryl::graph::QuoteKind>(
            6, "Q-6", "Sell", "New",
        )),
        MarketData::from(undated(7, "Q-7")),
        MarketData::from(execution(8, "E-8", "Buy")),
        MarketData::from(trade(9, "T-9", 3)),
        MarketData::from(first),
        MarketData::from(book(11)),
        MarketData::from(trade(12, "T-12", 2)),
        MarketData::from(book(13)),
        MarketData::from(second),
    ]
}

fn stream() -> BatchReader {
    MarketData::arrow_reader(leaves(), None, None).unwrap()
}

fn drained(reader: BatchReader) -> yggdryl::Result<RecordBatch> {
    let schema = reader.schema();
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(yggdryl::arrow::from_reader_error)?);
    }
    Ok(arrow_select::concat::concat_batches(&schema, &batches).unwrap())
}

fn view(target: &MarketView, lifts: &[FieldPath]) -> RecordBatch {
    drained(MarketData::apply_view(target, lifts, stream()).unwrap())
        .unwrap_or_else(|error| panic!("{target}: {error}"))
}

fn names(batch: &RecordBatch) -> Vec<String> {
    batch
        .schema_ref()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect()
}

fn column<'batch>(batch: &'batch RecordBatch, name: &str) -> &'batch ArrayRef {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name} in {:?}", names(batch)))
}

/// The categories an `int32` `marketdatakind` column states.
fn kinds(array: &ArrayRef) -> Vec<Option<MarketDataKind>> {
    array
        .as_any()
        .downcast_ref::<arrow_array::UInt8Array>()
        .unwrap()
        .iter()
        .map(|code| code.and_then(MarketDataKind::from_code))
        .collect()
}

fn texts(array: &ArrayRef) -> Vec<Option<&str>> {
    array
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .iter()
        .collect()
}

fn uuids(array: &ArrayRef) -> Vec<Option<Vec<u8>>> {
    array
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap()
        .iter()
        .map(|uuid| uuid.map(<[u8]>::to_vec))
        .collect()
}

fn instants(array: &ArrayRef) -> Vec<Option<i64>> {
    array
        .as_any()
        .downcast_ref::<TimestampNanosecondArray>()
        .unwrap()
        .iter()
        .collect()
}

/// The root's columns a flat view keeps, as the field names them.
fn flat() -> Vec<String> {
    MarketData::field()
        .unwrap()
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .filter(|name| !NESTED.contains(&name.as_str()))
        .collect()
}

/// The children of one nested column's item, each under `prefix`.
fn prefixed(nested: &str, prefix: &str) -> Vec<String> {
    let root = MarketData::field().unwrap();
    let column = root
        .fields()
        .iter()
        .find(|field| field.name() == nested)
        .unwrap();
    let item = column
        .dtype()
        .serie_item()
        .map_or_else(|| column.clone(), Clone::clone);
    item.fields()
        .iter()
        .map(|field| format!("{prefix}.{}", field.name()))
        .collect()
}

#[test]
fn a_view_is_read_by_its_spelling_and_only_the_lifecycle_takes_a_crosscode() {
    assert_eq!(MarketView::ALL.len(), 6);
    for (spelling, expected) in [
        ("orders", MarketView::Orders),
        ("QUOTES", MarketView::Quotes),
        ("Executions", MarketView::Executions),
        ("trades", MarketView::Trades),
        ("Books", MarketView::Books),
    ] {
        let read = MarketView::read(spelling, None).unwrap();
        assert_eq!(read, expected, "{spelling}");
        assert!(read.as_str().eq_ignore_ascii_case(spelling));
        assert!(MarketView::ALL.contains(&read.as_str()));
        let error = MarketView::read(spelling, Some("C-1"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("crosscode"), "{spelling}: {error}");
    }
    assert_eq!(
        MarketView::read("LifeCycle", Some("C-1")).unwrap(),
        MarketView::Lifecycle {
            crosscode: SmolStr::new("C-1")
        }
    );
    let error = MarketView::read("lifecycle", None).unwrap_err().to_string();
    assert!(error.contains("crosscode"), "{error}");
    // A book side is no leaf, and no view reads one.
    let error = MarketView::read("book_sides", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("books, lifecycle"), "{error}");
}

#[test]
fn every_plan_is_built_as_its_text_reads_back() {
    let lifts: Vec<FieldPath> = vec!["securityids['isin'] as isin".parse().unwrap()];
    for spelling in MarketView::ALL {
        let view = MarketView::read(spelling, (spelling == "lifecycle").then_some("C-1")).unwrap();
        for lifted in [&[][..], &lifts[..]] {
            let plan = MarketData::plan(&view, lifted).unwrap();
            let text = plan.to_string();
            assert_eq!(text.parse::<Plan>().unwrap(), plan, "{text}");
            assert_eq!(text.parse::<Plan>().unwrap().to_string(), text);
            assert_eq!(lifted.is_empty(), !text.contains("isin"), "{text}");
        }
    }
    let nested = NESTED.join(", ");
    assert_eq!(
        MarketData::plan(&MarketView::Orders, &lifts)
            .unwrap()
            .to_string(),
        format!(
            "select * exclude ({nested}), securityids['isin'] as isin \
             where marketdatakind = 'ORDR'"
        )
    );
    assert_eq!(
        MarketData::plan(&MarketView::Trades, &[])
            .unwrap()
            .to_string(),
        format!(
            "select * exclude ({nested}), unnest(executions) as execution \
             where marketdatakind = 'TRAD'"
        )
    );
    assert_eq!(
        MarketData::plan(&MarketView::Books, &[])
            .unwrap()
            .to_string(),
        "select * exclude (executions) where marketdatakind = 'BOOK' \
         and (delta is not null or events is not null)"
    );
    assert_eq!(
        MarketData::plan(
            &MarketView::Lifecycle {
                crosscode: SmolStr::new("C-1")
            },
            &[]
        )
        .unwrap()
        .to_string(),
        format!("select * exclude ({nested}) where crosscode = 'C-1' order by transunix")
    );
}

#[test]
fn applying_a_view_is_applying_its_plan() {
    for spelling in MarketView::ALL {
        let view = MarketView::read(spelling, (spelling == "lifecycle").then_some("C-1")).unwrap();
        let viewed = drained(MarketData::apply_view(&view, &[], stream()).unwrap()).unwrap();
        let planned = drained(
            MarketData::plan(&view, &[])
                .unwrap()
                .apply_arrow_reader(stream())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(viewed, planned, "{spelling}");
    }
}

#[test]
fn an_operation_view_keeps_its_two_kinds_and_every_flat_column() {
    for (target, kind, rows) in [
        (MarketView::Orders, MarketDataKind::Order, 4),
        (MarketView::Quotes, MarketDataKind::Quotation, 2),
        (MarketView::Executions, MarketDataKind::Execution, 1),
    ] {
        let out = view(&target, &[]);
        assert_eq!(names(&out), flat(), "{target}");
        assert_eq!(out.num_rows(), rows, "{target}");
        assert!(
            kinds(column(&out, "marketdatakind"))
                .iter()
                .all(|held| *held == Some(kind)),
            "{target}"
        );
    }
    // One category covers a leaf dated and undated: the quote event, then
    // the undated quote, which states no instant.
    let out = view(&MarketView::Quotes, &[]);
    let dated: Vec<bool> = (0..out.num_rows())
        .map(|row| !column(&out, "transunix").is_null(row))
        .collect();
    assert_eq!(dated, [true, false]);
}

#[test]
fn a_trade_is_one_row_per_execution_its_own_columns_beside_it() {
    let out = view(&MarketView::Trades, &[]);
    let mut expected = flat();
    expected.extend(prefixed("executions", "execution"));
    assert_eq!(names(&out), expected);
    assert_eq!(out.num_rows(), 3 + 2, "one row per execution of each trade");
    assert_eq!(
        texts(column(&out, "crosscode")),
        [
            Some("21:0:T-9"),
            Some("21:0:T-9"),
            Some("21:0:T-9"),
            Some("21:0:T-12"),
            Some("21:0:T-12")
        ]
    );
    // Each trade's executions in the order the trade holds them.
    let held: Vec<Option<String>> = [trade(9, "T-9", 3), trade(12, "T-12", 2)]
        .iter()
        .flat_map(|trade| {
            trade
                .executions()
                .iter()
                .map(|execution| Some(execution.get_crosscode().to_owned()))
                .collect::<Vec<_>>()
        })
        .collect();
    let read: Vec<Option<String>> = texts(column(&out, "execution.crosscode"))
        .into_iter()
        .map(|code| code.map(str::to_owned))
        .collect();
    assert_eq!(read, held);
    assert!(
        kinds(column(&out, "execution.marketdatakind"))
            .iter()
            .all(|kind| *kind == Some(MarketDataKind::Execution))
    );
}

#[test]
fn a_book_is_one_row_its_alive_entries_delta_and_events_kept_nested() {
    let out = view(&MarketView::Books, &[]);
    let expected: Vec<String> = MarketData::field()
        .unwrap()
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .filter(|name| name != "executions")
        .collect();
    assert_eq!(names(&out), expected);
    assert_eq!(out.num_rows(), 2);
    assert!(
        kinds(column(&out, "marketdatakind"))
            .iter()
            .all(|kind| *kind == Some(MarketDataKind::Book))
    );
    let books = [book(11), book(13)];
    assert_eq!(
        instants(column(&out, "transunix")),
        books.map(|book| Some(book.get_transunix()))
    );
    // Each book's one bid and one ask, alive and applied at its instant,
    // and no other event recorded there.
    for (name, held) in [("alive", [2, 2]), ("delta", [2, 2]), ("events", [0, 0])] {
        let list = column(&out, name)
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        assert_eq!(
            (0..out.num_rows())
                .map(|row| list.value_length(row))
                .collect::<Vec<_>>(),
            held,
            "{name}"
        );
    }
}

/// The books view keeps every book a walk emits - with no grid each a
/// delta book, its `alive` cell null, the instant that recorded only an
/// execution an event-only one - and a row stating its `events` alone, its
/// `delta` cell null, and drops a snapshot control, which states none of
/// the three lists.
#[test]
fn the_books_view_keeps_a_delta_book_and_an_event_only_row_and_drops_a_snapshot_control() {
    let mut values = BookIterator::new(
        [
            MarketData::from(operation::<OrderKind>(20, "O-20", "Buy", "New")),
            MarketData::from(operation::<OrderKind>(21, "O-21", "Buy", "New")),
            MarketData::from(execution(22, "E-22", "Buy")),
        ]
        .into_iter(),
        0,
    )
    .unwrap()
    .map(|book| book.map(MarketData::from))
    .collect::<yggdryl::Result<Vec<_>>>()
    .unwrap();
    assert_eq!(
        values
            .iter()
            .map(|book| {
                let book = book.as_book_event().unwrap();
                (book.is_complete(), book.delta().len(), book.events().len())
            })
            .collect::<Vec<_>>(),
        [(false, 1, 0), (false, 1, 0), (false, 0, 1)]
    );
    let mut control = OrderEvent::at(23);
    control.set_crosscode("W-23".to_owned());
    control.set_ticker(Some(SmolStr::new("ACME")), true);
    control.finalize();
    values.push(MarketData::from(SnapshotEvent::snapshot(
        &control,
        Some(SmolStr::new("Symbol=ACME")),
    )));
    let written = drained(MarketData::arrow_reader(values, None, None).unwrap()).unwrap();
    for name in ["alive", "delta", "events"] {
        assert!(
            column(&written, name).is_null(3),
            "a control states no {name}"
        );
    }
    let books = |batch: RecordBatch| {
        drained(
            MarketData::apply_view(
                &MarketView::Books,
                &[],
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            )
            .unwrap(),
        )
        .unwrap()
    };

    let out = books(written.clone());
    assert_eq!(
        instants(column(&out, "transunix")),
        [Some(20), Some(21), Some(22)]
    );
    let alive = column(&out, "alive");
    assert_eq!(
        (0..3).map(|row| alive.is_null(row)).collect::<Vec<_>>(),
        [true, true, true]
    );
    assert_eq!(column(&out, "delta").null_count(), 0);
    assert_eq!(lengths(column(&out, "events")), [0, 0, 1]);

    // The event-only row's `delta` cell null: its `events` alone keep it.
    let delta = column(&written, "delta")
        .as_any()
        .downcast_ref::<ListArray>()
        .unwrap();
    assert_eq!(
        delta.value_length(2),
        0,
        "the event-only book's delta is empty"
    );
    let arrow_schema::DataType::List(item) = delta.data_type() else {
        unreachable!("the delta column is a list");
    };
    let nulled = ListArray::new(
        Arc::clone(item),
        delta.offsets().clone(),
        Arc::clone(delta.values()),
        Some(NullBuffer::from(vec![true, true, false, false])),
    );
    let mut columns = written.columns().to_vec();
    columns[written.schema().index_of("delta").unwrap()] = Arc::new(nulled);
    let stated = RecordBatch::try_new(written.schema(), columns).unwrap();
    let out = books(stated);
    assert_eq!(
        instants(column(&out, "transunix")),
        [Some(20), Some(21), Some(22)]
    );
    assert!(column(&out, "delta").is_null(2));
    assert_eq!(lengths(column(&out, "events")), [0, 0, 1]);
}

/// The items each row of a list column holds.
fn lengths(array: &ArrayRef) -> Vec<i32> {
    let list = array.as_any().downcast_ref::<ListArray>().unwrap();
    (0..list.len()).map(|row| list.value_length(row)).collect()
}

#[test]
fn a_lifecycle_is_one_chain_in_the_order_it_happened() {
    let out = view(
        // A chain is read by the cross code it is stored under: its
        // category, its side and its base.
        &MarketView::Lifecycle {
            crosscode: SmolStr::new("10:1:C-1"),
        },
        &[],
    );
    assert_eq!(names(&out), flat());
    assert_eq!(
        instants(column(&out, "transunix")),
        [Some(10), Some(20), Some(30)]
    );
    let current = uuids(column(&out, "uuid"));
    let previous = uuids(column(&out, "prevuuid"));
    assert_eq!(previous[0], None, "the first element follows nothing");
    assert_eq!(previous[1], current[0]);
    assert_eq!(previous[2], current[1]);
    let [first, second, third] = chain("C-1");
    assert_eq!(
        current,
        [first, second, third].map(|order| Some(order.get_uuid().into_bytes().to_vec()))
    );
    // A crosscode no row states is the empty chain.
    let out = view(
        &MarketView::Lifecycle {
            crosscode: SmolStr::new("NOWHERE"),
        },
        &[],
    );
    assert_eq!(out.num_rows(), 0);
    assert_eq!(names(&out), flat());
}

#[test]
fn a_lifecycle_keeps_the_leaves_that_share_an_instant_in_the_order_they_happened() {
    // Thirty-two leaves at one instant, past the twenty rows an unstable
    // sort keeps by chance. The stream states the last leaf first, so the
    // ordering has to move rows, and every tie keeps the order it arrived in.
    let chain = tied_chain("C-9", 33);
    let (last, tied) = chain.split_last().unwrap();
    let stream = MarketData::arrow_reader(
        std::iter::once(last)
            .chain(tied)
            .cloned()
            .map(MarketData::from)
            .collect::<Vec<_>>(),
        None,
        None,
    )
    .unwrap();
    let target = MarketView::Lifecycle {
        crosscode: SmolStr::new("10:1:C-9"),
    };
    let out = drained(MarketData::apply_view(&target, &[], stream).unwrap()).unwrap();
    let current = uuids(column(&out, "uuid"));
    assert_eq!(
        current,
        chain
            .iter()
            .map(|order| Some(order.get_uuid().into_bytes().to_vec()))
            .collect::<Vec<_>>()
    );
    let previous = uuids(column(&out, "prevuuid"));
    assert_eq!(previous[0], None, "the first leaf follows nothing");
    for leaf in 1..chain.len() {
        assert_eq!(previous[leaf], current[leaf - 1], "leaf {leaf}");
    }
}

#[test]
fn a_lift_reads_one_identifier_of_a_root_column_and_null_where_it_is_missing() {
    let lifts: Vec<FieldPath> = vec![
        "securityids['isin'] as isin".parse().unwrap(),
        "securityids['cusip'] as cusip".parse().unwrap(),
        "securityids['derived:cusip'] as derived".parse().unwrap(),
        "metadata['securityids.derived:cusip'] as stated"
            .parse()
            .unwrap(),
        "securityids['wkn'] as wkn".parse().unwrap(),
    ];
    let out = view(&MarketView::Orders, &lifts);
    let mut expected = flat();
    expected.extend([
        "isin".to_owned(),
        "cusip".to_owned(),
        "derived".to_owned(),
        "stated".to_owned(),
        "wkn".to_owned(),
    ]);
    assert_eq!(names(&out), expected);
    let codes = texts(column(&out, "crosscode"));
    let isins = texts(column(&out, "isin"));
    for (code, isin) in codes.iter().zip(&isins) {
        let expected = (*code == Some("10:1:O-5")).then_some(ISIN);
        assert_eq!(*isin, expected, "{code:?}");
    }
    // A United States ISIN states its CUSIP, which the set derives: the cell
    // holds the type's answer under its base key alone, and what derived it
    // is side information in the metadata cell under its map's name,
    // `securityids.derived:cusip`, never a second key of the map; no order
    // states a WKN.
    let cusips = texts(column(&out, "cusip"));
    for (code, cusip) in codes.iter().zip(&cusips) {
        let expected = (*code == Some("10:1:O-5")).then_some(&ISIN[2..11]);
        assert_eq!(*cusip, expected, "{code:?}");
    }
    assert_eq!(column(&out, "derived").null_count(), out.num_rows());
    let stated = texts(column(&out, "stated"));
    for (code, source) in codes.iter().zip(&stated) {
        let expected = (*code == Some("10:1:O-5")).then_some(&ISIN[2..11]);
        assert_eq!(*source, expected, "{code:?}");
    }
    assert_eq!(column(&out, "wkn").null_count(), out.num_rows());
    // The key is read as it is stored, folded lower case: another case is
    // another key.
    let upper: Vec<FieldPath> = vec!["securityids['ISIN'] as isin".parse().unwrap()];
    let out = view(&MarketView::Orders, &upper);
    assert_eq!(column(&out, "isin").null_count(), out.num_rows());
    // A lift reads the parent row beside a flattened one.
    let out = view(&MarketView::Trades, &lifts[..1]);
    assert_eq!(names(&out).last().map(String::as_str), Some("isin"));
    assert_eq!(column(&out, "isin").null_count(), out.num_rows());
    let out = view(&MarketView::Books, &lifts[..1]);
    assert_eq!(names(&out).last().map(String::as_str), Some("isin"));

    // A lift naming a column the root does not hold is refused where the
    // plan binds.
    let missing: Vec<FieldPath> = vec!["nothing['isin'] as isin".parse().unwrap()];
    let error = MarketData::apply_view(&MarketView::Orders, &missing, stream())
        .and_then(drained)
        .unwrap_err()
        .to_string();
    assert!(error.contains("nothing"), "{error}");
    // Two columns of one name are refused, a lift over a kept column too.
    let twice: Vec<FieldPath> = vec!["securityids['isin'] as marketdatakind".parse().unwrap()];
    let error = MarketData::apply_view(&MarketView::Orders, &twice, stream())
        .and_then(drained)
        .unwrap_err()
        .to_string();
    assert!(error.contains("twice"), "{error}");
}

#[test]
fn a_view_publishes_its_schema_before_any_row() {
    let root = MarketData::field().unwrap();
    for spelling in MarketView::ALL {
        let view = MarketView::read(spelling, (spelling == "lifecycle").then_some("C-1")).unwrap();
        let published = MarketData::plan(&view, &[])
            .unwrap()
            .field_from(&root)
            .unwrap();
        let out = view_of(&view);
        let names: Vec<&str> = published.fields().iter().map(Field::name).collect();
        let held: Vec<&str> = out
            .schema_ref()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        assert_eq!(names, held, "{spelling}");
    }
}

fn view_of(target: &MarketView) -> RecordBatch {
    view(target, &[])
}

#[test]
fn a_quote_leaf_is_a_quote_whatever_it_is_dated() {
    // The fixture's undated quote and dated quote are the quotes view.
    let quote: QuoteEvent = operation(6, "Q-6", "Sell", "New");
    let out = view(&MarketView::Quotes, &[]);
    assert!(uuids(column(&out, "uuid")).contains(&Some(quote.get_uuid().into_bytes().to_vec())));
}

/// A lift reaches one identifier of an identifier map by its key - `src:type`,
/// a base key its type alone: the value where the row states it, null where
/// it does not, whichever of the three maps holds it.
#[test]
fn a_lift_by_key_reads_an_identifier_a_party_and_null_where_one_is_absent() {
    let stated = |unix: i64, code: &str, clordid: Option<&str>, account: Option<&str>| {
        let mut order: OrderEvent = operation(unix, code, "Buy", "New");
        if let Some(clordid) = clordid {
            order
                .insert_identifier(Identifier::new(IdKey::base(IdType::ClOrdId), clordid).unwrap())
                .unwrap();
        }
        if let Some(account) = account {
            order
                .insert_partyid(Identifier::new(IdKey::base(IdType::Account), account).unwrap())
                .unwrap();
        }
        order.finalize();
        MarketData::from(order)
    };
    let leaves = vec![
        stated(1, "O-1", Some("C-1"), Some("ACC-1")),
        stated(2, "O-2", None, Some("ACC-2")),
        stated(3, "O-3", Some("C-3"), None),
        stated(4, "O-4", None, None),
    ];
    let lifts: Vec<FieldPath> = vec![
        "identifiers['clordid'] as clordid".parse().unwrap(),
        "partyids['account'] as account".parse().unwrap(),
        "identifiers['orderid'] as orderid".parse().unwrap(),
    ];
    let stream = MarketData::arrow_reader(leaves, None, None).unwrap();
    let out =
        drained(MarketData::apply_view(&MarketView::Orders, &lifts, stream).unwrap()).unwrap();
    let mut expected = flat();
    expected.extend([
        "clordid".to_owned(),
        "account".to_owned(),
        "orderid".to_owned(),
    ]);
    assert_eq!(names(&out), expected);
    assert_eq!(
        texts(column(&out, "clordid")),
        [Some("C-1"), None, Some("C-3"), None]
    );
    assert_eq!(
        texts(column(&out, "account")),
        [Some("ACC-1"), Some("ACC-2"), None, None]
    );
    assert_eq!(column(&out, "orderid").null_count(), out.num_rows());

    // The ISIN is a column of the row as well as a security identifier: the
    // lift by key and the column read the same value.
    let isins: Vec<FieldPath> = vec!["securityids['isin'] as isin".parse().unwrap()];
    let stream = MarketData::arrow_reader(
        vec![
            MarketData::from(identified_order(5, "O-5")),
            MarketData::from(identified_order(6, "O-6")),
        ],
        None,
        None,
    )
    .unwrap();
    let out =
        drained(MarketData::apply_view(&MarketView::Orders, &isins, stream).unwrap()).unwrap();
    assert_eq!(texts(column(&out, "isin")), [Some(ISIN), Some(ISIN)]);
    assert_eq!(texts(column(&out, "isincode")), [Some(ISIN), Some(ISIN)]);
}

/// A lifecycle is one chain, named by the exact stored cross code it is
/// filed under - category, side and base, a quote's side `0` whatever it
/// tags - so the same base under another side or category is another chain,
/// and the bare base or the old `BUYS:` spelling names none.
#[test]
fn a_lifecycle_is_read_by_the_stored_cross_code_alone() {
    let leaves = |unix: i64| {
        vec![
            MarketData::from(operation::<yggdryl::graph::OrderKind>(
                unix, "X-1", "Buy", "New",
            )),
            MarketData::from(operation::<yggdryl::graph::OrderKind>(
                unix + 1,
                "X-1",
                "Sell",
                "New",
            )),
            MarketData::from(operation::<yggdryl::graph::OrderKind>(
                unix + 2,
                "X-1",
                "Unknown",
                "New",
            )),
            MarketData::from(operation::<yggdryl::graph::QuoteKind>(
                unix + 3,
                "X-1",
                "Buy",
                "New",
            )),
            MarketData::from(operation::<yggdryl::graph::ExecutionKind>(
                unix + 4,
                "X-1",
                "Sell",
                "Filled",
            )),
        ]
    };
    let read = |stored: &str| {
        let target = MarketView::Lifecycle {
            crosscode: SmolStr::new(stored),
        };
        let stream = MarketData::arrow_reader(leaves(10), None, None).unwrap();
        let out = drained(MarketData::apply_view(&target, &[], stream).unwrap()).unwrap();
        (
            out.num_rows(),
            texts(column(&out, "crosscode"))
                .into_iter()
                .map(|code| code.map(str::to_owned))
                .collect::<Vec<_>>(),
        )
    };
    for (stored, instant) in [
        ("10:1:X-1", 10),
        ("10:2:X-1", 11),
        ("10:0:X-1", 12),
        ("14:0:X-1", 13),
        ("8:2:X-1", 14),
    ] {
        let (rows, codes) = read(stored);
        assert_eq!(rows, 1, "{stored}");
        assert_eq!(codes, [Some(stored.to_owned())], "{stored}");
        let target = MarketView::Lifecycle {
            crosscode: SmolStr::new(stored),
        };
        let stream = MarketData::arrow_reader(leaves(10), None, None).unwrap();
        let out = drained(MarketData::apply_view(&target, &[], stream).unwrap()).unwrap();
        assert_eq!(
            instants(column(&out, "transunix")),
            [Some(instant)],
            "{stored}"
        );
    }
    for unnamed in ["X-1", "BUYS:X-1", "10:3:X-1", "21:0:X-1"] {
        assert_eq!(read(unnamed).0, 0, "{unnamed}");
    }
}
