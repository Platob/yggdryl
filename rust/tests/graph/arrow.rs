use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{
    Array, ArrayRef, Decimal128Array, ListArray, RecordBatch, RecordBatchReader as _, StringArray,
    StructArray, UInt64Array, new_null_array,
};
use smol_str::SmolStr;
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::graph::book::{ENTRY_ID, SnapshotPartition};
use yggdryl::graph::{
    Book, BookControl, BookInput, BookRef, Element, Event, Market, MarketEventData,
    MarketOperation, MarketOperationEventData, MdUpdateAction, Operation, OperationKind, Trade,
};
use yggdryl::{Ccy, Decimal18, Side, State, Unit};

fn operation(kind: &str, unix: i64, code: &str) -> Operation {
    let mut data = MarketOperationEventData::at(unix);
    data.set_crosscode(code.to_owned());
    data.set_seqnum(u64::try_from(unix).unwrap());
    data.set_creaunix(Some(unix - 3));
    data.set_execunix(Some(unix - 2));
    data.set_recdunix(Some(unix - 1));
    data.set_price(Some(Decimal18::from_int(100 + unix)));
    data.set_quantity(Some(Decimal18::from_int(10 + unix)));
    data.set_currency(Ccy::new("USD").unwrap());
    data.set_unit(Unit::new("share").unwrap());
    data.set_side(Side::read(if kind == "quote" { "Sell" } else { "Buy" }).unwrap());
    data.set_ticker(Some(SmolStr::new("ACME")));
    data.set_state(State::read(if kind == "execution" { "Filled" } else { "New" }).unwrap());
    data.insert_altid(ENTRY_ID, code).unwrap();
    data.insert_altid("ORDERID", &format!("ORDER-{code}"))
        .unwrap();
    data.insert_accountid("ACCOUNT", "ACC-1").unwrap();
    data.finalize();
    let mut operation =
        Operation::new(OperationKind::read(kind).unwrap(), data).expect("an operation kind");
    operation.finalize();
    operation
}

/// An entry as a market-data message states it: an operation with its
/// typed book control.
fn entry(kind: &str, unix: i64, code: &str, action: MdUpdateAction) -> Operation {
    let mut entry = operation(kind, unix, code).with_book(BookRef {
        action: Some(action),
        scope: Some(SmolStr::new("Symbol=ACME")),
        position: Some(1),
        entry_px: Some(Decimal18::from_int(100 + unix)),
        entry_size: Some(Decimal18::from_int(10 + unix)),
    });
    entry.finalize();
    entry
}

fn execution(unix: i64, code: &str, side: &str) -> Operation {
    let mut execution = operation("execution", unix, code);
    execution.set_side(Side::read(side).unwrap());
    execution.finalize();
    execution
}

fn trade(unix: i64, code: &str) -> BookInput {
    let mut event = MarketOperationEventData::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_ticker(Some(SmolStr::new("ACME")));
    event.set_state(State::read("Filled").unwrap());
    event.finalize();
    Trade::from_parts(
        event,
        vec![
            execution(unix, &format!("{code}-SELL"), "Sell"),
            execution(unix, &format!("{code}-BUY"), "Buy"),
        ],
    )
    .unwrap()
    .into()
}

fn snapshot(unix: i64, code: &str) -> BookInput {
    let mut event = MarketEventData::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_ticker(Some(SmolStr::new("ACME")));
    event.finalize();
    BookControl::snapshot(event, Some(SmolStr::new("Symbol=ACME"))).into()
}

fn book(unix: i64) -> Book {
    let mut book = Book::new(unix, "ACME");
    book.add_operations([
        operation("order", unix, &format!("O-{unix}")).into(),
        operation("quote", unix, &format!("Q-{unix}")).into(),
        operation("execution", unix, &format!("E-{unix}")).into(),
    ])
    .unwrap();
    book
}

/// A book whose last change was a full snapshot of one scope, so it
/// remembers the partition it replaced.
fn snapshot_book(unix: i64) -> Book {
    let mut book = Book::new(unix, "ACME");
    book.add_operations([
        entry(
            "quote",
            unix,
            &format!("Q-{unix}"),
            MdUpdateAction::Snapshot,
        )
        .into(),
        snapshot(unix, &format!("W-{unix}")),
    ])
    .unwrap();
    book
}

fn replace_struct_child(array: &StructArray, name: &str, child: ArrayRef) -> StructArray {
    let index = array
        .fields()
        .iter()
        .position(|field| field.name() == name)
        .unwrap();
    let mut columns = array.columns().to_vec();
    columns[index] = child;
    StructArray::new(array.fields().clone(), columns, array.nulls().cloned())
}

#[test]
fn operations_round_trip_in_bounded_streaming_batches() {
    let expected: Vec<BookInput> = vec![
        operation("order", 1, "O-1").into(),
        entry("quote", 2, "Q-2", MdUpdateAction::Change).into(),
        operation("execution", 3, "E-3").into(),
        trade(4, "T-4"),
        snapshot(5, "W-5"),
        entry("execution", 6, "E-6", MdUpdateAction::New).into(),
    ];
    let mut encoded = BookInput::arrow_reader(expected.clone(), Some(2), None).unwrap();
    // The kind, the sixteen event columns, the nineteen market columns, the
    // eight operation columns, the five book-control columns and the
    // executions.
    assert_eq!(encoded.schema().fields().len(), 50);
    assert_eq!(encoded.schema().field(0).name(), "operationkind");
    assert_eq!(encoded.schema().field(1).name(), "currunix");
    assert_eq!(encoded.schema().field(16).name(), "state");
    assert_eq!(encoded.schema().field(17).name(), "price");
    assert_eq!(encoded.schema().field(19).name(), "quantity");
    assert_eq!(encoded.schema().field(35).name(), "metadata");
    assert_eq!(encoded.schema().field(36).name(), "marketoperationid");
    assert_eq!(encoded.schema().field(41).name(), "altids");
    assert_eq!(encoded.schema().field(43).name(), "ask");
    assert_eq!(encoded.schema().field(44).name(), "mdupdateaction");
    assert_eq!(encoded.schema().field(45).name(), "bookscope");
    assert_eq!(encoded.schema().field(48).name(), "mdentrysize");
    assert_eq!(encoded.schema().field(49).name(), "executions");

    let first = encoded.next().unwrap().unwrap();
    let second = encoded.next().unwrap().unwrap();
    let third = encoded.next().unwrap().unwrap();
    assert_eq!(first.num_rows(), 2);
    assert_eq!(second.num_rows(), 2);
    assert_eq!(third.num_rows(), 2);
    assert!(encoded.next().is_none());

    let source = batch_reader(first.schema(), [first, second, third]);
    let actual = BookInput::from_arrow_reader(source)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
    let BookInput::Operation(decoded) = &actual[1] else {
        panic!("an entry decodes as an operation")
    };
    assert_eq!(decoded.action(), Some(MdUpdateAction::Change));
    assert_eq!(decoded.scope(), "Symbol=ACME");
    assert_eq!(decoded.get_altids().get(ENTRY_ID), Some("Q-2"));
    assert_eq!(decoded.get_accountids().get("ACCOUNT"), Some("ACC-1"));
    assert!(actual[4].is_full_snapshot());
}

#[test]
fn empty_snapshot_controls_round_trip_as_operations() {
    let expected = snapshot(4, "RESET");
    let batches = BookInput::arrow_reader([expected.clone()], Some(1), Some(1024)).unwrap();
    let actual = BookInput::from_arrow_reader(batches)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(actual, expected);
    let BookInput::Snapshot(control) = &actual else {
        panic!("a snapshot control decodes as one")
    };
    assert_eq!(control.book.action, Some(MdUpdateAction::Snapshot));
    assert_eq!(control.book.scope.as_deref(), Some("Symbol=ACME"));
    assert_eq!(control.event.get_ticker(), Some("ACME"));
}

#[test]
fn books_round_trip_with_live_deltas_and_executions() {
    let expected = vec![book(10), Book::new(20, "EMPTY"), snapshot_book(30)];
    assert_eq!(expected[0].bid().live().count(), 1);
    assert_eq!(expected[0].bid().deltas().len(), 1);
    assert_eq!(expected[0].ask().live().count(), 1);
    assert_eq!(expected[0].ask().deltas().len(), 1);
    assert_eq!(expected[0].executions().len(), 1);
    assert!(expected[0].snapshot_partitions().is_empty());
    assert_eq!(
        expected[2].snapshot_partitions(),
        &BTreeSet::from([SnapshotPartition {
            symbol: Some(SmolStr::new("ACME")),
            scope: SmolStr::new("Symbol=ACME"),
        }])
    );

    let mut encoded = Book::arrow_reader(expected.clone(), Some(1), None).unwrap();
    // The sixteen event columns, the nineteen market columns, the two sides,
    // the executions and the partitions the last snapshot replaced.
    assert_eq!(encoded.schema().fields().len(), 39);
    assert_eq!(encoded.schema().field(0).name(), "currunix");
    assert_eq!(encoded.schema().field(16).name(), "price");
    assert_eq!(encoded.schema().field(18).name(), "quantity");
    assert_eq!(encoded.schema().field(35).name(), "bid");
    assert_eq!(encoded.schema().field(36).name(), "ask");
    assert_eq!(encoded.schema().field(37).name(), "executions");
    assert_eq!(encoded.schema().field(38).name(), "snapshotpartitions");

    let first = encoded.next().unwrap().unwrap();
    let second = encoded.next().unwrap().unwrap();
    let third = encoded.next().unwrap().unwrap();
    assert_eq!(first.num_rows(), 1);
    assert_eq!(second.num_rows(), 1);
    assert_eq!(third.num_rows(), 1);
    assert!(encoded.next().is_none());

    let source = batch_reader(first.schema(), [first, second, third]);
    let actual = Book::from_arrow_reader(source)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual[2].snapshot_partitions(),
        expected[2].snapshot_partitions()
    );
}

#[test]
fn encoding_respects_row_and_byte_bounds_without_pulling_ahead() {
    struct Counted {
        pulled: Arc<AtomicUsize>,
        next: i64,
    }

    impl Iterator for Counted {
        type Item = BookInput;

        fn next(&mut self) -> Option<Self::Item> {
            (self.next < 3).then(|| {
                self.pulled.fetch_add(1, Ordering::SeqCst);
                let next = self.next;
                self.next += 1;
                operation("order", next + 1, &format!("O-{next}")).into()
            })
        }
    }

    let pulled = Arc::new(AtomicUsize::new(0));
    let mut reader = BookInput::arrow_reader(
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
        Ok(operation("order", 1, "O-1").into()),
        Err(yggdryl::Error::InvalidRecord {
            path: "$[1]".into(),
            reason: "broken source".into(),
        }),
    ];
    let mut reader = BookInput::arrow_reader(source, Some(2), None).unwrap();
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    assert!(
        reader
            .next()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("broken source")
    );
    assert!(reader.next().is_none());
}

#[test]
fn encoding_yields_a_completed_prefix_before_a_located_identity_error() {
    let mut invalid = operation("order", 2, "O-2");
    invalid.set_currhashcode(u64::MAX);
    let mut reader =
        BookInput::arrow_reader([operation("order", 1, "O-1"), invalid], Some(2), None).unwrap();
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$[1].curruuid"), "{error}");
    assert!(reader.next().is_none());
}

#[test]
fn decoding_names_an_unknown_kind_then_fuses() {
    let mut encoded =
        BookInput::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let mut columns = batch.columns().to_vec();
    columns[0] = Arc::new(StringArray::from(vec!["auction"])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source: BatchReader = batch_reader(batch.schema(), [batch]);
    let mut decoded = BookInput::from_arrow_reader(source).unwrap();
    let error = decoded.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("operationkind"), "{error}");
    assert!(error.contains("auction"), "{error}");
    assert!(decoded.next().is_none());
}

#[test]
fn decoding_requires_an_execution_payload_only_for_trades() {
    let mut encoded =
        BookInput::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let mut columns = batch.columns().to_vec();
    columns[0] = Arc::new(StringArray::from(vec!["trade"])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = BookInput::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[0].executions"), "{error}");
    assert!(error.contains("non-null"), "{error}");

    let mut encoded = BookInput::arrow_reader([trade(2, "T-2")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let mut columns = batch.columns().to_vec();
    columns[0] = Arc::new(StringArray::from(vec!["order"])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = BookInput::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[0].executions"), "{error}");
    assert!(error.contains("expected null for order"), "{error}");
}

#[test]
fn decoding_refuses_identity_facts_not_derived_from_the_row() {
    let mut encoded =
        BookInput::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let column = batch.schema().index_of("currhashcode").unwrap();
    let mut columns = batch.columns().to_vec();
    columns[column] = Arc::new(UInt64Array::from(vec![u64::MAX])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = BookInput::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("curruuid") || error.contains("currhashcode"),
        "{error}"
    );
}

#[test]
fn operation_decoding_locates_an_invalid_typed_code() {
    let mut encoded =
        BookInput::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let column = batch.schema().index_of("miccode").unwrap();
    let mut columns = batch.columns().to_vec();
    columns[column] = Arc::new(StringArray::from(vec![Some("ABCDE")])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = BookInput::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[0].miccode"), "{error}");
}

#[test]
fn book_decoding_locates_an_invalid_nested_typed_code() {
    let mut encoded = Book::arrow_reader([book(10)], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let bid_column = batch.schema().index_of("bid").unwrap();
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
    let mut columns = batch.columns().to_vec();
    columns[bid_column] = Arc::new(bid);
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = Book::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[0].bid.live[0].miccode"), "{error}");
}

/// A level is a price: the price column is nullable, but a null cell in a
/// live row is refused where it stands. The per-row derivation check meets
/// it first - the row's lane still states the price the element would fill
/// from - so the location is what this pins; the side's own refusal of an
/// operation stating no price is pinned where it is applied, in `book.rs`.
#[test]
fn book_decoding_refuses_a_null_price_cell_in_a_live_row() {
    let mut encoded = Book::arrow_reader([book(10)], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let bid_column = batch.schema().index_of("bid").unwrap();
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
    let price = operations.column_by_name("price").unwrap();
    let unstated = new_null_array(price.data_type(), operations.len());
    let operations = replace_struct_child(operations, "price", unstated);
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
    let mut columns = batch.columns().to_vec();
    columns[bid_column] = Arc::new(bid);
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = Book::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[0].bid.live[0].price"), "{error}");
}

/// A price or a quantity an operation does not state is a null cell in
/// the two nullable columns, and a null cell read back is none stated: an
/// execution reporting only what it last executed keeps that as `lastpx`
/// and `lastqty` across the round trip, and no zero appears anywhere.
#[test]
fn an_operation_stating_no_price_or_quantity_round_trips_as_null() {
    let mut data = MarketOperationEventData::at(5);
    data.set_crosscode("E-5".to_owned());
    data.set_ticker(Some(SmolStr::new("ACME")));
    data.set_side(Side::read("Buy").unwrap());
    data.set_state(State::read("Filled").unwrap());
    data.set_lastpx(Some(Decimal18::from_int(105)));
    data.set_lastqty(Some(Decimal18::from_int(15)));
    data.finalize();
    let mut unstated = Operation::new(OperationKind::Execution, data).unwrap();
    unstated.finalize();
    assert_eq!(
        (unstated.get_price(), unstated.get_quantity()),
        (None, None)
    );
    assert_eq!(unstated.get_bid(), None, "no fact to state on the lane");
    let expected: BookInput = unstated.into();

    let mut encoded = BookInput::arrow_reader([expected.clone()], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let schema = batch.schema();
    for name in ["price", "quantity"] {
        let column = schema.index_of(name).unwrap();
        assert!(schema.field(column).is_nullable(), "{name} is nullable");
        assert_eq!(batch.column(column).null_count(), 1, "{name} is null");
    }
    for name in ["lastpx", "lastqty"] {
        let column = schema.index_of(name).unwrap();
        assert_eq!(batch.column(column).null_count(), 0, "{name} is stated");
    }
    let source = batch_reader(schema, [batch]);
    let decoded = BookInput::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(decoded, expected);
    let BookInput::Operation(decoded) = decoded else {
        panic!("expected an operation, got {decoded:?}")
    };
    assert_eq!((decoded.get_price(), decoded.get_quantity()), (None, None));
    assert_eq!(decoded.get_lastpx(), Some(Decimal18::from_int(105)));
    assert_eq!(decoded.get_lastqty(), Some(Decimal18::from_int(15)));
}

#[test]
fn book_encoding_refuses_a_stale_summary_at_the_source_row() {
    let mut invalid = book(10);
    invalid.set_price(Some(Decimal18::from_int(999)));
    let mut encoded = Book::arrow_reader([invalid], Some(1), None).unwrap();
    let error = encoded.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$[0].price"), "{error}");
    assert!(encoded.next().is_none());
}

#[test]
fn book_encoding_refuses_root_lifecycle_bounds_that_contradict_nested_operations() {
    fn refusal(invalid: Book, path: &str) {
        let error = Book::arrow_reader([invalid], Some(1), None)
            .unwrap()
            .next()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(error.contains(path), "{error}");
    }

    let mut invalid = book(10);
    invalid.set_seqnum(0);
    refusal(invalid, "bid.live[0].seqnum");

    let mut invalid = book(10);
    invalid.set_creaunix(Some(8));
    refusal(invalid, "bid.live[0].creaunix");

    let mut invalid = book(10);
    invalid.set_recdunix(Some(10));
    refusal(invalid, "bid.live[0].recdunix");

    let mut invalid = book(10);
    invalid.set_execunix(Some(7));
    refusal(invalid, "bid.live[0].execunix");
}

#[test]
fn book_decoding_refuses_a_serialized_summary_that_disagrees_with_live_depth() {
    let mut encoded = Book::arrow_reader([book(10)], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let column = batch.schema().index_of("price").unwrap();
    let mut columns = batch.columns().to_vec();
    columns[column] = Arc::new(
        Decimal128Array::from(vec![Decimal18::from_int(999).units()])
            .with_precision_and_scale(Decimal18::PRECISION, Decimal18::SCALE)
            .unwrap(),
    ) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = Book::from_arrow_reader(source)
        .unwrap()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string();
    assert!(error.contains("$[0].price"), "{error}");
}
