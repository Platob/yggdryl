//! Exhaustive per-type tests for the typed cursor surface, driven through
//! `ByteCursor`. Bit access lives on the (separate) bit buffer, not here.

use yggdryl_buffer::{ByteBuffer, IOBase, IoError, Whence};

#[test]
fn every_primitive_round_trips() {
    let mut c = ByteBuffer::new().byte_cursor();
    c.pwrite_i8(-8, Whence::Start).unwrap();
    c.pwrite_u8(200, Whence::Current).unwrap();
    c.pwrite_i16(-1234, Whence::Current).unwrap();
    c.pwrite_u16(0xBEEF, Whence::Current).unwrap();
    c.pwrite_i32(-123_456, Whence::Current).unwrap();
    c.pwrite_u32(0xDEAD_BEEF, Whence::Current).unwrap();
    c.pwrite_i64(-1, Whence::Current).unwrap();
    c.pwrite_u64(0xDEAD_BEEF_CAFE_F00D, Whence::Current)
        .unwrap();
    c.pwrite_f32(1.5, Whence::Current).unwrap();
    c.pwrite_f64(-2.5, Whence::Current).unwrap();

    c.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(c.pread_i8(Whence::Current).unwrap(), -8);
    assert_eq!(c.pread_u8(Whence::Current).unwrap(), 200);
    assert_eq!(c.pread_i16(Whence::Current).unwrap(), -1234);
    assert_eq!(c.pread_u16(Whence::Current).unwrap(), 0xBEEF);
    assert_eq!(c.pread_i32(Whence::Current).unwrap(), -123_456);
    assert_eq!(c.pread_u32(Whence::Current).unwrap(), 0xDEAD_BEEF);
    assert_eq!(c.pread_i64(Whence::Current).unwrap(), -1);
    assert_eq!(c.pread_u64(Whence::Current).unwrap(), 0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(c.pread_f32(Whence::Current).unwrap(), 1.5);
    assert_eq!(c.pread_f64(Whence::Current).unwrap(), -2.5);
}

#[test]
fn typed_arrays_round_trip_and_truncate() {
    let mut c = ByteBuffer::new().byte_cursor();
    let values = [1i64, -2, 3, -4, 5];
    assert_eq!(c.pwrite_i64_array(&values, Whence::Start).unwrap(), 5);

    c.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(c.pread_i64_array(5, Whence::Current).unwrap(), values);

    // Over-request returns only the whole values that fit.
    c.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(c.pread_i64_array(100, Whence::Current).unwrap(), values);
}

#[test]
fn float_edge_values() {
    let mut c = ByteBuffer::new().byte_cursor();
    c.pwrite_f32_array(&[f32::NEG_INFINITY, 0.0, 1.5], Whence::Start)
        .unwrap();
    c.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(
        c.pread_f32_array(3, Whence::Current).unwrap(),
        vec![f32::NEG_INFINITY, 0.0, 1.5]
    );
}

#[test]
fn typed_write_is_little_endian() {
    let mut c = ByteBuffer::new().byte_cursor();
    c.pwrite_i32(0x0102_0304, Whence::Start).unwrap();
    assert_eq!(
        c.pread_byte_array(4, Whence::Start).unwrap(),
        [0x04, 0x03, 0x02, 0x01]
    );
}

#[test]
fn pwrite_io_drains_source() {
    let mut source = ByteBuffer::from_bytes(b"XYZ").byte_cursor();
    let mut dest = ByteBuffer::from_bytes(b"..........").byte_cursor();
    let n = dest.pwrite_io(&mut source, 3, Whence::Start).unwrap();
    assert_eq!(n, 3);
    assert_eq!(
        dest.pread_byte_array(10, Whence::Start).unwrap(),
        b"XYZ......."
    );
}

#[test]
fn transfer_large_payload() {
    let payload: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    let mut source = ByteBuffer::from_bytes(&payload).byte_cursor();
    let mut sink = ByteBuffer::new().byte_cursor();
    source
        .pread_io(&mut sink, payload.len(), Whence::Start)
        .unwrap();
    assert_eq!(
        sink.pread_byte_array(payload.len(), Whence::Start).unwrap(),
        payload
    );
}

#[test]
fn empty_array_ops() {
    let mut c = ByteBuffer::new().byte_cursor();
    assert_eq!(c.pwrite_i64_array(&[], Whence::Start).unwrap(), 0);
    assert!(c.pread_i64_array(0, Whence::Start).unwrap().is_empty());
    let _: Result<(), IoError> = Ok(());
}
