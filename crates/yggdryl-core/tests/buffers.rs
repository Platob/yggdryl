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

#[test]
fn byte_buffer_capacity_tracks_the_allocation() {
    let mut buf = ByteBuffer::from_bytes(vec![1, 2, 3]);
    assert!(buf.byte_capacity() >= 3);
    assert_eq!(buf.bit_capacity(), buf.byte_capacity() * 8);

    let grown = buf.resize_byte_capacity(64).unwrap();
    assert!(grown >= 64);
    assert_eq!(buf.byte_size(), 3); // capacity never changes the size

    let shrunk = buf.resize_byte_capacity(0).unwrap();
    assert!((3..64).contains(&shrunk)); // shrinks toward the length
    assert_eq!(buf.as_bytes(), &[1, 2, 3]);

    assert!(buf.resize_bit_capacity(100).unwrap() >= 104); // 13 bytes, in bits
}

#[test]
fn byte_buffer_resize_truncates_and_zero_fills() {
    let mut buf = ByteBuffer::from_bytes(vec![1, 2, 3]);
    buf.resize_bytes(5).unwrap();
    assert_eq!(buf.as_bytes(), &[1, 2, 3, 0, 0]);
    buf.resize_bytes(1).unwrap();
    assert_eq!(buf.as_bytes(), &[1]);
    // Byte-granular: bit resizes round up to whole bytes.
    buf.resize_bits(9).unwrap();
    assert_eq!((buf.byte_size(), buf.bit_size()), (2, 16));
}

#[test]
fn bit_buffer_resize_bits_is_exact() {
    let mut buf = BitBuffer::from_bytes(vec![0xFF]);
    buf.resize_bits(3).unwrap();
    assert_eq!((buf.bit_size(), buf.byte_size()), (3, 1));
    buf.resize_bits(0).unwrap();
    assert_eq!((buf.bit_size(), buf.byte_size()), (0, 0));
    buf.resize_bytes(2).unwrap();
    assert_eq!((buf.bit_size(), buf.byte_size()), (16, 2));
}

#[test]
fn unaligned_bit_round_trips_survive_the_packed_fast_path() {
    // Start at bit 3 with 13 bits: exercises head, packed body, and tail paths.
    let pattern: Vec<bool> = (0..13).map(|i| i % 3 == 0).collect();
    let mut buf = BitBuffer::new();
    buf.resize_bits(16).unwrap();
    buf.pwrite_bit_array(3, Whence::Start, &pattern).unwrap();
    assert_eq!(buf.pread_bit_array(3, Whence::Start, 13).unwrap(), pattern);
    // Neighbouring bits stay untouched.
    assert_eq!(
        buf.pread_bit_array(0, Whence::Start, 3).unwrap(),
        vec![false; 3]
    );
}

#[test]
fn stream_copy_across_buffer_types() {
    let source = ByteBuffer::from_bytes(vec![1, 2, 3, 4]);
    let mut sink = BitBuffer::new();
    source
        .pread_io(1, Whence::Start, 3, &mut sink, 0, Whence::Start)
        .unwrap();
    assert_eq!(sink.as_bytes(), &[2, 3, 4]);

    let mut back = ByteBuffer::new();
    back.pwrite_io(0, Whence::Start, &sink, 0, Whence::Start, 3)
        .unwrap();
    assert_eq!(back.as_bytes(), &[2, 3, 4]);
}

#[test]
fn stream_copy_larger_than_one_chunk() {
    // Three 64 KiB chunks plus a remainder.
    let payload: Vec<u8> = (0..200_000usize).map(|i| (i % 251) as u8).collect();
    let source = ByteBuffer::from_bytes(payload.clone());
    let mut sink = ByteBuffer::new();
    source
        .pread_io(0, Whence::Start, payload.len(), &mut sink, 0, Whence::Start)
        .unwrap();
    assert_eq!(sink.as_bytes(), payload.as_slice());
}

#[test]
fn bit_buffer_truncation_zeroes_padding_and_does_not_resurrect_bits() {
    // Regression: truncating to a non-byte-aligned size must zero the dropped bits.
    let mut buf = BitBuffer::from_bytes(vec![0xFF]); // 8 set bits
    buf.resize_bits(3).unwrap();
    assert_eq!(buf.as_bytes(), &[0b1110_0000]); // padding zeroed
    buf.resize_bits(8).unwrap(); // grow back — the resurrected bits must be zero
    assert_eq!(
        buf.pread_bit_array(0, Whence::Start, 8).unwrap(),
        vec![true, true, true, false, false, false, false, false]
    );
    // Two logically-equal 3-bit buffers compare equal (backing bytes match).
    let mut other = BitBuffer::new();
    other
        .pwrite_bit_array(0, Whence::Start, &[true, true, true])
        .unwrap();
    let mut trunc = BitBuffer::from_bytes(vec![0xFF]);
    trunc.resize_bits(3).unwrap();
    assert_eq!(trunc, other);
}

#[test]
fn empty_write_is_a_no_op_even_past_the_end() {
    let mut byte = ByteBuffer::new();
    byte.pwrite_byte_array(100, Whence::Start, &[]).unwrap();
    assert_eq!(byte.byte_size(), 0);
    byte.pwrite_bit_array(100, Whence::Start, &[]).unwrap();
    assert_eq!(byte.byte_size(), 0);

    let mut bit = BitBuffer::new();
    bit.pwrite_bit_array(100, Whence::Start, &[]).unwrap();
    assert_eq!(bit.bit_size(), 0);
}

#[test]
fn offset_overflow_errors_instead_of_wrapping() {
    let mut buf = ByteBuffer::from_bytes(vec![1, 2, 3]);
    buf.seek(usize::MAX, Whence::Start).unwrap(); // seek is unbounded
    let error = buf.pread_byte_one(1, Whence::Current).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
}

#[test]
fn stream_append_via_end_stays_anchored_while_growing() {
    let source = ByteBuffer::from_bytes((0..200_000u32).map(|i| (i % 251) as u8).collect());
    let mut sink = ByteBuffer::from_bytes(vec![9, 9]);
    // End is resolved once (to 2) before the chunked copy starts growing the sink.
    source
        .pread_io(0, Whence::Start, 200_000, &mut sink, 0, Whence::End)
        .unwrap();
    assert_eq!(sink.byte_size(), 200_002);
    assert_eq!(&sink.as_bytes()[..2], &[9, 9]);
    assert_eq!(&sink.as_bytes()[2..], source.as_bytes());
}
