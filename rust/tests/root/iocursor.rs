//! `rust/src/iocursor.rs`: the one retained position over a positional
//! resource.

use super::counting;

mod over_pages {
    use std::io::{Read, Seek, SeekFrom, Write};

    use super::counting::Counting;
    use yggdryl::IOBase;
    use yggdryl::IOCursor;
    use yggdryl::holder::Buffer;
    use yggdryl::holder::buffered::BufferedOptions;

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
        let path = yggdryl::local::Folder::temporary()
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

        let mut handle = yggdryl::local::File::new(&path)
            .unwrap()
            .buffered(BufferedOptions::default().with_page_size(512));

        // The wrapper answers for the file it wraps.
        assert_eq!(handle.size(), 5_000);
        assert_eq!(handle.kind(), yggdryl::IOKind::File);
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
}

mod retained {
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
        let encoded = yggdryl::gzip::dump(&plain).unwrap();
        let handle = yggdryl::coding::Coding::new(
            Counting::from_bytes(encoded.clone()),
            yggdryl::Codec::Gzip,
        );
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
}
