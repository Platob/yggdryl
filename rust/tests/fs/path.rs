//! `rust/src/fs/path.rs`: an unresolved location over one bound filesystem
//! path - the file it names read as that file, and the directory it names read
//! as the stream of the files beneath it.

use std::sync::Arc;

use yggdryl::fs::{FileSystem, FsPath, LocalFileSystem, MemoryFileSystem};
use yggdryl::local::LocalFolder;
use yggdryl::{ByteStream, Codec, Error, IOBase, IOKind, Result};

/// Write `bytes` at `path`, creating the directories above it.
fn put(filesystem: &dyn FileSystem, path: &str, bytes: &[u8]) {
    if let Some((parent, _)) = path.rsplit_once('/') {
        filesystem.create_dir(parent, true).unwrap();
    }
    let mut writer = filesystem.open_output_stream(path, None).unwrap();
    let mut offset = 0;
    while offset < bytes.len() {
        offset += writer.write(&bytes[offset..]).unwrap();
    }
    writer.close().unwrap();
}

/// A memory filesystem holding two files under `logs/`, one a directory
/// deeper, the first without a final newline.
fn memory() -> Arc<dyn FileSystem> {
    let memory: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    put(memory.as_ref(), "logs/a.log", b"a1\na2");
    put(memory.as_ref(), "logs/sub/b.log", b"b1\n");
    memory
}

/// Every chunk a stream yields, failing on the first error.
fn chunks(stream: ByteStream<'_>) -> Vec<Vec<u8>> {
    stream.collect::<Result<Vec<_>>>().unwrap()
}

#[test]
fn a_location_at_a_directory_streams_the_files_beneath_it() {
    let location = FsPath::from_path(memory(), "logs", None).unwrap();
    let streamed: &[u8] = b"a1\na2b1\n";

    // The filesystem refuses the file open with `IsADirectory`, and that
    // refusal is what routes the location to its container role.
    assert_eq!(location.kind(), IOKind::Directory);
    let read = chunks(location.pstream_bytes(0, 3).unwrap());
    assert_eq!(read.concat(), streamed);
    assert_eq!(
        read.iter().map(Vec::len).collect::<Vec<_>>(),
        [3, 3, 2],
        "full chunks across the file boundary"
    );
    assert_eq!(
        chunks(location.pstream_bytes(4, 3).unwrap()).concat(),
        &streamed[4..]
    );
    assert_eq!(location.read_all_bytes().unwrap(), streamed);
    assert_eq!(location.read_range_bytes(2, 4).unwrap(), &streamed[2..6]);
    assert_eq!(location.size(), 0, "a directory holds no bytes of its own");
}

#[test]
fn a_location_at_a_file_streams_that_file_alone() {
    let location = FsPath::from_path(memory(), "logs/a.log", None).unwrap();

    assert_eq!(location.kind(), IOKind::File);
    assert_eq!(
        chunks(location.pstream_bytes(0, 2).unwrap()).concat(),
        b"a1\na2"
    );
    assert_eq!(
        chunks(location.pstream_bytes(3, 2).unwrap()).concat(),
        b"a2"
    );
    assert_eq!(location.read_all_bytes().unwrap(), b"a1\na2");
    assert_eq!(location.read_range_bytes(1, 3).unwrap(), b"1\na");
    assert_eq!(location.size(), 5);
}

#[test]
fn a_location_at_nothing_streams_nothing() {
    let location = FsPath::from_path(memory(), "logs/absent.log", None).unwrap();
    assert!(location.pstream_bytes(0, 4).unwrap().next().is_none());
    assert!(location.read_all_bytes().unwrap().is_empty());

    // A zero batch is refused whatever the location turns out to be.
    let refused = location.pstream_bytes(0, 0).unwrap_err();
    assert!(
        matches!(&refused, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput),
        "{refused:?}"
    );
}

#[test]
fn a_local_directory_streams_its_decoded_files_in_sorted_order() {
    let root = LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fs-path-directory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("logs/sub")).unwrap();
    std::fs::write(root.join("logs/b.log"), b"b1\n").unwrap();
    std::fs::write(root.join("logs/a.log"), b"a1\n").unwrap();
    std::fs::write(root.join("logs/.hidden"), b"h\n").unwrap();
    std::fs::write(
        root.join("logs/sub/c.log.gz"),
        Codec::Gzip.dump(b"c1\n").unwrap(),
    )
    .unwrap();

    let directory = root.join("logs").to_string_lossy().replace('\\', "/");
    let location = FsPath::from_path(Arc::new(LocalFileSystem::new()), directory, None).unwrap();
    // Sorted per directory whatever order the files were written in, the
    // hidden file left out, and the gzip file read as the text it holds.
    assert_eq!(location.read_all_bytes().unwrap(), b"a1\nb1\nc1\n");
    assert_eq!(
        chunks(location.pstream_bytes(5, 2).unwrap()).concat(),
        b"\nc1\n"
    );

    let _ = std::fs::remove_dir_all(&root);
}
