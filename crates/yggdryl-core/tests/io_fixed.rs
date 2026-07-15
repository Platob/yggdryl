//! Edge-case tests for the `io::fixed` typed layer — the generic `Buffer<T>` / `Scalar<T>` /
//! `Serie<T>` and the `DataType` / `Field` descriptors, exercised through the two concrete
//! types this build ships (`u8` and `i32`). Focus: element vs byte lengths, little-endian
//! codec, null handling + lazy validity, and serialization round-trips through the
//! `IOCursor` byte sink.

use yggdryl_core::io::fixed::{
    Buffer, Field, I32Buffer, I32Scalar, I32Serie, PrimitiveType, Scalar, TypedField, U8Buffer,
    U8Serie,
};
use yggdryl_core::io::{Bytes, DataType, IOBase, IOCursor, IOSlice, IoError};

// -------------------------------------------------------------------------------------
// Deserialization robustness (corrupt / hostile input)
// -------------------------------------------------------------------------------------

#[test]
fn serie_read_from_rejects_corrupt_length() {
    // A header declaring `u64::MAX` elements would overflow `len * WIDTH`; the decode is
    // refused with a guided error rather than attempting a runaway allocation / panicking.
    let mut sink = Bytes::from_vec(vec![0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
    assert!(matches!(
        I32Serie::read_from(&mut sink),
        Err(IoError::CorruptLength { .. })
    ));
}

#[test]
fn typed_slice_rejects_a_misaligned_window() {
    // A byte window that is not element-aligned would break typed access, so `slice` rejects
    // it with a guided error rather than producing a buffer that panics on `as_slice`.
    let buffer = I32Buffer::from_vec(vec![1, 2, 3, 4]); // 16 bytes
    assert!(matches!(
        buffer.slice(2, 8), // byte offset 2 is not a multiple of 4
        Err(IoError::SliceMisaligned { width: 4, .. })
    ));
    assert!(matches!(
        buffer.slice(4, 6), // length 6 is not a multiple of 4
        Err(IoError::SliceMisaligned { .. })
    ));
    // Element-aligned windows are fine; byte buffers (width 1) never hit this.
    assert_eq!(buffer.slice(4, 8).unwrap().to_vec(), vec![2, 3]);
    assert_eq!(
        U8Buffer::from_slice(b"hello")
            .slice(1, 3)
            .unwrap()
            .as_slice(),
        b"ell"
    );
}

#[test]
fn as_slice_ignores_a_trailing_partial_element() {
    // A byte-built buffer whose length is not a whole number of elements exposes only its
    // whole elements (matching `count`) — no panic on the trailing partial bytes.
    let buffer = I32Buffer::from_bytes(&[1, 0, 0, 0, 2, 0, 0, 0, 9, 9]); // 2 × i32 + 2 stray
    assert_eq!(buffer.count(), 2);
    assert_eq!(buffer.as_slice(), &[1, 2]);
    assert_eq!(buffer.to_vec(), vec![1, 2]);
    assert!(I32Buffer::from_bytes(&[1, 2, 3]).as_slice().is_empty());
}

// -------------------------------------------------------------------------------------
// DataType / Field descriptors
// -------------------------------------------------------------------------------------

#[test]
fn data_type_names_and_widths() {
    assert_eq!(<PrimitiveType<u8>>::new().name(), "u8");
    assert_eq!(<PrimitiveType<u8>>::new().byte_width(), 1);
    assert_eq!(<PrimitiveType<i32>>::new().name(), "i32");
    assert_eq!(<PrimitiveType<i32>>::new().byte_width(), 4);
    assert!(<PrimitiveType<i32>>::new().is_fixed_width());
    // Zero-sized descriptors of the same type are equal.
    assert_eq!(<PrimitiveType<i32>>::new(), PrimitiveType::<i32>::default());
}

#[test]
fn field_erase_and_value_semantics() {
    let typed = <TypedField<i32>>::new("id", false);
    assert_eq!(typed.name(), "id");
    assert!(!typed.nullable());
    assert_eq!(typed.data_type().byte_width(), 4);

    let field = typed.erase();
    assert_eq!(field.type_name(), "i32");
    assert_eq!(field.byte_width(), 4);
    assert!(!field.nullable());
    assert_eq!(field, Field::new("id", &<PrimitiveType<i32>>::new(), false));

    // Field is a plain value: usable as a distinct map key.
    use std::collections::HashSet;
    let set: HashSet<Field> = [
        Field::new("a", &<PrimitiveType<u8>>::new(), true),
        Field::new("a", &<PrimitiveType<i32>>::new(), true),
        Field::new("a", &<PrimitiveType<u8>>::new(), true), // dup
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 2);
}

// -------------------------------------------------------------------------------------
// Buffer<T> typed access (element vs byte length, LE codec)
// -------------------------------------------------------------------------------------

#[test]
fn buffer_element_vs_byte_length_and_get_set_push() {
    let mut b = I32Buffer::from_vec(vec![1, 2, 3]);
    assert_eq!(b.count(), 3); // elements
    assert_eq!(b.len(), 12); // bytes (IOBase contract)
    assert_eq!(b.get(0), Some(1));
    assert_eq!(b.get(2), Some(3));
    assert_eq!(b.get(3), None); // out of range

    b.set(1, 20);
    assert_eq!(b.get(1), Some(20));
    b.push(4);
    assert_eq!(b.count(), 4);
    assert_eq!(b.as_slice(), &[1, 20, 3, 4]);
    assert_eq!(b.to_vec(), vec![1, 20, 3, 4]);
}

#[test]
fn buffer_little_endian_byte_layout() {
    // 1i32 is 01 00 00 00 little-endian; the raw bytes prove the codec.
    let b = I32Buffer::from_slice(&[1, 258]); // 258 = 0x0102
    assert_eq!(b.as_bytes(), &[1, 0, 0, 0, 2, 1, 0, 0]);
    // Round-trips element-wise.
    assert_eq!(b.get(1), Some(258));
}

#[test]
fn buffer_byte_io_still_works_on_a_typed_buffer() {
    // The byte-I/O family is available on any Buffer<T>, addressing the raw bytes.
    let mut b = I32Buffer::from_vec(vec![0; 2]);
    assert_eq!(b.pwrite(4, &[2, 1, 0, 0]), 4); // overwrite element 1's bytes
    assert_eq!(b.get(1), Some(258));
    let window = b.slice(4, 4).unwrap(); // zero-copy byte window == element 1
    assert_eq!(window.as_bytes(), &[2, 1, 0, 0]);
}

#[test]
fn u8_buffer_is_the_bytes_type() {
    // Bytes is exactly U8Buffer (= Buffer<u8>): element count == byte length.
    let b = U8Buffer::from_slice(b"abc");
    assert_eq!(b.count(), 3);
    assert_eq!(b.len(), 3);
    assert_eq!(b.as_slice(), b"abc");
    let _as_bytes: Bytes = b; // same type
}

// -------------------------------------------------------------------------------------
// Scalar<T> — nullable value + serialization
// -------------------------------------------------------------------------------------

#[test]
fn scalar_construction_and_nullability() {
    assert_eq!(I32Scalar::of(42).value(), Some(42));
    assert!(!I32Scalar::of(42).is_null());
    assert!(I32Scalar::null().is_null());
    assert_eq!(I32Scalar::new(None).value(), None);
    assert_eq!(Scalar::from(7i32), I32Scalar::of(7));
    assert_eq!(I32Scalar::serialized_width(), 5); // 1 validity + 4 value
}

#[test]
fn scalar_round_trips_through_a_byte_sink() {
    for scalar in [
        I32Scalar::of(-1),
        I32Scalar::of(0),
        I32Scalar::of(70000),
        I32Scalar::null(),
    ] {
        let mut sink = Bytes::new();
        scalar.write_to(&mut sink).unwrap();
        assert_eq!(sink.len(), I32Scalar::serialized_width() as u64);
        sink.rewind();
        assert_eq!(I32Scalar::read_from(&mut sink).unwrap(), scalar);
    }

    // The on-wire form: present validity byte then little-endian value.
    let mut sink = Bytes::new();
    I32Scalar::of(1).write_to(&mut sink).unwrap();
    assert_eq!(sink.as_slice(), &[1, 1, 0, 0, 0]);
    let mut null_sink = Bytes::new();
    I32Scalar::null().write_to(&mut null_sink).unwrap();
    assert_eq!(null_sink.as_slice(), &[0, 0, 0, 0, 0]);
}

#[test]
fn multiple_scalars_stream_in_sequence() {
    let mut sink = Bytes::new();
    for v in [10, 20, 30] {
        I32Scalar::of(v).write_to(&mut sink).unwrap();
    }
    sink.rewind();
    let read: Vec<_> = (0..3)
        .map(|_| I32Scalar::read_from(&mut sink).unwrap().value())
        .collect();
    assert_eq!(read, vec![Some(10), Some(20), Some(30)]);
}

#[test]
fn scalar_read_past_end_is_a_guided_error() {
    let mut sink = Bytes::from_slice(&[1, 1, 0]); // truncated (need 5 bytes)
    assert!(I32Scalar::read_from(&mut sink).is_err());
}

// -------------------------------------------------------------------------------------
// Serie<T> — nullable column + validity + serialization
// -------------------------------------------------------------------------------------

#[test]
fn serie_from_values_has_no_nulls() {
    let col = I32Serie::from_values(&[1, 2, 3]);
    assert_eq!(col.len(), 3);
    assert!(!col.is_empty());
    assert_eq!(col.null_count(), 0);
    assert!(!col.has_nulls());
    assert_eq!(col.get(0), Some(1));
    assert_eq!(col.get(3), None); // out of range
    assert_eq!(col.to_options(), vec![Some(1), Some(2), Some(3)]);
}

#[test]
fn serie_push_lazily_materializes_validity() {
    let mut col = I32Serie::new();
    col.push(Some(1));
    col.push(Some(2));
    assert!(!col.has_nulls()); // no validity mask yet
    col.push(None); // first null materializes the mask over the earlier all-valid elements
    col.push(Some(4));
    assert_eq!(col.len(), 4);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.to_options(), vec![Some(1), Some(2), None, Some(4)]);
    assert_eq!(col.get(2), None);
    assert_eq!(col.get(3), Some(4));
}

#[test]
fn serie_from_options_and_from_iter() {
    let col = I32Serie::from_options(&[Some(1), None, Some(3)]);
    assert_eq!(col.to_options(), vec![Some(1), None, Some(3)]);
    assert_eq!(col.null_count(), 1);

    let collected: I32Serie = [Some(5), None, None, Some(8)].into_iter().collect();
    assert_eq!(collected.len(), 4);
    assert_eq!(collected.null_count(), 2);
}

#[test]
fn serie_all_null_and_empty_edges() {
    let empty = I32Serie::new();
    assert!(empty.is_empty());
    assert_eq!(empty.null_count(), 0);

    let all_null = I32Serie::from_options(&[None, None, None]);
    assert_eq!(all_null.len(), 3);
    assert_eq!(all_null.null_count(), 3);
    assert_eq!(all_null.to_options(), vec![None, None, None]);
}

#[test]
fn serie_round_trips_through_a_byte_sink() {
    for col in [
        I32Serie::from_values(&[1, 2, 3, 4, 5]),
        I32Serie::from_options(&[Some(1), None, Some(3)]),
        I32Serie::from_options(&[None; 9]), // spans two validity bytes
        I32Serie::new(),
    ] {
        let mut sink = Bytes::new();
        col.write_to(&mut sink).unwrap();
        sink.rewind();
        let read = I32Serie::read_from(&mut sink).unwrap();
        assert_eq!(read, col);
        assert_eq!(read.to_options(), col.to_options());
    }
}

#[test]
fn serie_validity_bitmap_spans_byte_boundaries() {
    // 10 elements with nulls at 0 and 9 exercise both bytes of the validity mask.
    let mut col = I32Serie::new();
    for i in 0..10 {
        col.push(if i == 0 || i == 9 { None } else { Some(i) });
    }
    assert_eq!(col.null_count(), 2);
    assert_eq!(col.get(0), None);
    assert_eq!(col.get(9), None);
    assert_eq!(col.get(5), Some(5));

    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(I32Serie::read_from(&mut sink).unwrap(), col);
}

// -------------------------------------------------------------------------------------
// Genericity: the same API over u8
// -------------------------------------------------------------------------------------

#[test]
fn the_same_typed_api_works_for_u8() {
    let mut col = U8Serie::from_values(&[1, 2, 3]);
    col.push(None);
    assert_eq!(col.null_count(), 1);
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(U8Serie::read_from(&mut sink).unwrap(), col);

    // Buffer<u8> typed access coincides with its bytes.
    let b = Buffer::<u8>::from_vec(vec![9, 8, 7]);
    assert_eq!(b.get(1), Some(8));
    assert_eq!(b.as_slice(), &[9, 8, 7]);
}
