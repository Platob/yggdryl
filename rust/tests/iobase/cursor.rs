//! The one place a cursor is retained, over any handle.

use std::io::{Read, Seek, SeekFrom, Write};

use super::counting::Counting;
use yggdryl::holder::Buffer;
use yggdryl::{IOBase, IOCursor};

#[test]
fn a_cursor_advances_and_seeks_over_any_handle() {
    let mut cursor = Buffer::new().cursor();
    assert_eq!(cursor.tell(), 0);

    // Writes advance; a later write continues where the first ended.
    assert_eq!(cursor.write_next(b"symbol,").unwrap(), 7);
    assert_eq!(cursor.write_next(b"price\n").unwrap(), 6);
    assert_eq!(cursor.tell(), 13);
    assert_eq!(cursor.size(), 13);

    // Seeks speak all three origins, and reads advance from the target.
    assert_eq!(IOCursor::seek(&mut cursor, SeekFrom::Start(7)).unwrap(), 7);
    let mut word = [0_u8; 5];
    assert_eq!(cursor.read_next(&mut word).unwrap(), 5);
    assert_eq!(&word, b"price");
    assert_eq!(
        IOCursor::seek(&mut cursor, SeekFrom::Current(-5)).unwrap(),
        7
    );
    assert_eq!(IOCursor::seek(&mut cursor, SeekFrom::End(-6)).unwrap(), 7);
    assert!(IOCursor::seek(&mut cursor, SeekFrom::Current(-100)).is_err());

    // Reading past the end is emptiness, exactly as pread spells it.
    cursor.seek_to(100);
    assert_eq!(cursor.read_next(&mut word).unwrap(), 0);
}

#[test]
fn the_standard_io_traits_ride_the_same_position() {
    let mut cursor = Buffer::new().cursor();
    cursor.write_all(b"symbol,price\n").unwrap();
    Seek::seek(&mut cursor, SeekFrom::Start(0)).unwrap();

    let mut text = String::new();
    cursor.read_to_string(&mut text).unwrap();
    assert_eq!(text, "symbol,price\n");
    assert_eq!(cursor.tell(), 13);
}

#[test]
fn a_cursor_is_the_handle_it_wraps() {
    let mut cursor = Buffer::new().cursor();
    cursor.write_next(b"AAPL").unwrap();

    // The IOBase surface answers unchanged: positional reads ignore the
    // cursor's position, exactly as a second pread caller would.
    assert_eq!(cursor.read_all_bytes().unwrap(), b"AAPL");
    assert_eq!(cursor.tell(), 4);
}

#[test]
fn a_cursor_stream_bypasses_the_wrapped_page_cache() {
    let handle = Buffer::from_bytes(vec![0xA5; 256 * 1024]).buffered(
        yggdryl::holder::buffered::BufferedOptions::default()
            .with_page_size(4 * 1024)
            .with_max_bytes(32 * 1024),
    );
    let mut cursor = handle.cursor_at(17);
    let total: usize = cursor
        .stream_bytes(4 * 1024)
        .unwrap()
        .map(|chunk| chunk.unwrap().len())
        .sum();

    assert_eq!(total, 256 * 1024 - 17);
    assert_eq!(cursor.handle().cached_pages(), 0);
    assert_eq!(cursor.tell(), 256 * 1024);
}

#[test]
fn a_cursor_stream_keeps_one_compression_decoder_alive() {
    let plain = b"symbol,price\nAAPL,1\n".repeat(4 * 1024);
    let encoded = yggdryl::coding::gzip::dump(&plain).unwrap();
    let handle =
        yggdryl::coding::Coding::new(Counting::from_bytes(encoded.clone()), yggdryl::Codec::Gzip);
    let mut cursor = handle.cursor();
    let decoded = cursor
        .stream_bytes(31)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
        .concat();

    assert_eq!(decoded, plain);
    assert_eq!(cursor.tell(), plain.len() as u64);
    assert_eq!(cursor.handle().handle().sizes(), 0);
    assert!(
        cursor.handle().handle().reads() <= encoded.len().div_ceil(31) + 2,
        "the cursor rebuilt its decoder instead of advancing one encoded stream"
    );
    assert!(!cursor.handle().opened());
}
