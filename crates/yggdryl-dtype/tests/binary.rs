//! Integration tests for the `binary` data type — the variable-size byte type.

use yggdryl_dtype::{arrow_schema, BinaryType, DataError, DataType, DataTypeId, TypedDataType};

#[test]
fn binary_describes_itself_and_round_trips() {
    assert_eq!(BinaryType.name(), "binary");
    assert_eq!(BinaryType.arrow_format(), "z");
    assert_eq!(
        (BinaryType.byte_width(), BinaryType.bit_width()),
        (None, None)
    );
    assert_eq!(BinaryType::ID, DataTypeId::Binary);
    assert_eq!(BinaryType::ID.arrow_format(), Some("z"));

    assert_eq!(BinaryType.to_arrow(), arrow_schema::DataType::Binary);
    assert_eq!(
        BinaryType::from_arrow(&BinaryType.to_arrow()).unwrap(),
        BinaryType
    );
    assert!(matches!(
        BinaryType::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn binary_codec_is_the_identity() {
    let bytes = BinaryType.native_to_bytes(&vec![1, 2, 3]);
    assert_eq!(bytes, vec![1, 2, 3]);
    assert_eq!(BinaryType.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
    // Any byte length is a valid binary value — even empty.
    assert_eq!(BinaryType.native_from_bytes(&[]).unwrap(), Vec::<u8>::new());
    assert_eq!(BinaryType.default_value(), Vec::<u8>::new());
}

#[test]
fn binary_is_send_sync_and_joins_dyn_schemas() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BinaryType>();

    let types: Vec<Box<dyn DataType>> = vec![Box::new(BinaryType)];
    assert_eq!(types[0].name(), "binary");
}
