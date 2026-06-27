//! Unit tests for the serie foundation: the [factory](crate::from_arrow), the
//! [`Serie`](crate::Serie) base accessors and the typed primitive series.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Float64Array, Int32Array, LargeStringArray,
    NullArray, StringArray, TimestampMicrosecondArray,
};
use yggdryl_schema::{DataType, Field, TypeCategory};

use crate::{
    from_array, from_arrow, BinarySerie, BooleanSerie, Date32Serie, Float64Serie, Int32Serie,
    Serie, TimestampMicrosecondSerie, TypedSerie, VarcharSerie,
};

#[test]
fn from_array_builds_int_serie() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(3)]));
    let serie = from_array("id", array).unwrap();

    assert_eq!(serie.name(), "id");
    assert_eq!(serie.len(), 3);
    assert!(!serie.is_empty());
    assert_eq!(serie.null_count(), 1);
    assert_eq!(serie.data_type(), &DataType::int(32, true));
    assert_eq!(serie.category(), TypeCategory::Primitive);
    assert!(serie.is_null(1));
    assert!(serie.is_valid(0));
    assert!(serie.is_null(99)); // out of bounds reads as null

    let ints = serie.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(ints.get(0), Some(1));
    assert_eq!(ints.get(1), None);
    assert_eq!(ints.value(2), 3);
    assert_eq!(ints.to_vec(), vec![Some(1), None, Some(3)]);
}

#[test]
fn from_values_constructors() {
    let ints = Int32Serie::from_values("n", vec![Some(1), None, Some(3)]);
    assert_eq!(ints.len(), 3);
    assert_eq!(ints.get(1), None);
    assert_eq!(ints.data_type(), &DataType::int(32, true));

    let strings = VarcharSerie::<i32>::from_values("s", vec![Some("x"), None, Some("z")]);
    assert_eq!(strings.len(), 3);
    assert_eq!(strings.str_value(0), Some("x"));
    assert_eq!(strings.get(2), Some("z".to_string()));

    let flags = BooleanSerie::from_values("b", vec![Some(true), Some(false), None]);
    assert_eq!(flags.get(0), Some(true));
    assert_eq!(flags.null_count(), 1);
}

#[test]
fn float_and_boolean_series() {
    let floats: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));
    let serie = from_array("x", floats).unwrap();
    let typed = serie.as_any().downcast_ref::<Float64Serie>().unwrap();
    assert_eq!(typed.value(1), 2.5);

    let bools: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), None]));
    let serie = from_array("flag", bools).unwrap();
    assert_eq!(serie.data_type(), &DataType::Boolean);
    let typed = serie.as_any().downcast_ref::<BooleanSerie>().unwrap();
    assert_eq!(typed.iter().collect::<Vec<_>>(), vec![Some(true), None]);
}

#[test]
fn string_and_binary_series() {
    let utf8: ArrayRef = Arc::new(StringArray::from(vec![Some("a"), None, Some("c")]));
    let serie = from_array("name", utf8).unwrap();
    assert_eq!(serie.data_type(), &DataType::varchar());
    let typed = serie.as_any().downcast_ref::<VarcharSerie<i32>>().unwrap();
    assert_eq!(typed.str_value(0), Some("a"));
    assert_eq!(typed.get(2), Some("c".to_string()));

    let large: ArrayRef = Arc::new(LargeStringArray::from(vec![Some("big")]));
    let serie = from_array("big", large).unwrap();
    assert!(serie.data_type().is_large());

    let bin: ArrayRef = Arc::new(BinaryArray::from(vec![Some(&b"ab"[..]), None]));
    let serie = from_array("blob", bin).unwrap();
    assert_eq!(serie.data_type(), &DataType::binary());
    let typed = serie.as_any().downcast_ref::<BinarySerie<i32>>().unwrap();
    assert_eq!(typed.bytes_value(0), Some(&b"ab"[..]));
    assert_eq!(typed.get(1), None);
}

#[test]
fn temporal_series() {
    let dates: ArrayRef = Arc::new(Date32Array::from(vec![0, 1, 2]));
    let serie = from_array("d", dates).unwrap();
    assert_eq!(serie.data_type(), &DataType::date());
    assert_eq!(serie.category(), TypeCategory::Logical);
    let typed = serie.as_any().downcast_ref::<Date32Serie>().unwrap();
    assert_eq!(typed.value(2), 2);

    let ts: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![10, 20]));
    let serie = from_array("ts", ts).unwrap();
    assert!(serie.data_type().is_temporal());
    let typed = serie
        .as_any()
        .downcast_ref::<TimestampMicrosecondSerie>()
        .unwrap();
    assert_eq!(typed.get(0), Some(10));
}

#[test]
fn slice_preserves_type_and_name() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![10, 20, 30, 40]));
    let serie = from_array("n", array).unwrap();
    let sliced = serie.slice(1, 2);

    assert_eq!(sliced.len(), 2);
    assert_eq!(sliced.name(), "n");
    assert_eq!(sliced.data_type(), &DataType::int(32, true));
    let typed = sliced.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(typed.value(0), 20);
    assert_eq!(typed.value(1), 30);
}

#[test]
fn from_arrow_checks_field_type() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![1, 2]));
    // field says int64 but the array is int32
    let err = from_arrow(Field::new("x", DataType::int(64, true), true), array).unwrap_err();
    assert!(matches!(err, crate::SerieError::TypeMismatch { .. }));
}

#[test]
fn unsupported_arrow_type_errors() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let err = from_array("n", array).unwrap_err();
    assert!(matches!(err, crate::SerieError::Unsupported(_)));
}

#[test]
fn explicit_field_carries_metadata() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let field = Field::new("id", DataType::int(32, true), false).with_comment("primary key");
    let serie = from_arrow(field, array).unwrap();
    assert!(!serie.is_nullable());
    assert_eq!(serie.field().comment(), Some("primary key"));
}
