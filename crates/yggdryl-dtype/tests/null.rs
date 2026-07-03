//! Integration tests for the `null` data type — the storage-free type whose every
//! value is null.

use yggdryl_dtype::{arrow_schema, DataType, DataTypeId, NullType};

#[test]
fn null_describes_itself_and_round_trips() {
    assert_eq!(NullType.name(), "null");
    assert_eq!(NullType.arrow_format(), "n");
    assert_eq!((NullType.byte_width(), NullType.bit_width()), (None, None));
    assert_eq!(NullType::ID, DataTypeId::Null);
    assert_eq!(NullType::ID.name(), NullType.name());
    assert_eq!(NullType::ID.arrow_format(), Some("n"));

    assert_eq!(NullType.to_arrow(), arrow_schema::DataType::Null);
    assert_eq!(
        NullType::from_arrow(&NullType.to_arrow()).unwrap(),
        NullType
    );
    assert!(NullType::from_arrow(&arrow_schema::DataType::Int64).is_err());
}

#[test]
fn null_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<NullType>();
}
