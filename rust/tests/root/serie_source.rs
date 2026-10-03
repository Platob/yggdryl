//! `rust/src/serie_source.rs`: the one intake a verb reading rows from a
//! caller takes - a held column, a held chunked column, or a stream.

use yggdryl::{ChunkedSerie, DataType, Field, Scalar, Serie, SerieReader, SerieSource, StructType};

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

fn trades() -> Serie {
    Serie::from_scalars(trade_root(), [trade(1, 10), trade(2, 20), trade(3, 30)])
        .expect("three trades")
}

fn names(root: &Field) -> Vec<&str> {
    root.fields().iter().map(Field::name).collect()
}

#[test]
fn a_source_names_its_root_and_whether_it_is_held() {
    let held = SerieSource::from(trades());
    assert!(held.is_held());
    assert_eq!(held.root().expect("a record root").name(), "trade");
    assert_eq!(held.memory_size(), Some(trades().memory_size()));

    // A plain column keys as the one child of a `row` record.
    let plain = SerieSource::from(
        Serie::from_scalars(DataType::Int64.required_field("id"), [Scalar::from(1_i64)])
            .expect("a column"),
    );
    let root = plain.root().expect("a record root");
    assert_eq!(root.name(), "row");
    assert_eq!(names(&root), ["id"]);

    let chunked = SerieSource::from(ChunkedSerie::from_serie(trades()).expect("one chunk"));
    assert!(chunked.is_held());
    assert_eq!(chunked.memory_size(), Some(trades().memory_size()));
    let stream = SerieSource::from(SerieReader::from_serie(trades()).expect("a stream"));
    assert!(!stream.is_held());
    assert_eq!(stream.memory_size(), None);
    assert_eq!(stream.root().expect("the stream's root").name(), "trade");

    let run = SerieSource::from(Serie::new(vec![Scalar::from(1_i64)]));
    assert!(run.root().is_err(), "a run names no column");
}

#[test]
fn into_reader_yields_a_held_column_as_one_batch_and_chunks_one_each() {
    let held: Vec<Serie> = SerieSource::from(trades())
        .into_reader()
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("one batch");
    assert_eq!(held.len(), 1);
    assert_eq!(held[0], trades());

    let chunked = ChunkedSerie::from_series(
        Some(&trade_root()),
        [trades(), trades().slice(0, 1).expect("one row")],
        Default::default(),
    )
    .expect("two chunks");
    let chunks: Vec<Serie> = SerieSource::from(chunked)
        .into_reader()
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("two batches");
    assert_eq!(
        chunks.iter().map(Serie::len).collect::<Vec<_>>(),
        [3, 1],
        "a chunk is one batch, none joined"
    );

    // A plain column is read as the one child of a `row` record.
    let plain = Serie::from_scalars(
        DataType::Int64.required_field("id"),
        [Scalar::from(1_i64), Scalar::from(2_i64)],
    )
    .expect("a column");
    let reader = SerieSource::from(plain).into_reader().expect("a reader");
    assert_eq!(reader.field().name(), "row");
    assert_eq!(names(reader.field()), ["id"]);
    let records: Vec<Serie> = reader.collect::<Result<_, _>>().expect("one batch");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].len(), 2);
}

#[test]
fn into_reader_hands_a_stream_back_as_itself_and_refuses_a_run() {
    let stream = SerieReader::from_serie(trades()).expect("a stream");
    let reader = SerieSource::from(stream)
        .into_reader()
        .expect("the same stream");
    assert_eq!(reader.field().name(), "trade");
    assert_eq!(reader.count(), 1);

    let error = SerieSource::from(Serie::new(vec![Scalar::from(1_i64)]))
        .into_reader()
        .expect_err("a run names no layout");
    assert!(
        error.to_string().contains("run") || error.to_string().contains("field"),
        "{error}"
    );
}
