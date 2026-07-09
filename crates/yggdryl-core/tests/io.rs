//! Tests for the cursor-oriented byte IO: `ByteBuffer` storage + `ByteCursor`.

use std::collections::HashSet;

use yggdryl_core::{ByteBuffer, ByteCursor, IOBase, IOCursor, IoError, TypedIOBase, Whence};

#[test]
fn cursor_reads_and_writes_advance() {
    let mut cursor = ByteBuffer::new().byte_cursor();
    assert_eq!(
        cursor
            .pwrite_byte_array(b"hello world", Whence::Start)
            .unwrap(),
        11
    );
    assert_eq!(cursor.byte_tell().unwrap(), 11, "write advanced the cursor");

    cursor.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(
        cursor.pread_byte_array(5, Whence::Current).unwrap(),
        b"hello"
    );
    assert_eq!(cursor.byte_tell().unwrap(), 5, "read advanced the cursor");
    assert_eq!(
        cursor.pread_byte_array(6, Whence::Current).unwrap(),
        b" world"
    );
}

#[test]
fn write_is_copy_on_write_leaving_the_buffer_intact() {
    let buffer = ByteBuffer::from_bytes(b"abcdef");
    let mut cursor = buffer.byte_cursor();
    cursor.pwrite_byte_array(b"XYZ", Whence::Start).unwrap();
    assert_eq!(buffer.as_bytes(), b"abcdef", "source buffer untouched");
    assert_eq!(
        cursor.pread_byte_array(6, Whence::Start).unwrap(),
        b"XYZdef"
    );
}

#[test]
fn seek_position_and_set_position() {
    let mut cursor = ByteBuffer::from_bytes(&[0; 10]).byte_cursor();
    assert_eq!(cursor.byte_seek(3, Whence::Start).unwrap(), 3);
    assert_eq!(cursor.byte_seek(2, Whence::Current).unwrap(), 5);
    assert_eq!(cursor.byte_seek(-1, Whence::End).unwrap(), 9);
    assert_eq!(cursor.position(), 9);
    cursor.set_position(2);
    assert_eq!(cursor.byte_tell().unwrap(), 2);
}

#[test]
fn negative_seek_is_rejected() {
    let mut cursor = ByteBuffer::new().byte_cursor();
    assert_eq!(
        cursor.byte_seek(-1, Whence::Start).unwrap_err(),
        IoError::InvalidSeek {
            offset: -1,
            whence: Whence::Start
        }
    );
}

#[test]
fn read_past_end_truncates_write_past_end_grows() {
    let mut cursor = ByteBuffer::from_bytes(b"abc").byte_cursor();
    assert_eq!(cursor.pread_byte_array(100, Whence::Start).unwrap(), b"abc");

    let mut grow = ByteBuffer::new().byte_cursor();
    grow.pwrite_byte_array(b"xy", Whence::Start).unwrap();
    grow.byte_seek(5, Whence::Start).unwrap();
    grow.pwrite_byte_array(b"z", Whence::Current).unwrap();
    assert_eq!(grow.as_bytes().len(), 6, "grew to 6 bytes total");
    assert_eq!(
        grow.byte_size().unwrap(),
        0,
        "at the end, nothing remaining"
    );
    assert_eq!(
        grow.pread_byte_array(6, Whence::Start).unwrap(),
        b"xy\0\0\0z"
    );
}

#[test]
fn typed_round_trip_and_endianness() {
    let mut cursor = ByteBuffer::new().byte_cursor();
    cursor.pwrite_i32(0x0102_0304, Whence::Start).unwrap();
    cursor.pwrite_i64(-1, Whence::Current).unwrap();
    cursor.pwrite_array(&[7u8, 8, 9], Whence::Current).unwrap();

    cursor.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(cursor.pread_i32(Whence::Current).unwrap(), 0x0102_0304);
    assert_eq!(cursor.pread_i64(Whence::Current).unwrap(), -1);
    assert_eq!(
        cursor.pread_array(3, Whence::Current).unwrap(),
        vec![7, 8, 9]
    );

    // little-endian byte layout
    let mut le = ByteBuffer::new().byte_cursor();
    le.pwrite_u16(0xBEEF, Whence::Start).unwrap();
    assert_eq!(le.pread_byte_array(2, Whence::Start).unwrap(), [0xEF, 0xBE]);
}

#[test]
fn typed_read_past_end_is_eof() {
    let mut cursor = ByteBuffer::from_bytes(&[1, 2, 3]).byte_cursor();
    assert!(matches!(
        cursor.pread_i32(Whence::Start).unwrap_err(),
        IoError::UnexpectedEof {
            needed: 4,
            available: 3
        }
    ));
}

#[test]
fn size_and_capacity() {
    let buffer = ByteBuffer::with_byte_capacity(64);
    assert!(buffer.byte_capacity() >= 64);
    assert_eq!(buffer.byte_size(), 0);

    let mut cursor = ByteBuffer::from_bytes(&[0; 10]).byte_cursor();
    assert_eq!(cursor.byte_size().unwrap(), 10);
    assert_eq!(cursor.bit_size().unwrap(), 80);
    assert_eq!(cursor.large_byte_size().unwrap(), 10u64);
    assert_eq!(TypedIOBase::<u8>::size(&cursor).unwrap(), 10);

    let sized = ByteCursor::with_byte_capacity(128);
    assert!(sized.byte_capacity().unwrap() >= 128);
    assert!(sized.bit_capacity().unwrap() >= 1024);

    // typed with_capacity is in T units
    let typed = <ByteCursor as TypedIOBase<u8>>::with_capacity(50);
    assert!(typed.capacity().unwrap() >= 50);
    let _ = &mut cursor;
}

#[test]
fn cursor_write_preserves_preallocated_capacity() {
    // A cursor over a preallocated buffer keeps that headroom after copy-on-write,
    // so filling it does not reallocate.
    let mut cursor = ByteBuffer::with_byte_capacity(1024).byte_cursor();
    cursor.pwrite_byte_array(b"x", Whence::Start).unwrap(); // triggers COW
    assert!(cursor.byte_capacity().unwrap() >= 1024);
}

#[test]
fn pread_into_zero_copy_path() {
    let mut cursor = ByteBuffer::from_bytes(b"abcdefgh").byte_cursor();
    let mut out = [0u8; 4];
    assert_eq!(cursor.pread_into(&mut out, Whence::Start).unwrap(), 4);
    assert_eq!(&out, b"abcd");
    assert_eq!(cursor.byte_tell().unwrap(), 4, "pread_into advanced");
}

#[test]
fn transfer_between_cursors() {
    let mut source = ByteBuffer::from_bytes(b"abcdef").byte_cursor();
    let mut sink = ByteBuffer::new().byte_cursor();
    let n = source.pread_io(&mut sink, 3, Whence::Start).unwrap();
    assert_eq!(n, 3);
    assert_eq!(sink.pread_byte_array(3, Whence::Start).unwrap(), b"abc");
    assert_eq!(source.byte_tell().unwrap(), 3);
}

#[test]
fn buffer_value_semantics_and_serialize() {
    let a = ByteBuffer::from_bytes(b"data");
    let b = ByteBuffer::from_bytes(b"data");
    assert_eq!(a, b);
    let set: HashSet<ByteBuffer> = [a.clone(), b, ByteBuffer::from_bytes(b"other")]
        .into_iter()
        .collect();
    assert_eq!(set.len(), 2);
    assert_eq!(a.serialize_bytes(), b"data");
    assert_eq!(ByteBuffer::deserialize_bytes(&a.serialize_bytes()), a);
}

#[test]
fn whence_resolve_and_error_display() {
    assert_eq!(Whence::End.resolve(-4, 0, 10).unwrap(), 6);
    let err = IoError::InvalidSeek {
        offset: -1,
        whence: Whence::End,
    };
    assert!(err.to_string().contains("from end"));
}

#[test]
fn cursor_size_is_remaining_not_total() {
    let mut cursor = ByteBuffer::from_bytes(&[0; 10]).byte_cursor();
    assert_eq!(
        cursor.byte_size().unwrap(),
        10,
        "all remaining at the start"
    );
    assert_eq!(cursor.bit_size().unwrap(), 80);

    cursor.byte_seek(4, Whence::Start).unwrap();
    assert_eq!(cursor.byte_size().unwrap(), 6, "6 bytes left after byte 4");
    assert_eq!(cursor.bit_size().unwrap(), 48);
    assert_eq!(TypedIOBase::<u8>::size(&cursor).unwrap(), 6);

    // A read consumes from the remaining extent.
    cursor.pread_byte_array(2, Whence::Current).unwrap();
    assert_eq!(cursor.byte_size().unwrap(), 4);

    // Seeking to (or past) the end leaves nothing remaining.
    cursor.byte_seek(0, Whence::End).unwrap();
    assert_eq!(cursor.byte_size().unwrap(), 0);
    cursor.byte_seek(5, Whence::Current).unwrap(); // past the end
    assert_eq!(cursor.byte_size().unwrap(), 0);

    // Capacity stays total, not remaining.
    let sized = ByteBuffer::with_byte_capacity(64).byte_cursor();
    assert!(sized.byte_capacity().unwrap() >= 64);
}

#[test]
fn byte_buffer_typed_cursor_over_any_primitive() {
    let mut cursor = ByteBuffer::new().cursor::<i32>();
    cursor.pwrite_array(&[1, -2, 3], Whence::Start).unwrap();
    assert_eq!(
        cursor.pread_array(3, Whence::Start).unwrap(),
        vec![1, -2, 3]
    );
}

#[test]
fn bit_position_mirrors_byte_position() {
    let mut cursor = ByteBuffer::from_bytes(&[0; 10]).byte_cursor();
    assert_eq!(cursor.bit_tell().unwrap(), 0);

    // Byte-aligned bit seeks resolve to the matching byte position.
    assert_eq!(cursor.bit_seek(16, Whence::Start).unwrap(), 16);
    assert_eq!(cursor.byte_tell().unwrap(), 2);
    assert_eq!(cursor.bit_tell().unwrap(), 16);

    assert_eq!(cursor.bit_seek(-8, Whence::Current).unwrap(), 8);
    assert_eq!(cursor.byte_tell().unwrap(), 1);
    assert_eq!(cursor.bit_seek(0, Whence::End).unwrap(), 80); // 10 bytes * 8
    assert_eq!(cursor.byte_tell().unwrap(), 10);
}

#[test]
fn unaligned_bit_seek_is_rejected_with_guidance() {
    let mut cursor = ByteBuffer::from_bytes(&[0; 4]).byte_cursor();
    let err = cursor.bit_seek(17, Whence::Start).unwrap_err();
    assert_eq!(err, IoError::UnalignedBitSeek { offset: 17 });
    let message = err.to_string();
    assert!(message.contains("17"), "names the offending offset");
    assert!(
        message.contains("byte-aligned") || message.contains("multiple of 8"),
        "guides the fix: {message}"
    );
    // A negative bit seek before the start is a plain invalid seek (bit_seek
    // delegates to byte_seek, so the reported offset is in bytes: -8 bits = -1 byte).
    assert_eq!(
        cursor.bit_seek(-8, Whence::Start).unwrap_err(),
        IoError::InvalidSeek {
            offset: -1,
            whence: Whence::Start
        }
    );
}

mod arrow_interop {
    use yggdryl_core::arrow_buffer::Buffer;
    use yggdryl_core::{ByteBuffer, IOBase, Whence};

    #[test]
    fn from_arrow_round_trips_and_reads() {
        let arrow = Buffer::from_vec(b"arrow payload".to_vec());
        let buffer = ByteBuffer::from_arrow_byte_buffer(arrow);
        assert_eq!(buffer.as_bytes(), b"arrow payload");
        assert_eq!(buffer.byte_size(), 13);
        let mut cursor = buffer.byte_cursor();
        assert_eq!(cursor.pread_byte_array(5, Whence::Start).unwrap(), b"arrow");
    }

    #[test]
    fn sliced_and_empty_arrow_buffers() {
        let arrow = Buffer::from_vec(b"0123456789".to_vec());
        let sliced = arrow.slice_with_length(3, 4); // "3456" — offset view
        assert_eq!(
            ByteBuffer::from_arrow_byte_buffer(sliced).as_bytes(),
            b"3456"
        );
        assert!(ByteBuffer::from_arrow_byte_buffer(Buffer::from_vec(Vec::<u8>::new())).is_empty());
    }

    #[test]
    fn to_arrow_round_trips() {
        let arrow = ByteBuffer::from_bytes(b"owned").to_arrow_byte_buffer();
        assert_eq!(arrow.as_slice(), b"owned");
        assert_eq!(
            ByteBuffer::from_arrow_byte_buffer(arrow).as_bytes(),
            b"owned"
        );
    }

    #[test]
    fn cursor_write_over_arrow_is_copy_on_write() {
        let arrow = Buffer::from_vec(b"abcdef".to_vec());
        let buffer = ByteBuffer::from_arrow_byte_buffer(arrow.clone());
        let mut cursor = buffer.byte_cursor();
        cursor.pwrite_byte_array(b"XYZ", Whence::Start).unwrap();
        assert_eq!(cursor.as_bytes(), b"XYZdef");
        assert_eq!(arrow.as_slice(), b"abcdef", "arrow allocation intact");
        assert_eq!(buffer.as_bytes(), b"abcdef", "ByteBuffer intact");
    }

    #[test]
    fn from_arrow_bit_buffer_wraps_packed_bits() {
        let arrow = Buffer::from_vec(vec![0b1010_0101u8]);
        assert_eq!(
            ByteBuffer::from_arrow_bit_buffer(arrow).as_bytes(),
            &[0b1010_0101]
        );
    }
}
