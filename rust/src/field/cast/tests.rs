//! A typed field casts to its own array type.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Datum, Float64Array, Int32Array, Int64Array, StringArray};

use crate::field::{Int64Field, StructField, TimestampField, Utf8Field};
use crate::{DataType, Field, TimeUnit};

#[test]
fn a_typed_field_returns_its_own_array_type() {
    let field = Int64Field::new("id", false);
    let source: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));

    // The binding is an Int64Array; no downcast at the call site.
    let ids: Int64Array = field.cast_arrow_array(source, false).unwrap();
    assert_eq!(ids.values(), &[1, 2, 3]);
}

#[test]
fn a_string_field_parses_and_formats_through_the_same_call() {
    let field = Utf8Field::new("symbol", false);
    let numbers: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));

    let text: StringArray = field.cast_arrow_array(numbers, false).unwrap();
    assert_eq!(text.value(0), "1.5");
    assert_eq!(text.value(1), "2.5");
}

#[test]
fn an_unsafe_cast_fails_and_a_safe_one_defaults() {
    let field = Int64Field::new("id", false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    assert!(field.cast_arrow_array(Arc::clone(&text), false).is_err());

    // Safe casting nulls the failure, and a non-null field then defaults it.
    let ids = field.cast_arrow_array(text, true).unwrap();
    assert_eq!(ids.values(), &[1, 0]);
    assert_eq!(ids.null_count(), 0);
}

#[test]
fn a_nullable_field_keeps_the_null_a_safe_cast_produced() {
    let field = Int64Field::new("id", true);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

    let ids = field.cast_arrow_array(text, true).unwrap();
    assert!(ids.is_null(1));
}

#[test]
fn a_struct_field_casts_children_by_name() {
    let field = StructField::try_from_field(Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])
        .unwrap(),
        false,
    ))
    .unwrap();

    let source: ArrayRef = Arc::new(arrow_array::StructArray::from(vec![
        (
            Arc::new(arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int32,
                false,
            )),
            Arc::new(Int32Array::from(vec![7])) as ArrayRef,
        ),
        (
            Arc::new(arrow_schema::Field::new(
                "symbol",
                arrow_schema::DataType::Utf8,
                true,
            )),
            Arc::new(StringArray::from(vec!["ACME"])) as ArrayRef,
        ),
    ]));

    let row = field.cast_arrow_array(source, false).unwrap();
    assert_eq!(row.num_columns(), 2);
    assert_eq!(row.column(0).data_type(), &arrow_schema::DataType::Int64);
}

#[test]
fn a_parameterized_temporal_field_casts_to_a_shared_array() {
    // A unit decides the physical width, so the result stays an ArrayRef.
    let field = TimestampField::try_new(
        "at",
        DataType::Timestamp(TimeUnit::Millisecond, None),
        false,
    )
    .unwrap();
    let source: ArrayRef = Arc::new(Int64Array::from(vec![1_700_000_000_000]));

    let cast: ArrayRef = field.cast_arrow_array(source, false).unwrap();
    assert_eq!(cast.len(), 1);
    assert_eq!(
        cast.data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None)
    );
}

#[test]
fn a_scalar_cast_requires_exactly_one_value() {
    let field = Int64Field::new("id", false);
    let one: ArrayRef = Arc::new(Int32Array::from(vec![9]));
    let two: ArrayRef = Arc::new(Int32Array::from(vec![9, 10]));

    let scalar = field.cast_arrow_scalar(one, false).unwrap();
    let (array, is_scalar) = scalar.get();
    assert!(is_scalar);
    assert_eq!(array.len(), 1);

    let message = field.cast_arrow_scalar(two, false).unwrap_err().to_string();
    assert!(message.contains("exactly 1 value"), "{message}");
}

#[test]
fn a_borrowed_typed_field_casts_the_same_way() {
    let field = Int64Field::new("id", false);
    let borrowed = field.as_typed_ref();
    let source: ArrayRef = Arc::new(Int32Array::from(vec![4]));

    assert_eq!(
        borrowed.cast_arrow_array(source, false).unwrap().values(),
        &[4]
    );
}
