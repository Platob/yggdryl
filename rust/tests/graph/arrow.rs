use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{
    Array, ArrayRef, Decimal128Array, ListArray, RecordBatch, RecordBatchReader as _, StringArray,
    StructArray, UInt64Array,
};
use yggdryl::arrow::{BatchReader, batch_reader};
use yggdryl::graph::{
    Book, Element, Event, Execution, MarketElement, MarketEventData, MarketOperation, Order, Quote,
};
use yggdryl::{Currency, Decimal18, Side, State};

fn operation(kind: &str, unix: i64, code: &str) -> MarketOperation {
    let mut event = MarketEventData::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_seqnum(u64::try_from(unix).unwrap());
    event.set_creaunix(Some(unix - 3));
    event.set_execunix(Some(unix - 2));
    event.set_recdunix(Some(unix - 1));
    event.set_px(Decimal18::from_int(100 + unix));
    event.set_qty(Decimal18::from_int(10 + unix));
    event.set_currency(Currency::new("USD").unwrap());
    event.set_unit("share".to_owned());
    event.set_side(Side::read(if kind == "quote" { "Sell" } else { "Buy" }).unwrap());
    event.set_symbolticker(Some("ACME".to_owned()));
    event.set_state(State::read(if kind == "execution" { "Filled" } else { "New" }).unwrap());
    event.finalize();
    match kind {
        "order" => Order::from(event).into(),
        "quote" => Quote::from(event).into(),
        "execution" => Execution::from(event).into(),
        _ => unreachable!(),
    }
}

fn book(unix: i64) -> Book {
    let mut book = Book::new(unix, "ACME");
    book.add_operations([
        operation("order", unix, &format!("O-{unix}")),
        operation("quote", unix, &format!("Q-{unix}")),
        operation("execution", unix, &format!("E-{unix}")),
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
    let expected = vec![
        operation("order", 1, "O-1"),
        operation("quote", 2, "Q-2"),
        operation("execution", 3, "E-3"),
    ];
    let mut encoded = MarketOperation::arrow_reader(expected.clone(), Some(2), None).unwrap();
    assert_eq!(encoded.schema().fields().len(), 50);
    assert_eq!(encoded.schema().field(0).name(), "operationkind");
    assert_eq!(encoded.schema().field(1).name(), "currunix");
    assert_eq!(encoded.schema().field(20).name(), "px");

    let first = encoded.next().unwrap().unwrap();
    let second = encoded.next().unwrap().unwrap();
    assert_eq!(first.num_rows(), 2);
    assert_eq!(second.num_rows(), 1);
    assert!(encoded.next().is_none());

    let source = batch_reader(first.schema(), [first, second]);
    let actual = MarketOperation::from_arrow_reader(source)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn empty_snapshot_controls_round_trip_as_operations() {
    let mut event = MarketEventData::at(4);
    event.set_crosscode("RESET".to_owned());
    event.set_symbolticker(Some("ACME".to_owned()));
    event.set_identifiers(std::collections::BTreeMap::from([
        ("BookScope".to_owned(), "ACME".to_owned()),
        ("MDUpdateAction".to_owned(), "SNAPSHOT".to_owned()),
    ]));
    event.finalize();
    let expected = MarketOperation::Snapshot(event);
    let batches = MarketOperation::arrow_reader([expected.clone()], Some(1), Some(1024)).unwrap();
    let actual = MarketOperation::from_arrow_reader(batches)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn books_round_trip_with_live_deltas_and_executions() {
    let expected = vec![book(10), Book::new(20, "EMPTY")];
    assert_eq!(expected[0].bid().live().count(), 1);
    assert_eq!(expected[0].bid().deltas().len(), 1);
    assert_eq!(expected[0].ask().live().count(), 1);
    assert_eq!(expected[0].ask().deltas().len(), 1);
    assert_eq!(expected[0].executions().len(), 1);

    let mut encoded = Book::arrow_reader(expected.clone(), Some(1), None).unwrap();
    assert_eq!(encoded.schema().fields().len(), 52);
    assert_eq!(encoded.schema().field(0).name(), "currunix");
    assert_eq!(encoded.schema().field(19).name(), "px");
    assert_eq!(encoded.schema().field(49).name(), "bid");
    assert_eq!(encoded.schema().field(50).name(), "ask");
    assert_eq!(encoded.schema().field(51).name(), "executions");

    let first = encoded.next().unwrap().unwrap();
    let second = encoded.next().unwrap().unwrap();
    assert_eq!(first.num_rows(), 1);
    assert_eq!(second.num_rows(), 1);
    assert!(encoded.next().is_none());

    let source = batch_reader(first.schema(), [first, second]);
    let actual = Book::from_arrow_reader(source)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn encoding_respects_row_and_byte_bounds_without_pulling_ahead() {
    struct Counted {
        pulled: Arc<AtomicUsize>,
        next: i64,
    }

    impl Iterator for Counted {
        type Item = MarketOperation;

        fn next(&mut self) -> Option<Self::Item> {
            (self.next < 3).then(|| {
                self.pulled.fetch_add(1, Ordering::SeqCst);
                let next = self.next;
                self.next += 1;
                operation("order", next + 1, &format!("O-{next}"))
            })
        }
    }

    let pulled = Arc::new(AtomicUsize::new(0));
    let mut reader = MarketOperation::arrow_reader(
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
        Ok(operation("order", 1, "O-1")),
        Err(yggdryl::Error::InvalidRecord {
            path: "$[1]".into(),
            reason: "broken source".into(),
        }),
    ];
    let mut reader = MarketOperation::arrow_reader(source, Some(2), None).unwrap();
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
        MarketOperation::arrow_reader([operation("order", 1, "O-1"), invalid], Some(2), None)
            .unwrap();
    assert_eq!(reader.next().unwrap().unwrap().num_rows(), 1);
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$[1].curruuid"), "{error}");
    assert!(reader.next().is_none());
}

#[test]
fn decoding_names_an_unknown_kind_then_fuses() {
    let mut encoded =
        MarketOperation::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let mut columns = batch.columns().to_vec();
    columns[0] = Arc::new(StringArray::from(vec!["trade"])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source: BatchReader = batch_reader(batch.schema(), [batch]);
    let mut decoded = MarketOperation::from_arrow_reader(source).unwrap();
    let error = decoded.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("operationkind"), "{error}");
    assert!(error.contains("trade"), "{error}");
    assert!(decoded.next().is_none());
}

#[test]
fn decoding_refuses_identity_facts_not_derived_from_the_row() {
    let mut encoded =
        MarketOperation::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let mut columns = batch.columns().to_vec();
    columns[12] = Arc::new(UInt64Array::from(vec![u64::MAX])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = MarketOperation::from_arrow_reader(source)
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
        MarketOperation::arrow_reader([operation("order", 1, "O-1")], Some(1), None).unwrap();
    let batch = encoded.next().unwrap().unwrap();
    let column = batch.schema().index_of("miccode").unwrap();
    let mut columns = batch.columns().to_vec();
    columns[column] = Arc::new(StringArray::from(vec![Some("ABCDE")])) as ArrayRef;
    let batch = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let source = batch_reader(batch.schema(), [batch]);
    let error = MarketOperation::from_arrow_reader(source)
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

#[test]
fn book_encoding_refuses_a_stale_summary_at_the_source_row() {
    let mut invalid = book(10);
    invalid.set_px(Decimal18::from_int(999));
    let mut encoded = Book::arrow_reader([invalid], Some(1), None).unwrap();
    let error = encoded.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("$[0].px"), "{error}");
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
    let mut columns = batch.columns().to_vec();
    columns[19] = Arc::new(
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
    assert!(error.contains("$[0].px"), "{error}");
}
