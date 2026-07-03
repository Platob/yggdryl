//! Integration tests for the `binary` family — the variable-size byte type whose
//! scalar doubles as a `yggdryl-core` positioned-IO resource.

use yggdryl_data::yggdryl_core::{
    ByteBuffer, RawIOBase, RawIOCursor, RawIOSlice, Seekable, Whence,
};
use yggdryl_data::{
    arrow_array, arrow_schema, Binary, BinaryField, BinaryScalar, DataError, DataType, DataTypeId,
    Field, ListScalar, OptionalScalar, OptionalType, RawDataType, RawField, RawScalar,
};

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
    assert_eq!(Binary.default_scalar(), BinaryScalar::new(Vec::new()));
}

#[test]
fn binary_field_carries_both_layers() {
    let payload = BinaryField::new("payload", true);
    assert_eq!(payload.name(), "payload");
    assert_eq!(payload.data_type().name(), "binary");
    assert_eq!(
        BinaryField::from_arrow(&payload.to_arrow()).unwrap(),
        payload
    );

    fn type_name<F: Field<Vec<u8>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&payload), "binary");
}

#[test]
fn binary_scalar_holds_bytes_or_null() {
    let blob = BinaryScalar::new(vec![1, 2, 3]);
    assert!(!blob.is_null());
    assert_eq!(blob.value(), Some(&[1, 2, 3][..]));

    // Byte access borrows the held buffer — same address, no copy.
    let borrowed = blob.as_bytes().unwrap();
    assert_eq!(borrowed, &[1, 2, 3][..]);
    assert_eq!(borrowed.as_ptr(), blob.io().unwrap().as_bytes().as_ptr());

    // UTF-8 bytes convert to str; anything else errors naming the shape.
    assert_eq!(BinaryScalar::new(b"hi".to_vec()).as_str().unwrap(), "hi");
    assert!(matches!(
        BinaryScalar::new(vec![0xFF]).as_str(),
        Err(DataError::InexactConversion { target: "str", .. })
    ));
    // No numeric conversions (the trait defaults, naming the data type).
    assert!(matches!(
        blob.as_i64(),
        Err(DataError::UnsupportedConversion { data_type, target: "i64" }) if data_type == "binary"
    ));

    // The empty value and null are distinct states; a null holds no value.
    assert!(!BinaryScalar::new(Vec::new()).is_null());
    let missing = BinaryScalar::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert!(matches!(missing.as_bytes(), Err(DataError::NullValue)));
    assert!(matches!(missing.as_str(), Err(DataError::NullValue)));
    assert!(missing.io().is_none());
    assert!(missing.clone().into_io().is_none());
    assert_eq!(BinaryScalar::default(), missing); // like the integers: Default is null

    // Construction from native shapes.
    assert_eq!(BinaryScalar::from(vec![1u8, 2, 3]), blob);
    assert_eq!(BinaryScalar::from(&[1u8, 2, 3][..]), blob);
    assert_eq!(BinaryScalar::from(None::<Vec<u8>>), missing);
    assert_eq!(
        BinaryScalar::from(ByteBuffer::from_bytes(vec![1, 2, 3])),
        blob
    );
}

#[test]
fn binary_scalar_is_a_positioned_io_resource() {
    let blob = BinaryScalar::new(vec![10, 20, 30, 40]);

    // Borrowed positioned reads through the core RawIOBase surface.
    let io = blob.io().unwrap();
    assert_eq!(io.byte_size(), 4);
    assert_eq!(io.pread_byte_one(2, Whence::Start).unwrap(), 30);
    assert_eq!(
        io.pread_byte_array(1, Whence::Start, 2).unwrap(),
        vec![20, 30]
    );

    // Moved into the core cursor adapter: a Seekable stream over the same value.
    let mut cursor = RawIOCursor::new(blob.clone().into_io().unwrap());
    assert_eq!(cursor.seek(1, Whence::Start).unwrap(), 1);
    assert_eq!(
        cursor.pread_byte_array(0, Whence::Current, 2).unwrap(),
        vec![20, 30]
    );
    assert_eq!(cursor.tell(), 3); // reads advance the cursor

    // And back: the resource rebuilds the scalar — the inverse of into_io.
    assert_eq!(BinaryScalar::from(cursor.into_inner()), blob);

    // The slice adapter bounds reads to a byte window of the value.
    let window = RawIOSlice::new(blob.clone().into_io().unwrap(), 1, 3);
    assert_eq!(window.byte_size(), 2);
    assert_eq!(
        window.pread_byte_array(0, Whence::Start, 2).unwrap(),
        vec![20, 30]
    );
}

#[test]
fn binary_scalar_arrow_round_trips_all_shapes() {
    // Bytes, the empty value and null are three distinct states.
    for scalar in [
        BinaryScalar::new(vec![1, 2, 3]),
        BinaryScalar::new(Vec::new()),
        BinaryScalar::null(),
    ] {
        let arrow = scalar.to_arrow();
        assert_eq!(arrow_array::Array::len(arrow.as_ref()), 1);
        assert_eq!(BinaryScalar::from_arrow(arrow.as_ref()).unwrap(), scalar);
    }

    // A non-binary array and a multi-element array are refused.
    assert!(matches!(
        BinaryScalar::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    let two = arrow_array::BinaryArray::from_iter_values([&b"a"[..], &b"b"[..]]);
    assert!(matches!(
        BinaryScalar::from_arrow(&two),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
}

#[test]
fn binary_composes_with_the_optional_and_list_families() {
    // Optional over binary: union storage, access redirected to the inner scalar.
    let optional = OptionalType::new(Binary);
    assert_eq!(optional.default_value(), Vec::<u8>::new());
    let some = OptionalScalar::new(BinaryScalar::new(b"hi".to_vec()));
    assert_eq!(some.as_bytes().unwrap(), b"hi");
    assert_eq!(some.as_str().unwrap(), "hi");
    assert_eq!(
        OptionalScalar::from_arrow(some.to_arrow().as_ref()).unwrap(),
        some
    );
    assert!(matches!(
        OptionalScalar::<Binary, BinaryScalar>::null().as_bytes(),
        Err(DataError::NullValue)
    ));

    // A list of binary: the scalar accessors hand back inner scalars and owned
    // native values (`Vec<u8>`, the owned form of the unsized `[u8]`).
    let blobs = ListScalar::<Binary, BinaryScalar>::new(vec![
        BinaryScalar::new(vec![1]),
        BinaryScalar::null(),
    ]);
    assert_eq!(blobs.len(), 2);
    assert_eq!(blobs.get_scalar_at(0), Some(BinaryScalar::new(vec![1])));
    assert_eq!(blobs.get_value_at(0), Some(vec![1u8]));
    assert_eq!(blobs.get_value_at(1), None); // a null element holds no value
    assert_eq!(
        ListScalar::from_arrow(blobs.to_arrow().as_ref()).unwrap(),
        blobs
    );
}

#[test]
fn binary_is_send_sync_and_joins_dyn_schemas() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Binary>();
    assert_send_sync::<BinaryField>();
    assert_send_sync::<BinaryScalar>();

    let types: Vec<Box<dyn RawDataType>> = vec![Box::new(Binary)];
    assert_eq!(types[0].name(), "binary");
}
