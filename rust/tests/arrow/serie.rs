//! A column at the Arrow runtime boundary: what it refuses, what a
//! multi-batch stream drains into, and the one root shape that is not rows.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::arrow::batch_reader;
use yggdryl::{DataType, Field, Scalar, Serie, SerieValue};

use super::root;

/// The two-column record root every stream below is read under.
fn quotes_root() -> Field {
    root([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::utf8(), false),
    ])
}

/// One Arrow batch of `count` rows starting at `first`.
fn quotes_batch(first: i64, count: i64) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    let ids: ArrayRef = Arc::new(Int64Array::from((first..first + count).collect::<Vec<_>>()));
    let symbols: ArrayRef = Arc::new(StringArray::from(
        (first..first + count).map(|_| "AAPL").collect::<Vec<_>>(),
    ));
    RecordBatch::try_new(schema, vec![ids, symbols]).expect("two columns of one length")
}

/// Every row of `column` as a value, in order.
fn rows(column: &Serie) -> Vec<Scalar> {
    column.scalars().expect("readable rows")
}

#[test]
fn an_array_of_another_layout_is_refused_rather_than_reinterpreted() {
    let field = Field::new("price", DataType::Int64, false);

    // Same width, different layout: the boundary compares the exact physical
    // datatype and never reconciles one into another.
    let doubles: ArrayRef = Arc::new(arrow_array::Float64Array::from(vec![1.0, 2.0]));
    let refusal =
        Serie::from_arrow_array(field.clone(), doubles).expect_err("float64 is not int64");
    assert!(
        refusal.to_string().to_lowercase().contains("int64"),
        "the refusal names the layout it wanted: {refusal}"
    );

    // And a nullable array under a required field is refused for the null it
    // carries, not for its layout.
    let holes: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    assert!(Serie::from_arrow_array(field, ArrayRef::clone(&holes)).is_err());

    // The same array under a nullable field is a column.
    let nullable = Field::new("price", DataType::Int64, true);
    let read = Serie::from_arrow_array(nullable, holes).expect("a nullable column");
    assert_eq!(read.len(), 2);
    assert_eq!(read.scalar(1).unwrap(), Scalar::Null);
    assert!(read.is_null(1));
}

#[test]
fn a_multi_batch_stream_drains_into_one_column_of_every_row() {
    let reader = batch_reader(
        quotes_batch(0, 2).schema(),
        [quotes_batch(0, 2), quotes_batch(2, 3), quotes_batch(5, 1)],
    );

    let column = Serie::from_arrow_reader(reader).expect("three batches of one schema");
    assert_eq!(column.len(), 6);
    assert_eq!(column.field().name(), "row");
    assert_eq!(column.field().dtype(), quotes_root().dtype());

    // Every row is there, in the order the batches yielded them - and the
    // child is a column of its own, so the ids come off its values buffer.
    let ids = column
        .as_struct()
        .expect("a record column")
        .child("id")
        .expect("a named child")
        .as_int64()
        .expect("an int64 child");
    assert_eq!(ids.values(), &[0, 1, 2, 3, 4, 5]);
}

#[test]
fn an_empty_stream_drains_into_the_empty_column_of_its_declared_root() {
    let reader = batch_reader(quotes_batch(0, 0).schema(), []);
    let column = Serie::from_arrow_reader(reader).expect("a schema with no batches");

    assert!(column.is_empty());
    assert_eq!(column.field().dtype(), quotes_root().dtype());
    // Empty and still exact, which is the whole point of carrying the field.
    assert_eq!(
        column.dtype().unwrap(),
        DataType::list(column.field().clone())
    );
}

#[test]
fn a_column_round_trips_a_batch_through_the_reader_it_streams() {
    let values = (0..4_i64)
        .map(|index| {
            Scalar::from_struct([
                ("id", Scalar::from(index)),
                ("symbol", Scalar::from("AAPL")),
            ])
            .expect("one record")
        })
        .collect::<Vec<_>>();
    let column = Serie::from_scalars(quotes_root(), values).expect("four records");

    let back = Serie::from_arrow_reader(column.into_arrow_reader().unwrap()).unwrap();
    assert_eq!(rows(&back), rows(&column));

    // One held column is one table, so the stream yields exactly one batch.
    let batches = column
        .into_arrow_reader()
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .collect::<Vec<_>>();
    assert_eq!(batches, vec![4]);
}

#[test]
fn a_root_that_is_not_a_record_is_refused_by_name() {
    let column = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64)],
    )
    .unwrap();

    let refusal = column
        .into_arrow_batch()
        .expect_err("a column of a leaf field is not a table");
    assert!(
        refusal.to_string().contains("price"),
        "the refusal names the field it was handed: {refusal}"
    );
    assert!(column.into_arrow_reader().is_err());

    // A nullable Struct root is not a record root either: a batch has no row
    // validity to carry the rows a nullable root admits.
    let nullable = Field::new("row", quotes_root().dtype().clone(), true);
    let holes = Serie::from_scalars(nullable, [Scalar::Null]).unwrap();
    assert!(holes.into_arrow_batch().is_err());
}

#[test]
fn the_buffers_a_column_was_read_from_are_the_buffers_it_hands_back() {
    let field = Field::new("price", DataType::Int64, false);
    let array: ArrayRef = Arc::new(Int64Array::from(vec![7_i64, 8, 9]));

    let column = Serie::from_arrow_array(field, ArrayRef::clone(&array)).expect("an int64 column");
    let back = column.into_arrow_array();

    // No row was decoded on the way in or on the way out: the values buffer
    // that came in is the one that goes back out.
    assert!(
        std::ptr::eq(
            array.to_data().buffers()[0].as_ptr(),
            back.to_data().buffers()[0].as_ptr(),
        ),
        "the column shares the values buffer it was read from"
    );
}
