//! Integration tests for the `utf8` string type family: the [`StringScalar`]
//! (a `StringBuffer`-backed value), its logical-over-binary data type, and the
//! `char`-view / positioned-IO surface.

use yggdryl_scalar::yggdryl_core::{IOBase, Latin1, RawIOBase, Whence};
use yggdryl_scalar::yggdryl_dtype::{DataError, DataType, Logical, StringType};
use yggdryl_scalar::{arrow_array, Scalar, ScalarFactory, StringScalar, TypedScalar};

#[test]
fn holds_a_string_or_null_and_borrows_it() {
    let greeting = StringScalar::new("hé".to_string());
    assert!(!greeting.is_null());
    assert_eq!(greeting.value(), Some("hé"));
    assert_eq!(greeting.as_str(None).unwrap(), "hé"); // borrowed, never copied
    assert_eq!(greeting.as_bytes().unwrap(), &[b'h', 0xC3, 0xA9][..]);
    assert_eq!(greeting.data_type().name(), "utf8");

    let missing = StringScalar::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert!(matches!(missing.as_str(None), Err(DataError::NullValue)));
    assert_eq!(StringScalar::default(), missing); // default is null

    // The empty string is a present, non-null value.
    assert!(!StringScalar::new(String::new()).is_null());
    assert_eq!(StringType.default_scalar().value(), Some(""));
}

#[test]
fn is_a_logical_type_over_binary_storage() {
    // A string is stored as binary bytes but serialises to Arrow's Utf8.
    assert_eq!(StringType.storage().name(), "binary");
    assert_eq!(
        StringType.to_arrow(),
        arrow_array::Array::data_type(&arrow_array::StringArray::from(vec!["x"])).clone()
    );
    // The data type is the scalar / field factory.
    assert_eq!(
        StringType.scalar("x".to_string()),
        StringScalar::new("x".to_string())
    );
}

#[test]
fn value_is_a_positioned_io_resource_with_a_char_view() {
    let greeting = StringScalar::new("héllo".to_string());
    let io = greeting.io().unwrap();
    // Byte surface (RawIOBase over the UTF-8 bytes).
    assert_eq!(io.byte_size(), 6); // 'é' takes two bytes
    assert_eq!(io.pread_byte_one(0, Whence::Start).unwrap(), b'h');
    // Typed char view (IOBase<char>).
    assert_eq!(IOBase::<char>::size(io), 5); // five chars

    // Moving the value out yields the StringBuffer for cursor / slice adapters.
    let mut buffer = greeting.into_io().unwrap();
    buffer
        .pwrite_one(buffer.byte_size(), Whence::Start, &'!')
        .unwrap();
    assert_eq!(buffer.as_str().unwrap(), "héllo!");
}

#[test]
fn round_trips_through_arrow_and_decodes_charsets() {
    let greeting = StringScalar::new("hi".to_string());
    let arrow = greeting.to_arrow_scalar();
    assert_eq!(arrow.len(), 1);
    assert_eq!(StringScalar::from_arrow(arrow.as_ref()).unwrap(), greeting);
    assert!(StringScalar::null().to_arrow_scalar().is_null(0));

    // More than one value is not a scalar; a wrong array type is refused.
    let two = arrow_array::StringArray::from(vec!["a", "b"]);
    assert!(matches!(
        StringScalar::from_arrow(&two),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    let wrong = arrow_array::BinaryArray::from_iter_values([b"x".as_ref()]);
    assert!(matches!(
        StringScalar::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // An explicit charset re-decodes the UTF-8 bytes through it.
    let e_acute = StringScalar::new("é".to_string()); // bytes 0xC3 0xA9
    assert_eq!(e_acute.as_str(Some(&Latin1)).unwrap(), "Ã©"); // read as latin1
}

#[test]
fn generic_bounds_and_send_sync_compose() {
    fn is_null<S: TypedScalar<StringType, str, arrow_array::StringArray>>(scalar: &S) -> bool {
        scalar.is_null()
    }
    assert!(is_null(&StringScalar::null()));
    assert!(!is_null(&StringScalar::new("x".to_string())));

    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StringScalar>();
}
