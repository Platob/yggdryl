//! `rust/src/graph/arrow.rs`: the lifted `marketdata` rows every
//! `MarketData` leaf is written in and read back from.

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
    BookEvent, BookRef, Element, ElementColumn, Event, EventColumn, Execution, ExecutionEvent,
    FxRates, Market, MarketColumn, MarketData, MarketKind, MdUpdateAction, Operation,
    OperationColumn, OperationEvent, OperationKind, Order, OrderEvent, Quote, QuoteEvent,
    SnapshotEvent, TradeEvent,
};
use yggdryl::{
    ArrowCastOptions, Ccy, Decimal, Field, IdSource, IdType, Identifier, Limit, MarketDataKind,
    Scalar, Serie, Side, State, Unit,
};

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
    operation.set_execunix(Some(unix - 2), true);
    operation.set_recdunix(Some(unix - 1));
    operation.set_price(Some(Decimal::from_int(100 + unix)), true);
    operation.set_quantity(Some(Decimal::from_int(10 + unix)), true);
    operation.set_currency(Ccy::new("USD").unwrap(), true);
    operation.set_unit(Unit::new("share").unwrap(), true);
    operation.set_side(Side::read(side).unwrap(), true);
    operation.set_ticker(Some(SmolStr::new("ACME")), true);
    operation.set_state(State::read(state).unwrap());
    operation
        .insert_identifier(Identifier::new(IdSource::Fix, ENTRY_ID, code).unwrap())
        .unwrap();
    operation
        .insert_identifier(
            Identifier::new(IdSource::Fix, IdType::OrderId, &format!("ORDER-{code}")).unwrap(),
        )
        .unwrap();
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
    root.set_ticker(Some(SmolStr::new("ACME")), true);
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
    event.set_ticker(Some(SmolStr::new("ACME")), true);
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

/// A book whose last change was a full snapshot of one scope: its entry
/// states a walk-time control no row carries.
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
/// `quantity`.
fn resting(unix: i64, code: &str, side: &str, price: Option<i64>, quantity: i64) -> MarketData {
    let mut entry: OrderEvent = operation(unix, code, side, "New");
    entry.set_price(price.map(Decimal::from_int), true);
    entry.set_quantity(Some(Decimal::from_int(quantity)), true);
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

/// The column `name` of `batch`.
fn column_of(batch: &RecordBatch, name: &str) -> ArrayRef {
    Arc::clone(batch.column_by_name(name).unwrap())
}

/// The limits one `bidlimits` or `asklimits` cell states, read through the
/// landed column and [`Limit::from_scalar`].
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

/// `list` - a `bidlimits` or `asklimits` list - with its items' `name`
/// child replaced by `child`, that child's field nullable where `nullable`
/// says so: the shape a foreign writer may state.
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
fn the_field_is_every_fact_in_trait_order_then_the_nested_columns() {
    let field = MarketData::field().unwrap();
    let names: Vec<&str> = field.fields().iter().map(Field::name).collect();
    assert_eq!(names.len(), 6 + 9 + 34 + 5 + 1 + 5);
    // The element's facts, the event's, the market's and the operation's,
    // in the order their traits state them: the columns every generated
    // schema opens with.
    let shared: Vec<&str> = ElementColumn::ALL
        .map(ElementColumn::name)
        .into_iter()
        .chain(EventColumn::ALL.map(EventColumn::name))
        .chain(MarketColumn::ALL.map(MarketColumn::name))
        .chain(OperationColumn::ALL.map(OperationColumn::name))
        .collect();
    assert_eq!(names[..54], shared[..]);
    assert_eq!(shared.len(), 6 + 9 + 34 + 5);
    assert_eq!(names[0], "curruuid");
    assert_eq!(names[6], "currunix");
    assert!(
        field.fields()[..15].iter().all(Field::is_nullable),
        "an undated leaf states no clock, and a root states only its leaf's identity"
    );
    // The category opens the market's facts, and is never absent.
    assert_eq!(names[15], "marketdatakind");
    assert_eq!(
        field.fields()[15].dtype(),
        &yggdryl::DataType::MarketDataKind
    );
    assert!(!field.fields()[15].is_nullable());
    // The type of its kind follows, stated as none where it is none.
    assert_eq!(names[16], "marketdatatype");
    assert_eq!(
        field.fields()[16].dtype(),
        &yggdryl::DataType::MarketDataType
    );
    assert!(!field.fields()[16].is_nullable());
    assert_eq!(
        names[17..24],
        [
            "price",
            "stoppx",
            "currency",
            "quantity",
            "displayqty",
            "hiddenqty",
            "unit"
        ]
    );
    // When an element last executed is a market fact, stated among the
    // market columns rather than the event's.
    assert_eq!(names[28..32], ["miccode", "execunix", "lastpx", "lastqty"]);
    assert_eq!(names[34..37], ["leavesqty", "cxlqty", "prevpx"]);
    assert_eq!(
        names[49..54],
        [
            "ordqty",
            "timeinforce",
            "tradable",
            "identifiers",
            "partyids"
        ]
    );
    // The one book control a row states; the rest is walk-time.
    assert_eq!(names[54], "bookscope");
    for gone in [
        "mdupdateaction",
        "mdentrypositionno",
        "mdentrypx",
        "mdentrysize",
        "spread",
        "crossed",
        "locked",
        "userids",
        "bidside",
        "askside",
        "snapshotpartitions",
        "limits",
        "bid",
        "ask",
    ] {
        assert!(!names.contains(&gone), "{gone}");
    }
    assert!(field.fields()[54..].iter().all(Field::is_nullable));
    assert_eq!(
        names[55..],
        ["alive", "deltas", "executions", "bidlimits", "asklimits"]
    );
    // An operation row, the item of every operation list: nothing nested.
    let item = field.fields()[55].dtype().serie_item().unwrap().clone();
    let item: Vec<&str> = item.fields().iter().map(Field::name).collect();
    assert_eq!(item.len(), 6 + 9 + 34 + 5 + 1);
    assert_eq!(item, names[..55]);
    // A book's two sides are its price levels, one limit each.
    for side in &field.fields()[58..] {
        assert_eq!(side.dtype(), &yggdryl::DataType::serie(Limit::field()));
    }
}

/// The rows every leaf writes, then the rows the leaves read back write.
fn rewritten(values: Vec<MarketData>, rows: usize) -> Vec<RecordBatch> {
    MarketData::arrow_reader(values, Some(rows), None)
        .unwrap()
        .map(Result::unwrap)
        .collect()
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
        [5, 5, 1]
    );
    let kinds = batches[0]
        .column_by_name("marketdatakind")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::UInt8Array>()
        .unwrap();
    assert_eq!(kinds.value(0), MarketDataKind::Order.code());
    assert_eq!(kinds.value(1), MarketDataKind::Quotation.code());
    assert_eq!(kinds.value(2), MarketDataKind::Execution.code());
    assert_eq!(kinds.value(3), MarketDataKind::Order.code());
    assert_eq!(kinds.value(4), MarketDataKind::Quotation.code());

    let actual = read(batch_reader(batches[0].schema(), batches.clone())).unwrap();
    assert_eq!(
        actual.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [
            MarketKind::Order,
            MarketKind::Quote,
            MarketKind::Execution,
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
    // Every fact a row states round trips, and the leaves read back write
    // the same rows; a book control's walk-time facts - an action, a
    // position, an entry's own price and size - are no row fact, so the two
    // leaves stating one read back without them and keep their identity.
    assert_eq!(rewritten(actual.clone(), 5), batches);
    for (index, (read, stated)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(read.get_curruuid(), stated.get_curruuid(), "{index}");
        if index != 4 && index != 8 {
            assert_eq!(read, stated, "{index}");
        }
    }
    let entry = actual[4].as_quote_event().unwrap();
    assert_eq!(entry.action(), None, "an action is walk-time");
    assert_eq!(entry.scope(), "Symbol=ACME");
    assert_eq!(entry.get_identifiers().get(&ENTRY_ID), Some("Q-6"));
    assert_eq!(actual[6].as_trade_event().unwrap().executions().len(), 2);
    let book = actual[7].as_book_event().unwrap();
    assert_eq!(book.alive().count(), 2);
    assert_eq!(book.deltas().count(), 2);
    assert_eq!(book.executions().len(), 1);
    let replaced = actual[8].as_book_event().unwrap();
    assert_eq!(replaced.alive().count(), 1);
    let control = actual[10].as_snapshot_event().unwrap();
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
/// a `marketdatakind` column names them, here as text: a foreign column is
/// ignored, a column of another castable type is cast - the names to their
/// members - and the book columns are simply absent.
#[test]
fn a_lifecycle_shaped_batch_reads_into_operation_events() {
    let expected = vec![
        MarketData::from(order(1, "O-1")),
        MarketData::from(execution(2, "E-2", "Sell")),
        MarketData::from(order(3, "O-3")),
    ];
    let facts: Vec<(Field, Vec<Scalar>)> = ElementColumn::ALL
        .into_iter()
        .map(|column| {
            let cells = expected
                .iter()
                .map(|value| column.fact(value).unwrap_or(Scalar::Null))
                .collect();
            (column.field().unwrap(), cells)
        })
        .chain(EventColumn::ALL.into_iter().map(|column| {
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
        if field.name() == "marketdatakind" {
            // A category written as its names, in any case: cast to members.
            fields.push(arrow_schema::Field::new(
                "MarketDataKind",
                arrow_schema::DataType::Utf8,
                true,
            ));
            columns.push(Arc::new(StringArray::from(vec!["ORDR", "EXEC", "ordr"])));
            continue;
        }
        let array = Serie::from_scalars(field.clone(), cells)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        fields.push(field.into_arrow_field().unwrap());
        columns.push(array);
    }
    let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn a_batch_with_no_kind_is_refused_at_its_first_row() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let schema = batch.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| schema.field(*at).name() != "marketdatakind")
        .collect();
    let batch = batch.project(&kept).unwrap();
    let error = refusal(batch);
    assert!(error.contains("$[0].marketdatakind"), "{error}");
    assert!(
        error.contains("expected ORDR, QUOT, EXEC, TRAD or BOOK, got null"),
        "{error}"
    );
}

#[test]
fn an_unknown_or_null_kind_is_named_then_the_stream_fuses() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let error = refusal(with_column(
        &batch,
        "marketdatakind",
        Arc::new(arrow_array::UInt8Array::from(vec![
            MarketDataKind::Account.code(),
        ])),
    ));
    assert!(error.contains("$[0].marketdatakind"), "{error}");
    assert!(
        error.contains("expected ORDR, QUOT, EXEC, TRAD or BOOK, got ACCT"),
        "{error}"
    );
    // A null category, in a column a reader declared nullable.
    let at = batch.schema().index_of("marketdatakind").unwrap();
    let mut fields: Vec<arrow_schema::Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    fields[at] = fields[at].clone().with_nullable(true);
    let mut columns = batch.columns().to_vec();
    columns[at] = new_null_array(&arrow_schema::DataType::UInt8, 1);
    let nulled = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap();
    let error = refusal(nulled);
    assert!(error.contains("$[0].marketdatakind"), "{error}");
    assert!(error.contains("got null"), "{error}");
}

/// A trade is an event: a `TRAD` row stating no instant names no leaf.
#[test]
fn an_undated_trade_is_refused() {
    let batch = written(vec![MarketData::from(trade(8, "T-8"))]);
    let schema = batch.schema();
    let without = schema.index_of("currunix").unwrap();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| *at != without)
        .collect();
    let error = refusal(batch.project(&kept).unwrap());
    assert!(error.contains("$[0].marketdatakind"), "{error}");
    assert!(
        error.contains("expected a dated TRAD row, got currunix null"),
        "{error}"
    );
}

/// The instant is what dates a leaf: an `ORDR` row without one is an order
/// entry, and with one an order event.
#[test]
fn the_instant_says_whether_a_row_is_an_element_or_an_event() {
    let undated = MarketData::from(element::<yggdryl::graph::OrderKind>(1, "O-1"));
    let dated = MarketData::from(order(1, "O-1"));
    let batch = written(vec![undated.clone(), dated.clone()]);
    let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
    assert_eq!(
        actual.iter().map(MarketData::kind).collect::<Vec<_>>(),
        [MarketKind::Order, MarketKind::OrderEvent]
    );
    assert_eq!(actual, [undated, dated]);
}

/// A `BOOK` row is dated, and a dated one a book where it states its
/// `alive` entries and a snapshot control where the cell is null; a batch
/// that laid out no `alive` column cannot say which.
#[test]
fn a_dated_book_row_is_told_by_its_alive_entries() {
    let batch = written(vec![MarketData::from(book(10))]);
    let schema = batch.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| schema.field(*at).name() != "alive")
        .collect();
    let error = refusal(batch.project(&kept).unwrap());
    assert!(error.contains("$[0].alive"), "{error}");
    assert!(
        error.contains(
            "expected the alive column that tells a book_event from a snapshot_event, got none"
        ),
        "{error}"
    );

    // A book row with its entries nulled reads as a snapshot control, whose
    // identity is not the one the row states.
    let held = batch.column_by_name("alive").unwrap();
    let error = refusal(with_column(
        &batch,
        "alive",
        new_null_array(held.data_type(), 1),
    ));
    assert!(error.contains("$[0].curruuid"), "{error}");

    // An undated `BOOK` row names no leaf.
    let without = schema.index_of("currunix").unwrap();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| *at != without)
        .collect();
    let error = refusal(batch.project(&kept).unwrap());
    assert!(error.contains("$[0].marketdatakind"), "{error}");
    assert!(
        error.contains("expected a dated BOOK row, got currunix null"),
        "{error}"
    );

    // An empty book states its entries, none, and reads back a book.
    let empty = written(vec![MarketData::from(BookEvent::new(11, "EMPTY"))]);
    assert!(!empty.column_by_name("alive").unwrap().is_null(0));
    let actual = read(batch_reader(empty.schema(), [empty])).unwrap();
    assert_eq!(actual[0].kind(), MarketKind::BookEvent);

    let controls = written(vec![MarketData::from(snapshot(12, "W-12"))]);
    assert!(controls.column_by_name("alive").unwrap().is_null(0));
    let actual = read(batch_reader(controls.schema(), [controls])).unwrap();
    assert_eq!(actual[0].kind(), MarketKind::SnapshotEvent);
}

/// `batch` as a table storing a null list as an empty one reads it back -
/// PyIceberg reads a null list of structs back as `[]`: every null cell of
/// every list column an empty list.
fn with_null_lists_emptied(batch: &RecordBatch) -> RecordBatch {
    let columns = batch
        .columns()
        .iter()
        .map(|column| match column.as_any().downcast_ref::<ListArray>() {
            Some(list) if list.null_count() > 0 => {
                let arrow_schema::DataType::List(item) = list.data_type() else {
                    unreachable!("a list array is a list");
                };
                for row in 0..list.len() {
                    if list.is_null(row) {
                        assert_eq!(list.value_length(row), 0, "a null list spans nothing");
                    }
                }
                Arc::new(ListArray::new(
                    Arc::clone(item),
                    list.offsets().clone(),
                    Arc::clone(list.values()),
                    None,
                )) as ArrayRef
            }
            _ => Arc::clone(column),
        })
        .collect();
    let fields: Vec<arrow_schema::Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).unwrap()
}

#[test]
fn every_leaf_reads_back_where_a_table_stores_a_null_list_as_an_empty_one() {
    // A snapshot control states no `alive` entries and a book states its
    // entries, even none; read back empty, a control is told from an empty
    // book by the identity it states - a book's folds in its sides - with
    // or without a scope.
    let mut unscoped = OrderEvent::at(13);
    unscoped.set_crosscode("W-13".to_owned());
    unscoped.finalize();
    let mut expected = every_leaf();
    expected.push(MarketData::from(SnapshotEvent::snapshot(&unscoped, None)));
    let batch = written(expected.clone());
    let emptied = with_null_lists_emptied(&batch);
    assert_ne!(emptied, batch, "some leaf leaves a list null");
    let alive = emptied.column_by_name("alive").unwrap();
    assert_eq!(alive.null_count(), 0);
    let actual = read(batch_reader(emptied.schema(), [emptied.clone()])).unwrap();
    assert_eq!(
        actual.iter().map(MarketData::kind).collect::<Vec<_>>(),
        expected.iter().map(MarketData::kind).collect::<Vec<_>>()
    );
    for (index, (read, stated)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(read.get_curruuid(), stated.get_curruuid(), "{index}");
    }
    assert_eq!(
        actual[11].as_snapshot_event().unwrap().book().scope,
        None,
        "the unscoped control"
    );
}

#[test]
fn a_trade_requires_its_executions() {
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let error = refusal(with_column(
        &batch,
        "marketdatakind",
        Arc::new(arrow_array::UInt8Array::from(vec![
            MarketDataKind::Trade.code(),
        ])),
    ));
    assert!(error.contains("$[0].executions"), "{error}");
}

/// `isincode` is the `isin` of `securityids`, projected: written from it,
/// filling an absent one on the way back, and refused where the two
/// disagree.
#[test]
fn isincode_projects_the_securityids_isin() {
    let mut listed = order(1, "O-1");
    listed
        .insert_securityid(Identifier::new(IdSource::Base, IdType::Isin, "US0378331005").unwrap())
        .unwrap();
    listed.finalize();
    assert_eq!(listed.get_isincode(), Some("US0378331005"));
    let expected = MarketData::from(listed);
    let batch = written(vec![expected.clone()]);
    let cell = batch
        .column_by_name("isincode")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .value(0)
        .to_owned();
    assert_eq!(cell, "US0378331005");
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        std::slice::from_ref(&expected)
    );

    // Alone, it fills the ISIN - and the CUSIP the ISIN carries derives.
    let schema = batch.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| schema.field(*at).name() != "securityids")
        .collect();
    let alone = batch.project(&kept).unwrap();
    let actual = read(batch_reader(alone.schema(), [alone])).unwrap();
    assert_eq!(actual, [expected]);
    assert_eq!(
        actual[0].get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );

    // A second ISIN beside the one `securityids` states is refused there.
    let error = refusal(with_column(
        &batch,
        "isincode",
        Arc::new(StringArray::from(vec!["GB0002634946"])),
    ));
    assert!(error.contains("$[0].isincode"), "{error}");
    assert!(
        error.contains(r#"expected the securityids isin "US0378331005", got "GB0002634946""#),
        "{error}"
    );
}

/// The flat keys of an identifier map column, one list per row, `None`
/// for a null cell.
fn map_keys(batch: &RecordBatch, name: &str) -> Vec<Option<Vec<String>>> {
    let map = column_of(batch, name);
    let map = map.as_any().downcast_ref::<MapArray>().unwrap();
    let keys = map
        .entries()
        .column_by_name("key")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .clone();
    (0..map.len())
        .map(|row| {
            (!map.is_null(row)).then(|| {
                let offsets = map.value_offsets();
                (offsets[row]..offsets[row + 1])
                    .map(|at| keys.value(usize::try_from(at).unwrap()).to_owned())
                    .collect()
            })
        })
        .collect()
}

/// The `src:type=value` of every identifier row an identifier map column
/// holds, flat, read through the item struct's three cells.
fn map_rows(batch: &RecordBatch, name: &str) -> Vec<String> {
    let map = column_of(batch, name);
    let map = map.as_any().downcast_ref::<MapArray>().unwrap();
    let item = map.entries().column(1);
    let item = item.as_any().downcast_ref::<StructArray>().unwrap();
    let cell = |child: &str| {
        item.column_by_name(child)
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .clone()
    };
    let (src, kind, value) = (cell("src"), cell("type"), cell("value"));
    (0..item.len())
        .map(|at| format!("{}:{}={}", src.value(at), kind.value(at), value.value(at)))
        .collect()
}

/// `batch` with the identifier map `name` stating `keys` - one flat list over
/// every row - in place of the keys it was written with, everything else
/// as it was.
fn with_map_keys(batch: &RecordBatch, name: &str, keys: Vec<&str>) -> RecordBatch {
    let held = column_of(batch, name);
    let map = held.as_any().downcast_ref::<MapArray>().unwrap();
    let arrow_schema::DataType::Map(entries, sorted) = held.data_type() else {
        panic!("expected a map, got {}", held.data_type());
    };
    let replaced = MapArray::try_new(
        Arc::clone(entries),
        map.offsets().clone(),
        replace_struct_child(map.entries(), "key", Arc::new(StringArray::from(keys))),
        map.nulls().cloned(),
        *sorted,
    )
    .unwrap();
    with_column(batch, name, Arc::new(replaced))
}

/// The three identifier columns - `securityids`, `identifiers`, `partyids` -
/// are sorted maps from the key `src:type` to the identifier row, written
/// in the order the set holds them, null where a row states none, and read
/// back as the sets they were.
#[test]
fn the_three_identifier_columns_are_sorted_maps_keyed_by_source_and_type() {
    let venue: IdSource = "venue".parse().unwrap();
    let mut stated = order(1, "O-1");
    for id in [
        Identifier::new(IdSource::Fix, IdType::ClOrdId, "C-1").unwrap(),
        Identifier::new(venue.clone(), IdType::OrderId, "V-1").unwrap(),
    ] {
        stated.insert_identifier(id).unwrap();
    }
    for id in [
        Identifier::new(IdSource::Fix, IdType::Account, "ACC-1").unwrap(),
        Identifier::new(IdSource::Base, IdType::Account, "ACC-0").unwrap(),
    ] {
        stated.insert_partyid(id).unwrap();
    }
    for id in [
        Identifier::new(IdSource::Base, IdType::Ric, "AAPL.O").unwrap(),
        Identifier::new(IdSource::Base, IdType::Isin, "US0378331005").unwrap(),
    ] {
        stated.insert_securityid(id).unwrap();
    }
    stated.finalize();
    let bare = order(2, "O-2");
    let expected = vec![MarketData::from(stated), MarketData::from(bare)];
    let batch = written(expected.clone());

    for name in ["securityids", "identifiers", "partyids"] {
        assert!(
            matches!(
                column_of(&batch, name).data_type(),
                arrow_schema::DataType::Map(_, true)
            ),
            "{name} is a sorted map"
        );
    }
    let keys = |name: &str| map_keys(&batch, name);
    assert_eq!(
        keys("identifiers"),
        [
            Some(
                [
                    "fix:clordid",
                    "fix:mdentryid",
                    "fix:orderid",
                    "venue:orderid"
                ]
                .map(str::to_owned)
                .to_vec()
            ),
            Some(["fix:mdentryid", "fix:orderid"].map(str::to_owned).to_vec())
        ],
        "each row's keys in key order"
    );
    assert_eq!(
        map_rows(&batch, "identifiers")[..4],
        [
            "fix:clordid=C-1",
            "fix:mdentryid=O-1",
            "fix:orderid=ORDER-O-1",
            "venue:orderid=V-1"
        ],
        "each key beside the row it names"
    );
    assert_eq!(
        keys("partyids"),
        [
            Some(["base:account", "fix:account"].map(str::to_owned).to_vec()),
            None
        ],
        "a row stating no party is a null cell"
    );
    assert_eq!(
        map_rows(&batch, "partyids"),
        ["base:account=ACC-0", "fix:account=ACC-1"]
    );
    // The ISIN implies the national number it carries, derived and keyed by
    // the source that derived it.
    assert_eq!(
        keys("securityids"),
        [
            Some(
                ["base:isin", "base:ric", "derived:cusip"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            None
        ]
    );
    assert_eq!(
        map_rows(&batch, "securityids"),
        [
            "base:isin=US0378331005",
            "base:ric=AAPL.O",
            "derived:cusip=037833100"
        ]
    );
    assert_eq!(column_of(&batch, "partyids").null_count(), 1);
    assert_eq!(column_of(&batch, "securityids").null_count(), 1);
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        expected
    );
}

/// A key that is not the `src:type` of the row it keys has no one reading:
/// the landing refuses it on its row, naming the column, in each of the
/// three identifier maps.
#[test]
fn an_identifier_key_that_disagrees_with_its_row_is_refused_naming_the_column() {
    let mut stated = order(1, "O-1");
    stated
        .insert_identifier(Identifier::new(IdSource::Fix, IdType::ClOrdId, "C-1").unwrap())
        .unwrap();
    stated
        .insert_partyid(Identifier::new(IdSource::Fix, IdType::Account, "ACC-1").unwrap())
        .unwrap();
    stated
        .insert_securityid(Identifier::new(IdSource::Base, IdType::Ric, "AAPL.O").unwrap())
        .unwrap();
    stated.finalize();
    let batch = written(vec![MarketData::from(stated)]);
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()]))
            .unwrap()
            .len(),
        1,
        "the batch as written reads"
    );
    for (name, keys, expected, got) in [
        ("securityids", vec!["base:cusip"], "base:ric", "base:cusip"),
        ("partyids", vec!["fix:userid"], "fix:account", "fix:userid"),
        (
            "identifiers",
            vec!["fix:clordid", "fix:mdentryid", "fix:orderidx"],
            "fix:orderid",
            "fix:orderidx",
        ),
    ] {
        let error = refusal(with_map_keys(&batch, name, keys));
        assert!(error.contains(&format!("$[0].{name}")), "{name}: {error}");
        assert!(
            error.contains(&format!("expected the key {expected}, got \"{got}\"")),
            "{name}: {error}"
        );
    }
    // Keys out of the order a sorted map declares are refused before any
    // identifier is read.
    let error = refusal(with_map_keys(
        &batch,
        "identifiers",
        vec!["fix:orderid", "fix:mdentryid", "fix:clordid"],
    ));
    assert!(error.contains("$[0]"), "{error}");
    assert!(error.contains("sorted keys"), "{error}");
    // A key that is no `src:type` at all is refused the same way.
    let error = refusal(with_map_keys(&batch, "securityids", vec!["ric"]));
    assert!(error.contains("$[0].securityids"), "{error}");
    assert!(
        error.contains("expected the key base:ric, got \"ric\""),
        "{error}"
    );
}

/// Rates ride a sorted `map<ccy, decimal>`, target currency to the rate
/// to divide by, null where an element states none.
#[test]
fn fxrates_round_trip_and_are_null_where_none_is_stated() {
    let rates: FxRates = [("JPY", "150.25"), ("EUR", "0.92")]
        .into_iter()
        .map(|(target, rate)| (Ccy::new(target).unwrap(), rate.parse().unwrap()))
        .collect();
    let mut quoted = order(1, "O-1");
    quoted.set_fxrates(rates.clone(), true);
    quoted.finalize();
    assert_eq!(
        quoted
            .get_fxrates()
            .keys()
            .map(Ccy::as_str)
            .collect::<Vec<_>>(),
        ["EUR", "JPY"],
        "sorted by target"
    );
    let plain = MarketData::from(order(2, "O-2"));
    let expected = vec![MarketData::from(quoted), plain];
    let batch = written(expected.clone());
    let column = batch.column_by_name("fxrates").unwrap();
    assert!(matches!(
        column.data_type(),
        arrow_schema::DataType::Map(_, true)
    ));
    assert_eq!(column.null_count(), 1);
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        expected
    );

    // A stated rate that differs from the one the identity was derived over
    // is refused there; a rate that is no decimal is refused on its row.
    let mut other = order(1, "O-1");
    other.set_fxrates(
        [("EUR", "0.93"), ("JPY", "150.25")]
            .into_iter()
            .map(|(target, rate)| (Ccy::new(target).unwrap(), rate.parse().unwrap()))
            .collect(),
        true,
    );
    other.finalize();
    let differing = written(vec![
        MarketData::from(other),
        MarketData::from(order(2, "O-2")),
    ]);
    let error = refusal(with_column(
        &batch,
        "fxrates",
        Arc::clone(differing.column_by_name("fxrates").unwrap()),
    ));
    assert!(error.contains("$[0].curruuid"), "{error}");
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
    let live = batch.column_by_name("alive").unwrap();
    let live = live.as_any().downcast_ref::<ListArray>().unwrap();
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
    let error = refusal(with_column(&batch, "alive", Arc::new(live)));
    assert!(error.contains("$[0].alive[0].miccode"), "{error}");
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
    unstated.set_ticker(Some(SmolStr::new("ACME")), true);
    unstated.set_side(Side::read("Buy").unwrap(), true);
    unstated.set_state(State::read("Filled").unwrap());
    unstated.set_lastpx(Some(Decimal::from_int(105)), true);
    unstated.set_lastqty(Some(Decimal::from_int(15)), true);
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
    invalid.set_price(Some(Decimal::from_int(999)), true);
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
    refused(invalid, "alive[0].seqnum");

    let mut invalid = book(10);
    invalid.set_creaunix(Some(8));
    refused(invalid, "alive[0].creaunix");
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
    assert!(error.contains("$[0].marketdatakind"), "{error}");
}

#[test]
fn a_book_round_trips_its_price_levels() {
    let book = deep_book(10);
    let expected = vec![MarketData::from(book.clone())];
    let batch = written(expected.clone());
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        expected
    );

    // Each side states its limits, best first and the unpriced one last.
    let bid = limits_of(column_of(&batch, "bidlimits"), 0);
    assert_eq!(bid, book.limits(Side::Buy).collect::<Vec<_>>());
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
    let alive: Vec<_> = book
        .alive()
        .filter(|entry| entry.get_side().is_bid())
        .map(Element::get_curruuid)
        .collect();
    assert_eq!(
        bid.iter()
            .flat_map(|limit| limit.uuids.clone())
            .collect::<Vec<_>>(),
        alive
    );
    let ask = limits_of(column_of(&batch, "asklimits"), 0);
    assert_eq!(ask, book.limits(Side::Sell).collect::<Vec<_>>());
    assert_eq!(ask.last().unwrap().price, None);
    // The first level that can trade is the side's best: the top of each
    // side here, since no entry of the fixture states it cannot.
    let best = |limits: &[Limit]| limits.iter().find(|limit| limit.tradable).cloned();
    assert_eq!(
        best(&bid).and_then(|limit| limit.price),
        book.best_price(Side::Buy)
    );
    assert_eq!(
        best(&ask).map(|limit| limit.quantity),
        book.best_quantity(Side::Sell)
    );
    assert_eq!(
        (book.get_bidpx(), book.get_askpx()),
        (bid[0].price, ask[0].price)
    );
    assert!(book.get_bidpx().is_some() && book.get_askpx().is_some());
    // The book's entries, alive on both sides, one list.
    let alive = batch.column_by_name("alive").unwrap();
    let alive = alive.as_any().downcast_ref::<ListArray>().unwrap();
    assert_eq!(alive.value_length(0), 6);

    // An empty side is an empty list, and an operation states no levels.
    let mut one_sided = BookEvent::new(10, "ACME");
    one_sided
        .add_operations([resting(10, "B-1", "Buy", Some(101), 3)])
        .unwrap();
    let batch = written(vec![MarketData::from(one_sided)]);
    let asks = column_of(&batch, "asklimits");
    let asks = asks.as_any().downcast_ref::<ListArray>().unwrap();
    assert!(!asks.is_null(0));
    assert_eq!(asks.value_length(0), 0);
    let quoted = written(vec![MarketData::from(order(1, "O-1"))]);
    for name in ["bidlimits", "asklimits"] {
        assert!(column_of(&quoted, name).is_null(0), "{name}");
    }
}

#[test]
fn a_stated_limit_that_differs_is_refused_on_its_row() {
    let batch = written(vec![MarketData::from(deep_book(10))]);
    let limits = column_of(&batch, "bidlimits");
    let error = refusal(with_column(
        &batch,
        "bidlimits",
        with_limit_child(&limits, "quantity", decimals(&[3, 2, 6]), false),
    ));
    assert!(error.contains("$[0].bidlimits"), "{error}");

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
    let error = refusal(with_column(
        &batch,
        "bidlimits",
        with_limit_child(&limits, "quantity", quantity, true),
    ));
    assert!(
        error.contains("required Arrow field $.bidlimits[].quantity holds 1 null values"),
        "{error}"
    );
}

#[test]
fn a_book_limit_states_whether_its_level_trades() {
    let mut book = BookEvent::new(10, "ACME");
    let stating = |code: &str, price: i64, tradable: bool| {
        let MarketData::OrderEvent(mut entry) = resting(10, code, "Buy", Some(price), 1) else {
            unreachable!("an order rests as an order");
        };
        entry.set_tradable(Some(tradable), true);
        entry.finalize();
        MarketData::from(entry)
    };
    // 101 trades - one entry says so and the other states nothing - and
    // 100 does not: its one entry says it cannot.
    book.add_operations([
        stating("B-T", 101, true),
        resting(10, "B-1", "Buy", Some(101), 3),
        stating("B-2", 100, false),
    ])
    .unwrap();
    let expected = vec![MarketData::from(book.clone())];
    let batch = written(expected.clone());
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        expected
    );
    let bid = limits_of(column_of(&batch, "bidlimits"), 0);
    assert_eq!(
        bid.iter().map(|limit| limit.tradable).collect::<Vec<_>>(),
        [true, false]
    );

    // A stated flag that differs from the fold is refused on its row.
    let limits = column_of(&batch, "bidlimits");
    let flipped: ArrayRef = Arc::new(BooleanArray::from(vec![false, false]));
    let error = refusal(with_column(
        &batch,
        "bidlimits",
        with_limit_child(&limits, "tradable", flipped, false),
    ));
    assert!(error.contains("$[0].bidlimits"), "{error}");

    // A null flag, where a foreign schema lets one stand, is refused by the
    // cast under the required field, naming its path.
    let nulled: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), None]));
    let error = refusal(with_column(
        &batch,
        "bidlimits",
        with_limit_child(&limits, "tradable", nulled, true),
    ));
    assert!(
        error.contains("required Arrow field $.bidlimits[].tradable holds 1 null values"),
        "{error}"
    );
}

/// A null cell states nothing: a book's price levels nulled read the book
/// its entries derive.
#[test]
fn a_null_limits_cell_states_nothing() {
    let expected = vec![MarketData::from(deep_book(10))];
    let mut batch = written(expected.clone());
    for name in ["bidlimits", "asklimits"] {
        let held = column_of(&batch, name);
        batch = with_column(&batch, name, new_null_array(held.data_type(), 1));
    }
    assert_eq!(
        read(batch_reader(batch.schema(), [batch])).unwrap(),
        expected
    );
}

/// A batch stating no price levels at all reads the same values: the book
/// derives them from its entries.
#[test]
fn a_batch_stating_no_limits_columns_still_reads() {
    let expected = vec![MarketData::from(deep_book(10))];
    let batch = written(expected.clone());
    let schema = batch.schema();
    let kept: Vec<usize> = (0..schema.fields().len())
        .filter(|at| !schema.field(*at).name().ends_with("limits"))
        .collect();
    let batch = batch.project(&kept).unwrap();
    assert_eq!(batch.schema().fields().len(), 6 + 9 + 34 + 5 + 1 + 3);
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
        order.set_metadata(metadata(value), true);
        order.finalize();
        let mut book = deep_book(3);
        book.set_metadata(metadata(value), true);
        book.finalize();
        vec![MarketData::from(order), MarketData::from(book)]
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
        OffsetBuffer::new(ScalarBuffer::from(vec![0_i32, 2, 3])),
        StructArray::new(
            children.clone(),
            vec![
                Arc::new(StringArray::from(vec!["k", "k", "k"])),
                Arc::new(StringArray::from(vec!["v", "w", "v"])),
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

/// Every fact a setter fills - the side's quote, the iceberg's hidden part,
/// the order's quantities against its state, the third of an FX triple, a
/// single fill's average - is a column the row states, and the row read
/// back is the element whatever order its columns land in.
#[test]
fn every_filled_fact_is_a_column_and_reads_back() {
    use yggdryl::graph::Operation;

    let mut working = OrderEvent::at(20);
    working.set_crosscode("F-1".to_owned());
    working.set_state(State::read("PartiallyFilled").unwrap());
    working.set_currency(Ccy::new("USD").unwrap(), true);
    working.set_side(Side::read("Buy").unwrap(), true);
    working.set_price(Some(Decimal::from_int(101)), true);
    working.set_ordqty(Some(Decimal::from_int(100)), true);
    working.set_cumqty(Some(Decimal::from_int(40)), true);
    working.set_displayqty(Some(Decimal::from_int(10)), true);
    working.set_lastpx(Some(Decimal::from_int(3)), true);
    working.set_forwardpoints(Some(Decimal::from_int(1)), true);
    working.set_lastqty(Some(Decimal::from_int(40)), true);
    working.finalize();

    let mut canceled = OrderEvent::at(21);
    canceled.set_crosscode("F-2".to_owned());
    canceled.set_ordqty(Some(Decimal::from_int(10)), true);
    canceled.set_cumqty(Some(Decimal::from_int(4)), true);
    canceled.set_state(State::read("Canceled").unwrap());
    canceled.finalize();

    let expected = vec![MarketData::from(working), MarketData::from(canceled)];
    let batch = written(expected.clone());
    let decimal = |name: &str, row: usize| {
        let column = batch.column_by_name(name).expect(name);
        let serie =
            Serie::from_arrow_array(None, Arc::clone(column), ArrowCastOptions::new()).unwrap();
        Decimal::from_scalar(&serie.scalar(row).unwrap())
    };
    let int = |value: i64| Some(Decimal::from_int(value));
    assert_eq!(decimal("leavesqty", 0), int(60), "ordered less traded");
    assert_eq!(decimal("quantity", 0), int(60), "what is left open");
    assert_eq!(decimal("bidpx", 0), int(101), "a buyer's bid");
    assert_eq!(decimal("bidqty", 0), int(60));
    assert_eq!(
        decimal("hiddenqty", 0),
        int(50),
        "the quantity past the peak"
    );
    assert_eq!(decimal("spotrate", 0), int(2), "last less the points");
    assert_eq!(decimal("avgpx", 0), int(3), "one fill is its own average");
    assert_eq!(decimal("ordqty", 1), int(10));
    assert_eq!(
        decimal("leavesqty", 1),
        int(0),
        "nothing left once canceled"
    );
    assert_eq!(decimal("cxlqty", 1), int(6), "the rest canceled");

    // Read back, the rows are the elements, fills and all.
    assert_eq!(
        read(batch_reader(batch.schema(), [batch])).unwrap(),
        expected
    );
}
