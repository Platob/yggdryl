//! Tests for the element-typed cursor `TypedCursor<T>`: `T`-unit positioning,
//! per-type round trips, default-fill growth, and capacity.

use yggdryl_buffer::{
    ByteBuffer, IOBase, IOCursor, IoError, IoPrimitive, TypedCursor, TypedIOBase, Whence,
};

/// Builds a `TypedCursor<T>` over `values` (their little-endian bytes) — the buffer-free
/// stand-in for a typed buffer's `.cursor()` now that buffers live in `yggdryl-buffer`.
fn typed_cursor<T: IoPrimitive>(values: &[T]) -> TypedCursor<T> {
    let mut bytes = Vec::new();
    for &value in values {
        value.write_le(&mut bytes);
    }
    TypedCursor::new(ByteBuffer::from_vec(bytes))
}

#[test]
fn typed_cursor_round_trips_per_type() {
    let mut i16c = typed_cursor::<i16>(&[-1, 2, -3]);
    assert_eq!(i16c.pread_array(3, Whence::Start).unwrap(), vec![-1, 2, -3]);

    let mut i64c = typed_cursor::<i64>(&[i64::MIN, 0, i64::MAX]);
    assert_eq!(
        i64c.pread_array(3, Whence::Start).unwrap(),
        vec![i64::MIN, 0, i64::MAX]
    );

    let mut f64c = typed_cursor::<f64>(&[1.5, -2.5, f64::INFINITY]);
    assert_eq!(
        f64c.pread_array(3, Whence::Start).unwrap(),
        vec![1.5, -2.5, f64::INFINITY]
    );

    // u8 typed cursor coincides with the byte surface.
    let mut u8c = typed_cursor::<u8>(&[7, 8, 9]);
    assert_eq!(u8c.pread_one(Whence::Start).unwrap(), 7);
}

#[test]
fn tell_and_seek_count_in_t_units() {
    let mut cursor = typed_cursor::<i32>(&[10, 20, 30, 40]);
    assert_eq!(cursor.tell().unwrap(), 0);

    assert_eq!(cursor.pread_one(Whence::Start).unwrap(), 10);
    assert_eq!(cursor.tell().unwrap(), 1, "one i32 in");
    assert_eq!(cursor.byte_tell().unwrap(), 4, "four bytes in");

    assert_eq!(cursor.seek(2, Whence::Start).unwrap(), 2);
    assert_eq!(cursor.byte_tell().unwrap(), 8);
    assert_eq!(cursor.pread_one(Whence::Current).unwrap(), 30);

    // End- and Current-relative seeks are in T units too.
    assert_eq!(cursor.seek(-1, Whence::End).unwrap(), 3);
    assert_eq!(cursor.pread_one(Whence::Current).unwrap(), 40);
    // `size` is the *remaining* T values — none, now at the end.
    assert_eq!(cursor.size().unwrap(), 0);
    assert_eq!(cursor.seek(0, Whence::Start).unwrap(), 0);
    assert_eq!(cursor.size().unwrap(), 4, "4 i32 remaining from the start");
}

#[test]
fn negative_typed_seek_before_start_is_rejected() {
    let mut cursor = typed_cursor::<i32>(&[1, 2]);
    // seek delegates to byte_seek, so the reported offset is in bytes (-1 i32 = -4).
    assert_eq!(
        cursor.seek(-1, Whence::Start).unwrap_err(),
        IoError::InvalidSeek {
            offset: -4,
            whence: Whence::Start
        }
    );
}

#[test]
fn write_past_end_fills_the_gap_with_the_type_default() {
    let mut cursor = typed_cursor::<i32>(&[]);
    cursor.pwrite_one(1, Whence::Start).unwrap();
    // Skip two i32 values, then write at T-index 3; the gap is default (zero) filled.
    cursor.seek(3, Whence::Start).unwrap();
    cursor.pwrite_one(9, Whence::Current).unwrap();

    // 4 i32 total; read them from the start (which also resets the remaining count).
    assert_eq!(cursor.seek(0, Whence::Start).unwrap(), 0);
    assert_eq!(cursor.size().unwrap(), 4);
    assert_eq!(
        cursor.pread_array(4, Whence::Start).unwrap(),
        vec![1, 0, 0, 9],
        "gap filled with the i32 default"
    );
}

#[test]
fn default_accessors_describe_the_fill() {
    let cursor = typed_cursor::<i32>(&[]);
    assert_eq!(cursor.default_value(), 0);
    assert_eq!(cursor.default_byte_array(2), vec![0u8; 8]); // two i32 defaults
}

#[test]
fn typed_arrays_truncate_on_over_request() {
    let mut cursor = typed_cursor::<i16>(&[1, 2, 3]);
    // Over-request returns only the whole values that fit.
    assert_eq!(
        cursor.pread_array(100, Whence::Start).unwrap(),
        vec![1, 2, 3]
    );
    // A single read past the end is EOF.
    cursor.seek(3, Whence::Start).unwrap();
    assert!(matches!(
        cursor.pread_one(Whence::Current).unwrap_err(),
        IoError::UnexpectedEof { needed: 2, .. }
    ));
}

#[test]
fn typed_capacity_is_in_t_units_and_grows() {
    let cursor = <TypedCursor<i32> as TypedIOBase<i32>>::with_capacity(50);
    assert!(cursor.capacity().unwrap() >= 50, "50 i32 capacity");
    assert!(cursor.byte_capacity().unwrap() >= 200);
    assert_eq!(cursor.size().unwrap(), 0);

    // Growing by writing keeps the preallocated headroom (no reallocation needed).
    let mut cursor = <TypedCursor<i64> as TypedIOBase<i64>>::with_capacity(128);
    cursor.pwrite_one(1, Whence::Start).unwrap(); // triggers copy-on-write
    assert!(cursor.capacity().unwrap() >= 128);
    assert_eq!(cursor.size().unwrap(), 0, "at the end after the write");
    assert_eq!(cursor.seek(0, Whence::Start).unwrap(), 0);
    assert_eq!(cursor.size().unwrap(), 1, "one i64 total");
}

#[test]
fn write_is_copy_on_write_leaving_the_buffer_intact() {
    // A shared source ByteBuffer of three i32; the cursor's write must not touch it.
    let mut bytes = Vec::new();
    for value in [1_i32, 2, 3] {
        value.write_le(&mut bytes);
    }
    let source = ByteBuffer::from_vec(bytes);
    let mut cursor = TypedCursor::<i32>::new(source.clone());
    cursor.pwrite_array(&[9, 9], Whence::Start).unwrap();
    assert_eq!(cursor.pread_array(3, Whence::Start).unwrap(), vec![9, 9, 3]);
    assert_eq!(
        source.as_bytes(),
        [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0],
        "source buffer untouched"
    );
}

#[test]
fn from_byte_cursor_shares_position() {
    let byte_cursor = ByteBuffer::from_bytes(&[1, 0, 2, 0, 3, 0]).byte_cursor();
    let mut typed = TypedCursor::<i16>::from_byte_cursor(byte_cursor);
    assert_eq!(typed.pread_array(3, Whence::Start).unwrap(), vec![1, 2, 3]);
    // IOCursor byte position is preserved on the typed wrapper.
    typed.set_position(2);
    assert_eq!(typed.position(), 2);
    assert_eq!(typed.pread_one(Whence::Current).unwrap(), 2);
}
