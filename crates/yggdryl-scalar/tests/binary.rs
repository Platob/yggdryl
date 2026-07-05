//! Integration tests for the `binary` scalar — the byte value that doubles as a
//! `yggdryl-core` positioned-IO resource.

use yggdryl_scalar::yggdryl_core::{
    ByteBuffer, ByteBufferSlice, Latin1, RawIOBase, RawIOCursor, RawIOSlice, Seekable, Whence,
};
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError};
use yggdryl_scalar::{arrow_array, BinaryScalar, Scalar, TypedOptionalScalar, TypedSerie};

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
    assert_eq!(
        BinaryScalar::new(b"hi".to_vec()).as_str(None).unwrap(),
        "hi"
    );
    assert!(matches!(
        BinaryScalar::new(vec![0xFF]).as_str(None),
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
    assert!(matches!(missing.as_str(None), Err(DataError::NullValue)));
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
        let arrow = scalar.to_arrow_scalar().into_inner();
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
    let some = TypedOptionalScalar::new(BinaryScalar::new(b"hi".to_vec()));
    assert_eq!(some.as_bytes().unwrap(), b"hi");
    assert_eq!(some.as_str(None).unwrap(), "hi");
    assert_eq!(
        TypedOptionalScalar::from_arrow(some.to_arrow_scalar().into_inner().as_ref()).unwrap(),
        some
    );
    assert!(matches!(
        TypedOptionalScalar::<dtype::BinaryType, BinaryScalar>::null().as_bytes(),
        Err(DataError::NullValue)
    ));

    // A list of binary: the scalar accessors hand back inner scalars and owned
    // native values (`Vec<u8>`, the owned form of the unsized `[u8]`).
    let blobs = TypedSerie::<dtype::BinaryType, BinaryScalar>::new(vec![
        BinaryScalar::new(vec![1]),
        BinaryScalar::null(),
    ]);
    assert_eq!(blobs.len(), 2);
    assert_eq!(blobs.scalar_at(0), Some(BinaryScalar::new(vec![1])));
    assert_eq!(blobs.value_at::<Vec<u8>>(0).unwrap(), vec![1u8]);
    assert!(matches!(
        blobs.value_at::<Vec<u8>>(1),
        Err(DataError::NullValue) // a null element holds no value
    ));
    assert_eq!(
        TypedSerie::from_arrow(blobs.to_arrow_scalar().into_inner().as_ref()).unwrap(),
        blobs
    );
}

#[test]
fn binary_reads_as_a_byte_buffer_slice_and_any_charset() {
    // into_io_slice moves the buffer into a full-window core slice — zero copy.
    let blob = BinaryScalar::new(vec![10, 20, 30]);
    let window = blob.clone().into_io_slice().unwrap();
    assert_eq!(window.byte_size(), 3);
    assert_eq!(window.pread_byte_one(1, Whence::Start).unwrap(), 20);
    assert_eq!(window.pread_i8(2, Whence::Start).unwrap(), 30);
    assert!(BinaryScalar::null().into_io_slice().is_none());

    // An explicit core charset decodes as_str; the default stays borrowed UTF-8.
    let accented = BinaryScalar::new(vec![0xE9]);
    assert_eq!(accented.as_str(Some(&Latin1)).unwrap(), "\u{e9}");
    assert!(accented.as_str(None).is_err()); // not valid UTF-8
    let plain = BinaryScalar::new(b"hi".to_vec());
    assert!(matches!(
        plain.as_str(None).unwrap(),
        std::borrow::Cow::Borrowed("hi")
    ));

    // The generic native accessor: a binary serie element as bytes, String or a
    // positioned-IO window.
    let blobs =
        TypedSerie::<dtype::BinaryType, BinaryScalar>::new(vec![BinaryScalar::new(b"hi".to_vec())]);
    assert_eq!(blobs.value_at::<Vec<u8>>(0).unwrap(), b"hi".to_vec());
    assert_eq!(blobs.value_at::<String>(0).unwrap(), "hi");
    let window = blobs.value_at::<ByteBufferSlice>(0).unwrap();
    assert_eq!(window.byte_size(), 2);
    assert_eq!(window.pread_byte_one(0, Whence::Start).unwrap(), b'h');
}

#[test]
fn binary_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BinaryScalar>();
}
