//! Tests for the [`ByteSlice`] bounded window.

use yggdryl_core::{Buffer, ByteIo, ByteSlice, IoError};

#[test]
fn windows_and_reads_zero_copy() {
    let buf = Buffer::from_vec(b"hello world".to_vec());
    let world = ByteSlice::new(buf.clone(), 6, 11).unwrap();
    assert_eq!(world.byte_len().unwrap(), 5);
    assert_eq!(world.start(), 6);
    assert_eq!(world.end(), 11);
    let read = world.positional_read_bytes(0, 5).unwrap();
    assert_eq!(read.as_slice(), b"world");
    // Zero-copy: shares the original allocation.
    assert_eq!(read.as_slice().as_ptr(), buf.as_slice()[6..].as_ptr());
}

#[test]
fn reads_are_clamped_to_the_window() {
    let buf = Buffer::from_vec(b"0123456789".to_vec());
    let mid = ByteSlice::new(buf, 3, 7).unwrap(); // covers "3456"
                                                  // Asking past the window end returns only what is inside it.
    assert_eq!(mid.positional_read_bytes(2, 10).unwrap().as_slice(), b"56");
    // An offset past the window errors.
    assert_eq!(mid.positional_read_bytes(5, 1), Err(IoError::OutOfBounds));
}

#[test]
fn writes_stay_within_the_window() {
    let buf = Buffer::from_vec(b"0123456789".to_vec());
    let mut mid = ByteSlice::new(buf, 3, 7).unwrap(); // covers "3456"
                                                      // Writing more than fits is clamped to the window (2 of 4 bytes land).
    assert_eq!(mid.positional_write_bytes(2, b"ABCD").unwrap(), 2);
    // Only the two in-window bytes reached the inner io.
    assert_eq!(mid.into_inner().as_slice(), b"01234AB789");
}

#[test]
fn rejects_out_of_range_bounds() {
    let buf = Buffer::from_vec(b"abc".to_vec());
    assert_eq!(
        ByteSlice::new(buf.clone(), 2, 1).err(),
        Some(IoError::OutOfBounds)
    );
    assert_eq!(ByteSlice::new(buf, 0, 4).err(), Some(IoError::OutOfBounds));
}
