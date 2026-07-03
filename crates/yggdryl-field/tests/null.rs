//! Integration tests for the `null` field.

use yggdryl_field::yggdryl_dtype::DataType;
use yggdryl_field::{Field, NullField};

#[test]
fn null_field_round_trips() {
    let gap = NullField::new("gap", true);
    assert_eq!(
        (gap.name(), gap.data_type().name(), gap.is_nullable()),
        ("gap", "null", true)
    );
    assert_eq!(NullField::from_arrow(&gap.to_arrow()).unwrap(), gap);
}

#[test]
fn null_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NullField>();
}
