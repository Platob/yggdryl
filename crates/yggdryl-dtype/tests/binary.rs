//! Integration tests for the `binary` data type — the variable-size byte type.

use yggdryl_dtype::{arrow_schema, Binary, DataError, DataType, DataTypeId, RawDataType};

#[test]
fn binary_describes_itself_and_round_trips() {
    assert_eq!(Binary.name(), "binary");
    assert_eq!(Binary.arrow_format(), "z");
    assert_eq!((Binary.byte_width(), Binary.bit_width()), (None, None));
    assert_eq!(Binary::ID, DataTypeId::Binary);
    assert_eq!(Binary::ID.arrow_format(), Some("z"));

    assert_eq!(Binary.to_arrow(), arrow_schema::DataType::Binary);
    assert_eq!(Binary::from_arrow(&Binary.to_arrow()).unwrap(), Binary);
    assert!(matches!(
        Binary::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn binary_codec_is_the_identity() {
    let bytes = Binary.native_to_bytes(&vec![1, 2, 3]);
    assert_eq!(bytes, vec![1, 2, 3]);
    assert_eq!(Binary.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
    // Any byte length is a valid binary value — even empty.
    assert_eq!(Binary.native_from_bytes(&[]).unwrap(), Vec::<u8>::new());
    assert_eq!(Binary.default_value(), Vec::<u8>::new());
}

#[test]
fn binary_is_send_sync_and_joins_dyn_schemas() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Binary>();

    let types: Vec<Box<dyn RawDataType>> = vec![Box::new(Binary)];
    assert_eq!(types[0].name(), "binary");
}
