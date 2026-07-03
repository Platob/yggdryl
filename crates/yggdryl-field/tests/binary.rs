//! Integration tests for the `binary` field.

use yggdryl_field::yggdryl_dtype::RawDataType;
use yggdryl_field::{Binary, Field, RawField};

#[test]
fn binary_field_carries_both_layers() {
    let payload = Binary::new("payload", true);
    assert_eq!(payload.name(), "payload");
    assert_eq!(payload.data_type().name(), "binary");
    assert_eq!(Binary::from_arrow(&payload.to_arrow()).unwrap(), payload);

    fn type_name<F: Field<Vec<u8>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&payload), "binary");
}

#[test]
fn binary_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Binary>();
}
