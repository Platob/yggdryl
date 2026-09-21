//! `rust/src/bytestream.rs`: the chunked byte reader every stream door
//! answers with.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::holder::Buffer;
use yggdryl::internals::bytestream::{ByteSource, from_source};
use yggdryl::{ByteStream, Error, IOBase, IOCursor as _, Result};

struct CountedReader {
    bytes: std::io::Cursor<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}

impl Read for CountedReader {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.bytes.read(target)
    }
}

#[test]
fn positional_streams_start_lazily_and_chunk_exactly() {
    let handle = Buffer::from_bytes(b"0123456789".to_vec());
    let chunks = handle
        .pstream_bytes(2, 3)
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(chunks, [b"234".to_vec(), b"567".to_vec(), b"89".to_vec()]);

    let reads = Arc::new(AtomicUsize::new(0));
    let mut stream = ByteStream::from_reader(
        CountedReader {
            bytes: std::io::Cursor::new(b"abc".to_vec()),
            reads: Arc::clone(&reads),
        },
        2,
    )
    .unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    assert_eq!(stream.next().unwrap().unwrap(), b"ab");
    assert!(reads.load(Ordering::Relaxed) > 0);
}

#[test]
fn zero_batch_size_is_refused_before_reading() {
    let error = Buffer::new().pstream_bytes(0, 0).unwrap_err();
    assert!(matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput));
}

#[test]
fn a_byte_stream_is_a_bounded_standard_reader() {
    let handle = Buffer::from_bytes(b"abcdefgh".to_vec());
    let mut stream = handle.pstream_bytes(1, 3).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"bcdefgh");
}

#[test]
fn positional_streaming_remains_object_safe() {
    let handle: Box<dyn IOBase> = Box::new(Buffer::from_bytes(b"abcdef".to_vec()));
    let chunks = handle
        .pstream_bytes(1, 2)
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(chunks, [b"bc".to_vec(), b"de".to_vec(), b"f".to_vec()]);
}

#[test]
fn a_cursor_stream_advances_only_as_it_is_consumed() {
    let mut cursor = Buffer::from_bytes(b"01234567".to_vec()).cursor_at(2);
    {
        let mut stream = cursor.stream_bytes(3).unwrap();
        assert_eq!(stream.next().unwrap().unwrap(), b"234");
    }
    assert_eq!(cursor.tell(), 5);
    assert_eq!(cursor.read_next(&mut [0_u8; 0]).unwrap(), 0);
}

struct FailsAfterPrefix(bool);

impl Read for FailsAfterPrefix {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            return Err(std::io::Error::other("later failure"));
        }
        self.0 = true;
        target[..2].copy_from_slice(b"ok");
        Ok(2)
    }
}

#[test]
fn a_failure_follows_its_prefix_once_then_fuses() {
    let mut stream = ByteStream::from_reader(FailsAfterPrefix(false), 4).unwrap();
    assert_eq!(stream.next().unwrap().unwrap(), b"ok");
    assert!(stream.next().is_some_and(|item| item.is_err()));
    assert!(stream.next().is_none());
    assert!(stream.next().is_none());
}

struct Overreports;

impl ByteSource for Overreports {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        Ok(target.len() + 1)
    }
}

#[test]
fn a_source_cannot_report_bytes_outside_the_supplied_buffer() {
    let mut stream = from_source(Overreports, 4).unwrap();
    let error = stream.next().unwrap().unwrap_err();
    assert!(matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidData));
    assert!(stream.next().is_none());
}
