//! Tests for the [`BitIo`] bit-addressed layer.

use yggdryl_core::{BitIo, Buffer, ByteSlice, IoError};

#[test]
fn reads_bits_lsb_first() {
    // byte 0 = 0b1010_0101, byte 1 = 0b0000_0010.
    let buf = Buffer::from_vec(vec![0b1010_0101, 0b0000_0010]);
    assert!(buf.read_bit(0).unwrap()); // bit 0 of byte 0 = 1
    assert!(!buf.read_bit(1).unwrap()); // bit 1 = 0
    assert!(buf.read_bit(2).unwrap()); // bit 2 = 1
    assert!(buf.read_bit(7).unwrap()); // bit 7 = 1
    assert!(buf.read_bit(9).unwrap()); // bit 1 of byte 1 = 1
                                       // A bit whose byte is past the end errors.
    assert_eq!(buf.read_bit(16), Err(IoError::OutOfBounds));
}

#[test]
fn writes_a_bit_in_place() {
    let mut buf = Buffer::from_vec(vec![0b0000_0000]);
    buf.write_bit(0, true).unwrap();
    buf.write_bit(3, true).unwrap();
    assert_eq!(buf.as_slice(), &[0b0000_1001]);
    // Clearing leaves the other bits untouched.
    buf.write_bit(0, false).unwrap();
    assert_eq!(buf.as_slice(), &[0b0000_1000]);
}

#[test]
fn bits_work_through_a_slice() {
    let buf = Buffer::from_vec(vec![0x00, 0b0000_0001]);
    let tail = ByteSlice::new(buf, 1, 2).unwrap();
    // Bit 0 of the slice is bit 0 of byte 1 of the buffer.
    assert!(tail.read_bit(0).unwrap());
    assert!(!tail.read_bit(1).unwrap());
}
