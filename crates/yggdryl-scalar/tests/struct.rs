//! Integration tests for the `struct` scalar — one row of one-element Arrow
//! columns.

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataError};
use yggdryl_scalar::{arrow_array, Scalar, StructScalar};

fn point_type() -> dtype::StructType {
    dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]))
}

#[test]
fn struct_scalar_validates_and_round_trips() {
    let point = point_type();
    let column = |value: i64| -> arrow_array::ArrayRef {
        std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([value]))
    };

    let row = StructScalar::new(point.clone(), vec![column(1), column(2)]).unwrap();
    assert!(!row.is_null());
    assert_eq!(row.value().map(<[_]>::len), Some(2));
    let arrow = row.to_arrow_scalar();
    assert_eq!(arrow.len(), 1);
    assert_eq!(StructScalar::from_arrow(arrow.as_ref()).unwrap(), row);

    let missing = StructScalar::null(point.clone());
    assert!(missing.is_null());
    assert_eq!(
        StructScalar::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
        missing
    );

    // Wrong column count, wrong length and wrong type are all actionable errors.
    assert!(matches!(
        StructScalar::new(point.clone(), vec![column(1)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    let two: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([1, 2]));
    assert!(matches!(
        StructScalar::new(point.clone(), vec![two, column(2)]),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    let wrong: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::UInt8Array::from_iter_values([1]));
    assert!(matches!(
        StructScalar::new(point.clone(), vec![wrong, column(2)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    // A null in a non-nullable child is refused at construction, not a panic later.
    let null_column: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::Int64Array::new_null(1));
    assert!(matches!(
        StructScalar::new(point, vec![null_column, column(2)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn struct_scalar_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StructScalar>();
}
