//! Tests for the `std::io::{Read, Write, Seek}` impls on `dyn IOBase` — a cursor drives
//! and is driven by the standard streaming ecosystem with no wrapper type.

use std::io::{self, Read, Seek, SeekFrom, Write};

use yggdryl_buffer::{ByteBuffer, IOBase};

#[test]
fn dyn_iobase_reads_sequentially() {
    let mut cursor = ByteBuffer::from_bytes(b"hello world").byte_cursor();
    let io: &mut dyn IOBase = &mut cursor;

    let mut buf = Vec::new();
    assert_eq!(io.read_to_end(&mut buf).unwrap(), 11);
    assert_eq!(buf, b"hello world");
    // A read at EOF yields 0 (no error), so `read_to_end` terminates.
    assert_eq!(io.read(&mut [0u8; 4]).unwrap(), 0);
}

#[test]
fn dyn_iobase_writes_and_advances() {
    let mut cursor = ByteBuffer::new().byte_cursor();
    {
        let io: &mut dyn IOBase = &mut cursor;
        io.write_all(b"abc").unwrap();
        io.write_all(b"def").unwrap();
        io.flush().unwrap();
    }
    assert_eq!(cursor.as_bytes(), b"abcdef");
    assert_eq!(cursor.byte_tell().unwrap(), 6);
}

#[test]
fn dyn_iobase_seeks_like_std_cursor() {
    let mut cursor = ByteBuffer::from_bytes(b"0123456789").byte_cursor();
    let io: &mut dyn IOBase = &mut cursor;

    assert_eq!(io.seek(SeekFrom::Start(3)).unwrap(), 3);
    assert_eq!(io.stream_position().unwrap(), 3);

    let mut two = [0u8; 2];
    io.read_exact(&mut two).unwrap();
    assert_eq!(&two, b"34"); // read advanced from position 3
    assert_eq!(io.stream_position().unwrap(), 5);

    assert_eq!(io.seek(SeekFrom::Current(-1)).unwrap(), 4);
    assert_eq!(io.seek(SeekFrom::End(-2)).unwrap(), 8); // 2 back from the 10-byte end
    io.read_exact(&mut two).unwrap();
    assert_eq!(&two, b"89");
}

#[test]
fn seek_before_start_errors() {
    let mut cursor = ByteBuffer::from_bytes(b"abc").byte_cursor();
    let io: &mut dyn IOBase = &mut cursor;
    // Seeking to a negative absolute position is an error, matching `std::io::Cursor`.
    let err = io.seek(SeekFrom::Current(-1)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    // A `Start` offset beyond `i64::MAX` is rejected rather than silently wrapping.
    assert!(io.seek(SeekFrom::Start(u64::MAX)).is_err());
}

#[test]
fn seek_past_end_then_write_zero_fills() {
    // Seeking past the end is allowed; a subsequent write zero-fills the gap (as
    // `std::io::Cursor` does).
    let mut cursor = ByteBuffer::from_bytes(b"ab").byte_cursor();
    {
        let io: &mut dyn IOBase = &mut cursor;
        assert_eq!(io.seek(SeekFrom::Start(5)).unwrap(), 5);
        io.write_all(b"Z").unwrap();
    }
    assert_eq!(cursor.as_bytes(), b"ab\0\0\0Z");
}

#[test]
fn io_copy_between_two_cursors() {
    let mut source = ByteBuffer::from_bytes(&b"stream me ".repeat(50)).byte_cursor();
    let mut sink = ByteBuffer::new().byte_cursor();
    let copied = {
        let src: &mut dyn IOBase = &mut source;
        let dst: &mut dyn IOBase = &mut sink;
        io::copy(src, dst).unwrap()
    };
    assert_eq!(copied, 500);
    assert_eq!(sink.as_bytes(), b"stream me ".repeat(50).as_slice());
}

#[test]
fn write_then_seek_zero_then_read_round_trips() {
    let mut cursor = ByteBuffer::new().byte_cursor();
    let io: &mut dyn IOBase = &mut cursor;
    io.write_all(b"round trip").unwrap();
    io.seek(SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    io.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, b"round trip");
}

#[test]
fn bounded_slice_reads_only_its_window() {
    // A `ByteSlice` is also an `IOBase`, so the same `Read` impl bounds reads to the
    // slice window.
    let buffer = ByteBuffer::from_bytes(b"abcdefghij");
    let mut slice = buffer.byte_slice(2, 4); // "cdef"
    let io: &mut dyn IOBase = &mut slice;
    let mut buf = Vec::new();
    assert_eq!(io.read_to_end(&mut buf).unwrap(), 4);
    assert_eq!(buf, b"cdef");
}
