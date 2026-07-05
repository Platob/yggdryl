//! Integration tests for the `utf8` string type family: the [`Utf8Scalar`]
//! (a `Utf8Buffer`-backed value), its logical-over-binary data type, and the
//! `char`-view / positioned-IO surface.

use yggdryl_scalar::yggdryl_core::{IOBase, Latin1, RawIOBase, Whence};
use yggdryl_scalar::yggdryl_dtype::{DataError, DataType, Logical, Utf8Type};
use yggdryl_scalar::{arrow_array, Scalar, ScalarFactory, TypedScalar, Utf8Scalar};

#[test]
fn holds_a_string_or_null_and_borrows_it() {
    let greeting = Utf8Scalar::new("hé".to_string());
    assert!(!greeting.is_null());
    assert_eq!(greeting.value(), Some("hé"));
    assert_eq!(greeting.as_str(None).unwrap(), "hé"); // borrowed, never copied
    assert_eq!(greeting.as_bytes().unwrap(), &[b'h', 0xC3, 0xA9][..]);
    assert_eq!(greeting.data_type().name(), "utf8");

    let missing = Utf8Scalar::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert!(matches!(missing.as_str(None), Err(DataError::NullValue)));
    assert_eq!(Utf8Scalar::default(), missing); // default is null

    // The empty string is a present, non-null value.
    assert!(!Utf8Scalar::new(String::new()).is_null());
    assert_eq!(Utf8Type.default_scalar().value(), Some(""));
}

#[test]
fn is_a_logical_type_over_binary_storage() {
    // A string is stored as binary bytes but serialises to Arrow's Utf8.
    assert_eq!(Utf8Type.storage().name(), "binary");
    assert_eq!(
        Utf8Type.to_arrow(),
        arrow_array::Array::data_type(&arrow_array::StringArray::from(vec!["x"])).clone()
    );
    // The data type is the scalar / field factory.
    assert_eq!(
        Utf8Type.scalar("x".to_string()),
        Utf8Scalar::new("x".to_string())
    );
}

#[test]
fn value_is_a_positioned_io_resource_with_a_char_view() {
    let greeting = Utf8Scalar::new("héllo".to_string());
    let io = greeting.io().unwrap();
    // Byte surface (RawIOBase over the UTF-8 bytes).
    assert_eq!(io.byte_size(), 6); // 'é' takes two bytes
    assert_eq!(io.pread_byte_one(0, Whence::Start).unwrap(), b'h');
    // Typed char view (IOBase<char>).
    assert_eq!(IOBase::<char>::size(io), 5); // five chars

    // Moving the value out yields the Utf8Buffer for cursor / slice adapters.
    let mut buffer = greeting.into_io().unwrap();
    buffer
        .pwrite_one(buffer.byte_size(), Whence::Start, &'!')
        .unwrap();
    assert_eq!(buffer.as_str().unwrap(), "héllo!");
}

#[test]
fn round_trips_through_arrow_and_decodes_charsets() {
    let greeting = Utf8Scalar::new("hi".to_string());
    let arrow = greeting.to_arrow_scalar().into_inner();
    assert_eq!(arrow.len(), 1);
    assert_eq!(Utf8Scalar::from_arrow(arrow.as_ref()).unwrap(), greeting);
    assert!(Utf8Scalar::null().to_arrow_scalar().into_inner().is_null(0));

    // More than one value is not a scalar; a wrong array type is refused.
    let two = arrow_array::StringArray::from(vec!["a", "b"]);
    assert!(matches!(
        Utf8Scalar::from_arrow(&two),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    let wrong = arrow_array::BinaryArray::from_iter_values([b"x".as_ref()]);
    assert!(matches!(
        Utf8Scalar::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // An explicit charset re-decodes the UTF-8 bytes through it.
    let e_acute = Utf8Scalar::new("é".to_string()); // bytes 0xC3 0xA9
    assert_eq!(e_acute.as_str(Some(&Latin1)).unwrap(), "Ã©"); // read as latin1
}

#[test]
fn generic_bounds_and_send_sync_compose() {
    fn is_null<S: TypedScalar<Utf8Type, str, arrow_array::StringArray>>(scalar: &S) -> bool {
        scalar.is_null()
    }
    assert!(is_null(&Utf8Scalar::null()));
    assert!(!is_null(&Utf8Scalar::new("x".to_string())));

    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Utf8Scalar>();
}
