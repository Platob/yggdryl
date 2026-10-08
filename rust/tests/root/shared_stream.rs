//! `rust/src/shared_stream.rs`: a stream a `Serie` holds - passed through
//! while one handle owns it untouched, held for every clone that shares it,
//! held whole by a question only held rows answer, its failure kept.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, RecordBatch, RecordBatchIterator};
use arrow_schema::{ArrowError, DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::arrow::BatchReader;
use yggdryl::{ArrowCastOptions, IOMedia, IOMode, Scalar, Serie, StreamChunkedSerie, StreamSerie};

/// One `price` batch of `values`.
fn batch(values: &[i64]) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "price",
        ArrowDataType::Int64,
        false,
    )]));
    let column: ArrayRef = Arc::new(Int64Array::from(values.to_vec()));
    RecordBatch::try_new(schema, vec![column]).expect("a batch")
}

/// `batches` as one reader, then `failure` where one is given.
fn reader(batches: &[&[i64]], failure: Option<&str>) -> BatchReader {
    let schema = batch(&[]).schema();
    let mut items: Vec<Result<RecordBatch, ArrowError>> =
        batches.iter().map(|values| Ok(batch(values))).collect();
    if let Some(failure) = failure {
        items.push(Err(ArrowError::ComputeError(failure.to_owned())));
    }
    Box::new(RecordBatchIterator::new(items, schema))
}

/// `batches` as a stream serie, nothing pulled.
fn stream(batches: &[&[i64]], failure: Option<&str>) -> Serie {
    Serie::from(
        StreamChunkedSerie::from_arrow_reader(
            None,
            reader(batches, failure),
            ArrowCastOptions::new(),
        )
        .expect("a stream"),
    )
}

/// The `price` cells of a record serie, in row order.
fn prices(serie: &Serie) -> Vec<Scalar> {
    (0..serie.len())
        .map(|row| {
            let record = serie.scalar(row).expect("a row");
            record.get(0).expect("a price").into_owned()
        })
        .collect()
}

/// `values` as the cells [`prices`] reads.
fn ints(values: &[i64]) -> Vec<Scalar> {
    values.iter().copied().map(Scalar::from).collect()
}

#[test]
fn a_stream_its_only_handle_owns_untouched_is_passed_through_as_itself() {
    let source = batch(&[1, 2]);
    let schema = source.schema();
    let serie = Serie::from(
        StreamChunkedSerie::from_arrow_reader(
            None,
            Box::new(RecordBatchIterator::new([Ok(source.clone())], schema)),
            ArrowCastOptions::new(),
        )
        .expect("a stream"),
    );
    assert!(!serie.is_held(), "nothing is pulled to make it a serie");
    assert_eq!(
        serie.memory_size(),
        0,
        "a stream counts what it holds so far"
    );
    let mut back = StreamChunkedSerie::from_serie(serie)
        .expect("the stream itself")
        .into_arrow_reader();
    let first = back.next().expect("a batch").expect("transport");
    assert!(
        Arc::ptr_eq(first.column(0), source.column(0)),
        "the source batch itself"
    );
    assert!(back.next().is_none());
}

#[test]
fn clones_share_one_pull_and_every_clone_reads_every_row() {
    let serie = stream(&[&[1, 2], &[3]], None);
    let clone = serie.clone();
    // One clone streams: each chunk it pulls is held for the other.
    let streamed: Vec<Serie> = StreamChunkedSerie::from_serie(clone)
        .expect("a stream over the shared one")
        .into_chunks()
        .collect::<Result<_, _>>()
        .expect("every chunk");
    assert_eq!(streamed.iter().map(Serie::len).collect::<Vec<_>>(), [2, 1]);
    // The other answers from what was held, pulling nothing again.
    assert_eq!(serie.len(), 3);
    assert!(serie.is_held());
    assert_eq!(prices(&serie), ints(&[1, 2, 3]));
    assert_eq!(serie.field().expect("a root").field_len(), 1);
}

#[test]
fn a_question_only_held_rows_answer_holds_the_rest_and_one_column_joins_them_once() {
    let serie = stream(&[&[1, 2], &[3]], None);
    assert_eq!(serie.len(), 3, "a length holds every row");
    assert!(serie.memory_size() > 0);
    // A typed narrowing asks for one column: the held chunks joined, kept.
    let record = serie.as_struct().expect("a record column");
    assert_eq!(record.children()[0].len(), 3);
    assert!(std::ptr::eq(
        serie.as_struct().expect("the same join"),
        record
    ));
    // Identity is the rows alone.
    let held = Serie::from_scalars(
        serie.field().expect("a root").clone(),
        (1..=3_i64).map(|price| Scalar::from_sequence([Scalar::from(price)])),
    )
    .expect("the rows held");
    assert_eq!(serie, held);
}

#[test]
fn a_failed_stream_answers_what_it_held_and_raises_its_failure_after() {
    let serie = stream(&[&[1, 2]], Some("the wire dropped"));
    assert_eq!(serie.len(), 2, "what was held before the failure");
    let refused = serie.scalar(0).expect_err("the failure, raised");
    assert!(
        refused.to_string().contains("could not hold its rows")
            && refused.to_string().contains("the wire dropped"),
        "{refused}"
    );
    // A clone shares the failure.
    assert!(serie.clone().scalar(1).is_err());
}

#[test]
fn a_shared_streams_reader_raises_the_failure_where_the_stream_ended() {
    let serie = stream(&[&[1]], Some("cut"));
    let clone = serie.clone();
    let mut chunks = StreamChunkedSerie::from_serie(clone)
        .expect("a stream")
        .into_chunks();
    assert_eq!(chunks.next().expect("a chunk").expect("held").len(), 1);
    let failure = chunks.next().expect("the failure").expect_err("raised");
    assert!(failure.to_string().contains("cut"), "{failure}");
    assert!(chunks.next().is_none(), "fused");
}

#[test]
fn a_generic_serie_exports_its_chunks_without_joining_them() {
    let serie = stream(&[&[1, 2], &[3]], None);
    let sizes: Vec<_> = serie
        .into_arrow_reader()
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .collect();
    assert_eq!(
        sizes,
        [2, 1],
        "composite export keeps the source batches apart"
    );
}

#[test]
fn a_row_stream_is_a_serie_of_its_rows() {
    let rows = StreamSerie::from_arrow_reader(reader(&[&[4, 5]], None)).expect("rows");
    let serie = Serie::from(rows);
    assert_eq!(serie.len(), 2);
    assert_eq!(prices(&serie), ints(&[4, 5]));
}

#[test]
fn a_failed_row_stream_keeps_the_rows_before_its_failure() {
    let root = StreamChunkedSerie::from_serie(
        Serie::from_scalars(
            yggdryl::DataType::Int64.required_field("price"),
            [Scalar::from(1_i64)],
        )
        .unwrap(),
    )
    .unwrap()
    .field()
    .clone();
    let rows = StreamSerie::from_rows(
        root,
        [
            Ok(Scalar::from_sequence([Scalar::from(1_i64)])),
            Err(yggdryl::Error::InvalidRecord {
                path: "row".into(),
                reason: "cut after one row".into(),
            }),
        ],
    );
    let serie = Serie::from(rows);
    assert_eq!(serie.len(), 1, "the prefix stays held before the error");
    assert!(
        serie
            .scalar(0)
            .unwrap_err()
            .to_string()
            .contains("cut after one row")
    );
}

#[test]
fn every_kind_is_written_and_read_back_through_the_serie_doors() {
    let mut handle = yggdryl::holder::Buffer::new().with_media_type(
        yggdryl::Url::from_str("file:///prices.arrows")
            .expect("a url")
            .media_type(),
    );
    let written = handle
        .write_serie(stream(&[&[1, 2], &[3]], None), IOMode::Overwrite, None)
        .expect("a stream written");
    assert_eq!(written.written_rows, 3);
    let rows = StreamSerie::from_arrow_reader(reader(&[&[4]], None)).expect("rows");
    handle
        .append_serie(Serie::from(rows), None)
        .expect("a row stream appended");
    let read = Serie::from(
        yggdryl::StreamChunkedSerie::from_serie(handle.read_serie(None).expect("a read"))
            .expect("native record stream"),
    );
    assert_eq!(prices(&read), ints(&[1, 2, 3, 4]));
    // The Arrow door is the serie door's transport face.
    let batches: usize = handle
        .read_arrow_reader(&handle.record_options().expect("its options"))
        .expect("a read")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(batches, 4);
}
