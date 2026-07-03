//! Integration tests for the `list` field.

use yggdryl_field::yggdryl_dtype::{Int64, RawDataType, RawList};
use yggdryl_field::{Field, List, RawField};

#[test]
fn list_field_carries_both_layers() {
    let scores = List::<Int64>::new("scores", true);
    assert_eq!(scores.name(), "scores");
    assert_eq!(scores.data_type().name(), "list");
    assert_eq!(scores.data_type().value_type().name(), "int64");
    assert_eq!(List::from_arrow(&scores.to_arrow()).unwrap(), scores);

    fn type_name<F: Field<Vec<i64>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&scores), "list");
}

#[test]
fn list_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<List<Int64>>();
}
