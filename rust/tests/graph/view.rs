//! `rust/src/graph/view.rs`: the named readings of a `marketdata` stream,
//! each one plan over the rows every leaf is written in.

use arrow_array::{
    Array, ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray, TimestampNanosecondArray,
};
use smol_str::SmolStr;
use yggdryl::arrow::BatchReader;
use yggdryl::graph::{
    BookEvent, Element, Event, ExecutionEvent, Market, MarketData, MarketView, OperationEvent,
    OperationKind, OrderEvent, QuoteEvent, TradeEvent,
};
use yggdryl::{Decimal, Field, FieldPath, Plan, SecType, SecurityId, Side, State};

/// The nested columns of the root row: what a flat view drops. The nested
/// `limits` column is named whether or not the root holds it, and a name
/// the root does not hold excludes nothing.
const NESTED: [&str; 7] = [
    "executions",
    "bidside",
    "askside",
    "snapshotpartitions",
    "live",
    "deltas",
    "limits",
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
    operation.set_price(Some(Decimal::from_int(100 + unix)));
    operation.set_quantity(Some(Decimal::from_int(10 + unix)));
    operation.set_side(Side::read(side).unwrap());
    operation.set_ticker(Some(SmolStr::new("ACME")));
    operation.set_state(State::read(state).unwrap());
    operation.finalize();
    operation
}

/// An order that states an ISIN among its security identifiers.
fn identified_order(unix: i64, code: &str) -> OrderEvent {
    let mut order: OrderEvent = operation(unix, code, "Buy", "New");
    order
        .insert_securityid(SecurityId::new(SecType::read("ISIN").unwrap(), ISIN).unwrap())
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
    root.set_ticker(Some(SmolStr::new("ACME")));
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

/// A book of one order, one quote and one execution.
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
        order.set_quantity(Some(Decimal::from_int(i64::try_from(1 + index).unwrap())));
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
    assert_eq!(MarketView::ALL.len(), 7);
    for (spelling, expected) in [
        ("orders", MarketView::Orders),
        ("QUOTES", MarketView::Quotes),
        ("Executions", MarketView::Executions),
        ("trades", MarketView::Trades),
        ("book_sides", MarketView::BookSides),
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
    let error = MarketView::read("book-sides", None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("book_sides"), "{error}");
}

#[test]
fn every_plan_is_built_as_its_text_reads_back() {
    let lifts: Vec<FieldPath> = vec!["securityids['ISIN'] as isin".parse().unwrap()];
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
            "select * exclude ({nested}), securityids['ISIN'] as isin \
             where kind in ('order', 'order_event')"
        )
    );
    assert_eq!(
        MarketData::plan(&MarketView::Trades, &[])
            .unwrap()
            .to_string(),
        format!(
            "select * exclude ({nested}), unnest(executions) as execution \
             where kind = 'trade_event'"
        )
    );
    assert_eq!(
        MarketData::plan(&MarketView::BookSides, &[])
            .unwrap()
            .to_string(),
        "select currunix, snapunix, unnest([bidside, askside]) as side where kind = 'book_event'"
    );
    assert_eq!(
        MarketData::plan(&MarketView::Books, &[])
            .unwrap()
            .to_string(),
        "select * exclude (executions, live, deltas, limits) where kind = 'book_event'"
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
        format!("select * exclude ({nested}) where crosscode = 'C-1' order by currunix")
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
    for (target, kinds, rows) in [
        (MarketView::Orders, ["order", "order_event"], 4),
        (MarketView::Quotes, ["quote", "quote_event"], 2),
        (MarketView::Executions, ["execution", "execution_event"], 1),
    ] {
        let out = view(&target, &[]);
        assert_eq!(names(&out), flat(), "{target}");
        assert_eq!(out.num_rows(), rows, "{target}");
        assert!(
            texts(column(&out, "kind"))
                .iter()
                .all(|kind| kinds.contains(&kind.unwrap())),
            "{target}"
        );
    }
    let out = view(&MarketView::Quotes, &[]);
    assert_eq!(
        texts(column(&out, "kind")),
        [Some("quote_event"), Some("quote")]
    );
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
            Some("T-9"),
            Some("T-9"),
            Some("T-9"),
            Some("T-12"),
            Some("T-12")
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
        texts(column(&out, "execution.kind"))
            .iter()
            .all(|kind| kind.is_some())
    );
}

#[test]
fn a_book_is_two_side_rows_beside_its_clocks() {
    let out = view(&MarketView::BookSides, &[]);
    let mut expected = vec!["currunix".to_owned(), "snapunix".to_owned()];
    expected.extend(prefixed("bidside", "side"));
    assert_eq!(names(&out), expected);
    assert_eq!(out.num_rows(), 2 * 2, "two sides per book");
    let books = [book(11), book(13)];
    let clocks: Vec<Option<i64>> = books
        .iter()
        .flat_map(|book| [Some(book.get_currunix()), Some(book.get_currunix())])
        .collect();
    assert_eq!(instants(column(&out, "currunix")), clocks);
    let snaps: Vec<Option<i64>> = books
        .iter()
        .flat_map(|book| [book.get_snapunix(), book.get_snapunix()])
        .collect();
    assert_eq!(instants(column(&out, "snapunix")), snaps);
    // Bid then ask, book by book.
    let sides: Vec<Option<Vec<u8>>> = books
        .iter()
        .flat_map(|book| {
            [
                Some(book.bid().get_curruuid().into_bytes().to_vec()),
                Some(book.ask().get_curruuid().into_bytes().to_vec()),
            ]
        })
        .collect();
    assert_eq!(uuids(column(&out, "side.curruuid")), sides);

    let out = view(&MarketView::Books, &[]);
    let expected: Vec<String> = MarketData::field()
        .unwrap()
        .fields()
        .iter()
        .map(|field| field.name().to_owned())
        .filter(|name| !["executions", "live", "deltas", "limits"].contains(&name.as_str()))
        .collect();
    assert_eq!(names(&out), expected);
    assert_eq!(out.num_rows(), 2);
    assert!(
        texts(column(&out, "kind"))
            .iter()
            .all(|kind| *kind == Some("book_event"))
    );
}

#[test]
fn a_lifecycle_is_one_chain_in_the_order_it_happened() {
    let out = view(
        &MarketView::Lifecycle {
            crosscode: SmolStr::new("C-1"),
        },
        &[],
    );
    assert_eq!(names(&out), flat());
    assert_eq!(
        instants(column(&out, "currunix")),
        [Some(10), Some(20), Some(30)]
    );
    let current = uuids(column(&out, "curruuid"));
    let previous = uuids(column(&out, "prevuuid"));
    assert_eq!(previous[0], None, "the first element follows nothing");
    assert_eq!(previous[1], current[0]);
    assert_eq!(previous[2], current[1]);
    let [first, second, third] = chain("C-1");
    assert_eq!(
        current,
        [first, second, third].map(|order| Some(order.get_curruuid().into_bytes().to_vec()))
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
        crosscode: SmolStr::new("C-9"),
    };
    let out = drained(MarketData::apply_view(&target, &[], stream).unwrap()).unwrap();
    let current = uuids(column(&out, "curruuid"));
    assert_eq!(
        current,
        chain
            .iter()
            .map(|order| Some(order.get_curruuid().into_bytes().to_vec()))
            .collect::<Vec<_>>()
    );
    let previous = uuids(column(&out, "prevuuid"));
    assert_eq!(previous[0], None, "the first leaf follows nothing");
    for leaf in 1..chain.len() {
        assert_eq!(previous[leaf], current[leaf - 1], "leaf {leaf}");
    }
}

#[test]
fn a_lift_reads_one_key_of_a_root_column_and_null_where_it_is_missing() {
    let lifts: Vec<FieldPath> = vec![
        "securityids['ISIN'] as isin".parse().unwrap(),
        "securityids['CUSIP'] as cusip".parse().unwrap(),
        "securityids['WKN'] as wkn".parse().unwrap(),
    ];
    let out = view(&MarketView::Orders, &lifts);
    let mut expected = flat();
    expected.extend(["isin".to_owned(), "cusip".to_owned(), "wkn".to_owned()]);
    assert_eq!(names(&out), expected);
    let codes = texts(column(&out, "crosscode"));
    let isins = texts(column(&out, "isin"));
    for (code, isin) in codes.iter().zip(&isins) {
        let expected = (*code == Some("O-5")).then_some(ISIN);
        assert_eq!(*isin, expected, "{code:?}");
    }
    // A United States ISIN states its CUSIP, which the set derives; no
    // order states a WKN.
    let cusips = texts(column(&out, "cusip"));
    for (code, cusip) in codes.iter().zip(&cusips) {
        let expected = (*code == Some("O-5")).then_some(&ISIN[2..11]);
        assert_eq!(*cusip, expected, "{code:?}");
    }
    assert_eq!(column(&out, "wkn").null_count(), out.num_rows());
    // The key is read as it is stored: another case is another key.
    let lower: Vec<FieldPath> = vec!["securityids['isin'] as isin".parse().unwrap()];
    let out = view(&MarketView::Orders, &lower);
    assert_eq!(column(&out, "isin").null_count(), out.num_rows());
    // A lift reads the parent row beside a flattened one.
    let out = view(&MarketView::Trades, &lifts[..1]);
    assert_eq!(names(&out).last().map(String::as_str), Some("isin"));
    assert_eq!(column(&out, "isin").null_count(), out.num_rows());
    let out = view(&MarketView::BookSides, &lifts[..1]);
    assert_eq!(names(&out).last().map(String::as_str), Some("isin"));

    // A lift naming a column the root does not hold is refused where the
    // plan binds.
    let missing: Vec<FieldPath> = vec!["nothing['ISIN'] as isin".parse().unwrap()];
    let error = MarketData::apply_view(&MarketView::Orders, &missing, stream())
        .and_then(drained)
        .unwrap_err()
        .to_string();
    assert!(error.contains("nothing"), "{error}");
    // Two columns of one name are refused, a lift over a kept column too.
    let twice: Vec<FieldPath> = vec!["securityids['ISIN'] as kind".parse().unwrap()];
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
    assert!(
        uuids(column(&out, "curruuid")).contains(&Some(quote.get_curruuid().into_bytes().to_vec()))
    );
}
