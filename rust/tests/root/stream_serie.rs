//! `rust/src/stream_serie.rs`: a lazy stream of rows under one record field
//! - built from rows, crossed to Arrow batches and read back.

use yggdryl::{DataType, Field, Scalar, StreamSerie, StructType};

fn trade_root() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Int64.required_field("size"),
    ])
    .map(DataType::from)
    .expect("a record datatype")
    .required_field("trade")
}

fn trade(id: i64, size: i64) -> Scalar {
    Scalar::from_sequence([Scalar::from(id), Scalar::from(size)])
}

#[test]
fn rows_are_pulled_as_they_are_asked_for() {
    let pulled = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&pulled);
    let mut rows = StreamSerie::from_rows(
        trade_root(),
        (1..=3_i64).map(move |id| {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(trade(id, id * 10))
        }),
    );
    assert_eq!(rows.field().name(), "trade");
    assert_eq!(
        pulled.load(std::sync::atomic::Ordering::Relaxed),
        0,
        "nothing pulled yet"
    );
    assert_eq!(rows.next().expect("a row").expect("ok"), trade(1, 10));
    assert_eq!(
        rows.collect_rows().expect("the rest"),
        [trade(2, 20), trade(3, 30)]
    );
}

#[test]
fn rows_cross_to_arrow_batches_and_back() {
    let rows = StreamSerie::from_rows(trade_root(), [Ok(trade(1, 10)), Ok(trade(2, 20))]);
    let batches = rows.into_arrow_reader().expect("a reader");
    let back = StreamSerie::from_arrow_reader(batches).expect("rows again");
    assert_eq!(back.field().fields().len(), 2);
    assert_eq!(
        back.collect_rows().expect("every row"),
        [trade(1, 10), trade(2, 20)]
    );
}

#[test]
fn a_failed_row_is_the_item_where_it_stands() {
    let rows = StreamSerie::from_rows(
        trade_root(),
        [
            Ok(trade(1, 10)),
            Err(yggdryl::Error::InvalidRecord {
                path: "trade".into(),
                reason: "a broken row".into(),
            }),
        ],
    );
    let refused = rows.collect_rows().expect_err("the failure");
    assert!(refused.to_string().contains("a broken row"), "{refused}");
}
