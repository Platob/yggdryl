//! The commit cadence an integration test cannot reach.
//!
//! `commit_arrow_readers` is the crate-private slicer every bounded write
//! pulls through: it cuts a stream at the declared row cadence without reading
//! ahead. Everything a caller can observe lives in `tests/media/options.rs`.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};

use crate::arrow::BatchReader;
use crate::ipc::IpcOptions;
use crate::media::{IORecordOptions, RecordOptions};
use crate::{DataType, Field, StructType};

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
}

/// One batch holding `ids` as its only column.
fn batch(ids: std::ops::Range<i64>) -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![Arc::new(Int64Array::from_iter_values(ids))],
    )
    .unwrap()
}

/// A reader over `count` batches of `per_batch` rows each.
fn reader(count: i64, per_batch: i64) -> BatchReader {
    let batches: Vec<RecordBatch> = (0..count)
        .map(|index| batch(index * per_batch..(index + 1) * per_batch))
        .collect();
    crate::arrow::batch_reader(schema().into_arrow_schema().unwrap(), batches)
}

/// The total rows a limited reader yields.
fn rows(reader: BatchReader) -> usize {
    reader.map(|batch| batch.unwrap().num_rows()).sum()
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
fn commit_readers_slice_exact_cadences_across_batch_boundaries() {
    let schema = schema().into_arrow_schema().unwrap();
    let source =
        crate::arrow::batch_reader(Arc::clone(&schema), [batch(0..2), batch(2..6), batch(6..7)]);
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(3);
    let commits = options
        .commit_arrow_readers(source)
        .unwrap()
        .map(|commit| rows(commit.unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(commits, [3, 3, 1]);
}

#[test]
fn a_commit_larger_than_the_stream_yields_one_final_remainder() {
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(20);
    let commits = options
        .commit_arrow_readers(reader(3, 2))
        .unwrap()
        .map(|commit| rows(commit.unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(commits, [6]);
}

#[test]
fn a_full_commit_does_not_read_ahead() {
    let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Box::new(Counting {
        inner: reader(2, 4),
        pulls: Arc::clone(&pulls),
    });
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(2);
    let mut commits = options.commit_arrow_readers(counted).unwrap();

    assert_eq!(rows(commits.next().unwrap().unwrap()), 2);
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The next cadence is the unconsumed slice of that same input batch.
    assert_eq!(rows(commits.next().unwrap().unwrap()), 2);
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
