//! `rust/src/graph/arrow.rs`: the lifted `marketdata` rows every
//! `MarketData` leaf is written in and read back from.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{
    Array, ArrayRef, BooleanArray, Decimal128Array, Int64Array, ListArray, MapArray, RecordBatch,
    RecordBatchReader as _, StringArray, StructArray, UInt64Array, new_null_array,
};
use arrow_buffer::{OffsetBuffer, ScalarBuffer};
use arrow_schema::{Fields, Schema};
use smol_str::SmolStr;
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::graph::book::ENTRY_ID;
use yggdryl::graph::{
    BookEvent, BookRef, BookSide, Element, Event, EventColumn, Execution, ExecutionEvent, Market,
    MarketColumn, MarketData, MarketKind, MdUpdateAction, Operation, OperationColumn,
    OperationEvent, OperationKind, Order, OrderEvent, Quote, QuoteEvent, SnapshotEvent,
    SnapshotPartition, TradeEvent,
};
use yggdryl::{ArrowCastOptions, Ccy, Decimal, Field, Limit, Scalar, Serie, Side, State, Unit};

/// One operation as a market-data message states it, finalized.
fn operation<K: OperationKind>(
    unix: i64,
    code: &str,
    side: &str,
    state: &str,
) -> OperationEvent<K> {
    let mut operation = OperationEvent::<K>::at(unix);
    operation.set_crosscode(code.to_owned());
    operation.set_seqnum(u64::try_from(unix).unwrap());
    operation.set_creaunix(Some(unix - 3));
    operation.set_execunix(Some(unix - 2));
    operation.set_recdunix(Some(unix - 1));
    operation.set_price(Some(Decimal::from_int(100 + unix)));
    operation.set_quantity(Some(Decimal::from_int(10 + unix)));
    operation.set_currency(Ccy::new("USD").unwrap());
    operation.set_unit(Unit::new("share").unwrap());
    operation.set_side(Side::read(side).unwrap());
    operation.set_ticker(Some(SmolStr::new("ACME")));
    operation.set_state(State::read(state).unwrap());
    operation.insert_altid(ENTRY_ID, code).unwrap();
    operation
        .insert_altid("ORDERID", &format!("ORDER-{code}"))
        .unwrap();
    operation.insert_accountid("ACCOUNT", "ACC-1").unwrap();
    operation.finalize();
    operation
}

fn order(unix: i64, code: &str) -> OrderEvent {
    operation(unix, code, "Buy", "New")
}

fn quote(unix: i64, code: &str) -> QuoteEvent {
    operation(unix, code, "Sell", "New")
}

fn execution(unix: i64, code: &str, side: &str) -> ExecutionEvent {
    operation(unix, code, side, "Filled")
}

/// An entry with its typed book control.
fn entry(unix: i64, code: &str, action: MdUpdateAction) -> QuoteEvent {
    let mut entry = quote(unix, code).with_book(BookRef {
        action: Some(action),
        scope: Some(SmolStr::new("Symbol=ACME")),
        position: Some(1),
        entry_px: Some(Decimal::from_int(100 + unix)),
        entry_size: Some(Decimal::from_int(10 + unix)),
    });
    entry.finalize();
    entry
}

fn trade(unix: i64, code: &str) -> TradeEvent {
    let mut root = OrderEvent::at(unix);
    root.set_crosscode(code.to_owned());
    root.set_ticker(Some(SmolStr::new("ACME")));
    root.set_state(State::read("Filled").unwrap());
    root.finalize();
    TradeEvent::from_parts(
        &root,
        vec![
            execution(unix, &format!("{code}-SELL"), "Sell"),
            execution(unix, &format!("{code}-BUY"), "Buy"),
        ],
    )
    .unwrap()
}

fn snapshot(unix: i64, code: &str) -> SnapshotEvent {
    let mut event = OrderEvent::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_ticker(Some(SmolStr::new("ACME")));
    event.finalize();
    SnapshotEvent::snapshot(&event, Some(SmolStr::new("Symbol=ACME")))
}

fn book(unix: i64) -> BookEvent {
    let mut book = BookEvent::new(unix, "ACME");
    book.add_operations([
        MarketData::from(order(unix, &format!("O-{unix}"))),
        MarketData::from(quote(unix, &format!("Q-{unix}"))),
        MarketData::from(execution(unix, &format!("E-{unix}"), "Buy")),
    ])
    .unwrap();
    book
}

/// A book whose last change was a full snapshot of one scope, so it
/// remembers the partition it replaced.
fn snapshot_book(unix: i64) -> BookEvent {
    let mut book = BookEvent::new(unix, "ACME");
    book.add_operations([
        MarketData::from(entry(unix, &format!("Q-{unix}"), MdUpdateAction::Snapshot)),
        MarketData::from(snapshot(unix, &format!("W-{unix}"))),
    ])
    .unwrap();
    book
}

/// An undated operation, finalized.
fn element<K: OperationKind>(unix: i64, code: &str) -> yggdryl::graph::OperationElement<K> {
    let mut element = operation::<K>(unix, code, "Buy", "New").into_element();
    element.finalize();
    element
}

/// One of every leaf.
fn every_leaf() -> Vec<MarketData> {
    vec![
        MarketData::from(element::<yggdryl::graph::OrderKind>(1, "O-1")),
        MarketData::from(element::<yggdryl::graph::QuoteKind>(2, "Q-2")),
        MarketData::from(element::<yggdryl::graph::ExecutionKind>(3, "E-3")),
        MarketData::from(book(4).bid().clone()),
        MarketData::from(order(5, "O-5")),
        MarketData::from(entry(6, "Q-6", MdUpdateAction::Change)),
        MarketData::from(execution(7, "E-7", "Buy")),
        MarketData::from(trade(8, "T-8")),
        MarketData::from(book(9)),
        MarketData::from(snapshot_book(10)),
        MarketData::from(BookEvent::new(11, "EMPTY")),
        MarketData::from(snapshot(12, "W-12")),
    ]
}

fn read(source: BatchReader) -> yggdryl::Result<Vec<MarketData>> {
    MarketData::from_arrow_reader(source)?.collect()
}

fn written(values: Vec<MarketData>) -> RecordBatch {
    let mut encoded = MarketData::arrow_reader(values, None, None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    assert!(encoded.next().is_none());
    batch
}

fn refusal(batch: RecordBatch) -> String {
    let source = batch_reader(batch.schema(), [batch]);
    let mut decoded = MarketData::from_arrow_reader(source).unwrap();
    let error = decoded.next().unwrap().unwrap_err().to_string();
    assert!(decoded.next().is_none(), "the stream fuses after {error}");
    error
}

/// `batch` with its column `name` replaced, the field retyped to the new
/// column's type.
fn with_column(batch: &RecordBatch, name: &str, column: ArrayRef) -> RecordBatch {
    let schema = batch.schema();
    let at = schema.index_of(name).unwrap();
    let mut fields: Vec<arrow_schema::Field> = schema
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    fields[at] = fields[at]
        .clone()
        .with_data_type(column.data_type().clone());
    let mut columns = batch.columns().to_vec();
    columns[at] = column;
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
}

/// One order of `side` stating `price` - `None` a market order - and
/// `quantity`, its lanes filled from those facts alone.
fn resting(unix: i64, code: &str, side: &str, price: Option<i64>, quantity: i64) -> MarketData {
    let mut entry: OrderEvent = operation(unix, code, side, "New");
    entry.set_price(price.map(Decimal::from_int));
    entry.set_quantity(Some(Decimal::from_int(quantity)));
    entry.set_bid(None);
    entry.set_ask(None);
    entry.finalize();
    MarketData::from(entry)
}

/// A book of two priced levels and one unpriced entry on each side.
fn deep_book(unix: i64) -> BookEvent {
    let mut book = BookEvent::new(unix, "ACME");
    book.add_operations([
        resting(unix, "B-1", "Buy", Some(101), 3),
        resting(unix, "B-2", "Buy", Some(100), 2),
        resting(unix, "B-M", "Buy", None, 5),
        resting(unix, "A-1", "Sell", Some(102), 1),
        resting(unix, "A-2", "Sell", Some(103), 4),
        resting(unix, "A-M", "Sell", None, 6),
    ])
    .unwrap();
    book
}

/// The `name` child of the struct column `column` of `batch`.
fn child_of(batch: &RecordBatch, column: &str, name: &str) -> ArrayRef {
    let parent = batch.column_by_name(column).unwrap();
    let parent = parent.as_any().downcast_ref::<StructArray>().unwrap();
    Arc::clone(parent.column_by_name(name).unwrap())
}

/// `batch` with the `name` child of its struct column `column` replaced.
fn with_child(batch: &RecordBatch, column: &str, name: &str, child: ArrayRef) -> RecordBatch {
    let parent = batch.column_by_name(column).unwrap();
    let parent = parent.as_any().downcast_ref::<StructArray>().unwrap();
    with_column(
        batch,
        column,
        Arc::new(replace_struct_child(parent, name, child)),
    )
}

/// The limits one `limits` list cell states, read through the landed
/// column and [`Limit::from_scalar`].
fn limits_of(list: ArrayRef, row: usize) -> Vec<Limit> {
    let serie = Serie::from_arrow_array(None, list, ArrowCastOptions::new()).unwrap();
    serie
        .scalar(row)
        .unwrap()
        .sequence_rows()
        .unwrap()
        .iter()
        .map(|item| Limit::from_scalar(item).unwrap())
        .collect()
}

/// `list` - a `limits` list - with its items' `name` child replaced by
/// `child`, that child's field nullable where `nullable` says so: the
/// shape a foreign writer may state.
fn with_limit_child(list: &ArrayRef, name: &str, child: ArrayRef, nullable: bool) -> ArrayRef {
    let list = list.as_any().downcast_ref::<ListArray>().unwrap();
    let items = list
        .values()
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let item = match list.data_type() {
        arrow_schema::DataType::List(item) => Arc::clone(item),
        other => panic!("expected a list, got {other}"),
    };
    let fields: Fields = items
        .fields()
        .iter()
        .map(|field| {
            if field.name() == name {
                Arc::new(field.as_ref().clone().with_nullable(nullable))
            } else {
                Arc::clone(field)
            }
        })
        .collect();
    let columns = items
        .fields()
        .iter()
        .zip(items.columns())
        .map(|(field, column)| {
            if field.name() == name {
                Arc::clone(&child)
            } else {
                Arc::clone(column)
            }
        })
        .collect();
    let items = StructArray::new(fields, columns, None);
    let item = item
        .as_ref()
        .clone()
        .with_data_type(items.data_type().clone());
    Arc::new(ListArray::new(
        Arc::new(item),
        list.offsets().clone(),
        Arc::new(items),
        list.nulls().cloned(),
    ))
}

fn decimals(values: &[i64]) -> ArrayRef {
    Arc::new(
        Decimal128Array::from(
            values
                .iter()
                .map(|value| Decimal::from_int(*value).units())
                .collect::<Vec<_>>(),
        )
        .with_precision_and_scale(Decimal::PRECISION, Decimal::SCALE)
        .unwrap(),
    )
}

/// `array` - a struct - without its `name` child.
fn without_struct_child(array: &StructArray, name: &str) -> StructArray {
    let (fields, columns): (Vec<_>, Vec<_>) = array
        .fields()
        .iter()
        .zip(array.columns())
        .filter(|(field, _)| field.name() != name)
        .map(|(field, column)| (Arc::clone(field), Arc::clone(column)))
        .unzip();
    StructArray::new(Fields::from(fields), columns, array.nulls().cloned())
}

fn replace_struct_child(array: &StructArray, name: &str, child: ArrayRef) -> StructArray {
    let index = array
        .fields()
        .iter()
        .position(|field| field.name() == name)
        .unwrap();
    let mut fields: Vec<arrow_schema::FieldRef> = array.fields().iter().cloned().collect();
    fields[index] = Arc::new(
        fields[index]
            .as_ref()
            .clone()
            .with_data_type(child.data_type().clone()),
    );
    let mut columns = array.columns().to_vec();
    columns[index] = child;
    StructArray::new(Fields::from(fields), columns, array.nulls().cloned())
}

#[test]
fn the_field_is_the_kind_then_every_fact_then_the_nested_columns() {
    let field = MarketData::field().unwrap();
    let names: Vec<&str> = field.fields().iter().map(Field::name).collect();
    assert_eq!(names.len(), 1 + 16 + 19 + 8 + 5 + 3 + 7);
    assert_eq!(names[0], "kind");
    assert!(!field.fields()[0].is_nullable());
    assert_eq!(names[1], "currunix");
    assert!(
        field.fields()[1..17].iter().all(Field::is_nullable),
        "an undated leaf states no clock"
    );
    assert_eq!(names[17], "price");
    assert_eq!(names[36], "marketoperationid");
    assert_eq!(
        names[44..49],
        [
            "mdupdateaction",
            "bookscope",
            "mdentrypositionno",
            "mdentrypx",
            "mdentrysize"
        ]
    );
    assert_eq!(names[49..52], ["spread", "crossed", "locked"]);
    assert!(field.fields()[49..].iter().all(Field::is_nullable));
    assert_eq!(
        names[52..],
        [
            "executions",
            "bidside",
            "askside",
            "snapshotpartitions",
            "live",
            "deltas",
            "limits"
        ]
    );
    // A side row: its six element facts, the market, and its three lists.
    let side = &field.fields()[53];
    let side: Vec<&str> = side.fields().iter().map(Field::name).collect();
    assert_eq!(side.len(), 6 + 19 + 3);
    assert_eq!(side[25..], ["live", "deltas", "limits"]);
}

#[test]
fn every_leaf_round_trips_in_bounded_batches() {
    let expected = every_leaf();
    let mut encoded = MarketData::arrow_reader(expected.clone(), Some(5), None).unwrap();
    assert_eq!(
        encoded.schema(),
        MarketData::field().unwrap().into_arrow_schema().unwrap()
    );
    let batches: Vec<RecordBatch> = encoded.by_ref().map(Result::unwrap).collect();
    assert_eq!(
        batches
            .iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [5, 5, 2]
    );
    let kinds = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(kinds.value(0), "order");
    assert_eq!(kinds.value(3), "book_side");
    assert_eq!(kinds.value(4), "order_event");

    let actual = read(batch_reader(batches[0].schema(), batches)).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [
            MarketKind::Order,
            MarketKind::Quote,
            MarketKind::Execution,
            MarketKind::BookSide,
            MarketKind::OrderEvent,
            MarketKind::QuoteEvent,
            MarketKind::ExecutionEvent,
            MarketKind::TradeEvent,
            MarketKind::BookEvent,
            MarketKind::BookEvent,
            MarketKind::BookEvent,
            MarketKind::SnapshotEvent,
        ]
    );
    let entry = actual[5].as_quote_event().unwrap();
    assert_eq!(entry.action(), Some(MdUpdateAction::Change));
    assert_eq!(entry.scope(), "Symbol=ACME");
    assert_eq!(entry.get_altids().get(ENTRY_ID), Some("Q-6"));
    assert_eq!(entry.get_accountids().get("ACCOUNT"), Some("ACC-1"));
    assert_eq!(actual[7].as_trade_event().unwrap().executions().len(), 2);
    let book = actual[8].as_book_event().unwrap();
    assert_eq!(book.bid().live().count(), 1);
    assert_eq!(book.executions().len(), 1);
    assert_eq!(
        actual[9].as_book_event().unwrap().snapshot_partitions(),
        &BTreeSet::from([SnapshotPartition {
            symbol: Some(SmolStr::new("ACME")),
            scope: SmolStr::new("Symbol=ACME"),
        }])
    );
    let control = actual[11].as_snapshot_event().unwrap();
    assert_eq!(control.book().action, Some(MdUpdateAction::Snapshot));
    assert_eq!(control.book().scope.as_deref(), Some("Symbol=ACME"));
}

#[test]
fn an_undated_leaf_states_no_clock() {
    let batch = written(vec![MarketData::from(
        element::<yggdryl::graph::OrderKind>(1, "O-1"),
    )]);
    for column in ["currunix", "creaunix", "prevuuid", "seqnum", "state"] {
        assert_eq!(
            batch.column_by_name(column).unwrap().null_count(),
            1,
            "{column}"
        );
    }
    for column in [
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "price",
    ] {
        assert_eq!(
            batch.column_by_name(column).unwrap().null_count(),
            0,
            "{column}"
        );
    }
}

/// What a FIX lifecycle writes - the event, market and operation columns in
/// its own order, beside columns of its own - reads as operation events once
/// a `kind` column names them: a foreign column is ignored, a column of
/// another castable type is cast, the book columns are simply absent.
#[test]
fn a_lifecycle_shaped_batch_reads_into_operation_events() {
    let expected = vec![
        MarketData::from(order(1, "O-1")),
        MarketData::from(execution(2, "E-2", "Sell")),
        MarketData::from(order(3, "O-3")),
    ];
    let facts: Vec<(Field, Vec<Scalar>)> = EventColumn::ALL
        .into_iter()
        .map(|column| {
            let cells = expected
                .iter()
                .map(|value| match value {
                    MarketData::OrderEvent(event) => column.fact(event),
                    MarketData::ExecutionEvent(event) => column.fact(event),
                    _ => unreachable!(),
                })
                .map(|cell| cell.unwrap_or(Scalar::Null))
                .collect();
            (column.field().unwrap(), cells)
        })
        .chain(MarketColumn::ALL.into_iter().map(|column| {
            let cells = expected
                .iter()
                .map(|value| column.fact(value).unwrap_or(Scalar::Null))
                .collect();
            (column.field().unwrap(), cells)
        }))
        .chain(OperationColumn::ALL.into_iter().map(|column| {
            let cells = expected
                .iter()
                .map(|value| match value {
                    MarketData::OrderEvent(event) => column.fact(event),
                    MarketData::ExecutionEvent(event) => column.fact(event),
                    _ => unreachable!(),
                })
                .map(|cell| cell.unwrap_or(Scalar::Null))
                .collect();
            (column.field().unwrap(), cells)
        }))
        .collect();
    let mut fields = vec![arrow_schema::Field::new(
        "msgtype",
        arrow_schema::DataType::Utf8,
        true,
    )];
    let mut columns: Vec<ArrayRef> = vec![Arc::new(StringArray::from(vec!["D", "8", "D"]))];
    // The lifecycle's own order: the facts reversed.
    for (field, cells) in facts.into_iter().rev() {
        if field.name() == "seqnum" {
            // A place written as a signed count: cast, not refused.
            fields.push(arrow_schema::Field::new(
                "SeqNum",
                arrow_schema::DataType::Int64,
                true,
            ));
            columns.push(Arc::new(Int64Array::from(vec![1, 2, 3])));
            continue;
        }
        let array = Serie::from_scalars(field.clone(), cells)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        fields.push(field.into_arrow_field().unwrap());
        columns.push(array);
    }
    fields.push(arrow_schema::Field::new(
        "Kind",
        arrow_schema::DataType::Utf8,
        true,
    ));
    columns.push(Arc::new(StringArray::from(vec![
        "order_event",
        "EXECUTION_EVENT",
        "Order_Event",
    ])));
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn a_batch_with_no_kind_is_refused_at_its_first_row() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let schema = batch.schema();
    let kept: Vec<usize> = (1..schema.fields().len()).collect();
    let batch = batch.project(&kept).unwrap();
    let error = refusal(batch);
    assert!(error.contains("$[0].kind"), "{error}");
    assert!(error.contains("got null"), "{error}");
}

#[test]
fn an_unknown_or_null_kind_is_named_then_the_stream_fuses() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let error = refusal(with_column(
        &batch,
        "kind",
        Arc::new(StringArray::from(vec!["auction"])),
    ));
    assert!(error.contains("$[0].kind"), "{error}");
    assert!(error.contains("auction"), "{error}");
}

#[test]
fn a_dated_leaf_requires_its_instant() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let schema = batch.schema();
    let without = schema.index_of("currunix").unwrap();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| *at != without)
        .collect();
    let error = refusal(batch.project(&kept).unwrap());
    assert!(error.contains("$[0].currunix"), "{error}");
}

#[test]
fn a_trade_requires_its_executions() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let error = refusal(with_column(
        &batch,
        "kind",
        Arc::new(StringArray::from(vec!["trade_event"])),
    ));
    assert!(error.contains("$[0].executions"), "{error}");
}

#[test]
fn two_columns_naming_one_fact_are_refused_before_a_batch() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let mut fields: Vec<arrow_schema::Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    let mut columns = batch.columns().to_vec();
    fields.push(arrow_schema::Field::new(
        "CURRUNIX",
        arrow_schema::DataType::Utf8,
        true,
    ));
    columns.push(Arc::new(StringArray::from(vec!["later"])));
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
    let error = MarketData::from_arrow_reader(batch_reader(batch.schema(), [batch]))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("CURRUNIX"), "{error}");
}

#[test]
fn decoding_refuses_identity_facts_not_derived_from_the_row() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let error = refusal(with_column(
        &batch,
        "currhashcode",
        Arc::new(UInt64Array::from(vec![u64::MAX])),
    ));
    assert!(error.contains("$[0].currhashcode"), "{error}");
}

/// An identity column that stands must state its identity: a null one is
/// refused, where an absent column states nothing.
#[test]
fn decoding_refuses_a_null_identity_fact() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    for name in ["curruuid", "crossuuid", "currhashcode", "crosshashcode"] {
        let held = batch.column_by_name(name).unwrap();
        let error = refusal(with_column(
            &batch,
            name,
            new_null_array(held.data_type(), 1),
        ));
        assert!(error.contains(&format!("$[0].{name}")), "{error}");
        assert!(error.contains("non-null identity"), "{error}");
    }
    let schema = batch.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| schema.field(*at).name() != "curruuid")
        .collect();
    let expected = MarketData::from(order(1, "O-1"));
    let batch = batch.project(&kept).unwrap();
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(actual, [expected]);
}

#[test]
fn decoding_locates_an_invalid_typed_code() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let error = refusal(with_column(
        &batch,
        "miccode",
        Arc::new(StringArray::from(vec![Some("ABCDE")])),
    ));
    assert!(error.contains("miccode"), "{error}");
}

#[test]
fn book_decoding_locates_an_invalid_nested_typed_code() {
    let batch = written(vec![MarketData::from(book(10))]);
    let bid_column = batch.schema().index_of("bidside").unwrap();
    let bid = batch.column(bid_column);
    let bid = bid.as_any().downcast_ref::<StructArray>().unwrap();
    let live = bid
        .column_by_name("live")
        .unwrap()
        .as_any()
        .downcast_ref::<ListArray>()
        .unwrap();
    let operations = live
        .values()
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let invalid_codes =
        Arc::new(StringArray::from(vec![Some("ABCDE"); operations.len()])) as ArrayRef;
    let operations = replace_struct_child(operations, "miccode", invalid_codes);
    let item = match live.data_type() {
        arrow_schema::DataType::List(item) => Arc::clone(item),
        other => panic!("expected a list, got {other}"),
    };
    let live = ListArray::new(
        item,
        live.offsets().clone(),
        Arc::new(operations),
        live.nulls().cloned(),
    );
    let bid = replace_struct_child(bid, "live", Arc::new(live));
    let error = refusal(with_column(&batch, "bidside", Arc::new(bid)));
    assert!(error.contains("bidside.live[0].miccode"), "{error}");
}

#[test]
fn book_decoding_refuses_a_serialized_summary_that_disagrees_with_live_depth() {
    let batch = written(vec![MarketData::from(book(10))]);
    let error = refusal(with_column(
        &batch,
        "price",
        Arc::new(
            Decimal128Array::from(vec![Decimal::from_int(999).units()])
                .with_precision_and_scale(Decimal::PRECISION, Decimal::SCALE)
                .unwrap(),
        ),
    ));
    assert!(error.contains("$[0].price"), "{error}");
}

/// A null cell states nothing: a book row whose price column is null reads
/// the price its live depth derives.
#[test]
fn a_null_cell_states_nothing() {
    let expected = MarketData::from(book(10));
    let batch = written(vec![expected.clone()]);
    let price = batch.column_by_name("price").unwrap();
    let batch = with_column(&batch, "price", new_null_array(price.data_type(), 1));
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(actual, [expected]);
}

/// A price or a quantity an operation does not state is a null cell, and a
/// null cell read back is none stated.
#[test]
fn an_operation_stating_no_price_or_quantity_round_trips_as_null() {
    let mut unstated = ExecutionEvent::at(5);
    unstated.set_crosscode("E-5".to_owned());
    unstated.set_ticker(Some(SmolStr::new("ACME")));
    unstated.set_side(Side::read("Buy").unwrap());
    unstated.set_state(State::read("Filled").unwrap());
    unstated.set_lastpx(Some(Decimal::from_int(105)));
    unstated.set_lastqty(Some(Decimal::from_int(15)));
    unstated.finalize();
    assert_eq!(
        (unstated.get_price(), unstated.get_quantity()),
        (None, None)
    );
    let expected = MarketData::from(unstated);
    let batch = written(vec![expected.clone()]);
    for name in ["price", "quantity"] {
        assert_eq!(
            batch.column_by_name(name).unwrap().null_count(),
            1,
            "{name}"
        );
    }
    for name in ["lastpx", "lastqty"] {
        assert_eq!(
            batch.column_by_name(name).unwrap().null_count(),
            0,
            "{name}"
        );
    }
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(actual, [expected]);
}

#[test]
fn encoding_respects_row_and_byte_bounds_without_pulling_ahead() {
    struct Counted {
        pulled: Arc<AtomicUsize>,
        next: i64,
    }

    impl Iterator for Counted {
        type Item = MarketData;

        fn next(&mut self) -> Option<Self::Item> {
            (self.next < 3).then(|| {
                self.pulled.fetch_add(1, Ordering::SeqCst);
                self.next += 1;
                MarketData::from(order(self.next, &format!("O-{}", self.next)))
            })
        }
    }

    let pulled = Arc::new(AtomicUsize::new(0));
    let mut reader = MarketData::arrow_reader(
        Counted {
            pulled: Arc::clone(&pulled),
            next: 0,
        },
        Some(100),
        Some(1),
    )
    .unwrap();
    assert_eq!(pulled.load(Ordering::SeqCst), 0);
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    assert_eq!(pulled.load(Ordering::SeqCst), 1);
}

#[test]
fn encoding_yields_a_completed_prefix_before_a_source_error() {
    let source = vec![
        Ok(MarketData::from(order(1, "O-1"))),
        Err(yggdryl::Error::InvalidRecord {
            path: "$[1]".into(),
            reason: "broken source".into(),
        }),
    ];
    let mut reader = MarketData::arrow_reader(source, Some(2), None).unwrap();
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("broken source"), "{error}");
    assert!(reader.next().is_none());
}

#[test]
fn encoding_yields_a_completed_prefix_before_a_located_identity_error() {
    let mut invalid = order(2, "O-2");
    invalid.set_currhashcode(u64::MAX);
    let mut reader = MarketData::arrow_reader(
        [MarketData::from(order(1, "O-1")), MarketData::from(invalid)],
        Some(2),
        None,
    )
    .unwrap();
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$[1].curruuid"), "{error}");
    assert!(reader.next().is_none());
}

#[test]
fn book_encoding_refuses_a_stale_summary_at_the_source_row() {
    let mut invalid = book(10);
    invalid.set_price(Some(Decimal::from_int(999)));
    let mut encoded = MarketData::arrow_reader([MarketData::from(invalid)], Some(1), None).unwrap();
    let error = encoded.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$[0].price"), "{error}");
    assert!(encoded.next().is_none());
}

#[test]
fn book_encoding_refuses_root_lifecycle_bounds_that_contradict_nested_operations() {
    fn refused(invalid: BookEvent, path: &str) {
        let error = MarketData::arrow_reader([MarketData::from(invalid)], Some(1), None)
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(error.contains(path), "{error}");
    }

    let mut invalid = book(10);
    invalid.set_seqnum(0);
    refused(invalid, "live[0].seqnum");

    let mut invalid = book(10);
    invalid.set_creaunix(Some(8));
    refused(invalid, "live[0].creaunix");
}

#[test]
fn undated_leaves_of_each_kind_are_distinct_rows() {
    let order = element::<yggdryl::graph::OrderKind>(1, "X-1");
    let quote: Quote = element(1, "X-1");
    let execution: Execution = element(1, "X-1");
    assert_ne!(order.get_curruuid(), quote.get_curruuid());
    let expected = vec![
        MarketData::from(order.clone()),
        MarketData::from(quote),
        MarketData::from(execution),
    ];
    let batch = written(expected.clone());
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(actual, expected);
    let _: &Order = actual[0].as_order().unwrap();
    assert_eq!(actual[0].as_order(), Some(&order));
}

#[test]
fn a_batch_of_foreign_columns_alone_reads_nothing_until_a_row_needs_a_kind() {
    let schema = Arc::new(Schema::new(vec![arrow_schema::Field::new(
        "msgtype",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let empty = RecordBatch::new_empty(Arc::clone(&schema));
    assert!(
        read(batch_reader(Arc::clone(&schema), [empty]))
            .unwrap()
            .is_empty()
    );
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(vec!["D"])) as ArrayRef],
    )
    .unwrap();
    let error = refusal(batch);
    assert!(error.contains("$[0].kind"), "{error}");
}

#[test]
fn a_book_round_trips_its_limits_lanes_and_facts() {
    let book = deep_book(10);
    let expected = vec![
        MarketData::from(book.clone()),
        MarketData::from(book.bid().clone()),
    ];
    let batch = written(expected.clone());
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        expected
    );

    // Each side row states its limits, best first and the unpriced one last.
    let bid = limits_of(child_of(&batch, "bidside", "limits"), 0);
    assert_eq!(bid, book.bid().limits().collect::<Vec<_>>());
    assert_eq!(
        bid.iter()
            .map(|limit| (limit.price, limit.quantity, limit.uuids.len()))
            .collect::<Vec<_>>(),
        [
            (Some(Decimal::from_int(101)), Decimal::from_int(3), 1),
            (Some(Decimal::from_int(100)), Decimal::from_int(2), 1),
            (None, Decimal::from_int(5), 1),
        ]
    );
    let live: Vec<_> = book.bid().live().map(Element::get_curruuid).collect();
    assert_eq!(
        bid.iter()
            .flat_map(|limit| limit.uuids.clone())
            .collect::<Vec<_>>(),
        live
    );
    let ask = limits_of(child_of(&batch, "askside", "limits"), 0);
    assert_eq!(ask, book.ask().limits().collect::<Vec<_>>());
    assert_eq!(ask.last().unwrap().price, None);

    // A book row states no limits of its own; a side row at the root does.
    let limits = batch.column_by_name("limits").unwrap();
    assert!(limits.is_null(0) && !limits.is_null(1));
    assert_eq!(limits_of(Arc::clone(limits), 1), bid);

    // The book's lanes state its bests, and nothing else.
    let price = child_of(&batch, "bid", "price");
    let price = price.as_any().downcast_ref::<Decimal128Array>().unwrap();
    assert_eq!(price.value(0), Decimal::from_int(101).units());
    let quantity = child_of(&batch, "ask", "quantity");
    let quantity = quantity.as_any().downcast_ref::<Decimal128Array>().unwrap();
    assert_eq!(quantity.value(0), Decimal::from_int(1).units());
    assert!(child_of(&batch, "bid", "currency").is_null(0));
    assert!(batch.column_by_name("bid").unwrap().is_null(1));

    // The three facts the book derives from its bests.
    let spread = batch.column_by_name("spread").unwrap();
    let spread = spread.as_any().downcast_ref::<Decimal128Array>().unwrap();
    assert_eq!(spread.value(0), Decimal::from_int(1).units());
    assert!(spread.is_null(1));
    for name in ["crossed", "locked"] {
        let column = batch.column_by_name(name).unwrap();
        let column = column.as_any().downcast_ref::<BooleanArray>().unwrap();
        assert!(!column.value(0), "{name}");
        assert!(column.is_null(1), "{name}");
    }
}

#[test]
fn a_stated_limit_that_differs_is_refused_on_its_row() {
    let batch = written(vec![MarketData::from(deep_book(10))]);
    let limits = child_of(&batch, "bidside", "limits");
    let error = refusal(with_child(
        &batch,
        "bidside",
        "limits",
        with_limit_child(&limits, "quantity", decimals(&[3, 2, 6]), false),
    ));
    assert!(error.contains("$[0].bidside.limits"), "{error}");

    // A null quantity, where a foreign schema lets one stand, is refused by
    // the cast under the required field, naming its path.
    let quantity = Arc::new(
        Decimal128Array::from(vec![
            Some(Decimal::from_int(3).units()),
            None,
            Some(Decimal::from_int(5).units()),
        ])
        .with_precision_and_scale(Decimal::PRECISION, Decimal::SCALE)
        .unwrap(),
    );
    let error = refusal(with_child(
        &batch,
        "bidside",
        "limits",
        with_limit_child(&limits, "quantity", quantity, true),
    ));
    assert!(
        error.contains("required Arrow field $.bidside.limits[].quantity holds 1 null values"),
        "{error}"
    );
}

#[test]
fn a_stated_spread_that_differs_is_refused() {
    let batch = written(vec![MarketData::from(deep_book(10))]);
    let error = refusal(with_column(&batch, "spread", decimals(&[2])));
    assert!(error.contains("$[0].spread"), "{error}");
    for name in ["crossed", "locked"] {
        let error = refusal(with_column(
            &batch,
            name,
            Arc::new(BooleanArray::from(vec![true])),
        ));
        assert!(error.contains(&format!("$[0].{name}")), "{error}");
    }
}

#[test]
fn a_stated_bid_lane_that_differs_is_refused() {
    let batch = written(vec![MarketData::from(deep_book(10))]);
    let error = refusal(with_child(&batch, "bid", "price", decimals(&[100])));
    assert!(error.contains("$[0].bid"), "{error}");
    let error = refusal(with_child(&batch, "ask", "quantity", decimals(&[5])));
    assert!(error.contains("$[0].ask"), "{error}");
}

#[test]
fn a_null_limits_cell_states_nothing() {
    let expected = vec![MarketData::from(deep_book(10))];
    let batch = written(expected.clone());
    let limits = child_of(&batch, "bidside", "limits");
    let mut batch = with_child(
        &batch,
        "bidside",
        "limits",
        new_null_array(limits.data_type(), limits.len()),
    );
    for name in ["spread", "crossed", "locked", "bid", "ask"] {
        let held = batch.column_by_name(name).unwrap();
        batch = with_column(&batch, name, new_null_array(held.data_type(), 1));
    }
    assert_eq!(
        read(batch_reader(batch.schema(), [batch])).unwrap(),
        expected
    );
}

/// A batch written before the book facts and the limits existed reads the
/// same values: every new column is nullable where it stands, so the cast
/// fills the missing ones with nulls and a null states nothing.
#[test]
fn a_batch_without_the_new_columns_still_reads() {
    let book = deep_book(10);
    let expected = vec![
        MarketData::from(book.clone()),
        MarketData::from(book.ask().clone()),
    ];
    let batch = written(expected.clone());
    let schema = batch.schema();
    let added = ["spread", "crossed", "locked", "limits"];
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| !added.contains(&schema.field(*at).name().as_str()))
        .collect();
    let mut batch = batch.project(&kept).unwrap();
    for side in ["bidside", "askside"] {
        let held = batch.column_by_name(side).unwrap();
        let held = held.as_any().downcast_ref::<StructArray>().unwrap();
        let without = without_struct_child(held, "limits");
        let at = batch.schema().index_of(side).unwrap();
        let mut fields: Vec<arrow_schema::Field> = batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.as_ref().clone())
            .collect();
        fields[at] =
            arrow_schema::Field::new(side, without.data_type().clone(), fields[at].is_nullable());
        let mut columns = batch.columns().to_vec();
        columns[at] = Arc::new(without);
        batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
    }
    assert_eq!(batch.schema().fields().len(), 1 + 16 + 19 + 8 + 5 + 6);
    assert_eq!(
        read(batch_reader(batch.schema(), [batch])).unwrap(),
        expected
    );
}

#[test]
fn metadata_round_trips_and_a_stated_entry_that_differs_is_refused() {
    let metadata = |value: &str| {
        Some(
            [(SmolStr::new("k"), SmolStr::new(value))]
                .into_iter()
                .collect(),
        )
    };
    let carrying = |value: &str| {
        let mut order = order(1, "O-1");
        order.set_metadata(metadata(value));
        order.finalize();
        let mut side: BookSide = deep_book(2).bid().clone();
        side.set_metadata(metadata(value));
        side.finalize();
        let mut book = deep_book(3);
        book.set_metadata(metadata(value));
        book.finalize();
        vec![
            MarketData::from(order),
            MarketData::from(side),
            MarketData::from(book),
        ]
    };
    let expected = carrying("v");
    let batch = written(expected.clone());
    let actual = read(batch_reader(batch.schema(), [batch.clone()])).unwrap();
    assert_eq!(actual, expected);
    for value in &actual {
        assert_eq!(
            value.get_metadata().get("k").map(SmolStr::as_str),
            Some("v"),
            "{:?}",
            value.kind()
        );
    }

    // Metadata is read, never derived, so a stated entry that differs from
    // the one a row's identity was derived over is refused at that identity.
    let other = written(carrying("w"));
    let error = refusal(with_column(
        &batch,
        "metadata",
        Arc::clone(other.column_by_name("metadata").unwrap()),
    ));
    assert!(error.contains("$[0].curruuid"), "{error}");

    // A key stated twice has no one reading, and the landing refuses it on
    // its row before any fact is read.
    let held = batch.column_by_name("metadata").unwrap();
    let arrow_schema::DataType::Map(entries, sorted) = held.data_type() else {
        panic!("expected a map, got {}", held.data_type());
    };
    let arrow_schema::DataType::Struct(children) = entries.data_type() else {
        panic!("expected struct entries, got {}", entries.data_type());
    };
    let twice = MapArray::try_new(
        Arc::clone(entries),
        OffsetBuffer::new(ScalarBuffer::from(vec![0_i32, 2, 3, 4])),
        StructArray::new(
            children.clone(),
            vec![
                Arc::new(StringArray::from(vec!["k", "k", "k", "k"])),
                Arc::new(StringArray::from(vec!["v", "w", "v", "v"])),
            ],
            None,
        ),
        None,
        *sorted,
    )
    .unwrap();
    let error = refusal(with_column(&batch, "metadata", Arc::new(twice)));
    assert!(error.contains("$[0]"), "{error}");
    assert!(error.contains("duplicate key"), "{error}");
}
