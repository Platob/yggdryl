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
use yggdryl::IdKey;
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::graph::book::ENTRY_ID;
use yggdryl::graph::{
    BookEvent, BookIterator, BookRef, Element, ElementColumn, Event, EventColumn, Execution,
    ExecutionEvent, FxRates, Market, MarketColumn, MarketData, MarketKind, MdUpdateAction,
    Operation, OperationColumn, OperationEvent, OperationKind, Order, OrderEvent, Quote,
    QuoteEvent, SnapshotEvent, TradeEvent,
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
        .insert_identifier(Identifier::new(IdKey::base(ENTRY_ID), code).unwrap())
        .unwrap();
    operation
        .insert_identifier(
            Identifier::new(IdKey::base(IdType::OrderId), &format!("ORDER-{code}")).unwrap(),
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

/// A book of one order and one quote: the execution beside them is pruned
/// before the fold.
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

/// A stream of books over `inputs`, as the serie a table of books reads
/// back as.
fn books_serie(inputs: Vec<MarketData>) -> yggdryl::StreamChunkedSerie {
    let books = yggdryl::graph::BookIterator::new(inputs.into_iter(), 0)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let rows = MarketData::arrow_reader(books, None, None).unwrap();
    yggdryl::StreamChunkedSerie::from_arrow_reader(
        Some(&MarketData::field().unwrap()),
        rows,
        yggdryl::ArrowCastOptions::new(),
    )
    .unwrap()
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
    assert_eq!(names.len(), 6 + 9 + 35 + 5 + 3 + 5);
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
    assert_eq!(names[..55], shared[..]);
    assert_eq!(shared.len(), 6 + 9 + 35 + 5);
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
    // The strike of the option it is about follows the ticker, before the
    // free-form metadata closes the market's facts.
    assert_eq!(names[47..50], ["ticker", "strikepx", "metadata"]);
    assert_eq!(field.fields()[48].dtype(), &yggdryl::DataType::Decimal);
    assert!(field.fields()[48].is_nullable());
    assert_eq!(
        names[50..55],
        [
            "ordqty",
            "timeinforce",
            "tradable",
            "identifiers",
            "partyids"
        ]
    );
    // The book controls a row states - what a book's deltas replay by;
    // the price and size an entry stated are walk-time.
    assert_eq!(names[55..58], ["bookscope", "bookaction", "bookposition"]);
    assert_eq!(field.fields()[57].dtype(), &yggdryl::DataType::UInt32);
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
    assert!(field.fields()[55..].iter().all(Field::is_nullable));
    assert_eq!(
        names[58..],
        ["alive", "deltas", "executions", "bidlimits", "asklimits"]
    );
    // An operation row, the item of every operation list: nothing nested.
    let item = field.fields()[58].dtype().serie_item().unwrap().clone();
    let item: Vec<&str> = item.fields().iter().map(Field::name).collect();
    assert_eq!(item.len(), 6 + 9 + 35 + 5 + 3);
    assert_eq!(item, names[..58]);
    // A book's two sides are its price levels, one limit each.
    for side in &field.fields()[61..] {
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
    // the same rows; a book control's walk-time facts - an entry's own price
    // and size - are no row fact, so the two leaves stating one read back
    // without them and keep their identity, their action and position.
    assert_eq!(rewritten(actual.clone(), 5), batches);
    for (index, (read, stated)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(read.get_curruuid(), stated.get_curruuid(), "{index}");
        if index != 4 && index != 8 {
            assert_eq!(read, stated, "{index}");
        }
    }
    let entry = actual[4].as_quote_event().unwrap();
    assert_eq!(entry.action(), Some(MdUpdateAction::Change));
    assert_eq!(entry.book().and_then(|book| book.position), Some(1));
    assert_eq!(
        entry.book().and_then(|book| book.entry_px),
        None,
        "an entry's own price is walk-time"
    );
    assert_eq!(entry.scope(), "Symbol=ACME");
    assert_eq!(entry.get_identifiers().get(&ENTRY_ID), Some("Q-6"));
    assert_eq!(actual[6].as_trade_event().unwrap().executions().len(), 2);
    let book = actual[7].as_book_event().unwrap();
    assert_eq!(book.alive().count(), 2);
    // The order and the quote it placed, and the execution it recorded.
    assert_eq!(book.deltas().len(), 3);
    // A trade's row states its executions; a book's states none.
    let executions = batches[1].column_by_name("executions").unwrap();
    assert!(executions.is_valid(1) && executions.is_null(2));
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

    // A book row with its entries nulled reads as a book stating its deltas
    // alone, which states no price level.
    let held = batch.column_by_name("alive").unwrap();
    let error = refusal(with_column(
        &batch,
        "alive",
        new_null_array(held.data_type(), 1),
    ));
    assert!(error.contains("$[0].bidlimits"), "{error}");
    assert!(
        error.contains("expected no limits on a book holding only its deltas, got 1"),
        "{error}"
    );

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

/// A book holds no execution, so a `BOOK` row whose `executions` cell holds
/// any is refused there; a null or empty cell states none.
#[test]
fn a_book_row_stating_executions_is_refused() {
    let stated = written(vec![MarketData::from(trade(8, "T-8"))]);
    let stated = stated.column_by_name("executions").unwrap().clone();
    let batch = written(vec![MarketData::from(book(9))]);
    let error = refusal(with_column(&batch, "executions", Arc::clone(&stated)));
    assert!(error.contains("$[0].executions"), "{error}");
    assert!(
        error.contains("expected no executions on a book_event, got 2"),
        "{error}"
    );

    let arrow_schema::DataType::List(item) = stated.data_type() else {
        panic!("executions is a list");
    };
    let values = stated
        .as_any()
        .downcast_ref::<ListArray>()
        .unwrap()
        .values();
    let empty = ListArray::new(
        Arc::clone(item),
        OffsetBuffer::new(ScalarBuffer::from(vec![0_i32, 0])),
        values.slice(0, 0),
        None,
    );
    let read = read(batch_reader(
        batch.schema(),
        [with_column(&batch, "executions", Arc::new(empty))],
    ))
    .unwrap();
    assert_eq!(read, [MarketData::from(book(9))]);
}

/// `isincode` is the `isin` of `securityids`, projected: written from it,
/// filling an absent one on the way back, and refused where the two
/// disagree.
#[test]
fn isincode_projects_the_securityids_isin() {
    let mut listed = order(1, "O-1");
    listed
        .insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "US0378331005").unwrap())
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

/// The `key=value` of every entry an identifier map column holds, flat.
fn map_rows(batch: &RecordBatch, name: &str) -> Vec<String> {
    let map = column_of(batch, name);
    let map = map.as_any().downcast_ref::<MapArray>().unwrap();
    let text = |array: &ArrayRef| {
        array
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("a text column")
            .clone()
    };
    let (keys, values) = (text(map.keys()), text(map.values()));
    (0..keys.len())
        .map(|at| format!("{}={}", keys.value(at), values.value(at)))
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
/// are sorted `map<utf8, utf8>`s from a type's base key to its value, one
/// per type, null where a row states none; every other key a map holds - a
/// source's statement, a derivation - is side information, written once
/// into `metadata` under its map's name and its `src:type` spelling beside
/// the leaf's own metadata, and the maps read back as they were.
#[test]
fn the_three_identifier_columns_are_sorted_maps_of_base_keys_with_their_side_information_in_metadata()
 {
    let venue: IdSource = "venue".parse().unwrap();
    let mut stated = order(1, "O-1");
    for id in [
        Identifier::new(IdKey::base(IdType::ClOrdId), "C-1").unwrap(),
        Identifier::new(IdKey::new(venue.clone(), IdType::OrderId), "V-1").unwrap(),
    ] {
        stated.insert_identifier(id).unwrap();
    }
    for id in [
        Identifier::new(IdKey::base(IdType::Account), "ACC-1").unwrap(),
        Identifier::new(IdKey::new(IdSource::Bic, IdType::Account), "DEUTDEFF").unwrap(),
    ] {
        stated.insert_partyid(id).unwrap();
    }
    for id in [
        Identifier::new(IdKey::base(IdType::Ric), "AAPL.O").unwrap(),
        Identifier::new(IdKey::base(IdType::Isin), "US0378331005").unwrap(),
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
                ["clordid", "mdentryid", "orderid"]
                    .map(str::to_owned)
                    .to_vec()
            ),
            Some(["mdentryid", "orderid"].map(str::to_owned).to_vec())
        ],
        "each row's base keys in key order, a type alone"
    );
    assert_eq!(
        map_rows(&batch, "identifiers")[..4],
        [
            "clordid=C-1",
            "mdentryid=O-1",
            "orderid=ORDER-O-1",
            "mdentryid=O-2"
        ],
        "each key beside its value"
    );
    assert_eq!(
        keys("partyids"),
        [Some(["account"].map(str::to_owned).to_vec()), None],
        "a row stating no party is a null cell"
    );
    assert_eq!(map_rows(&batch, "partyids"), ["account=ACC-1"]);
    // The ISIN implies the national number it carries, derived: its base
    // key is the type's answer, and the derivation side information.
    assert_eq!(
        keys("securityids"),
        [
            Some(["cusip", "isin", "ric"].map(str::to_owned).to_vec()),
            None
        ]
    );
    assert_eq!(
        map_rows(&batch, "securityids"),
        ["cusip=037833100", "isin=US0378331005", "ric=AAPL.O"]
    );
    // What the maps hold beside their answers - the venue's order
    // identifier, the BIC's account, the derived CUSIP - is side
    // information: in `metadata`, each key once under its map's name, in
    // key order.
    assert_eq!(
        keys("metadata"),
        [
            Some(
                [
                    "identifiers.venue:orderid",
                    "partyids.bic:account",
                    "securityids.derived:cusip"
                ]
                .map(str::to_owned)
                .to_vec()
            ),
            None
        ]
    );
    assert_eq!(
        map_rows(&batch, "metadata"),
        [
            "identifiers.venue:orderid=V-1",
            "partyids.bic:account=DEUTDEFF",
            "securityids.derived:cusip=037833100"
        ]
    );
    assert_eq!(column_of(&batch, "partyids").null_count(), 1);
    assert_eq!(column_of(&batch, "securityids").null_count(), 1);
    assert_eq!(column_of(&batch, "metadata").null_count(), 1);
    assert_eq!(
        read(batch_reader(batch.schema(), [batch.clone()])).unwrap(),
        expected
    );
}

/// A leaf whose own metadata spells the side information its row files - a
/// `src:type` key under the name of a map holding that identifier - is
/// refused where its row is written, located on the key: the identifier is
/// stated in its map, never beside it. A bare `src:type` key is the leaf's
/// own metadata - a bridge field whose lift the parser refused keeps its
/// spelling - filed and read back as it is.
#[test]
fn a_metadata_key_spelling_the_side_information_of_the_row_is_refused_where_it_is_written() {
    let venue: IdSource = "venue".parse().unwrap();
    let mut plain = order(1, "O-1");
    plain
        .insert_identifier(
            Identifier::new(IdKey::new(venue.clone(), IdType::OrderId), "V-1").unwrap(),
        )
        .unwrap();
    let mut metadata = yggdryl::graph::Metadata::new();
    metadata.insert("venue:orderid".into(), "V-2".into());
    metadata.insert("plain".into(), "kept".into());
    plain.set_metadata(Some(metadata), true);
    plain.finalize();
    let expected = vec![MarketData::from(plain)];
    let batch = written(expected.clone());
    assert_eq!(
        map_rows(&batch, "metadata"),
        [
            "identifiers.venue:orderid=V-1",
            "plain=kept",
            "venue:orderid=V-2"
        ],
        "the leaf's own key beside the side information, each under its spelling"
    );
    assert_eq!(
        read(batch_reader(batch.schema(), [batch])).unwrap(),
        expected
    );

    let mut stated = order(1, "O-1");
    stated
        .insert_identifier(Identifier::new(IdKey::new(venue, IdType::OrderId), "V-1").unwrap())
        .unwrap();
    let mut metadata = yggdryl::graph::Metadata::new();
    metadata.insert("identifiers.venue:orderid".into(), "V-2".into());
    stated.set_metadata(Some(metadata), true);
    stated.finalize();
    let mut reader = MarketData::arrow_reader([MarketData::from(stated)], None, None).unwrap();
    let error = reader
        .next()
        .expect("the refusal")
        .expect_err("a metadata key spelling the row's side information is refused")
        .to_string();
    assert!(
        error.contains("$[0].metadata['identifiers.venue:orderid']")
            && error.contains("state it in its map"),
        "{error}"
    );
    assert!(reader.next().is_none(), "fused after the refusal");
}

/// An entry no identifier reads - a key no `IdKey` spells, a value its type
/// refuses, a second value under one key - is refused by the landing on its
/// key, below the column, in each of the three identifier maps.
#[test]
fn an_identifier_entry_no_identifier_reads_is_refused_on_its_key() {
    let mut stated = order(1, "O-1");
    stated
        .insert_identifier(Identifier::new(IdKey::base(IdType::ClOrdId), "C-1").unwrap())
        .unwrap();
    stated
        .insert_partyid(Identifier::new(IdKey::base(IdType::Account), "ACC-1").unwrap())
        .unwrap();
    stated
        .insert_securityid(Identifier::new(IdKey::base(IdType::Ric), "AAPL.O").unwrap())
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
    for (name, keys, located, reason) in [
        (
            "securityids",
            vec!["cusip"],
            "$[0].securityids['cusip']",
            "cusip",
        ),
        (
            "partyids",
            vec!["fix:"],
            "$[0].partyids['fix:']",
            "expected an identifier key src:type or type",
        ),
        (
            "identifiers",
            vec!["clordid", "fix:clordid", "orderid"],
            "$[0].identifiers['fix:clordid']",
            "expected one value under clordid",
        ),
    ] {
        let error = refusal(with_map_keys(&batch, name, keys));
        assert!(error.contains(located), "{name}: {error}");
        assert!(error.contains(reason), "{name}: {error}");
    }
    // Keys out of the order a sorted map declares are refused before any
    // identifier is read.
    let error = refusal(with_map_keys(
        &batch,
        "identifiers",
        vec!["orderid", "mdentryid", "clordid"],
    ));
    assert!(error.contains("$[0]"), "{error}");
    assert!(error.contains("sorted keys"), "{error}");
}

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

/// The retired struct shape of an identifier map - `map<utf8,
/// struct<src, type, value>>` - is no `map<utf8, utf8>`: the stream binds,
/// since a struct casts into text as its JSON document, and the first row
/// stating an entry is refused where that document arrives as a value its
/// type's rule does not read, named by the row and the key.
#[test]
fn an_identifier_map_of_the_retired_struct_shape_is_refused() {
    use arrow_array::builder::{MapBuilder, MapFieldNames, StringBuilder, StructBuilder};
    let batch = written(vec![MarketData::from(order(1, "O-1"))]);
    let item = Fields::from(vec![
        arrow_schema::Field::new("src", arrow_schema::DataType::Utf8, true),
        arrow_schema::Field::new("type", arrow_schema::DataType::Utf8, true),
        arrow_schema::Field::new("value", arrow_schema::DataType::Utf8, true),
    ]);
    let mut builder = MapBuilder::new(
        Some(MapFieldNames {
            entry: "entries".into(),
            key: "key".into(),
            value: "securityid".into(),
        }),
        StringBuilder::new(),
        StructBuilder::from_fields(item, 1),
    );
    builder.keys().append_value("base:isin");
    let values = builder.values();
    for (at, text) in ["base", "isin", "US0378331005"].into_iter().enumerate() {
        values
            .field_builder::<arrow_array::builder::StringBuilder>(at)
            .unwrap()
            .append_value(text);
    }
    values.append(true);
    builder.append(true).unwrap();
    let retired = with_column(&batch, "securityids", Arc::new(builder.finish()));
    let error = refusal(retired);
    assert!(
        error.contains("$[0].securityids['base:isin']") && error.contains("isin"),
        "{error}"
    );
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
    assert_eq!(batch.schema().fields().len(), 6 + 9 + 35 + 5 + 3 + 3);
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

/// The books a walk with no grid emits over two orders of ACME at `unix`
/// and one at `unix + 1`: each its deltas alone, the first following no
/// book and the second the first.
fn walked_books(unix: i64) -> Vec<BookEvent> {
    BookIterator::new(
        [
            MarketData::from(order(unix, &format!("O-{unix}"))),
            MarketData::from(quote(unix, &format!("Q-{unix}"))),
            MarketData::from(order(unix + 1, &format!("O-{}", unix + 1))),
        ]
        .into_iter(),
        0,
    )
    .unwrap()
    .collect::<yggdryl::Result<Vec<_>>>()
    .unwrap()
}

/// A complete book holding no live entry and stating deltas - every entry
/// it folded ended - writes an empty `alive` list, which a table storing a
/// null list as an empty one cannot tell from the null a book stating its
/// deltas alone writes; stating no snapshot instant to be told by, as every
/// complete book a walk emits does, it reads back from either table as a
/// book stating its deltas alone, its identity and its deltas kept, and
/// rebuilds whole over the empty book it follows: the row does not carry
/// the completeness of an empty book, a loss the walk never meets. Only a
/// built or rebuilt book meets this.
#[test]
fn a_complete_empty_book_stating_deltas_and_no_snapshot_instant_reads_back_as_its_deltas() {
    let mut ended = order(1, "O-1");
    ended.set_state(State::Canceled);
    ended.finalize();
    // The fixtures' orders state ACME's ticker: a book keyed otherwise
    // refuses them.
    let mut book = BookEvent::new(1, "ACME");
    book.add_operations([MarketData::from(order(1, "O-1")), MarketData::from(ended)])
        .unwrap();
    assert!(book.is_complete());
    assert_eq!(
        (
            book.alive().count(),
            book.deltas().len(),
            book.get_snapunix()
        ),
        (0, 2, None)
    );
    let batch = written(vec![MarketData::from(book.clone())]);
    assert!(
        !column_of(&batch, "alive").is_null(0),
        "a complete book states its alive list, empty"
    );
    for batch in [batch.clone(), with_null_lists_emptied(&batch)] {
        let read_back = read(batch_reader(batch.schema(), [batch]))
            .unwrap()
            .remove(0);
        let read_back = read_back.as_book_event().unwrap();
        assert!(
            !read_back.is_complete(),
            "the row does not carry the completeness of an empty book"
        );
        assert_eq!(read_back.get_curruuid(), book.get_curruuid());
        assert_eq!(read_back.deltas().len(), 2);
        let rebuilt = read_back
            .clone()
            .with_previous(&BookEvent::new(1, "ACME"))
            .unwrap();
        assert!(rebuilt.is_complete());
        assert_eq!(rebuilt.alive().count(), 0);
        assert_eq!(rebuilt.get_curruuid(), book.get_curruuid());
    }
}

/// A book stating its deltas alone writes its `deltas` and leaves `alive`,
/// `bidlimits` and `asklimits` null; a complete book - an empty one -
/// writes all of them, even empty. Each reads back as the form it was
/// written in, its identity kept; read from a table that stores a null
/// list as an empty one, a book stating its deltas alone reads back as one
/// still.
#[test]
fn a_delta_book_row_states_no_alive_entry_and_no_limits() {
    let books = walked_books(20);
    assert!(!books[0].is_complete() && !books[1].is_complete());
    assert_eq!(books[0].get_prevuuid(), None);
    let values = vec![
        MarketData::from(books[0].clone()),
        MarketData::from(books[1].clone()),
        MarketData::from(BookEvent::new(30, "EMPTY")),
    ];
    let batch = written(values.clone());
    for name in ["alive", "bidlimits", "asklimits"] {
        let column = column_of(&batch, name);
        assert_eq!(
            (0..3).map(|row| column.is_null(row)).collect::<Vec<_>>(),
            [true, true, false],
            "{name}"
        );
    }
    assert_eq!(column_of(&batch, "deltas").null_count(), 0);
    for batch in [batch.clone(), with_null_lists_emptied(&batch)] {
        let actual = read(batch_reader(batch.schema(), [batch])).unwrap();
        let actual: Vec<&BookEvent> = actual
            .iter()
            .map(|value| value.as_book_event().unwrap())
            .collect();
        assert_eq!(
            actual
                .iter()
                .map(|book| book.is_complete())
                .collect::<Vec<_>>(),
            [false, false, true]
        );
        for (read, stated) in actual.iter().zip(&values) {
            assert_eq!(read.get_curruuid(), stated.get_curruuid());
        }
        assert_eq!(actual[1].deltas().len(), 1);
        assert_eq!(actual[1].alive().count(), 0);
        assert_eq!(
            actual[1].best_price(Side::Buy),
            books[1].best_price(Side::Buy)
        );
        // Read back, the first rebuilds over the empty book it follows and
        // the second over the first, each under its own identity.
        let first = actual[0]
            .clone()
            .with_previous(&BookEvent::new(20, "ACME"))
            .unwrap();
        assert_eq!(first.get_curruuid(), books[0].get_curruuid());
        assert_eq!(first.alive().count(), 2);
        let rebuilt = actual[1].clone().with_previous(&first).unwrap();
        assert_eq!(rebuilt.get_curruuid(), books[1].get_curruuid());
        assert_eq!(rebuilt.alive().count(), 3);
    }
}

/// A two-sided quote is listed once in `alive`, with the bids, while the
/// order its level holds on the ask side may be another: the row's price
/// levels state each side's own order, which the read restores, so a
/// complete book whose sides order a level differently reads back as
/// written - two quotes tied on the ask in the other order than on the bid,
/// an ask order before a quote at its price, and a grid tick's book.
#[test]
fn a_complete_book_whose_sides_order_a_level_differently_round_trips() {
    let quote = |code: &str, unix: i64, bid: (i64, i64), ask: (i64, i64)| {
        let mut quote = QuoteEvent::at(unix);
        quote.set_crosscode(code.to_owned());
        quote.set_ticker(Some(SmolStr::new("ACME")), true);
        quote.set_bidpx(Some(Decimal::from_int(bid.0)), true);
        quote.set_bidqty(Some(Decimal::from_int(bid.1)), true);
        quote.set_askpx(Some(Decimal::from_int(ask.0)), true);
        quote.set_askqty(Some(Decimal::from_int(ask.1)), true);
        quote.set_state(State::New);
        quote.finalize();
        MarketData::from(quote)
    };
    let mut tied = BookEvent::new(1, "ACME");
    tied.add_operations([
        quote("Q-2", 1, (98, 5), (101, 5)),
        quote("Q-1", 1, (99, 10), (101, 20)),
    ])
    .unwrap();
    let mut crossed_kinds = BookEvent::new(1, "ACME");
    crossed_kinds
        .add_operations([
            resting(1, "A-1", "Sell", Some(101), 1),
            quote("Q-1", 1, (98, 2), (101, 3)),
        ])
        .unwrap();
    let ticked: Vec<BookEvent> = BookIterator::new(
        [
            resting(1_000_000, "A-1", "Sell", Some(101), 1),
            quote("Q-1", 2_000_000, (98, 2), (101, 3)),
        ]
        .into_iter(),
        1,
    )
    .unwrap()
    .collect::<yggdryl::Result<_>>()
    .unwrap();
    let tick = ticked
        .into_iter()
        .rfind(BookEvent::is_complete)
        .expect("a grid tick");
    for book in [tied, crossed_kinds, tick] {
        let asks: Vec<&str> = book
            .alive_on(Side::Sell)
            .map(Element::get_crosscode)
            .collect();
        let listed: Vec<&str> = book.alive().map(Element::get_crosscode).collect();
        assert_ne!(
            asks,
            listed[listed.len() - asks.len()..],
            "the ask side orders its level otherwise than alive lists it"
        );
        let expected = vec![MarketData::from(book)];
        let batch = written(expected.clone());
        assert_eq!(
            read(batch_reader(batch.schema(), [batch])).unwrap(),
            expected
        );
    }
}

/// A book's deltas replay by the update action and the position each
/// states, which its row states: a walk positioning entries in a level and
/// deleting through a position, written and read back, rebuilds every book
/// as the walk held it.
#[test]
fn delta_books_read_back_replay_their_ranges_and_positions() {
    let positioned = |code: &str, unix: i64, price: i64, position: u32| {
        let mut quote = quote(unix, code);
        quote.set_side(Side::Buy, true);
        quote.set_price(Some(Decimal::from_int(price)), true);
        quote.set_book(Some(BookRef {
            action: Some(MdUpdateAction::New),
            scope: Some(SmolStr::new("PRIMARY")),
            position: Some(position),
            ..BookRef::default()
        }));
        quote.finalize();
        MarketData::from(quote)
    };
    let mut through = quote(3, "P-X");
    through.set_side(Side::Buy, true);
    through.set_state(State::Canceled);
    through.set_book(Some(BookRef {
        action: Some(MdUpdateAction::DeleteThru),
        scope: Some(SmolStr::new("PRIMARY")),
        position: Some(1),
        ..BookRef::default()
    }));
    through.finalize();
    let inputs = vec![
        positioned("P-A", 1, 97, 2),
        positioned("P-B", 2, 97, 1),
        positioned("P-C", 2, 96, 3),
        MarketData::from(through),
    ];
    let books: Vec<BookEvent> = BookIterator::new(inputs.into_iter(), 0)
        .unwrap()
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    let fold = |books: Vec<BookEvent>| {
        let mut last = BookEvent::new(0, "ACME");
        books
            .into_iter()
            .map(|book| {
                last = book.with_previous(&last).expect("a rebuild");
                last.clone()
            })
            .collect::<Vec<_>>()
    };
    let walked = fold(books.clone());
    let batch = written(books.into_iter().map(MarketData::from).collect());
    let read_back = fold(
        read(batch_reader(batch.schema(), [batch]))
            .unwrap()
            .into_iter()
            .map(|value| value.as_book_event().unwrap().clone())
            .collect(),
    );
    assert_eq!(read_back.len(), 3);
    for (read, walked) in read_back.iter().zip(&walked) {
        assert_eq!(read.get_curruuid(), walked.get_curruuid());
        assert_eq!(
            read.alive().map(Element::get_curruuid).collect::<Vec<_>>(),
            walked
                .alive()
                .map(Element::get_curruuid)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            read.limits(Side::Buy).collect::<Vec<_>>(),
            walked.limits(Side::Buy).collect::<Vec<_>>()
        );
    }
    // The level at 97 holds P-B before P-A by position; the delete through
    // position one took P-B off.
    let codes = |book: &BookEvent| {
        book.alive_on(Side::Buy)
            .map(|entry| entry.get_crosscode().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(codes(&walked[1]), ["14:0:P-B", "14:0:P-A", "14:0:P-C"]);
    assert_eq!(codes(&walked[2]), ["14:0:P-A", "14:0:P-C"]);
}

/// A book stating its deltas alone states the price and the quantity its
/// best bid and ask settle on, and a row stating another is refused there;
/// so is one stating a snapshot instant, which only a whole book states.
#[test]
fn a_delta_book_row_whose_price_disagrees_with_its_legs_is_refused() {
    let books = walked_books(20);
    let batch = written(vec![MarketData::from(books[1].clone())]);
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
    let error = refusal(with_column(
        &batch,
        "snapunix",
        Arc::new(Int64Array::from(vec![21_i64])),
    ));
    assert!(error.contains("$[0].snapunix"), "{error}");
    assert!(
        error.contains("expected no snapshot instant on a book holding only its deltas, got 21"),
        "{error}"
    );
}

/// A book holds both sides and so does a quote quoting both legs and
/// tagging neither: each row states side `BOTH` (code 99) and reads back
/// equal, and a book row stating another side is refused there.
#[test]
fn a_book_row_and_a_two_sided_quote_row_state_both_sides() {
    let book = deep_book(10);
    let mut quote = QuoteEvent::at(10);
    quote.set_crosscode("Q-1".to_owned());
    quote.set_bidpx(Some(Decimal::from_int(99)), true);
    quote.set_bidqty(Some(Decimal::from_int(2)), true);
    quote.set_askpx(Some(Decimal::from_int(101)), true);
    quote.set_askqty(Some(Decimal::from_int(3)), true);
    quote.finalize();
    assert_eq!(
        (book.get_side(), quote.get_side()),
        (Side::Both, Side::Both)
    );
    let values = vec![MarketData::from(book), MarketData::from(quote)];
    let batch = written(values.clone());
    let sides = column_of(&batch, "side");
    let sides = sides
        .as_any()
        .downcast_ref::<arrow_array::UInt8Array>()
        .unwrap();
    assert_eq!(sides.values().as_ref(), [99, 99]);
    assert_eq!(read(batch_reader(batch.schema(), [batch])).unwrap(), values);

    let book = written(vec![MarketData::from(deep_book(10))]);
    let error = refusal(with_column(
        &book,
        "side",
        Arc::new(arrow_array::UInt8Array::from(vec![0_u8])),
    ));
    assert!(error.contains("$[0].side"), "{error}");
}

/// A book stating its deltas alone is pinned by the book it follows and
/// the deltas it applied: a row naming another predecessor, or stating
/// other deltas, derives another identity and is refused at its
/// `curruuid`.
#[test]
fn a_delta_book_row_stating_another_chain_is_refused_at_its_identity() {
    let books = walked_books(20);
    let batch = written(vec![MarketData::from(books[1].clone())]);
    let itself = Arc::clone(batch.column_by_name("curruuid").unwrap());
    let error = refusal(with_column(&batch, "prevuuid", itself));
    assert!(error.contains("$[0].curruuid"), "{error}");

    let empty = written(vec![MarketData::from(BookEvent::new(21, "EMPTY"))]);
    let none = Arc::clone(empty.column_by_name("deltas").unwrap());
    let error = refusal(with_column(&batch, "deltas", none));
    assert!(error.contains("$[0].curruuid"), "{error}");
}

/// The deltas of a stream of books lay out as the rows of their kind, in
/// book order: every event each book recorded, of the kind asked for or of
/// every kind, and a stream holding a row that is no book is refused at
/// that row.
#[test]
fn the_deltas_of_books_lay_out_as_the_rows_of_their_kind() {
    let inputs = || {
        vec![
            MarketData::from(order(1, "O-1")),
            MarketData::from(execution(2, "E-2", "Buy")),
            MarketData::from(order(3, "O-3")),
        ]
    };
    let codes = |rows: &[MarketData]| {
        rows.iter()
            .map(|row| row.get_crosscode().to_owned())
            .collect::<Vec<_>>()
    };

    let executions = MarketData::deltas_serie(
        books_serie(inputs()),
        Some(yggdryl::MarketDataKind::Execution),
    )
    .unwrap();
    let executions = read(executions.into_arrow_reader()).unwrap();
    assert_eq!(codes(&executions), ["8:1:E-2"]);
    assert_eq!(executions[0].get_execunix(), Some(0));

    let every = MarketData::deltas_serie(books_serie(inputs()), None).unwrap();
    assert_eq!(
        codes(&read(every.into_arrow_reader()).unwrap()),
        ["10:1:O-1", "8:1:E-2", "10:1:O-3"]
    );

    // A stream that holds no book answers its deltas: none.
    let orders = MarketData::deltas_serie(
        books_serie(inputs()),
        Some(yggdryl::MarketDataKind::Quotation),
    )
    .unwrap();
    assert!(read(orders.into_arrow_reader()).unwrap().is_empty());

    // A row that is no book is refused as an item of the stream.
    let flat = MarketData::arrow_reader(inputs(), None, None).unwrap();
    let flat = yggdryl::StreamChunkedSerie::from_arrow_reader(
        Some(&MarketData::field().unwrap()),
        flat,
        yggdryl::ArrowCastOptions::new(),
    )
    .unwrap();
    let error = read(
        MarketData::deltas_serie(flat, None)
            .unwrap()
            .into_arrow_reader(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("expected a book_event"),
        "{error}"
    );
}

/// A stream of books over a fine grid repeats every alive entry of every
/// book at every tick, so one batch of books holds hundreds of thousands
/// of nested rows; cast to the layout a table stores - the digests and the
/// place widened to `decimal(20, 0)`, every enum to `int32`, the clocks to
/// microseconds - it casts whole, because a kernel's output is one slot per
/// row of the batch and the materialization budget charges none of it. The
/// batch is bounded by the codec's own batch bounds, never by a slot
/// ceiling the grid can cross.
#[test]
fn a_batch_of_books_with_many_alive_entries_casts_to_the_stored_layout_whole() {
    use yggdryl::graph::BookIterator;
    use yggdryl::{ArrowCastOptions, Decimal, Scheme, Serie, Side};
    const T: i64 = 1_700_000_000_000_000_000;
    const ALIVE: i64 = 250;
    const TICKS: i64 = 500;
    let order = |unix: i64, code: String, side: Side| {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code);
        order.set_ticker(Some("AAPL".into()), true);
        order.set_side(side, true);
        order.set_price(Some(Decimal::from_int(189)), true);
        order.set_quantity(Some(Decimal::from_int(100)), true);
        order.finalize();
        MarketData::from(order)
    };
    let mut inputs: Vec<MarketData> = (0..ALIVE)
        .map(|i| order(T + i, format!("B-{i}"), Side::Buy))
        .collect();
    inputs.push(order(
        T + TICKS * 1_000_000_000,
        "LAST".to_owned(),
        Side::Sell,
    ));
    // One second ticks: every tick a complete book of every alive entry.
    let books = BookIterator::new(inputs.into_iter(), 1_000)
        .unwrap()
        .map(|book| book.map(MarketData::from));
    let reader = MarketData::arrow_reader(books, None, None).unwrap();
    let stored = MarketData::field()
        .unwrap()
        .into_scheme_compat(&Scheme::ICEBERG)
        .unwrap();
    let mut books = 0;
    let mut entries = 0;
    for batch in reader {
        let batch = batch.unwrap();
        let alive = batch
            .column_by_name("alive")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        entries += alive.values().len();
        let cast = Serie::from_arrow_batch(Some(&stored), &batch, ArrowCastOptions::new())
            .expect("a batch of books casts to the stored layout whole");
        books += cast.len();
    }
    assert!(books > TICKS as usize, "{books} books");
    assert!(
        entries > 100_000,
        "{entries} alive entries: enough to have crossed the hidden-slot ceiling per cast column"
    );
}

/// A table with no unsigned type may store the two digests as the `long`
/// of their width, carrying their bits and stating nothing: the reader
/// reads such a cell as its bits at the root and in every nested row, and
/// verifies the identity each leaf derives as for any other layout.
#[test]
fn market_rows_whose_digests_a_table_stored_as_longs_read_back_as_their_leaves() {
    let expected = every_leaf();
    let written = rewritten(expected.clone(), expected.len());

    // The written root, its digests retyped to `int64` at every depth and
    // stating nothing: a foreign layout.
    let mut foreign = MarketData::field().unwrap();
    for prefix in ["", "alive[0].", "deltas[0].", "executions[0]."] {
        for name in ["currhashcode", "crosshashcode"] {
            let path = format!("{prefix}{name}");
            let mut child = foreign.get_field_by_path(&path).unwrap().clone();
            child.set_dtype(yggdryl::DataType::Int64).unwrap();
            foreign.set_field_by_path(&path, child).unwrap();
        }
    }
    let stored: Vec<RecordBatch> = yggdryl::StreamChunkedSerie::from_arrow_reader(
        Some(&foreign),
        MarketData::arrow_reader(expected.clone(), None, None).unwrap(),
        ArrowCastOptions::new().with_representation(yggdryl::Representation::Bits),
    )
    .unwrap()
    .into_arrow_reader()
    .map(Result::unwrap)
    .collect();
    let digests = stored[0]
        .column_by_name("currhashcode")
        .unwrap()
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("the digest is stored as a long")
        .clone();
    assert!(
        digests.values().iter().any(|digest| *digest < 0),
        "some digest is past i64::MAX, so its long is negative"
    );

    let actual = read(batch_reader(stored[0].schema(), stored)).unwrap();
    assert_eq!(actual.len(), expected.len());
    for (index, (read, stated)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(
            read.get_currhashcode(),
            stated.get_currhashcode(),
            "{index}"
        );
        assert_eq!(
            read.get_crosshashcode(),
            stated.get_crosshashcode(),
            "{index}"
        );
    }
    assert_eq!(rewritten(actual, expected.len()), written);
}
