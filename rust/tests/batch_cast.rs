//! Batch casting goes through the same struct-array path as every other cast.

use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::{ArrowCast, DataType, Field};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

#[test]
fn a_missing_column_is_filled_with_its_canonical_default() {
    let source = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        false,
    )]));
    let batch = RecordBatch::try_new(source, vec![Arc::new(Int32Array::from(vec![1, 2]))]).unwrap();

    let target = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
    ]);
    let cast = target.cast_arrow_batch(batch, true).unwrap();

    assert_eq!(cast.num_columns(), 2);
    assert_eq!(cast.num_rows(), 2);
    assert_eq!(cast.column(1).len(), 2);
    assert_eq!(cast.column(1).null_count(), 0);
}

#[test]
fn columns_reconcile_by_name_and_extra_columns_are_dropped() {
    let source = Arc::new(Schema::new(vec![
        ArrowField::new("SYMBOL", ArrowDataType::Utf8, true),
        ArrowField::new("unused", ArrowDataType::Int32, true),
        ArrowField::new("ID", ArrowDataType::Int32, false),
    ]));
    let batch = RecordBatch::try_new(
        source,
        vec![
            Arc::new(StringArray::from(vec!["AAPL"])),
            Arc::new(Int32Array::from(vec![9])),
            Arc::new(Int32Array::from(vec![1])),
        ],
    )
    .unwrap();

    let target = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ]);
    let cast = target.cast_arrow_batch(batch, true).unwrap();

    assert_eq!(cast.num_columns(), 2);
    assert_eq!(cast.schema().field(0).name(), "id");
    assert_eq!(cast.column(0).data_type(), &ArrowDataType::Int64);
}

#[test]
fn an_exact_batch_keeps_its_own_arrays() {
    let target = root([DataType::Int32.required_field("id")]);
    let schema = Arc::new(Schema::new(vec![
        target.fields()[0].to_arrow_ref().unwrap(),
    ]));
    let column: arrow_array::ArrayRef = Arc::new(Int32Array::from(vec![7]));
    let batch = RecordBatch::try_new(schema, vec![Arc::clone(&column)]).unwrap();

    let cast = target.cast_arrow_batch(batch, true).unwrap();
    assert!(Arc::ptr_eq(cast.column(0), &column));
}

#[test]
fn a_zero_column_batch_keeps_its_row_count() {
    let batch = RecordBatch::try_new_with_options(
        Arc::new(Schema::empty()),
        Vec::new(),
        &arrow_array::RecordBatchOptions::new().with_row_count(Some(3)),
    )
    .unwrap();

    let target = root([]);
    let cast = target.cast_arrow_batch(batch, true).unwrap();
    assert_eq!(cast.num_rows(), 3);
    assert_eq!(cast.num_columns(), 0);
}
