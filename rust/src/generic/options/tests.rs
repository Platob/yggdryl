//! The row and byte limits, exercised through the one shaping seam.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};

use crate::arrow::BatchReader;
use crate::generic::{IORecordOptions, RecordOptions};
use crate::ipc::IpcOptions;
use crate::{DataType, Field, Url};

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row")
}

/// One batch holding `ids` as its only column.
fn batch(ids: std::ops::Range<i64>) -> RecordBatch {
    RecordBatch::try_new(
        crate::arrow::schema_from_field(&schema()).unwrap(),
        vec![Arc::new(Int64Array::from_iter_values(ids))],
    )
    .unwrap()
}

/// A reader over `count` batches of `per_batch` rows each.
fn reader(count: i64, per_batch: i64) -> BatchReader {
    let batches: Vec<RecordBatch> = (0..count)
        .map(|index| batch(index * per_batch..(index + 1) * per_batch))
        .collect();
    crate::arrow::batch_reader(crate::arrow::schema_from_field(&schema()).unwrap(), batches)
}

/// The total rows a limited reader yields.
fn rows(reader: BatchReader) -> usize {
    reader.map(|batch| batch.unwrap().num_rows()).sum()
}

#[test]
fn a_zero_row_limit_reads_the_schema_and_no_batches() {
    let options = IpcOptions::new().with_max_row_size(0);
    let mut limited = options.limit_arrow_reader(reader(3, 2)).unwrap();

    // The schema still answers - `Some(0)` is a valid ask, not an error.
    assert_eq!(
        limited.schema(),
        crate::arrow::schema_from_field(&schema()).unwrap()
    );
    assert!(limited.next().is_none());
}

#[test]
fn a_zero_byte_limit_reads_the_schema_and_no_batches() {
    let options = IpcOptions::new().with_max_byte_size(0);
    let mut limited = options.limit_arrow_reader(reader(3, 2)).unwrap();

    assert_eq!(
        limited.schema(),
        crate::arrow::schema_from_field(&schema()).unwrap()
    );
    assert!(limited.next().is_none());
}

#[test]
fn a_row_limit_at_exactly_the_stored_count_keeps_every_row() {
    let options = IpcOptions::new().with_max_row_size(6);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

#[test]
fn a_row_limit_one_below_the_stored_count_slices_the_last_batch() {
    let options = IpcOptions::new().with_max_row_size(5);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(reader(3, 2))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // The bound is exact: the last batch is cut mid-way, not dropped whole.
    assert_eq!(
        batches
            .iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [2, 2, 1]
    );
}

#[test]
fn a_row_limit_one_above_the_stored_count_changes_nothing() {
    let options = IpcOptions::new().with_max_row_size(7);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

#[test]
fn a_byte_limit_landing_mid_batch_slices_a_view_of_the_last_batch() {
    let source = batch(0..8);
    let size = u64::try_from(source.get_array_memory_size()).unwrap();
    // One byte short of the whole batch: the last row no longer fits, and the
    // seven that do are handed out as a slice.
    let options = IpcOptions::new().with_max_byte_size(size - 1);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(crate::arrow::batch_reader(
            source.schema(),
            [source.clone()],
        ))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 7);
    // A slice is a view: the cut batch's column still points at the source's
    // buffer rather than at a copy of it.
    assert_eq!(
        batches[0].column(0).to_data().buffers()[0].as_ptr(),
        source.column(0).to_data().buffers()[0].as_ptr()
    );
}

#[test]
fn a_nonzero_byte_limit_smaller_than_one_row_still_yields_one_row() {
    let options = IpcOptions::new().with_max_byte_size(1);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(reader(2, 4))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // A bounded read must never be a silent total loss: only `Some(0)` yields
    // nothing.
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
}

#[test]
fn whichever_limit_binds_first_wins() {
    // The byte budget admits everything, so the row bound is what cuts.
    let generous = IpcOptions::new()
        .with_max_row_size(3)
        .with_max_byte_size(u64::MAX);
    assert_eq!(rows(generous.limit_arrow_reader(reader(3, 2)).unwrap()), 3);

    // One byte admits one row, so the byte bound cuts before the row bound.
    let tight = IpcOptions::new().with_max_row_size(3).with_max_byte_size(1);
    assert_eq!(rows(tight.limit_arrow_reader(reader(3, 2)).unwrap()), 1);
}

/// A reader counting how often its source is pulled.
struct Counting {
    inner: BatchReader,
    pulls: Arc<std::sync::atomic::AtomicUsize>,
}

impl Iterator for Counting {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.next()
    }
}

impl RecordBatchReader for Counting {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }
}

#[test]
fn a_satisfied_limit_stops_pulling_the_inner_reader() {
    let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Box::new(Counting {
        inner: reader(20, 2),
        pulls: Arc::clone(&pulls),
    });
    let options = IpcOptions::new().with_max_row_size(10);
    let mut limited = options.limit_arrow_reader(counted).unwrap();

    // At most one batch is retained: every yield costs exactly one pull, so
    // nothing is prefetched or buffered behind the caller's back.
    for served in 1..=5 {
        assert_eq!(limited.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), served);
    }

    // The tenth row satisfied the limit, so the source is never touched
    // again - fifteen batches of the file go undecoded.
    assert!(limited.next().is_none());
    assert!(limited.next().is_none());
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 5);
}

#[test]
fn a_limit_with_a_match_key_is_refused_naming_both_settings() {
    let options = IpcOptions::new()
        .with_max_row_size(10)
        .with_merge_by_names(["id"]);
    let Err(error) = options.limit_arrow_reader(reader(1, 2)) else {
        panic!("a limited merge must be refused");
    };
    let message = error.to_string();

    assert!(message.contains("max_row_size = 10"), "{message}");
    assert!(message.contains("merge_by_names [\"id\"]"), "{message}");
}

#[test]
fn an_empty_batch_passes_through_without_ending_the_stream() {
    let empty = RecordBatch::new_empty(crate::arrow::schema_from_field(&schema()).unwrap());
    let source = crate::arrow::batch_reader(empty.schema(), [empty, batch(0..2)]);
    let options = IpcOptions::new().with_max_row_size(2);
    assert_eq!(rows(options.limit_arrow_reader(source).unwrap()), 2);
}

#[test]
fn the_enum_mirrors_the_limits_of_the_encoding_it_holds() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let options = RecordOptions::for_media_type(&media_type)
        .unwrap()
        .with_max_row_size(7)
        .with_max_byte_size(1024);

    assert_eq!(options.max_row_size(), Some(7));
    assert_eq!(options.max_byte_size(), Some(1024));
    let RecordOptions::Ipc(inner) = options else {
        panic!("an arrows handle names the IPC encoding");
    };
    assert_eq!(inner.max_row_size, Some(7));
    assert_eq!(inner.max_byte_size, Some(1024));
}

#[test]
fn a_reader_without_limits_is_returned_as_it_stands() {
    // The default options carry no bounds, so the seam costs nothing when
    // unused: six rows flow untouched, exactly as the source yields them.
    let options = IpcOptions::new();
    assert_eq!(options.max_row_size(), None);
    assert_eq!(options.max_byte_size(), None);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}
