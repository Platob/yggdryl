//! [`DataFrame`] — the eager, Arrow-backed [`Frame`]: a materialised table over an
//! Arrow [`RecordBatch`]. Projection and row-slicing are zero-copy; `filter`
//! type-optimises the predicate (so its literals are typed) and evaluates it into a
//! boolean mask. Gated behind the `dataframe` feature.

use std::fmt;
use std::sync::Arc;

#[allow(unused_imports)]
use crate::log_event;
use crate::{ArrayColumn, CompareOp, Frame, FrameError, FrameHandle, Predicate, Scalar, Schema};

use arrow_array::builder::BooleanBuilder;
use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Date64Type, Decimal128Type, Float32Type, Float64Type, Int16Type, Int32Type,
    Int64Type, Int8Type, TimestampMicrosecondType, TimestampMillisecondType,
    TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{ArrayRef, BooleanArray, RecordBatch};
use arrow_schema::{DataType as ArrowType, TimeUnit as ArrowTimeUnit};

/// An eager, in-memory dataframe backed by an Arrow [`RecordBatch`] (a [`Schema`]
/// plus equal-length columns). It is cheap to clone (columns are reference
/// counted) and slices share storage. The first concrete [`Frame`] backing.
///
/// ```
/// use std::sync::Arc;
/// use arrow_array::{Int64Array, StringArray};
/// use yggdryl_saga::{DataFrame, Frame, Predicate, Scalar, Schema};
///
/// let df = DataFrame::new(
///     Schema::from_str("id: int64 not null, name: utf8").unwrap(),
///     vec![
///         Arc::new(Int64Array::from(vec![1, 2, 3])),
///         Arc::new(StringArray::from(vec!["a", "b", "c"])),
///     ],
/// )
/// .unwrap();
///
/// assert_eq!((df.height(), df.width().unwrap()), (Some(3), 2));
/// // `filter` types the untyped literal against the column, then applies it.
/// let big = df.filter(Predicate::gt("id", Scalar::any("1"))).unwrap();
/// assert_eq!(big.height(), Some(2));
/// ```
#[derive(Clone)]
pub struct DataFrame {
    batch: RecordBatch,
}

impl DataFrame {
    /// Builds a dataframe from a [`Schema`] and one [`ArrayRef`] per field. Every
    /// column must have the same length, and its Arrow type must match the field.
    pub fn new(schema: Schema, columns: Vec<ArrayRef>) -> Result<DataFrame, FrameError> {
        log_event!(debug, "DataFrame::new {} columns", columns.len());
        let arrow_schema = Arc::new(schema.to_arrow());
        let batch = if columns.is_empty() {
            RecordBatch::new_empty(arrow_schema)
        } else {
            RecordBatch::try_new(arrow_schema, columns)
                .map_err(|e| FrameError::Compute(e.to_string()))?
        };
        Ok(DataFrame { batch })
    }

    /// Wraps an existing Arrow [`RecordBatch`] (zero-copy).
    pub fn from_record_batch(batch: RecordBatch) -> DataFrame {
        DataFrame { batch }
    }

    /// An empty dataframe (zero rows) with the given schema.
    pub fn empty(schema: Schema) -> DataFrame {
        DataFrame {
            batch: RecordBatch::new_empty(Arc::new(schema.to_arrow())),
        }
    }

    /// Borrows the underlying Arrow [`RecordBatch`].
    pub fn record_batch(&self) -> &RecordBatch {
        &self.batch
    }

    /// Consumes the frame, returning its Arrow [`RecordBatch`].
    pub fn into_record_batch(self) -> RecordBatch {
        self.batch
    }

    /// A zero-copy row slice (`length` rows from `offset`), with both clamped to
    /// the height — the engine behind [`slice`](Frame::slice) / `head` / `tail`.
    fn sliced(&self, offset: usize, length: usize) -> DataFrame {
        let height = self.batch.num_rows();
        let offset = offset.min(height);
        let length = length.min(height - offset);
        DataFrame {
            batch: self.batch.slice(offset, length),
        }
    }
}

impl FrameHandle for DataFrame {
    fn schema(&self) -> Result<Schema, FrameError> {
        Ok(Schema::from_arrow(&self.batch.schema()))
    }
}

impl Frame for DataFrame {
    type Column = ArrayColumn;

    fn column(&self, name: &str) -> Result<ArrayColumn, FrameError> {
        let index = self
            .batch
            .schema()
            .index_of(name)
            .map_err(|_| FrameError::ColumnNotFound(name.to_string()))?;
        let field = crate::Field::from_arrow(self.batch.schema().field(index));
        Ok(ArrayColumn::new(field, self.batch.column(index).clone()))
    }

    fn height(&self) -> Option<usize> {
        Some(self.batch.num_rows())
    }

    fn select<S: AsRef<str>>(self, columns: &[S]) -> Result<DataFrame, FrameError> {
        let schema = self.batch.schema();
        let indices = columns
            .iter()
            .map(|name| {
                schema
                    .index_of(name.as_ref())
                    .map_err(|_| FrameError::ColumnNotFound(name.as_ref().to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let batch = self
            .batch
            .project(&indices)
            .map_err(|e| FrameError::Compute(e.to_string()))?;
        Ok(DataFrame { batch })
    }

    fn filter(self, predicate: Predicate) -> Result<DataFrame, FrameError> {
        // `filter` does the job: type the literals against the schema, then apply.
        let typed = self.optimize_predicate(predicate)?;
        log_event!(debug, "DataFrame::filter {typed}");
        let mask = evaluate(&self.batch, &typed)?;
        let batch = arrow_select::filter::filter_record_batch(&self.batch, &mask)
            .map_err(|e| FrameError::Compute(e.to_string()))?;
        Ok(DataFrame { batch })
    }

    fn limit(self, n: usize) -> Result<DataFrame, FrameError> {
        Ok(self.sliced(0, n))
    }

    fn tail(self, n: usize) -> Result<DataFrame, FrameError> {
        let height = self.batch.num_rows();
        let n = n.min(height);
        Ok(self.sliced(height - n, n))
    }

    fn slice(self, offset: usize, length: usize) -> Result<DataFrame, FrameError> {
        Ok(self.sliced(offset, length))
    }
}

impl fmt::Debug for DataFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DataFrame[{} x {}]",
            self.batch.num_rows(),
            self.batch.num_columns()
        )
    }
}

// --- predicate evaluation -------------------------------------------------
//
// A first, row-wise evaluator: it builds a boolean mask one row at a time. It is
// correct over the common flat column types; vectorising it over Arrow compute
// kernels is a later optimisation.

/// A single cell value read from an Arrow array, normalised to a comparable form.
/// Temporal columns surface as their integer tick count, decimals as their
/// unscaled `i128` — the same representation a typed [`Scalar`] carries.
enum Cell {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Dec(i128),
}

/// Evaluates `predicate` against every row of `batch`, returning the keep-mask.
fn evaluate(batch: &RecordBatch, predicate: &Predicate) -> Result<BooleanArray, FrameError> {
    let rows = batch.num_rows();
    let mut mask = BooleanBuilder::with_capacity(rows);
    for row in 0..rows {
        mask.append_value(eval_row(batch, row, predicate)?);
    }
    Ok(mask.finish())
}

/// Evaluates `predicate` for one row.
fn eval_row(batch: &RecordBatch, row: usize, predicate: &Predicate) -> Result<bool, FrameError> {
    Ok(match predicate {
        Predicate::Compare { column, op, value } => compare(&cell(batch, column, row)?, *op, value),
        Predicate::Between { column, low, high } => {
            let c = cell(batch, column, row)?;
            compare(&c, CompareOp::Ge, low) && compare(&c, CompareOp::Le, high)
        }
        Predicate::In { column, values } => {
            let c = cell(batch, column, row)?;
            values.iter().any(|v| compare(&c, CompareOp::Eq, v))
        }
        Predicate::NotIn { column, values } => {
            let c = cell(batch, column, row)?;
            !matches!(c, Cell::Null) && !values.iter().any(|v| compare(&c, CompareOp::Eq, v))
        }
        Predicate::IsNull(column) => matches!(cell(batch, column, row)?, Cell::Null),
        Predicate::IsNotNull(column) => !matches!(cell(batch, column, row)?, Cell::Null),
        Predicate::And(a, b) => eval_row(batch, row, a)? && eval_row(batch, row, b)?,
        Predicate::Or(a, b) => eval_row(batch, row, a)? || eval_row(batch, row, b)?,
        Predicate::Not(p) => !eval_row(batch, row, p)?,
    })
}

/// Reads the cell at `(column, row)`, downcasting the Arrow array by its type.
fn cell(batch: &RecordBatch, column: &str, row: usize) -> Result<Cell, FrameError> {
    let index = batch
        .schema()
        .index_of(column)
        .map_err(|_| FrameError::ColumnNotFound(column.to_string()))?;
    let array = batch.column(index);
    if array.is_null(row) {
        return Ok(Cell::Null);
    }
    let value = match array.data_type() {
        ArrowType::Boolean => Cell::Bool(array.as_boolean().value(row)),
        ArrowType::Int8 => Cell::Int(array.as_primitive::<Int8Type>().value(row) as i64),
        ArrowType::Int16 => Cell::Int(array.as_primitive::<Int16Type>().value(row) as i64),
        ArrowType::Int32 => Cell::Int(array.as_primitive::<Int32Type>().value(row) as i64),
        ArrowType::Int64 => Cell::Int(array.as_primitive::<Int64Type>().value(row)),
        ArrowType::UInt8 => Cell::Int(array.as_primitive::<UInt8Type>().value(row) as i64),
        ArrowType::UInt16 => Cell::Int(array.as_primitive::<UInt16Type>().value(row) as i64),
        ArrowType::UInt32 => Cell::Int(array.as_primitive::<UInt32Type>().value(row) as i64),
        ArrowType::UInt64 => Cell::Int(array.as_primitive::<UInt64Type>().value(row) as i64),
        ArrowType::Float32 => Cell::Float(array.as_primitive::<Float32Type>().value(row) as f64),
        ArrowType::Float64 => Cell::Float(array.as_primitive::<Float64Type>().value(row)),
        ArrowType::Utf8 => Cell::Str(array.as_string::<i32>().value(row).to_string()),
        ArrowType::LargeUtf8 => Cell::Str(array.as_string::<i64>().value(row).to_string()),
        ArrowType::Date32 => Cell::Int(array.as_primitive::<Date32Type>().value(row) as i64),
        ArrowType::Date64 => Cell::Int(array.as_primitive::<Date64Type>().value(row)),
        ArrowType::Timestamp(unit, _) => Cell::Int(match unit {
            ArrowTimeUnit::Second => array.as_primitive::<TimestampSecondType>().value(row),
            ArrowTimeUnit::Millisecond => {
                array.as_primitive::<TimestampMillisecondType>().value(row)
            }
            ArrowTimeUnit::Microsecond => {
                array.as_primitive::<TimestampMicrosecondType>().value(row)
            }
            ArrowTimeUnit::Nanosecond => array.as_primitive::<TimestampNanosecondType>().value(row),
        }),
        ArrowType::Decimal128(_, _) => Cell::Dec(array.as_primitive::<Decimal128Type>().value(row)),
        other => {
            return Err(FrameError::Compute(format!(
                "filter over column '{column}' of type {other:?} is not yet supported"
            )))
        }
    };
    Ok(value)
}

/// Tests `cell <op> value`. A null cell, or a value whose type does not line up
/// with the cell, never matches (the predicate's literal is type-optimised to the
/// column first, so a mismatch is genuinely absent data).
fn compare(cell: &Cell, op: CompareOp, value: &Scalar) -> bool {
    let ordering = match cell {
        Cell::Null => None,
        Cell::Int(a) => value.as_i64().map(|b| a.cmp(&b)),
        Cell::Float(a) => value.as_f64().and_then(|b| a.partial_cmp(&b)),
        Cell::Str(a) => value.as_str().map(|b| a.as_str().cmp(b)),
        Cell::Bool(a) => value.as_bool().map(|b| a.cmp(&b)),
        Cell::Dec(a) => value.as_i64().map(|b| a.cmp(&(b as i128))),
    };
    use std::cmp::Ordering::{Equal, Greater, Less};
    match ordering {
        None => false,
        Some(ord) => match op {
            CompareOp::Eq => ord == Equal,
            CompareOp::Ne => ord != Equal,
            CompareOp::Lt => ord == Less,
            CompareOp::Le => ord != Greater,
            CompareOp::Gt => ord == Greater,
            CompareOp::Ge => ord != Less,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Column;
    use arrow_array::{Int64Array, StringArray, TimestampNanosecondArray};

    fn trades() -> DataFrame {
        let schema =
            Schema::from_str("ts: timestamp(ns, UTC) not null, symbol: utf8 not null, px: int64")
                .unwrap();
        // The timestamp array must carry the same UTC zone as the schema field.
        let ts = TimestampNanosecondArray::from(vec![
            0,
            19723 * 86_400 * 1_000_000_000, // 2024-01-01
            20_000 * 86_400 * 1_000_000_000,
        ])
        .with_timezone("UTC");
        DataFrame::new(
            schema,
            vec![
                Arc::new(ts),
                Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
                Arc::new(Int64Array::from(vec![100, 200, 300])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn shape_schema_and_column() {
        let df = trades();
        assert_eq!(df.height(), Some(3));
        assert_eq!(df.width().unwrap(), 3);
        assert_eq!(df.schema().unwrap().names(), ["ts", "symbol", "px"]);
        let px = df.column("px").unwrap();
        assert_eq!(px.len(), Some(3));
        assert!(matches!(
            df.column("nope"),
            Err(FrameError::ColumnNotFound(_))
        ));
    }

    #[test]
    fn select_and_slice() {
        let df = trades();
        assert_eq!(
            df.clone()
                .select(&["px", "symbol"])
                .unwrap()
                .width()
                .unwrap(),
            2
        );
        assert_eq!(df.clone().head(2).unwrap().height(), Some(2));
        assert_eq!(df.clone().tail(1).unwrap().height(), Some(1));
        assert_eq!(df.slice(1, 5).unwrap().height(), Some(2));
    }

    #[test]
    fn filter_timestamp_range_with_untyped_literal() {
        // An untyped ISO string is cast to the ts column's timestamp type, then applied.
        let df = trades()
            .filter(Predicate::ge("ts", Scalar::any("2024-01-01")))
            .unwrap();
        assert_eq!(df.height(), Some(2)); // rows 1 and 2
    }

    #[test]
    fn filter_numeric_string_and_membership() {
        assert_eq!(
            trades()
                .filter(Predicate::gt("px", Scalar::any("150")))
                .unwrap()
                .height(),
            Some(2)
        );
        assert_eq!(
            trades()
                .filter(Predicate::eq("symbol", Scalar::utf8("AAPL")))
                .unwrap()
                .height(),
            Some(2)
        );
        assert_eq!(
            trades()
                .filter(Predicate::is_in(
                    "px",
                    [Scalar::int64(100), Scalar::int64(300)],
                ))
                .unwrap()
                .height(),
            Some(2)
        );
    }

    #[test]
    fn filter_between_and_conjunction() {
        let df = trades()
            .filter(
                Predicate::between("px", Scalar::any("100"), Scalar::any("250"))
                    .and(Predicate::eq("symbol", Scalar::utf8("MSFT"))),
            )
            .unwrap();
        assert_eq!(df.height(), Some(1)); // only MSFT@200
    }

    #[test]
    fn nulls_and_is_null() {
        let schema = Schema::from_str("px: int64").unwrap();
        let df = DataFrame::new(
            schema,
            vec![Arc::new(Int64Array::from(vec![Some(1), None, Some(3)]))],
        )
        .unwrap();
        assert_eq!(
            df.clone()
                .filter(Predicate::is_null("px"))
                .unwrap()
                .height(),
            Some(1)
        );
        assert_eq!(
            df.clone()
                .filter(Predicate::is_not_null("px"))
                .unwrap()
                .height(),
            Some(2)
        );
        // A comparison never matches a null cell.
        assert_eq!(
            df.filter(Predicate::gt("px", Scalar::int64(0)))
                .unwrap()
                .height(),
            Some(2)
        );
    }

    #[test]
    fn record_batch_round_trip() {
        let df = trades();
        let batch = df.record_batch().clone();
        let back = DataFrame::from_record_batch(batch);
        assert_eq!(back.height(), Some(3));
        assert_eq!(back.schema().unwrap(), df.schema().unwrap());
    }

    #[test]
    fn generic_frame_pipeline_runs_eagerly() {
        // The concrete DataFrame satisfies the generic Frame contract.
        fn first_match<F: Frame>(frame: F) -> Result<F, FrameError> {
            frame.select(&["symbol", "px"])?.head(2)
        }
        let out = first_match(trades()).unwrap();
        assert_eq!(out.width().unwrap(), 2);
        assert_eq!(out.height(), Some(2));
    }
}
