//! Tests for the concrete `ByteBuffer` and `BitBuffer` resources.

use yggdryl_core::{BitBuffer, ByteBuffer, IOError, RawIOBase, Seekable, Whence};

#[test]
fn byte_buffer_round_trips_and_appends() {
    let mut buf = ByteBuffer::new();
    buf.pwrite_byte_array(0, Whence::Start, &[1, 2, 3]).unwrap();
    buf.pwrite_byte_array(0, Whence::End, &[4, 5]).unwrap(); // append
    assert_eq!(
        buf.pread_byte_array(0, Whence::Start, 5).unwrap(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(buf.as_bytes(), &[1, 2, 3, 4, 5]);
    assert_eq!(buf.byte_size(), 5);
    assert_eq!(buf.bit_size(), 40); // default: byte_size * 8
}

#[test]
fn byte_buffer_bit_access_is_msb_first() {
    let mut buf = ByteBuffer::from_bytes(vec![0b1010_0000]);
    assert!(buf.pread_bit_one(0, Whence::Start).unwrap());
    assert!(!buf.pread_bit_one(1, Whence::Start).unwrap());
    buf.pwrite_bit_one(1, Whence::Start, true).unwrap();
    assert_eq!(buf.pread_byte_one(0, Whence::Start).unwrap(), 0b1110_0000);
}

#[test]
fn byte_buffer_seek_and_current_relative_read() {
    let mut buf = ByteBuffer::from_bytes(vec![10, 20, 30, 40]);
    assert_eq!(buf.seek(2, Whence::Start).unwrap(), 2);
    assert_eq!(buf.tell(), 2);
    // Current + 1 == absolute byte 3 == 40.
    assert_eq!(buf.pread_byte_one(1, Whence::Current).unwrap(), 40);
}

#[test]
fn byte_buffer_out_of_bounds_read_errors() {
    let buf = ByteBuffer::from_bytes(vec![1, 2]);
    let error = buf.pread_byte_array(0, Whence::Start, 3).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { offset: 3, len: 2 }));
}

#[test]
fn bit_buffer_tracks_an_exact_bit_length() {
    let mut buf = BitBuffer::new();
    buf.pwrite_bit_array(0, Whence::Start, &[true, false, true])
        .unwrap();
    assert_eq!(buf.bit_size(), 3);
    assert_eq!(buf.byte_size(), 1); // three bits round up to one byte
    assert_eq!(
        buf.pread_bit_array(0, Whence::Start, 3).unwrap(),
        vec![true, false, true]
    );
    // MSB-first: bits 0 and 2 set => 0b1010_0000.
    assert_eq!(buf.pread_byte_one(0, Whence::Start).unwrap(), 0b1010_0000);
}

#[test]
fn bit_buffer_appends_bits_via_end() {
    let mut buf = BitBuffer::new();
    buf.pwrite_bit_array(0, Whence::Start, &[true]).unwrap();
    buf.pwrite_bit_array(0, Whence::End, &[true]).unwrap(); // append one bit
    assert_eq!(buf.bit_size(), 2);
    assert_eq!(
        buf.pread_bit_array(0, Whence::Start, 2).unwrap(),
        vec![true, true]
    );
}

#[test]
fn bit_buffer_byte_writes_extend_the_bit_length() {
    let mut buf = BitBuffer::from_bytes(vec![1, 2]);
    assert_eq!(buf.bit_size(), 16);
    buf.pwrite_byte_array(2, Whence::Start, &[3]).unwrap();
    assert_eq!(buf.byte_size(), 3);
    assert_eq!(buf.bit_size(), 24);
    assert_eq!(buf.as_bytes(), &[1, 2, 3]);
}

#[test]
fn bit_buffer_out_of_bounds_bit_read_errors() {
    let buf = BitBuffer::from_bytes(vec![0xFF]); // 8 bits
    let error = buf.pread_bit_array(0, Whence::Start, 9).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { offset: 9, len: 8 }));
}
