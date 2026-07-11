//! Edge-case tests for the positioned-IO surface: negative-offset seeks, auto-resize
//! on write, and the `set_byte_capacity` reserve / reduce behaviour.

use yggdryl_buffer::{
    ByteBuffer, IOBase, IOCursor, IoError, IoPrimitive, TypedCursor, TypedIOBase, Whence,
};

/// Builds a `TypedCursor<T>` over `values`' little-endian bytes — the buffer-free
/// stand-in for a typed buffer's `.cursor()` now that buffers live in `yggdryl-buffer`.
fn typed_cursor<T: IoPrimitive>(values: &[T]) -> TypedCursor<T> {
    let mut bytes = Vec::new();
    for &value in values {
        value.write_le(&mut bytes);
    }
    TypedCursor::new(ByteBuffer::from_vec(bytes))
}

// -------------------------------------------------------------------------------
// Negative-offset seeks
// -------------------------------------------------------------------------------

#[test]
fn negative_offset_from_end_and_current_resolve() {
    let mut cursor = ByteBuffer::from_bytes(b"0123456789").byte_cursor();

    // 3 back from the end of a 10-byte resource -> absolute 7.
    assert_eq!(cursor.byte_seek(-3, Whence::End).unwrap(), 7);
    assert_eq!(cursor.pread_byte_array(3, Whence::Current).unwrap(), b"789");

    // From the current position (10), 4 back -> 6.
    assert_eq!(cursor.byte_seek(-4, Whence::Current).unwrap(), 6);
    assert_eq!(cursor.byte_tell().unwrap(), 6);
}

#[test]
fn seek_before_start_is_invalid_seek_with_guidance() {
    let mut cursor = ByteBuffer::from_bytes(b"abc").byte_cursor();

    let err = cursor.byte_seek(-1, Whence::Start).unwrap_err();
    assert!(matches!(err, IoError::InvalidSeek { offset: -1, .. }));
    assert!(err.to_string().contains("before the start"));

    // Negative past the start from End is equally rejected, and the position is
    // unchanged after a failed seek.
    cursor.byte_seek(2, Whence::Start).unwrap();
    assert!(cursor.byte_seek(-100, Whence::End).is_err());
    assert_eq!(
        cursor.byte_tell().unwrap(),
        2,
        "failed seek must not move the cursor"
    );
}

#[test]
fn seek_past_end_then_read_yields_empty_not_error() {
    let mut cursor = ByteBuffer::from_bytes(b"abc").byte_cursor();
    // Seeking past the end is allowed (like std::io::Cursor); the read just returns
    // nothing rather than erroring.
    assert_eq!(cursor.byte_seek(100, Whence::Start).unwrap(), 100);
    assert_eq!(cursor.pread_byte_array(4, Whence::Current).unwrap(), b"");
    assert_eq!(
        cursor.byte_size().unwrap(),
        0,
        "remaining past the end is zero"
    );
}

#[test]
fn read_into_past_end_returns_zero_without_panicking() {
    // Regression: pread_into indexed data[start..end] with start > end and panicked.
    let mut cursor = ByteBuffer::from_bytes(b"abcdef").byte_cursor();
    cursor.byte_seek(100, Whence::Start).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(cursor.pread_into(&mut buf, Whence::Current).unwrap(), 0);
    // The cursor does not jump backward on a past-end read.
    assert_eq!(cursor.byte_tell().unwrap(), 100);

    // The typed fast path (which routes through pread_into) is also safe past end.
    let mut typed = typed_cursor::<i32>(&[1, 2]);
    typed.byte_seek(100, Whence::Start).unwrap();
    assert!(matches!(
        typed.pread_one(Whence::Current),
        Err(IoError::UnexpectedEof { .. })
    ));
}

#[test]
fn typed_negative_seek_counts_in_elements() {
    let mut cursor = typed_cursor::<i32>(&[10, 20, 30, 40]);
    // Seek to element index 4 (the end), then 2 elements back -> index 2.
    assert_eq!(
        TypedIOBase::<i32>::seek(&mut cursor, -2, Whence::End).unwrap(),
        2
    );
    assert_eq!(cursor.pread_one(Whence::Current).unwrap(), 30);
    // A negative element seek before the start is rejected.
    assert!(TypedIOBase::<i32>::seek(&mut cursor, -1, Whence::Start).is_err());
}

// -------------------------------------------------------------------------------
// Auto-resize on write
// -------------------------------------------------------------------------------

#[test]
fn write_past_end_auto_grows_the_resource() {
    // Seek past the current end, then write: the gap is zero-filled and the resource
    // grows to fit.
    let mut cursor = ByteBuffer::from_bytes(b"ab").byte_cursor();
    cursor.byte_seek(5, Whence::Start).unwrap(); // past the 2-byte end
    cursor.pwrite_byte_array(b"XY", Whence::Current).unwrap();
    assert_eq!(cursor.as_bytes(), b"ab\x00\x00\x00XY");
    assert_eq!(cursor.as_bytes().len(), 7);
}

#[test]
fn append_grows_length_and_capacity() {
    let mut cursor = ByteBuffer::with_byte_capacity(2).byte_cursor();
    assert!(cursor.byte_capacity().unwrap() >= 2);

    let payload = vec![7u8; 1000];
    cursor.pwrite_byte_array(&payload, Whence::Start).unwrap();
    assert_eq!(cursor.as_bytes().len(), 1000, "write auto-grows the length");
    assert!(
        cursor.byte_capacity().unwrap() >= 1000,
        "capacity auto-grows to fit the write"
    );
}

#[test]
fn with_capacity_below_need_still_grows_on_write() {
    // A capacity hint that is too small never caps the data — writes auto-grow.
    let mut cursor = ByteBuffer::with_byte_capacity(1).byte_cursor();
    cursor
        .pwrite_i64_array(&[1, 2, 3, 4], Whence::Start)
        .unwrap(); // 32 bytes
    assert_eq!(cursor.byte_size().unwrap(), 0); // cursor is at the end
    assert_eq!(
        cursor.pread_i64_array(4, Whence::Start).unwrap(),
        vec![1, 2, 3, 4]
    );
}

// -------------------------------------------------------------------------------
// set_byte_capacity: reserve above, reduce below
// -------------------------------------------------------------------------------

#[test]
fn set_capacity_above_length_reserves_without_changing_content() {
    let mut cursor = ByteBuffer::from_bytes(b"abc").byte_cursor();
    let cap = cursor.set_byte_capacity(128);
    assert!(cap >= 128, "grows the reservation");
    assert_eq!(
        cursor.as_bytes(),
        b"abc",
        "content is untouched when growing"
    );
}

#[test]
fn set_capacity_below_length_reduces_the_inner_buffer() {
    let mut cursor = ByteBuffer::from_bytes(b"abcdefgh").byte_cursor();
    cursor.byte_seek(0, Whence::End).unwrap(); // position at 8

    cursor.set_byte_capacity(3); // below the length -> truncate the content
    assert_eq!(
        cursor.as_bytes(),
        b"abc",
        "content reduced to the new capacity"
    );
    assert_eq!(
        cursor.position(),
        3,
        "the cursor is clamped to the new, shorter end"
    );
}

#[test]
fn set_capacity_to_zero_empties_the_resource() {
    let mut cursor = ByteBuffer::from_bytes(b"data").byte_cursor();
    cursor.set_byte_capacity(0);
    assert_eq!(cursor.as_bytes(), b"");
    assert_eq!(cursor.position(), 0);
}

#[test]
fn set_capacity_leaves_the_source_buffer_intact() {
    let buffer = ByteBuffer::from_bytes(b"shared");
    let mut cursor = buffer.byte_cursor();
    cursor.set_byte_capacity(2); // copy-on-write reduce
    assert_eq!(cursor.as_bytes(), b"sh");
    assert_eq!(
        buffer.as_bytes(),
        b"shared",
        "the source buffer is untouched"
    );
}

#[test]
fn set_bit_capacity_rounds_up_to_whole_bytes() {
    let mut cursor = ByteBuffer::new().byte_cursor();
    let cap = cursor.set_bit_capacity(17); // 17 bits -> 3 bytes
    assert!(cap >= 3);
}
