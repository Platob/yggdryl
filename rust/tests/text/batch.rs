//! `rust/src/text/batch.rs`: lines to Arrow columns and back.
//!
//! The column-first build and the lazy reverse read are both reachable as
//! `yggdryl::text`, so nothing here needs a door: what a caller can ask for
//! is exactly what these pin.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{ArrayRef, Int64Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};

use yggdryl::Result;
use yggdryl::arrow::BatchReader;
use yggdryl::text::{
    TextBytes, TextLine, TextOptions, from_arrow_batch, from_arrow_reader, into_arrow_batch,
    into_arrow_reader,
};

fn line(index: u64, body: &str) -> TextLine {
    TextLine::from_bytes(
        index,
        TextBytes::from_bytes(body).expect("bytes"),
        Arc::new(TextOptions::new()),
    )
    .expect("line")
}

fn batch(fields: Vec<ArrowField>, columns: Vec<ArrayRef>) -> RecordBatch {
    RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).expect("batch")
}

fn bodies(values: Vec<Option<&str>>) -> RecordBatch {
    batch(
        vec![ArrowField::new("body", ArrowType::Utf8, true)],
        vec![Arc::new(StringArray::from(values))],
    )
}

fn reader(
    schema: arrow_schema::SchemaRef,
    steps: Vec<Option<std::result::Result<RecordBatch, arrow_schema::ArrowError>>>,
    pulls: &Arc<AtomicUsize>,
) -> BatchReader {
    let pulls = Arc::clone(pulls);
    let mut steps = steps.into_iter();
    Box::new(RecordBatchIterator::new(
        std::iter::from_fn(move || {
            pulls.fetch_add(1, Ordering::SeqCst);
            steps.next().flatten()
        }),
        schema,
    ))
}

#[test]
fn forward_doors_accept_owned_and_fallible_lines_and_preserve_errors() {
    let options = TextOptions::new();
    let owned = into_arrow_batch([line(0, "one")], &options).expect("owned");
    let fallible = into_arrow_batch([Ok(line(0, "one"))], &options).expect("fallible");
    assert_eq!(owned, fallible);
    let error = yggdryl::Error::InvalidRecord {
        path: "$.source".into(),
        reason: "original failure".into(),
    };
    let error = into_arrow_batch([Err::<TextLine, _>(error)], &options).expect_err("source error");
    assert!(
        matches!(error, yggdryl::Error::InvalidRecord { path, reason }
        if path == "$.source" && reason == "original failure")
    );
    let source = vec![Ok(line(0, "one")), Ok(line(1, "two"))];
    let read = into_arrow_reader(source, &options).expect("reader");
    assert_eq!(
        read.map(|held| held.expect("batch").num_rows())
            .sum::<usize>(),
        2
    );
    let read = into_arrow_reader([line(0, "one")], &options).expect("owned reader");
    assert_eq!(
        read.map(|held| held.expect("batch").num_rows())
            .sum::<usize>(),
        1
    );
}

#[test]
fn reverse_is_lazy_one_row_at_a_time_and_fuses_a_bad_later_row() {
    let source = bodies(vec![Some("first"), None, Some("never")]);
    let pulls = Arc::new(AtomicUsize::new(0));
    let source = reader(source.schema(), vec![Some(Ok(source))], &pulls);
    let mut read = from_arrow_reader(source, &TextOptions::new()).expect("plan");
    assert_eq!(pulls.load(Ordering::SeqCst), 0);
    assert_eq!(read.next().expect("first").expect("valid").body(), "first");
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
    let error = read.next().expect("error").expect_err("null body");
    assert!(error.to_string().contains("$[1].body"), "{error}");
    assert!(read.next().is_none());
    assert!(read.next().is_none());
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
}

#[test]
fn source_schema_drift_and_errors_fuse_without_pulling_later_batches() {
    let good = bodies(vec![Some("first")]);
    let drift = batch(
        vec![ArrowField::new("payload", ArrowType::Utf8, false)],
        vec![Arc::new(StringArray::from(vec!["other"]))],
    );
    for problem in [
        Ok(drift),
        Err(arrow_schema::ArrowError::ParseError(
            "source sentinel".into(),
        )),
    ] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = reader(
            good.schema(),
            vec![
                Some(Ok(good.clone())),
                Some(problem),
                Some(Ok(good.clone())),
            ],
            &pulls,
        );
        let mut read = from_arrow_reader(source, &TextOptions::new()).expect("plan");
        assert!(read.next().expect("prefix").is_ok());
        assert!(read.next().expect("failure").is_err());
        assert!(read.next().is_none());
        assert!(read.next().is_none());
        assert_eq!(pulls.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn eof_fuses_even_a_source_that_would_resume() {
    let good = bodies(vec![Some("never")]);
    let pulls = Arc::new(AtomicUsize::new(0));
    let source = reader(good.schema(), vec![None, Some(Ok(good))], &pulls);
    let mut read = from_arrow_reader(source, &TextOptions::new()).expect("plan");
    assert!(read.next().is_none());
    assert!(read.next().is_none());
    assert_eq!(pulls.load(Ordering::SeqCst), 1);
}

#[test]
fn row_numbers_remove_the_configured_offset_and_missing_columns_use_stream_ordinals() {
    for start in [-9, 0, 12] {
        let mut options = TextOptions::new();
        options.start_rownum = Some(start);
        let original = [line(2, "one"), line(7, "two")];
        let batch = into_arrow_batch(original, &options).expect("batch");
        let rows = from_arrow_batch(&batch, &options).expect("reverse");
        assert_eq!(rows.iter().map(TextLine::index).collect::<Vec<_>>(), [2, 7]);
    }
    let first = bodies(vec![Some("one"), Some("two")]);
    let last = bodies(vec![Some("three")]);
    let empty = first.slice(0, 0);
    let pulls = Arc::new(AtomicUsize::new(0));
    let source = reader(
        first.schema(),
        vec![Some(Ok(first)), Some(Ok(empty)), Some(Ok(last))],
        &pulls,
    );
    let rows = from_arrow_reader(source, &TextOptions::new())
        .expect("plan")
        .collect::<Result<Vec<_>>>()
        .expect("rows");
    assert_eq!(
        rows.iter().map(TextLine::index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

/// One body column beside whatever a case is about, because a row with
/// no body states no line.
fn body_field() -> ArrowField {
    ArrowField::new("body", ArrowType::Utf8, false)
}

fn body_column(rows: usize) -> ArrayRef {
    Arc::new(StringArray::from(vec!["line"; rows]))
}

#[test]
fn malformed_present_numbers_types_and_media_are_refused() {
    let mut options = TextOptions::new();
    options.start_rownum = Some(10);
    let source = batch(
        vec![
            body_field(),
            ArrowField::new("rownum", ArrowType::Int64, false),
        ],
        vec![body_column(1), Arc::new(Int64Array::from(vec![9]))],
    );
    let error = from_arrow_batch(&source, &options).expect_err("offset underflow");
    assert!(error.to_string().contains("$[0].rownum"));
    let source = batch(
        vec![
            body_field(),
            ArrowField::new("rownum", ArrowType::Utf8, false),
        ],
        vec![body_column(1), Arc::new(StringArray::from(vec!["wrong"]))],
    );
    assert!(
        from_arrow_batch(&source, &options)
            .expect_err("wrong layout")
            .to_string()
            .contains("$.rownum")
    );
    options.parse_mimetype = true;
    let source = batch(
        vec![
            body_field(),
            ArrowField::new("mimetype", ArrowType::Utf8, false),
        ],
        vec![
            body_column(1),
            Arc::new(StringArray::from(vec!["not a mime type"])),
        ],
    );
    assert!(
        from_arrow_batch(&source, &options)
            .expect_err("bad mime")
            .to_string()
            .contains("$[0].mimetype")
    );
    // A row number restored by an earlier column never relocates a later refusal.
    let source = batch(
        vec![
            body_field(),
            ArrowField::new("rownum", ArrowType::Int64, false),
            ArrowField::new("mimetype", ArrowType::Utf8, false),
        ],
        vec![
            body_column(1),
            Arc::new(Int64Array::from(vec![15])),
            Arc::new(StringArray::from(vec!["not a mime type"])),
        ],
    );
    let error = from_arrow_batch(&source, &options).expect_err("bad mime after rownum");
    assert!(error.to_string().contains("$[0].mimetype"), "{error}");
}

#[test]
fn a_row_that_states_no_body_states_no_line() {
    let options = TextOptions::new();
    // No column at all: the one absence the read path refuses, because
    // the body is what a line is made from.
    let source = batch(
        vec![ArrowField::new("rownum", ArrowType::Int64, false)],
        vec![Arc::new(Int64Array::from(vec![0]))],
    );
    let error = from_arrow_batch(&source, &options).expect_err("no body column");
    assert!(error.to_string().contains("$[0].body"), "{error}");
    assert!(
        error.to_string().contains("carries none"),
        "the refusal names the absence: {error}"
    );
    // A null cell, refused by the plan's own non-nullable column.
    let error = from_arrow_batch(&bodies(vec![None]), &options).expect_err("null body");
    assert!(error.to_string().contains("$[0].body"), "{error}");
    // An empty cell, which is a row stating nothing rather than a line.
    let error = from_arrow_batch(&bodies(vec![Some("")]), &options).expect_err("empty body");
    assert!(error.to_string().contains("$[0].body"), "{error}");
    assert!(
        error.to_string().contains("got an empty one"),
        "the refusal names the emptiness: {error}"
    );
}

#[test]
fn capture_holes_keep_indices_and_typed_values_have_canonical_text() {
    let options = TextOptions::new()
        .try_with_rowheader(r"^(?<first>[A-Z]+) (?<second>\d+) (?<third>[A-Z]+)")
        .expect("captures");
    let source = batch(
        vec![
            body_field(),
            ArrowField::new("second", ArrowType::Int64, true),
        ],
        vec![
            body_column(2),
            Arc::new(Int64Array::from(vec![Some(7), None])),
        ],
    );
    let rows = from_arrow_batch(&source, &options).expect("captures");
    assert_eq!(rows[0].captures().len(), 3);
    assert_eq!(
        (rows[0].capture(0), rows[0].capture(1), rows[0].capture(2)),
        (None, Some("7"), None)
    );
    assert!(rows[1].captures().iter().all(Option::is_none));
    let source = line(0, "body")
        .with_captures(vec![
            Some(TextBytes::from_bytes("AA").expect("text")),
            Some(TextBytes::from_bytes("0007").expect("text")),
            Some(TextBytes::from_bytes("ZZ").expect("text")),
        ])
        .expect("captures");
    let batch = into_arrow_batch([source], &options).expect("typed batch");
    let rows = from_arrow_batch(&batch, &options).expect("rows");
    assert_eq!(rows[0].capture(1), Some("7"));
}

mod text {
    use arrow_array::{Array as _, StringArray};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::text::TextOptions;

    use yggdryl::IOMedia as _;

    fn named(name: &str, bytes: &[u8]) -> Buffer {
        Buffer::from_bytes(bytes.to_vec()).with_media_type(
            yggdryl::Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    fn options(rowheader: &str) -> TextOptions {
        TextOptions::new().try_with_rowheader(rowheader).unwrap()
    }

    fn framed(rowheader: &str) -> TextOptions {
        options(rowheader).with_framing(true)
    }

    fn bodies(batches: &[arrow_array::RecordBatch]) -> Vec<Vec<u8>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("body").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .map(|value| value.unwrap().as_bytes().to_vec())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn an_unconvertible_next_record_follows_the_completed_record_batch_prefix() {
        // The second record's number cannot be represented, and the refusal
        // arrives after the batch the first record completed - never inside it.
        let source = named("invalid-next-header.log", b"A first\nB second\n");
        let mut options = framed(r"^(?<kind>(?-u:.)) ");
        options.start_rownum = Some(i64::MAX);
        options.set_batch_row_size(Some(8));
        let mut reader = source.read_arrow_reader(&options.into()).unwrap();

        let prefix = reader.next().unwrap().unwrap();
        assert_eq!(bodies(&[prefix]), [b"A first".to_vec()]);
        let error = reader.next().unwrap().unwrap_err().to_string();
        assert!(
            error.contains("text row number exceeds i64::MAX"),
            "{error}"
        );
        assert!(reader.next().is_none());
    }
}
