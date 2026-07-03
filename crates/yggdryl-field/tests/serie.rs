//! Integration tests for the `serie` field.

use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, Serie, TypedDataType};
use yggdryl_field::{Field, SerieField, TypedField};

#[test]
fn list_field_carries_both_layers() {
    let scores = SerieField::<Int64Type>::new("scores", true);
    assert_eq!(scores.name(), "scores");
    assert_eq!(scores.data_type().name(), "list");
    assert_eq!(scores.data_type().value_type().name(), "int64");
    assert_eq!(SerieField::from_arrow(&scores.to_arrow()).unwrap(), scores);

    fn type_name<DT: TypedDataType<Vec<i64>>, F: TypedField<DT, Vec<i64>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&scores), "list");
}

#[test]
fn list_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SerieField<Int64Type>>();
}
