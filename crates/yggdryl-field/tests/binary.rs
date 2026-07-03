//! Integration tests for the `binary` field.

use yggdryl_field::yggdryl_dtype::{DataType, TypedDataType};
use yggdryl_field::{BinaryField, Field, TypedField};

#[test]
fn binary_field_carries_both_layers() {
    let payload = BinaryField::new("payload", true);
    assert_eq!(payload.name(), "payload");
    assert_eq!(payload.data_type().name(), "binary");
    assert_eq!(
        BinaryField::from_arrow(&payload.to_arrow()).unwrap(),
        payload
    );

    fn type_name<DT: TypedDataType<Vec<u8>>, F: TypedField<DT, Vec<u8>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&payload), "binary");
}

#[test]
fn binary_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BinaryField>();
}
