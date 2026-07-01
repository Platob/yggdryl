//! Tests for the [`BitSlice`] bounded bit-window.

use yggdryl_core::{BitIo, BitSlice, Buffer, IoError};

#[test]
fn windows_and_reads_relative_bits() {
    // byte 0 = 0b1010_0101, byte 1 = 0b0000_0010.
    let buf = Buffer::from_vec(vec![0b1010_0101, 0b0000_0010]);
    let win = BitSlice::new(buf, 4, 12).unwrap(); // bits 4..12
    assert_eq!(win.bit_len(), 8);
    assert_eq!(win.start(), 4);
    assert_eq!(win.end(), 12);
    // Bit 0 of the window is bit 4 of the buffer.
    assert!(!win.read_bit(0).unwrap()); // bit 4 = 0
    assert!(win.read_bit(1).unwrap()); // bit 5 = 1
    assert!(win.read_bit(3).unwrap()); // bit 7 = 1
    assert!(win.read_bit(5).unwrap()); // bit 1 of byte 1 = 1
}

#[test]
fn reads_are_clamped_to_the_window() {
    let buf = Buffer::from_vec(vec![0xFF, 0xFF]);
    let win = BitSlice::new(buf, 2, 6).unwrap(); // bits 2..6 (4 bits)
    assert!(win.read_bit(3).unwrap());
    // A bit at or past the window end errors.
    assert_eq!(win.read_bit(4), Err(IoError::OutOfBounds));
}

#[test]
fn writes_stay_within_the_window() {
    let buf = Buffer::from_vec(vec![0b0000_0000]);
    let mut win = BitSlice::new(buf, 2, 6).unwrap(); // bits 2..6
    win.write_bit(0, true).unwrap(); // sets bit 2 of the buffer
    win.write_bit(3, true).unwrap(); // sets bit 5 of the buffer
                                     // A write past the window end is rejected and changes nothing.
    assert_eq!(win.write_bit(4, true), Err(IoError::OutOfBounds));
    assert_eq!(win.into_inner().as_slice(), &[0b0010_0100]);
}

#[test]
fn rejects_out_of_range_bounds() {
    let buf = Buffer::from_vec(vec![0xAB]); // 8 bits
    assert_eq!(
        BitSlice::new(buf.clone(), 5, 2).err(),
        Some(IoError::OutOfBounds)
    );
    assert_eq!(BitSlice::new(buf, 0, 9).err(), Some(IoError::OutOfBounds));
}
