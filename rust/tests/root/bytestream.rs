//! `rust/src/bytestream.rs`: the chunked byte reader every stream door
//! answers with.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use yggdryl::holder::Buffer;
use yggdryl::local::{LocalFile, LocalFolder};
use yggdryl::{ByteStream, Codec, Error, IOBase, IOCursor as _, Result};

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

/// A tree of text leaves under a fresh temporary root, and what its stream
/// holds: two leaves at the top - the second without a final newline - an
/// empty one, a folder holding a plain and a gzip leaf, a leaf sorting after
/// that folder, and a private name.
fn logs(label: &str) -> (std::path::PathBuf, &'static [u8]) {
    let root = tree(label);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.log"), b"a1\na2\n").unwrap();
    std::fs::write(root.join("b.log"), b"b1\nb2").unwrap();
    std::fs::write(root.join("empty.log"), b"").unwrap();
    std::fs::write(root.join("sub").join("c.log"), b"c1\n").unwrap();
    std::fs::write(
        root.join("sub").join("d.log.gz"),
        Codec::Gzip.dump(b"d1\n").unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("z.log"), b"z1\n").unwrap();
    std::fs::write(root.join(".hidden"), b"private\n").unwrap();
    // Sorted per directory, depth first: `sub` is walked where it sorts,
    // before `z.log`; `b2` runs straight into `c1`, the gzip leaf is its
    // text, and neither the empty leaf nor the private one adds a byte.
    (root, b"a1\na2\nb1\nb2c1\nd1\nz1\n")
}

/// A fresh, empty temporary root of this test's own.
fn tree(label: &str) -> std::path::PathBuf {
    let mut root = LocalFolder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-bytestream-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn cleanup(root: &std::path::Path) {
    LocalFolder::new(root)
        .expect("a local container")
        .remove(true)
        .expect("a removable tree");
}

fn drained(stream: ByteStream<'_>) -> Vec<Vec<u8>> {
    stream.collect::<Result<Vec<_>>>().unwrap()
}

#[test]
fn a_container_streams_its_leaves_end_to_end_in_listing_order() {
    let (root, content) = logs("order");
    let folder = LocalFolder::new(&root).unwrap();

    let chunks = drained(ByteStream::from_container(&folder, 0, 3).unwrap());
    assert_eq!(chunks.concat(), content);
    // A chunk fills across leaf boundaries: every one is the full batch but
    // the last, and no leaf - the empty one included - ends one early.
    let (last, full) = chunks.split_last().unwrap();
    assert!(full.iter().all(|chunk| chunk.len() == 3), "{chunks:?}");
    assert_eq!(last.len(), content.len() % 3, "{chunks:?}");

    cleanup(&root);
}

#[test]
fn a_position_counts_content_bytes_across_the_leaves() {
    let (root, content) = logs("position");
    let folder = LocalFolder::new(&root).unwrap();

    // Inside the first leaf; the first leaf passed whole; the empty leaf and
    // what precedes it passed; the plain leaf before the gzip one passed;
    // inside the gzip leaf's text; the gzip leaf passed by its text, not its
    // stored size; the last byte; the end; past the end.
    for position in [4_usize, 6, 11, 14, 15, 17, 19, 20, 64] {
        let streamed = drained(ByteStream::from_container(&folder, position as u64, 4).unwrap());
        assert_eq!(
            streamed.concat(),
            content.get(position..).unwrap_or_default(),
            "from {position}"
        );
        assert!(
            streamed.iter().all(|chunk| !chunk.is_empty()),
            "from {position}"
        );
    }

    cleanup(&root);
}

#[test]
fn a_leaf_with_no_bytes_contributes_nothing() {
    let root = tree("empty");
    std::fs::write(root.join("empty.log"), b"").unwrap();
    // A coded leaf holding nothing holds no header for a decoder to refuse.
    std::fs::write(root.join("empty.log.gz"), b"").unwrap();
    std::fs::create_dir_all(root.join("nested")).unwrap();

    let folder = LocalFolder::new(&root).unwrap();
    assert!(
        ByteStream::from_container(&folder, 0, 4)
            .unwrap()
            .next()
            .is_none()
    );

    // A container that is not there holds no leaves, and streaming it
    // creates nothing.
    let absent = root.join("absent");
    let missing = LocalFolder::new(&absent).unwrap();
    assert!(
        ByteStream::from_container(&missing, 0, 4)
            .unwrap()
            .next()
            .is_none()
    );
    assert!(!absent.exists());

    cleanup(&root);
}

#[test]
fn a_leaf_failure_follows_the_prefix_once_then_fuses() {
    let root = tree("failure");
    std::fs::write(root.join("a.log"), b"a1\n").unwrap();
    // The name declares gzip, and the bytes are not.
    std::fs::write(root.join("b.log.gz"), b"not a gzip member").unwrap();
    std::fs::write(root.join("c.log"), b"c1\n").unwrap();

    let folder = LocalFolder::new(&root).unwrap();
    let mut stream = ByteStream::from_container(&folder, 0, 64).unwrap();
    assert_eq!(stream.next().unwrap().unwrap(), b"a1\n");
    assert!(stream.next().is_some_and(|item| item.is_err()));
    // Nothing after the failure is read, the leaves still listed included.
    assert!(stream.next().is_none());
    assert!(stream.next().is_none());

    cleanup(&root);
}

#[test]
fn a_container_stream_refuses_a_zero_batch() {
    let (root, _) = logs("zero-batch");
    let folder = LocalFolder::new(&root).unwrap();

    let error = ByteStream::from_container(&folder, 0, 0).unwrap_err();
    assert!(matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput));

    cleanup(&root);
}

#[test]
fn a_container_stream_outlives_the_handle_it_was_built_from() {
    let (root, content) = logs("outlives");

    let stream: ByteStream<'static> = {
        let folder = LocalFolder::new(&root).unwrap();
        ByteStream::from_container(&folder, 0, 5).unwrap()
    };
    assert_eq!(drained(stream).concat(), content);

    cleanup(&root);
}

#[test]
fn a_leaf_has_no_leaves_to_stream() {
    let (root, _) = logs("leaf");

    let leaf = LocalFile::new(root.join("a.log")).unwrap();
    assert!(
        ByteStream::from_container(&leaf, 0, 4)
            .unwrap()
            .next()
            .is_none()
    );
    let buffer = Buffer::from_bytes(b"abc".to_vec());
    assert!(
        ByteStream::from_container(&buffer, 0, 4)
            .unwrap()
            .next()
            .is_none()
    );
    // Its own stream is its bytes, as ever.
    assert_eq!(leaf.read_all_bytes().unwrap(), b"a1\na2\n");

    cleanup(&root);
}

/// What a caller cannot reach: the byte source a stream is built over.
#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::bytestream::{ByteSource, from_source};
    use yggdryl::{Error, Result};

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
        assert!(
            matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidData)
        );
        assert!(stream.next().is_none());
    }
}
