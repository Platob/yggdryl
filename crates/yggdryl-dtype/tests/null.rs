//! Integration tests for the `null` data type — the storage-free type whose every
//! value is null.

use yggdryl_dtype::{arrow_schema, DataTypeId, Null, RawDataType};

#[test]
fn null_describes_itself_and_round_trips() {
    assert_eq!(Null.name(), "null");
    assert_eq!(Null.arrow_format(), "n");
    assert_eq!((Null.byte_width(), Null.bit_width()), (None, None));
    assert_eq!(Null::ID, DataTypeId::Null);
    assert_eq!(Null::ID.name(), Null.name());
    assert_eq!(Null::ID.arrow_format(), Some("n"));

    assert_eq!(Null.to_arrow(), arrow_schema::DataType::Null);
    assert_eq!(Null::from_arrow(&Null.to_arrow()).unwrap(), Null);
    assert!(Null::from_arrow(&arrow_schema::DataType::Int64).is_err());
}

#[test]
fn null_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Null>();
}
