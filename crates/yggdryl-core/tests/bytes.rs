//! Tests for the [`Bytes`] byte-serialization trait.

use yggdryl_core::{Bytes, IoError, Whence};

#[test]
fn integers_round_trip_little_endian() {
    assert_eq!(0x12u8.to_bytes(), vec![0x12]);
    assert_eq!(0x1122u16.to_bytes(), vec![0x22, 0x11]);
    assert_eq!(0x11223344u32.to_bytes(), vec![0x44, 0x33, 0x22, 0x11]);

    assert_eq!(u8::from_bytes(&[0x12]).unwrap(), 0x12);
    assert_eq!(u16::from_bytes(&[0x22, 0x11]).unwrap(), 0x1122);
    assert_eq!(
        u64::from_bytes(&0x0102030405060708u64.to_bytes()).unwrap(),
        0x0102030405060708
    );
}

#[test]
fn values_compose_sequentially_through_a_byte_io() {
    // Serialize two values by appending each to the end of a byte Io.
    let mut io: Vec<u8> = Vec::new();
    let a: u16 = 0x1122;
    let b: u32 = 0x33445566;
    assert_eq!(a.pwrite_bytes(&mut io, 0, Whence::End).unwrap(), 2);
    assert_eq!(b.pwrite_bytes(&mut io, 0, Whence::End).unwrap(), 4);
    assert_eq!(io, vec![0x22, 0x11, 0x66, 0x55, 0x44, 0x33]);

    // Read them back, advancing by the reported byte count.
    let (ra, used) = u16::pread_bytes(&io, 0, Whence::Start).unwrap();
    let (rb, _) = u32::pread_bytes(&io, used, Whence::Start).unwrap();
    assert_eq!((ra, rb), (a, b));
}

#[test]
fn truncated_input_errors() {
    assert_eq!(u32::from_bytes(&[1, 2]), Err(IoError::OutOfBounds));
    assert!(u64::from_bytes(&[]).is_err());
}
