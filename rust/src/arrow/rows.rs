//! Lazy row-value widening for the record I/O adapters.
//!
//! A row canonicalizes to one ordered
//! [`Scalar::Sequence`](crate::Scalar::Sequence) under a non-null Struct
//! [`Field`]. A sorted [`Scalar::Record`](crate::Scalar::Record) is the named
//! input shape, not a second schema model. Rust structs opt in with
//! `TryInto<Scalar>`, and the I/O methods widen that iterator into the one Arrow
//! reader primitive.

use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchOptions, StructArray};
use arrow_schema::{ArrowError, SchemaRef};

use crate::{Field, Scalar};

use super::{BatchReader, Error, Result, arrow_schema_from_field};

/// Default number of row values materialized into one Arrow batch.
pub(crate) const DEFAULT_BATCH_ROW_SIZE: usize = crate::generic::DEFAULT_RECORD_BATCH_ROW_SIZE;

/// Widen native row values into a lazy Arrow reader.
///
/// Only the current batch is held.  The source is not touched during
/// construction, and each call to `next` pulls at most `batch_row_size` rows.
/// After the first conversion, validation, or materialization failure the
/// reader is fused.
pub(crate) fn reader<I, R>(
    field: &Field,
    rows: I,
    batch_row_size: Option<usize>,
    commit_row_size: Option<usize>,
    max_row_size: Option<u64>,
) -> Result<BatchReader>
where
    I: IntoIterator<Item = R>,
    I::IntoIter: Send + 'static,
    R: TryInto<Scalar>,
    R::Error: Into<crate::Error>,
{
    let schema = arrow_schema_from_field(field)?;
    Ok(Box::new(Rows {
        rows: rows.into_iter(),
        field: field.clone(),
        schema,
        batch_row_size: batch_row_size.unwrap_or(DEFAULT_BATCH_ROW_SIZE).max(1),
        commit_row_size,
        rows_to_commit: commit_row_size,
        remaining_rows: max_row_size,
        done: false,
    }))
}

/// The one bounded row-to-batch iterator.
struct Rows<I> {
    rows: I,
    field: Field,
    schema: SchemaRef,
    batch_row_size: usize,
    commit_row_size: Option<usize>,
    rows_to_commit: Option<usize>,
    remaining_rows: Option<u64>,
    done: bool,
}

impl<I> Iterator for Rows<I>
where
    I: Iterator,
    I::Item: TryInto<Scalar>,
    <I::Item as TryInto<Scalar>>::Error: Into<crate::Error>,
{
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        // A native row must not be converted past either an observable
        // publication boundary or the operation-wide row limit. In
        // particular, `batch_row_size = 1024` and `commit_row_size = 1500` must
        // yield 1024 then 476 rows: touching row 1501 before the 1500-row
        // prefix has been handed to the writer would make a conversion error
        // erase a prefix that was ready to publish.
        let mut row_size = self.batch_row_size;
        if let Some(remaining) = self.rows_to_commit {
            row_size = row_size.min(remaining);
        }
        if let Some(remaining) = self.remaining_rows {
            row_size = row_size.min(usize::try_from(remaining).unwrap_or(usize::MAX));
        }
        if row_size == 0 {
            self.done = true;
            return None;
        }

        // `batch_row_size` is caller-controlled. Do not reserve it eagerly: an
        // enormous bound over a two-row iterator must not request an enormous
        // allocation. Vec's growth remains bounded by rows actually pulled.
        let mut values = Vec::new();
        for _ in 0..row_size {
            let Some(row) = self.rows.next() else {
                self.done = true;
                break;
            };
            let value = match row.try_into().map_err(Into::into) {
                Ok(value) => value,
                Err(error) => {
                    self.done = true;
                    return Some(Err(external(error)));
                }
            };
            // Validation owns shape, arity, nullability, and the error path.
            // Canonicalization then narrows values into their declared native
            // representation without repeating that validation walk.
            if let Err(error) = self.field.validate_value(&value) {
                self.done = true;
                return Some(Err(external(error)));
            }
            match self.field.canonicalize_value(value) {
                Ok(value) => values.push(value),
                Err(error) => {
                    self.done = true;
                    return Some(Err(external(error)));
                }
            }
        }

        if values.is_empty() {
            return None;
        }
        let batch = batch_from_values(&self.field, Arc::clone(&self.schema), &values)
            .map_err(|error| ArrowError::ExternalError(Box::new(error)));
        if batch.is_err() {
            self.done = true;
        } else {
            let accepted = values.len();
            if let Some(remaining) = &mut self.remaining_rows {
                *remaining -= u64::try_from(accepted).unwrap_or(u64::MAX);
                if *remaining == 0 {
                    self.done = true;
                }
            }
            if let Some(remaining) = &mut self.rows_to_commit {
                *remaining -= accepted;
                if *remaining == 0 {
                    *remaining = self
                        .commit_row_size
                        .expect("a cadence remainder has a cadence");
                }
            }
        }
        Some(batch)
    }
}

impl<I> arrow_array::RecordBatchReader for Rows<I>
where
    I: Iterator + Send,
    I::Item: TryInto<Scalar>,
    <I::Item as TryInto<Scalar>>::Error: Into<crate::Error>,
{
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

pub(super) fn batch_from_values(
    field: &Field,
    schema: SchemaRef,
    values: &[Scalar],
) -> Result<RecordBatch> {
    let refs: Vec<&Scalar> = values.iter().collect();
    let array = super::value::array_from_values(field, &refs)?;
    let struct_array = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| Error::internal("arrow::rows::batch_from_values"))?;
    let options = RecordBatchOptions::new().with_row_count(Some(values.len()));
    Ok(RecordBatch::try_new_with_options(
        schema,
        struct_array.columns().to_vec(),
        &options,
    )?)
}

fn external(error: crate::Error) -> ArrowError {
    ArrowError::ExternalError(Box::new(error))
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::{Int32Array, RecordBatchReader as _};
    use arrow_schema::ArrowError;

    use crate::{DataType, Error as CoreError, Field, Scalar};

    use super::reader;

    fn field() -> Field {
        DataType::from_fields([
            DataType::Int32.required_field("id"),
            DataType::Utf8.nullable_field("name"),
        ])
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
        let mut batches = reader(&field(), rows, Some(2), None, None).unwrap();
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
        let mut batches = reader::<_, Scalar>(&field(), [], None, None, None).unwrap();
        assert_eq!(batches.schema(), field().into_arrow_schema().unwrap());
        assert!(batches.next().is_none());
    }

    #[test]
    fn zero_batch_row_size_still_makes_forward_progress() {
        let rows = [Row { id: 1, name: None }, Row { id: 2, name: None }];
        let batches = reader(&field(), rows, Some(0), None, None).unwrap();
        assert_eq!(
            batches
                .map(|batch| batch.unwrap().num_rows())
                .sum::<usize>(),
            2
        );
    }

    #[test]
    fn invalid_row_is_typed_and_fuses_the_reader() {
        let rows = [
            Scalar::from_sequence([Scalar::from(1_i32), Scalar::from("ok")]),
            Scalar::from_sequence([Scalar::from("wrong"), Scalar::from("bad")]),
            Scalar::from_sequence([Scalar::from(3_i32), Scalar::from("unread")]),
        ];
        let mut batches = reader(&field(), rows, Some(3), None, None).unwrap();
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
        let root = DataType::from_fields([]).unwrap().required_field("empty");
        let mut batches = reader(
            &root,
            [Scalar::from_sequence([]), Scalar::from_sequence([])],
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
        let mut batches = reader(&field(), rows, Some(2), Some(3), Some(5)).unwrap();

        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
        assert_eq!(pulls.load(Ordering::Relaxed), 3);
        assert_eq!(batches.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(pulls.load(Ordering::Relaxed), 5);
        assert!(batches.next().is_none());
        assert_eq!(pulls.load(Ordering::Relaxed), 5);
    }
}
