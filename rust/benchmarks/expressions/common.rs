//! The one fixture every expression benchmark measures against.

use std::sync::Arc;

use arrow_array::{ArrayRef, Decimal128Array, Int64Array, RecordBatch, StringArray};
use yggdryl::{DataType, Field, Value};

/// How many rows the vectorized and row legs both run over.
pub(crate) const ROWS: usize = 65_536;

/// The root the fixture is described by.
pub(crate) fn schema() -> Field {
    DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
        DataType::Decimal128 {
            precision: 12,
            scale: 2,
        }
        .nullable_field("price"),
    ])
    .expect("a three-column struct")
    .required_field("row")
}

/// One batch of [`ROWS`] rows, with a null every seventeenth row.
pub(crate) fn fixture_batch() -> RecordBatch {
    let venues: Vec<Option<&str>> = (0..ROWS)
        .map(|row| match row % 17 {
            0 => None,
            index if index % 2 == 0 => Some("XNAS"),
            _ => Some("XNYS"),
        })
        .collect();
    let ids: Vec<Option<i64>> = (0..ROWS)
        .map(|row| (row % 17 != 0).then_some(row as i64))
        .collect();
    let prices: Vec<Option<i128>> = (0..ROWS)
        .map(|row| (row % 17 != 0).then_some((row as i128 % 5_000) * 7))
        .collect();
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(venues)),
        Arc::new(Int64Array::from(ids)),
        Arc::new(
            Decimal128Array::from(prices)
                .with_precision_and_scale(12, 2)
                .expect("a decimal column"),
        ),
    ];
    RecordBatch::try_new(
        yggdryl::arrow::schema_from_field(&schema()).expect("an Arrow schema"),
        arrays,
    )
    .expect("a fixture batch")
}

/// The same fixture as rows the row evaluator reads.
pub(crate) fn fixture_rows() -> Vec<Value> {
    let batch = fixture_batch();
    yggdryl::arrow::batch_to_value(&batch)
        .expect("rows")
        .as_sequence()
        .map(<[Value]>::to_vec)
        .expect("a sequence of rows")
}
