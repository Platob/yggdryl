use std::io::{Read, Seek, SeekFrom, Write};

use super::{Buffer, IOBase};
use crate::IOCursor;
use crate::holder::buffered::BufferedOptions;
use crate::holder::buffered::tests::Counting;

/// Small pages, so a modest fixture crosses several of them.
const PAGE: usize = 64;

fn options() -> BufferedOptions {
    BufferedOptions::default()
        .with_page_size(PAGE)
        .with_max_bytes(16 * PAGE as u64)
}

/// A counting handle holding `size` bytes that name their own offset.
fn counted(size: usize) -> Counting {
    Counting::from_bytes((0..size).map(|index| index as u8).collect())
}

#[test]
fn a_cursor_over_a_buffered_handle_streams_across_pages() {
    let mut cursor = counted(4 * PAGE).buffered(options()).cursor();

    // Sixteen sequential reads over four pages: one inner read per page,
    // because the cursor rides the cache rather than the handle.
    let mut streamed = Vec::new();
    let mut chunk = [0_u8; 16];
    while cursor.read(&mut chunk).unwrap() == 16 {
        streamed.extend_from_slice(&chunk);
    }
    assert_eq!(streamed.len(), 4 * PAGE);
    assert_eq!(streamed[PAGE + 1], (PAGE + 1) as u8);
    assert_eq!(cursor.handle().handle().reads(), 4);
    assert_eq!(cursor.tell(), 4 * PAGE as u64);
}

#[test]
fn a_cursor_seeks_to_the_end_without_re_reading() {
    let mut cursor = counted(4 * PAGE).buffered(options()).cursor();

    let mut head = [0_u8; 8];
    cursor.read_exact(&mut head).unwrap();
    assert_eq!(head[0], 0);
    Seek::seek(&mut cursor, SeekFrom::End(-8)).unwrap();

    let mut tail = [0_u8; 8];
    cursor.read_exact(&mut tail).unwrap();
    assert_eq!(tail[7], (4 * PAGE - 1) as u8);
    let reads = cursor.handle().handle().reads();

    // Both ends are pinned pages now, so going back to either is free.
    Seek::seek(&mut cursor, SeekFrom::Start(0)).unwrap();
    cursor.read_exact(&mut head).unwrap();
    Seek::seek(&mut cursor, SeekFrom::End(-8)).unwrap();
    cursor.read_exact(&mut tail).unwrap();
    assert_eq!(cursor.handle().handle().reads(), reads);

    // A seek past the end reads nothing, exactly as `pread` does.
    Seek::seek(&mut cursor, SeekFrom::End(64)).unwrap();
    assert_eq!(cursor.read(&mut tail).unwrap(), 0);
}

#[test]
fn a_cursor_writes_through_and_reads_back_what_it_wrote() {
    let mut cursor = Buffer::from_bytes(vec![7_u8; 4 * PAGE])
        .buffered(options())
        .cursor();

    cursor.read_exact(&mut [0_u8; 8]).unwrap();
    cursor.seek_to(PAGE as u64 - 2);
    cursor.write_all(b"ABCD").unwrap();

    cursor.seek_to(PAGE as u64 - 2);
    let mut written = [0_u8; 4];
    cursor.read_exact(&mut written).unwrap();
    assert_eq!(&written, b"ABCD");
    assert_eq!(cursor.size(), 4 * PAGE as u64);
}

#[test]
fn a_buffered_file_reads_from_pages_and_writes_through() {
    let path = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!(
            "yggdryl-buffered-file-{}-{:?}.bin",
            std::process::id(),
            std::thread::current().id()
        ));
    let _ = std::fs::remove_file(&path);
    let payload: Vec<u8> = (0..5_000_u32).map(|index| index as u8).collect();
    std::fs::write(&path, &payload).unwrap();

    let mut handle = crate::holder::local::File::new(&path)
        .unwrap()
        .buffered(BufferedOptions::default().with_page_size(512));

    // The wrapper answers for the file it wraps.
    assert_eq!(handle.size(), 5_000);
    assert_eq!(handle.kind(), crate::IOKind::File);
    assert_eq!(
        handle.url().unwrap().file_name(),
        path.file_name().and_then(std::ffi::OsStr::to_str)
    );
    assert_eq!(handle.read_all_bytes().unwrap(), payload);
    assert_eq!(handle.cached_pages(), 10);

    // A write lands in the file and in the pages that held those bytes.
    handle.pwrite(600, b"trade").unwrap();
    handle.flush().unwrap();
    assert_eq!(handle.read_range_bytes(600, 5).unwrap(), b"trade");
    assert_eq!(std::fs::read(&path).unwrap()[600..605], *b"trade");

    // Closing releases the pages and leaves a working handle behind.
    handle.close().unwrap();
    assert_eq!(handle.cached_pages(), 0);
    assert_eq!(handle.read_range_bytes(600, 5).unwrap(), b"trade");

    drop(handle);
    let _ = std::fs::remove_file(&path);
}
