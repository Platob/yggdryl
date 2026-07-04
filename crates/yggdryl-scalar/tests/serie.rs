//! Integration tests for the `serie` scalars — the dynamic [`Serie`], the generic
//! [`TypedSerie`] and the buffer-backed integer series, every signed and unsigned
//! width.

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError, DataType};
use yggdryl_scalar::{arrow_array, arrow_buffer, Int64Scalar, Scalar, Serie, TypedSerie};

type Int64GenericSerie = TypedSerie<dtype::Int64Type, Int64Scalar>;

#[test]
fn serie_scalar_round_trips_all_shapes() {
    // Elements, the empty serie and null are three distinct states.
    let numbers = Int64GenericSerie::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
    let arrow = numbers.to_arrow_scalar();
    assert_eq!(arrow.len(), 1);
    assert_eq!(
        Int64GenericSerie::from_arrow(arrow.as_ref()).unwrap(),
        numbers
    );

    // The scalar accessors read elements back out, as scalars or native values.
    assert_eq!(numbers.get_scalar_at(0), Some(Int64Scalar::new(1)));
    assert_eq!(numbers.get_scalar_at(1), Some(Int64Scalar::null()));
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

    let empty = Int64GenericSerie::new(Vec::new());
    assert!(!empty.is_null());
    assert_eq!(
        Int64GenericSerie::from_arrow(empty.to_arrow_scalar().as_ref()).unwrap(),
        empty
    );
    assert_eq!(Int64GenericSerie::default(), empty);

    let missing = Int64GenericSerie::null();
    assert!(missing.is_null());
    assert_eq!(
        Int64GenericSerie::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
        missing
    );

    // Construction from native shapes.
    assert_eq!(Int64GenericSerie::from(None::<Vec<Int64Scalar>>), missing);

    // A non-serie array is refused.
    assert!(matches!(
        Int64GenericSerie::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

// Every buffer-backed integer serie shares the same shape, so one macro drives one
// test module per type: buffer borrows, the core-IO bridge, the zero-copy Arrow
// round trip and the width's extremes.
macro_rules! int_serie_tests {
    ($mod:ident, $ty:ident, $scalar:ident, $native:ty, $array:ident, $width:literal) => {
        mod $mod {
            use super::*;
            use yggdryl_scalar::{$scalar, $ty};

            #[test]
            fn reads_borrowed_buffers() {
                let numbers = $ty::from(vec![1, 2, 3]);
                assert!(!numbers.is_null());
                assert_eq!(numbers.len(), 3);
                assert_eq!(numbers.values(), Some(&[1, 2, 3][..]));
                assert_eq!(numbers.value(), Some(&[1, 2, 3][..]));
                assert_eq!(numbers.get_at::<$native>(0).unwrap(), 1);
                assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2); // converted target
                assert!(matches!(
                    numbers.get_at::<$native>(3),
                    Err(DataError::OutOfBounds { index: 3, len: 3 })
                ));
                assert_eq!(numbers.get_scalar_at(2), Some($scalar::new(3)));
                assert_eq!(numbers.get_scalar_at(3), None);
                assert!(numbers.nulls().is_none());

                // The reassembled Arrow array borrows the same buffer — zero copy.
                let arrow = numbers.to_arrow_array();
                assert_eq!(arrow.values().as_ptr(), numbers.values().unwrap().as_ptr());

                // Per-element nulls are read null-aware; the raw buffer keeps the slots.
                let sparse = $ty::from(vec![Some(1), None]);
                assert_eq!(sparse.get_at::<$native>(0).unwrap(), 1);
                assert!(matches!(
                    sparse.get_at::<$native>(1),
                    Err(DataError::NullValue)
                ));
                assert_eq!(sparse.get_scalar_at(1), Some($scalar::null()));
                assert_eq!(sparse.values().map(<[$native]>::len), Some(2));
                assert_eq!(
                    sparse.nulls().map(arrow_buffer::NullBuffer::null_count),
                    Some(1)
                );

                // An all-valid null buffer is normalized away at construction, so the
                // stored form is canonical and equality holds trivially.
                let buffered = $ty::new(
                    arrow_buffer::ScalarBuffer::from(vec![1, 2, 3]),
                    Some(arrow_buffer::NullBuffer::new_valid(3)),
                )
                .unwrap();
                assert!(buffered.nulls().is_none());
                assert_eq!(buffered, numbers);

                // A null buffer of the wrong length is refused with an actionable error.
                assert!(matches!(
                    $ty::new(
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
            fn bridges_to_core_io_resources() {
                use yggdryl_scalar::yggdryl_core::{ByteBuffer, RawIOBase, Whence};

                // pwrite_io lays the elements out little-endian in one bulk write ...
                let numbers = $ty::from(vec![1, 2, 3]);
                let mut buffer = ByteBuffer::new();
                numbers.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
                assert_eq!(buffer.byte_size(), 3 * $width);
                assert_eq!(
                    buffer.pread_byte_array(0, Whence::Start, $width).unwrap(),
                    (1 as $native).to_le_bytes()
                );

                // ... and from_io reads them back in one bulk read: the exact inverse
                // for all-valid elements.
                assert_eq!($ty::from_io(&buffer).unwrap(), numbers);

                // Writing relative to End resolves the end once and appends the whole
                // serie contiguously after it.
                numbers.pwrite_io(&mut buffer, 0, Whence::End).unwrap();
                assert_eq!(buffer.byte_size(), 6 * $width);
                assert_eq!($ty::from_io(&buffer).unwrap().len(), 6);

                // An empty resource reads back as the empty serie, not null.
                let empty = ByteBuffer::new();
                let read = $ty::from_io(&empty).unwrap();
                assert!(read.is_empty() && !read.is_null());

                // A byte size that is not a whole number of elements is refused
                // (every size is whole for the 1-byte widths).
                if $width > 1 {
                    buffer.resize_bytes(6 * $width + 1).unwrap();
                    assert!(matches!(
                        $ty::from_io(&buffer),
                        Err(DataError::InvalidByteLength {
                            expected,
                            got,
                        }) if expected == 7 * $width && got == 6 * $width + 1
                    ));
                }

                // A null serie holds no elements to write.
                assert!(matches!(
                    $ty::null().pwrite_io(&mut buffer, 0, Whence::Start),
                    Err(DataError::NullValue)
                ));
            }

            #[test]
            fn round_trips_through_arrow_zero_copy() {
                let numbers = $ty::from(vec![Some(1), None, Some(3)]);
                let arrow = numbers.to_arrow_scalar();
                assert_eq!(arrow.len(), 1);
                assert_eq!($ty::from_arrow(arrow.as_ref()).unwrap(), numbers);

                // The serie's child elements are the same buffer, shared, not copied.
                let serie = arrow
                    .as_any()
                    .downcast_ref::<arrow_array::ListArray>()
                    .unwrap();
                let child = serie
                    .values()
                    .as_any()
                    .downcast_ref::<arrow_array::$array>()
                    .unwrap();
                assert_eq!(child.values().as_ptr(), numbers.values().unwrap().as_ptr());

                // The generic and the buffer-backed serie scalar agree on the Arrow shape.
                let generic = TypedSerie::new(vec![$scalar::new(1), $scalar::null(), $scalar::new(3)]);
                assert_eq!(generic.to_arrow_scalar().as_ref(), arrow.as_ref());

                // Empty and null are distinct states, both round-tripped.
                let empty = $ty::default();
                assert!(!empty.is_null());
                assert!(empty.is_empty());
                assert_eq!($ty::from_arrow(empty.to_arrow_scalar().as_ref()).unwrap(), empty);

                let missing = $ty::null();
                assert!(missing.is_null());
                assert_eq!(missing.values(), None);
                assert!(missing.to_arrow_array().is_empty()); // null → empty array
                assert!(matches!(
                    missing.get_at::<$native>(0),
                    Err(DataError::NullValue)
                ));
                assert_eq!(
                    $ty::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
                    missing
                );

                // A non-serie array is refused, and so is a serie of another element
                // type.
                assert!(matches!(
                    $ty::from_arrow(&arrow_array::$array::from_iter_values([1])),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
                let foreign = TypedSerie::new(vec![yggdryl_scalar::BinaryScalar::new(vec![1])]);
                assert!(matches!(
                    $ty::from_arrow(foreign.to_arrow_scalar().as_ref()),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
            }

            #[test]
            fn extremes_round_trip() {
                use yggdryl_scalar::yggdryl_core::{ByteBuffer, Whence};

                // MIN, 0 and MAX survive the buffer, the IO bridge and Arrow intact.
                let extremes = $ty::from(vec![<$native>::MIN, 0, <$native>::MAX]);
                assert_eq!(extremes.get_at::<$native>(0).unwrap(), <$native>::MIN);
                assert_eq!(extremes.get_at::<$native>(2).unwrap(), <$native>::MAX);

                let mut buffer = ByteBuffer::new();
                extremes.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
                assert_eq!($ty::from_io(&buffer).unwrap(), extremes);
                assert_eq!(
                    $ty::from_arrow(extremes.to_arrow_scalar().as_ref()).unwrap(),
                    extremes
                );
            }

            #[test]
            fn is_send_sync() {
                fn assert_send_sync<T: Send + Sync>() {}
                assert_send_sync::<$ty>();
            }
        }
    };
}

int_serie_tests!(int8, Int8Serie, Int8Scalar, i8, Int8Array, 1);
int_serie_tests!(int16, Int16Serie, Int16Scalar, i16, Int16Array, 2);
int_serie_tests!(int32, Int32Serie, Int32Scalar, i32, Int32Array, 4);
int_serie_tests!(int64, Int64Serie, Int64Scalar, i64, Int64Array, 8);
int_serie_tests!(uint8, UInt8Serie, UInt8Scalar, u8, UInt8Array, 1);
int_serie_tests!(uint16, UInt16Serie, UInt16Scalar, u16, UInt16Array, 2);
int_serie_tests!(uint32, UInt32Serie, UInt32Scalar, u32, UInt32Array, 4);
int_serie_tests!(uint64, UInt64Serie, UInt64Scalar, u64, UInt64Array, 8);

#[test]
fn serie_from_a_sliced_arrow_row_keeps_the_window() {
    use yggdryl_scalar::Int32Serie;

    // A two-row serie array sliced to its second row: the child window has a
    // non-zero offset, and from_arrow must honour it — sharing, not re-basing,
    // the underlying buffer.
    let rows =
        arrow_array::ListArray::from_iter_primitive::<arrow_array::types::Int32Type, _, _>(vec![
            Some(vec![Some(1), Some(2)]),
            Some(vec![Some(3), None, Some(5)]),
        ]);
    let row = rows.slice(1, 1);
    let serie = Int32Serie::from_arrow(&row).unwrap();
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.get_at::<i32>(0).unwrap(), 3);
    assert!(matches!(serie.get_at::<i32>(1), Err(DataError::NullValue)));
    assert_eq!(serie.get_at::<i32>(2).unwrap(), 5);
    assert_eq!(serie, Int32Serie::from(vec![Some(3), None, Some(5)]));

    // Shared, not copied: the serie's buffer is the original child buffer,
    // starting at the second row's offset (element index 2).
    let child = rows
        .values()
        .as_any()
        .downcast_ref::<arrow_array::Int32Array>()
        .unwrap();
    assert_eq!(
        serie.values().unwrap().as_ptr(),
        child.values()[2..].as_ptr()
    );
}

#[test]
fn narrowing_reads_are_exact_or_error() {
    use yggdryl_scalar::{Int8Serie, UInt64Serie};

    // A converted read follows the element scalar's exact-or-error contract.
    let huge = UInt64Serie::from(vec![u64::MAX]);
    assert!(matches!(
        huge.get_at::<i64>(0),
        Err(DataError::InexactConversion { .. })
    ));
    assert_eq!(huge.get_at::<u64>(0).unwrap(), u64::MAX);

    let negative = Int8Serie::from(vec![-1]);
    assert!(matches!(
        negative.get_at::<u64>(0),
        Err(DataError::InexactConversion { .. })
    ));
    assert_eq!(negative.get_at::<i64>(0).unwrap(), -1);
}

#[test]
fn dynamic_serie_erases_and_round_trips() {
    // The dynamic serie is reached by erasing a typed one; it keeps the element
    // array and round-trips through Arrow, element type erased.
    let numbers = Int64GenericSerie::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
    let dynamic = numbers.erase();
    assert!(!dynamic.is_null());
    assert_eq!(dynamic.len(), 2);
    assert_eq!(dynamic.data_type().name(), "list");
    assert_eq!(
        Serie::from_arrow(dynamic.to_arrow_scalar().as_ref()).unwrap(),
        dynamic
    );

    // A null typed serie erases to a null dynamic serie.
    let missing = Int64GenericSerie::null().erase();
    assert!(missing.is_null());
    assert!(missing.is_empty());
    assert!(missing.to_arrow_array().is_empty()); // null → empty element array
}

#[test]
fn serie_scalars_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Serie>();
    assert_send_sync::<Int64GenericSerie>();
}
