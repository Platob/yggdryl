//! `rust/src/arrow/rows.rs`: the row widening no caller can name.
//!
//! `reader` is what every record I/O method hands back, so what it pulls, when
//! it pulls it, and what it answers after a row that does not convert is
//! reached through `yggdryl::internals`. Everything a caller can observe is in
//! `rust/tests/arrow/value.rs` and `rust/tests/arrow/combined.rs`.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Int32Array, RecordBatchReader as _};
use arrow_schema::ArrowError;

use yggdryl::internals::arrow_rows::reader;
use yggdryl::{DataType, Error as CoreError, Field, Scalar, StructType};

fn field() -> Field {
    StructType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::utf8().nullable_field("name"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

#[derive(Clone)]
struct Row {
    id: i32,
    name: Option<&'static str>,
}

impl From<Row> for Scalar {
    fn from(row: Row) -> Self {
        Scalar::from_sequence([
            Scalar::from(row.id),
            row.name.map_or(Scalar::Null, Scalar::from),
        ])
    }
}

struct Counted {
    next: usize,
    end: usize,
    pulls: Arc<AtomicUsize>,
}

impl Iterator for Counted {
    type Item = Row;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        self.pulls.fetch_add(1, Ordering::Relaxed);
        let id = self.next as i32;
        self.next += 1;
        Some(Row {
            id,
            name: Some("row"),
        })
    }
}

#[test]
fn custom_structs_stream_in_bounded_batches() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let rows = Counted {
        next: 0,
        end: 5,
        pulls: Arc::clone(&pulls),
    };
    let mut batches = reader(&field(), rows, Some(2), None, None, None).unwrap();
    assert_eq!(pulls.load(Ordering::Relaxed), 0);

    let first = batches.next().unwrap().unwrap();
    assert_eq!(first.num_rows(), 2);
    assert_eq!(pulls.load(Ordering::Relaxed), 2);
    assert_eq!(
        first
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[0, 1]
    );
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
    assert!(batches.next().is_none());
    assert_eq!(pulls.load(Ordering::Relaxed), 5);
}

#[test]
fn empty_rows_keep_the_declared_schema_without_a_pull() {
    let mut batches = reader::<_, Scalar>(&field(), [], None, None, None, None).unwrap();
    assert_eq!(batches.schema(), field().into_arrow_schema().unwrap());
    assert!(batches.next().is_none());
}

#[test]
fn zero_batch_row_size_still_makes_forward_progress() {
    let rows = [Row { id: 1, name: None }, Row { id: 2, name: None }];
    let batches = reader(&field(), rows, Some(0), None, None, None).unwrap();
    assert_eq!(
        batches
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        2
    );
}

#[test]
fn an_invalid_row_follows_the_completed_batch_prefix_and_fuses_the_reader() {
    let rows = [
        Scalar::from_sequence([Scalar::from(1_i32), Scalar::from("ok")]),
        Scalar::from_sequence([Scalar::from("wrong"), Scalar::from("bad")]),
        Scalar::from_sequence([Scalar::from(3_i32), Scalar::from("unread")]),
    ];
    let mut batches = reader(&field(), rows, Some(3), None, None, None).unwrap();
    let prefix = batches.next().unwrap().unwrap();
    assert_eq!(prefix.num_rows(), 1);
    assert_eq!(
        prefix
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[1]
    );
    let error = batches.next().unwrap().unwrap_err();
    let ArrowError::ExternalError(error) = error else {
        panic!("expected a typed external error")
    };
    let error = error.downcast::<CoreError>().unwrap();
    assert!(matches!(*error, CoreError::InvalidRecord { .. }));
    assert!(batches.next().is_none());
}

#[test]
fn empty_struct_rows_preserve_their_row_count() {
    let root = DataType::from(StructType::from_fields([]).unwrap()).required_field("empty");
    let mut batches = reader(
        &root,
        [Scalar::from_sequence([]), Scalar::from_sequence([])],
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let batch = batches.next().unwrap().unwrap();
    assert_eq!(batch.num_columns(), 0);
    assert_eq!(batch.num_rows(), 2);
}

#[test]
fn infallible_into_value_uses_the_standard_try_into_path() {
    fn assert_error(_: Infallible) -> CoreError {
        unreachable!()
    }
    let _ = assert_error as fn(Infallible) -> CoreError;
    let batches = reader(
        &field(),
        [Row {
            id: 7,
            name: Some("x"),
        }],
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(batches.count(), 1);
}

#[test]
fn batches_align_to_non_divisible_commit_and_global_row_boundaries() {
    let pulls = Arc::new(AtomicUsize::new(0));
    let rows = Counted {
        next: 0,
        end: 10,
        pulls: Arc::clone(&pulls),
    };
    let mut batches = reader(&field(), rows, Some(2), None, Some(3), Some(5)).unwrap();

    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
    assert_eq!(pulls.load(Ordering::Relaxed), 3);
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
    assert_eq!(pulls.load(Ordering::Relaxed), 5);
    assert!(batches.next().is_none());
    assert_eq!(pulls.load(Ordering::Relaxed), 5);
}
