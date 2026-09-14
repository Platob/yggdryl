//! The merge reader an integration test cannot reach.
//!
//! `merged` is the crate-private reader every `IOMode::Merge` write pulls
//! through, and that it releases each incoming batch before pulling the next
//! is what keeps a merge bounded. Everything a caller can observe lives in
//! `tests/media/merge.rs`.

use std::sync::{Arc, Weak};

use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{ArrowError, SchemaRef};

use crate::arrow::BatchReader;
use crate::{DataType, Field};

fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

/// Produce payload batches only when the preceding one has been released.
///
/// This turns incoming-stream retention into a deterministic error instead of
/// relying on an allocator or a process-wide memory watermark.
struct ReleaseCheckedReader {
    schema: SchemaRef,
    next: i64,
    previous: Option<Weak<dyn Array>>,
}

impl Iterator for ReleaseCheckedReader {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self
            .previous
            .as_ref()
            .is_some_and(|array| array.strong_count() != 0)
        {
            return Some(Err(ArrowError::ComputeError(
                "the preceding incoming payload batch was retained".to_owned(),
            )));
        }
        if self.next == 2 {
            return None;
        }
        let id: ArrayRef = Arc::new(Int64Array::from(vec![self.next]));
        self.previous = Some(Arc::downgrade(&id));
        let symbol: ArrayRef = Arc::new(StringArray::from(vec![format!("symbol-{}", self.next)]));
        self.next += 1;
        Some(RecordBatch::try_new(
            Arc::clone(&self.schema),
            vec![id, symbol],
        ))
    }
}

impl arrow_array::RecordBatchReader for ReleaseCheckedReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

#[test]
fn incoming_payload_batches_are_released_before_the_next_is_pulled() {
    let arrow = schema().into_arrow_schema().unwrap();
    let stored = crate::arrow::batch_reader(Arc::clone(&arrow), []);
    let incoming: BatchReader = Box::new(ReleaseCheckedReader {
        schema: arrow,
        next: 0,
        previous: None,
    });

    let merged = super::merged(stored, incoming, &schema(), &["id".to_owned()], true).unwrap();
    let rows: usize = merged.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
}
