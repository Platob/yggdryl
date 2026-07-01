//! Tests for the [`ByteIo`] trait and its [`Buffer`] leaf implementation.

use yggdryl_core::{Buffer, ByteIo, IoError};

#[test]
fn buffer_positional_read_is_zero_copy() {
    let buf = Buffer::from_vec(b"hello world".to_vec());
    let world = buf.positional_read_bytes(6, 5).unwrap();
    assert_eq!(world.as_slice(), b"world");
    // Shares the backing allocation — same address, no copy.
    assert_eq!(world.as_slice().as_ptr(), buf.as_slice()[6..].as_ptr());
    assert_eq!(buf.byte_len().unwrap(), 11);
}

#[test]
fn buffer_read_clamps_at_eof_and_errors_past_the_end() {
    let buf = Buffer::from_vec(b"abc".to_vec());
    // More than is available returns only what's there.
    assert_eq!(buf.positional_read_bytes(1, 10).unwrap().as_slice(), b"bc");
    // Reading exactly at the end yields nothing.
    assert!(buf
        .positional_read_bytes(3, 4)
        .unwrap()
        .as_slice()
        .is_empty());
    // Starting past the end errors.
    assert_eq!(buf.positional_read_bytes(4, 1), Err(IoError::OutOfBounds));
}

#[test]
fn buffer_write_is_copy_on_write() {
    let mut buf = Buffer::from_vec(b"abc".to_vec());
    let shared = buf.clone();
    // Overwrite in place.
    assert_eq!(buf.positional_write_bytes(1, b"XY").unwrap(), 2);
    assert_eq!(buf.as_slice(), b"aXY");
    // Extend past the end.
    assert_eq!(buf.positional_write_bytes(3, b"Z").unwrap(), 1);
    assert_eq!(buf.as_slice(), b"aXYZ");
    // The earlier clone is untouched — the write copied.
    assert_eq!(shared.as_slice(), b"abc");
    // Writing with a gap past the end is rejected.
    assert_eq!(
        buf.positional_write_bytes(9, b"!"),
        Err(IoError::OutOfBounds)
    );
}
