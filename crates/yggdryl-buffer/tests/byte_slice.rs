//! Tests for the bounded windows `ByteSlice` and `TypedSlice<T>`: window bounds,
//! clamped reads/writes (no growth), positions, and copy-on-write.

use yggdryl_buffer::{
    i256, ByteBuffer, ByteSlice, IOBase, IOCursor, IOSlice, IoError, IoPrimitive, TypedCursor,
    TypedIOBase, TypedIOSlice, TypedSlice, Whence,
};

/// Builds a `ByteBuffer` of `values`' little-endian bytes — the buffer-free stand-in for
/// a typed buffer's bytes now that buffers live in `yggdryl-buffer`.
fn byte_buffer<T: IoPrimitive>(values: &[T]) -> ByteBuffer {
    let mut bytes = Vec::new();
    for &value in values {
        value.write_le(&mut bytes);
    }
    ByteBuffer::from_vec(bytes)
}

#[test]
fn byte_slice_bounds_and_clamped_read() {
    let buffer = ByteBuffer::from_bytes(b"hello world");
    let mut slice = buffer.byte_slice(6, 5); // "world"
    assert_eq!(slice.slice_offset(), 6);
    assert_eq!(slice.slice_len(), 5);
    assert_eq!(slice.byte_size().unwrap(), 5, "all remaining at the start");
    assert_eq!(slice.byte_capacity().unwrap(), 5);

    // An over-request is clamped to the window end.
    assert_eq!(
        slice.pread_byte_array(100, Whence::Start).unwrap(),
        b"world"
    );
    assert_eq!(slice.byte_size().unwrap(), 0, "window fully read");
    assert_eq!(slice.as_bytes(), b"world");
}

#[test]
fn byte_slice_positions_are_window_relative() {
    let buffer = ByteBuffer::from_bytes(b"0123456789");
    let mut slice = buffer.byte_slice(3, 4); // "3456"
    assert_eq!(slice.byte_tell().unwrap(), 0);
    assert_eq!(slice.pread_byte_array(2, Whence::Start).unwrap(), b"34");
    assert_eq!(slice.byte_tell().unwrap(), 2);
    assert_eq!(slice.position(), 2);

    // End resolves against the window end (len), not the buffer.
    assert_eq!(slice.byte_seek(-1, Whence::End).unwrap(), 3);
    assert_eq!(slice.pread_byte_array(10, Whence::Current).unwrap(), b"6");

    slice.set_position(1);
    assert_eq!(slice.pread_byte_array(1, Whence::Current).unwrap(), b"4");
}

#[test]
fn byte_slice_write_is_clamped_and_copy_on_write() {
    let buffer = ByteBuffer::from_bytes(b"hello world");
    let mut slice = buffer.byte_slice(6, 5); // "world"
                                             // A write longer than the window writes only what fits (no growth).
    assert_eq!(
        slice
            .pwrite_byte_array(b"EARTHLING", Whence::Start)
            .unwrap(),
        5
    );
    assert_eq!(slice.as_bytes(), b"EARTH");
    assert_eq!(buffer.as_bytes(), b"hello world", "source buffer untouched");
}

#[test]
fn byte_slice_clamps_window_to_the_buffer() {
    let buffer = ByteBuffer::from_bytes(b"abc");
    let slice = buffer.byte_slice(2, 100); // len clamped to 1
    assert_eq!(slice.slice_len(), 1);
    let past = buffer.byte_slice(10, 5); // offset past the end
    assert_eq!(past.slice_len(), 0);
}

#[test]
fn typed_slice_over_a_typed_buffer() {
    // Elements 1..4 of five i32 → the [20, 30, 40] window (bytes 4..16).
    let mut slice = TypedSlice::<i32>::new(byte_buffer(&[10_i32, 20, 30, 40, 50]), 4, 12);
    assert_eq!(slice.slice_offset(), 4, "byte offset of element 1");
    assert_eq!(slice.slice_len(), 12, "3 i32 = 12 bytes");
    assert_eq!(slice.size().unwrap(), 3, "3 i32 remaining");

    // Reads are clamped to the window.
    assert_eq!(
        slice.pread_array(100, Whence::Start).unwrap(),
        vec![20, 30, 40]
    );
    assert_eq!(slice.size().unwrap(), 0);

    // T-unit seek within the window.
    slice.seek(-1, Whence::End).unwrap();
    assert_eq!(slice.pread_one(Whence::Current).unwrap(), 40);
}

#[test]
fn typed_slice_write_is_clamped_to_whole_values() {
    let source = byte_buffer(&[0_i32, 0, 0]);
    let mut slice = TypedSlice::<i32>::new(source.clone(), 4, 8); // 2 i32 window
                                                                  // Writing 3 values into a 2-value window writes only the 2 that fit.
    assert_eq!(slice.pwrite_array(&[7, 8, 9], Whence::Start).unwrap(), 2);
    assert_eq!(slice.pread_array(2, Whence::Start).unwrap(), vec![7, 8]);
    assert_eq!(source.as_bytes(), [0u8; 12], "source buffer untouched");
}

#[test]
fn byte_slice_is_a_typed_io_slice() {
    // ByteSlice satisfies the TypedIOSlice<u8> blanket impl.
    fn assert_typed_slice<S: TypedIOSlice<u8>>(_: &S) {}
    let slice = ByteBuffer::from_bytes(b"data").byte_slice(0, 4);
    assert_typed_slice(&slice);
    assert_eq!(slice.slice_len(), 4);
}

#[test]
fn byte_slice_with_capacity_is_a_writable_zeroed_window() {
    let mut slice = ByteSlice::with_byte_capacity(8);
    assert_eq!(slice.slice_len(), 8);
    assert_eq!(slice.byte_capacity().unwrap(), 8);
    assert_eq!(slice.as_bytes(), &[0u8; 8]);
    // It is writable within its fixed length.
    assert_eq!(
        slice
            .pwrite_byte_array(b"abcdefghXX", Whence::Start)
            .unwrap(),
        8
    );
    assert_eq!(slice.as_bytes(), b"abcdefgh");
}

#[test]
fn byte_slice_seek_past_end_and_negative() {
    let mut slice = ByteBuffer::from_bytes(&[0; 10]).byte_slice(2, 4); // window of 4
                                                                       // Seeking past the window end is allowed; nothing remains and reads are empty.
    assert_eq!(slice.byte_seek(9, Whence::Start).unwrap(), 9);
    assert_eq!(slice.byte_size().unwrap(), 0);
    assert!(slice
        .pread_byte_array(4, Whence::Current)
        .unwrap()
        .is_empty());
    // A negative resolved position is rejected.
    assert_eq!(
        slice.byte_seek(-1, Whence::Start).unwrap_err(),
        IoError::InvalidSeek {
            offset: -1,
            whence: Whence::Start
        }
    );
}

#[test]
fn byte_slice_bit_seek_and_pread_into() {
    let mut slice = ByteBuffer::from_bytes(b"ABCDEFGH").byte_slice(2, 4); // "CDEF"
    assert_eq!(slice.bit_tell().unwrap(), 0);
    assert_eq!(slice.bit_seek(16, Whence::Start).unwrap(), 16); // byte 2 of the window
    assert_eq!(slice.byte_tell().unwrap(), 2);
    assert_eq!(slice.bit_seek(0, Whence::End).unwrap(), 32); // window is 4 bytes * 8

    // pread_into is clamped to the window.
    let mut buf = [0u8; 10];
    let n = slice.pread_into(&mut buf, Whence::Start).unwrap();
    assert_eq!(n, 4);
    assert_eq!(&buf[..4], b"CDEF");
}

#[test]
fn zero_length_slice_reads_and_writes_nothing() {
    let mut slice = ByteBuffer::from_bytes(b"data").byte_slice(2, 0);
    assert_eq!(slice.slice_len(), 0);
    assert_eq!(slice.byte_size().unwrap(), 0);
    assert!(slice
        .pread_byte_array(10, Whence::Start)
        .unwrap()
        .is_empty());
    assert_eq!(slice.pwrite_byte_array(b"x", Whence::Start).unwrap(), 0);
    assert!(slice.as_bytes().is_empty());
}

#[test]
fn byte_slice_from_byte_cursor_windows_the_cursor_bytes() {
    let cursor = ByteBuffer::from_bytes(b"0123456789").byte_cursor();
    let slice = ByteSlice::from_byte_cursor(cursor, 4, 3); // "456"
    assert_eq!(slice.as_bytes(), b"456");
    assert_eq!(slice.slice_offset(), 4);
}

#[test]
fn typed_slice_pread_one_is_eof_at_the_window_end() {
    let mut slice = TypedSlice::<i32>::new(byte_buffer(&[1_i32, 2]), 0, 8);
    assert_eq!(slice.pread_array(2, Whence::Start).unwrap(), vec![1, 2]);
    assert!(matches!(
        slice.pread_one(Whence::Current).unwrap_err(),
        IoError::UnexpectedEof { needed: 4, .. }
    ));
}

#[test]
fn wide_int_typed_slice_over_a_byte_buffer() {
    // A TypedSlice<i256> works for the wide integers (no dedicated typed buffer).
    let values = [i256::MIN, i256::from_i128(7), i256::MAX];
    let mut source = <TypedCursor<i256> as TypedIOBase<i256>>::with_capacity(3);
    source.pwrite_array(&values, Whence::Start).unwrap();
    let bytes = source.to_byte_buffer();

    // Window onto the middle i256 (byte range [32, 64)).
    let mut slice = TypedSlice::<i256>::new(bytes, 32, 32);
    assert_eq!(slice.slice_len(), 32);
    assert_eq!(slice.size().unwrap(), 1);
    assert_eq!(slice.pread_one(Whence::Start).unwrap(), i256::from_i128(7));
}
