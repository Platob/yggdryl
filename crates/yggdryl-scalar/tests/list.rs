//! Integration tests for the `list` scalars — the generic [`Serie`] and the
//! buffer-backed [`Int64Serie`].

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError};
use yggdryl_scalar::{arrow_array, arrow_buffer, Int64, Int64Serie, RawScalar, Serie};

type Int64ListScalar = Serie<dtype::Int64, Int64>;

#[test]
fn list_scalar_round_trips_all_shapes() {
    // Elements, the empty list and null are three distinct states.
    let numbers = Int64ListScalar::new(vec![Int64::new(1), Int64::null()]);
    let arrow = numbers.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(
        Int64ListScalar::from_arrow(arrow.as_ref()).unwrap(),
        numbers
    );

    // The scalar accessors read elements back out, as scalars or native values.
    assert_eq!(numbers.get_scalar_at(0), Some(Int64::new(1)));
    assert_eq!(numbers.get_scalar_at(1), Some(Int64::null()));
    assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1);
    assert_eq!(numbers.get_at::<i32>(0).unwrap(), 1); // converted, exact-or-error
    assert!(matches!(
        numbers.get_at::<i64>(1),
        Err(DataError::NullValue) // a null element holds no value
    ));
    assert!(matches!(
        numbers.get_at::<i64>(2),
        Err(DataError::OutOfBounds { index: 2, len: 2 })
    ));

    let empty = Int64ListScalar::new(Vec::new());
    assert!(!empty.is_null());
    assert_eq!(
        Int64ListScalar::from_arrow(empty.to_arrow().as_ref()).unwrap(),
        empty
    );
    assert_eq!(Int64ListScalar::default(), empty);

    let missing = Int64ListScalar::null();
    assert!(missing.is_null());
    assert_eq!(
        Int64ListScalar::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // Construction from native shapes.
    assert_eq!(Int64ListScalar::from(None::<Vec<Int64>>), missing);

    // A non-list array is refused.
    assert!(matches!(
        Int64ListScalar::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn int64_serie_reads_borrowed_buffers() {
    let numbers = Int64Serie::from(vec![1, 2, 3]);
    assert!(!numbers.is_null());
    assert_eq!(numbers.len(), 3);
    assert_eq!(numbers.values(), Some(&[1, 2, 3][..]));
    assert_eq!(numbers.value(), Some(&[1, 2, 3][..]));
    assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1);
    assert!(matches!(
        numbers.get_at::<i64>(3),
        Err(DataError::OutOfBounds { index: 3, len: 3 })
    ));
    assert_eq!(numbers.get_scalar_at(2), Some(Int64::new(3)));
    assert_eq!(numbers.get_scalar_at(3), None);
    assert!(numbers.nulls().is_none());

    // The reassembled Arrow array borrows the same buffer — zero copy.
    let arrow = numbers.array().unwrap();
    assert_eq!(arrow.values().as_ptr(), numbers.values().unwrap().as_ptr());

    // Per-element nulls are read null-aware; the raw buffer keeps the slots.
    let sparse = Int64Serie::from(vec![Some(1), None]);
    assert_eq!(sparse.get_at::<i64>(0).unwrap(), 1);
    assert!(matches!(sparse.get_at::<i64>(1), Err(DataError::NullValue)));
    assert_eq!(sparse.get_scalar_at(1), Some(Int64::null()));
    assert_eq!(sparse.values().map(<[i64]>::len), Some(2));
    assert_eq!(
        sparse.nulls().map(arrow_buffer::NullBuffer::null_count),
        Some(1)
    );

    // An all-valid null buffer is normalized away at construction, so the stored
    // form is canonical and equality holds trivially.
    let buffered = Int64Serie::new(
        arrow_buffer::ScalarBuffer::from(vec![1, 2, 3]),
        Some(arrow_buffer::NullBuffer::new_valid(3)),
    )
    .unwrap();
    assert!(buffered.nulls().is_none());
    assert_eq!(buffered, numbers);

    // A null buffer of the wrong length is refused with an actionable error.
    assert!(matches!(
        Int64Serie::new(
            arrow_buffer::ScalarBuffer::from(vec![1, 2, 3]),
            Some(arrow_buffer::NullBuffer::new_valid(2)),
        ),
        Err(DataError::MismatchedNullBufferLength {
            expected: 3,
            got: 2
        })
    ));
}

#[test]
fn int64_serie_bridges_to_core_io_resources() {
    use yggdryl_scalar::yggdryl_core::{ByteBuffer, RawIOBase, Whence};

    // pwrite_io lays the elements out little-endian through pwrite_i64 ...
    let numbers = Int64Serie::from(vec![1, -2, 3]);
    let mut buffer = ByteBuffer::new();
    numbers.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
    assert_eq!(buffer.byte_size(), 24);
    assert_eq!(buffer.pread_i64(8, Whence::Start).unwrap(), -2);

    // ... and from_io reads them back: the exact inverse for all-valid elements.
    assert_eq!(Int64Serie::from_io(&buffer).unwrap(), numbers);

    // A byte size that is not a whole number of elements is refused.
    buffer.resize_bytes(25).unwrap();
    assert!(matches!(
        Int64Serie::from_io(&buffer),
        Err(DataError::InvalidByteLength {
            expected: 32,
            got: 25
        })
    ));

    // A null serie holds no elements to write.
    assert!(matches!(
        Int64Serie::null().pwrite_io(&mut buffer, 0, Whence::Start),
        Err(DataError::NullValue)
    ));
}

#[test]
fn int64_serie_round_trips_through_arrow_zero_copy() {
    let numbers = Int64Serie::from(vec![Some(1), None, Some(3)]);
    let arrow = numbers.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(Int64Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);

    // The list's child elements are the same buffer, shared, not copied.
    let list = arrow
        .as_any()
        .downcast_ref::<arrow_array::ListArray>()
        .unwrap();
    let child = list
        .values()
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .unwrap();
    assert_eq!(child.values().as_ptr(), numbers.values().unwrap().as_ptr());

    // The generic and the buffer-backed list scalar agree on the Arrow shape.
    let generic = Int64ListScalar::new(vec![Int64::new(1), Int64::null(), Int64::new(3)]);
    assert_eq!(generic.to_arrow().as_ref(), arrow.as_ref());

    // Empty and null are distinct states, both round-tripped.
    let empty = Int64Serie::default();
    assert!(!empty.is_null());
    assert!(empty.is_empty());
    assert_eq!(
        Int64Serie::from_arrow(empty.to_arrow().as_ref()).unwrap(),
        empty
    );

    let missing = Int64Serie::null();
    assert!(missing.is_null());
    assert_eq!((missing.values(), missing.array()), (None, None));
    assert!(matches!(
        missing.get_at::<i64>(0),
        Err(DataError::NullValue)
    ));
    assert_eq!(
        Int64Serie::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // A non-list array is refused.
    assert!(matches!(
        Int64Serie::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn list_scalars_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Int64ListScalar>();
    assert_send_sync::<Int64Serie>();
}
