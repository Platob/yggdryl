use std::any::Any;
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::{Error, IOBase, IOKind, Result};

fn write(filesystem: &dyn FileSystem, path: &str, bytes: &[u8]) -> Result<()> {
    let mut writer = filesystem.open_output_stream(path, None)?;
    let mut offset = 0;
    while offset < bytes.len() {
        let written = writer.write(&bytes[offset..])?;
        if written == 0 {
            return Err(Error::Io(std::io::Error::from(
                std::io::ErrorKind::WriteZero,
            )));
        }
        offset += written;
    }
    writer.close()
}

fn read(filesystem: &dyn FileSystem, path: &str) -> Result<Vec<u8>> {
    let mut reader = filesystem.open_input_stream(path)?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 3];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    reader.close()?;
    Ok(bytes)
}

fn reference_conformance(filesystem: Arc<dyn FileSystem>, root: &str) {
    filesystem.create_dir(root, true).unwrap();
    filesystem
        .create_dir(&format!("{root}/sub"), false)
        .unwrap();
    write(filesystem.as_ref(), &format!("{root}/b.bin"), b"b").unwrap();
    write(filesystem.as_ref(), &format!("{root}/a.bin"), b"012345").unwrap();
    write(filesystem.as_ref(), &format!("{root}/sub/c.bin"), b"c").unwrap();

    let info = filesystem.file_info(&format!("{root}/a.bin")).unwrap();
    assert_eq!(info.kind, IOKind::File);
    assert_eq!(info.size, Some(6));
    assert!(info.mtime_ns.is_some());
    assert_eq!(
        filesystem.file_info(&format!("{root}/missing")).unwrap(),
        FileInfo::not_found(format!("{root}/missing"))
    );

    let mut sequential = filesystem
        .open_input_stream(&format!("{root}/a.bin"))
        .unwrap();
    let mut first = [0_u8; 2];
    assert_eq!(sequential.read(&mut first).unwrap(), 2);
    assert_eq!(&first, b"01");
    assert_eq!(sequential.tell(), 2);
    sequential.close().unwrap();
    sequential.close().unwrap();
    assert!(sequential.closed());

    let mut random = filesystem
        .open_input_file(&format!("{root}/a.bin"))
        .unwrap();
    let mut middle = [0_u8; 2];
    assert_eq!(random.read_at(2, &mut middle).unwrap(), 2);
    assert_eq!(&middle, b"23");
    assert_eq!(random.tell(), 0);
    assert_eq!(random.seek(SeekFrom::End(-2)).unwrap(), 4);
    assert_eq!(random.read(&mut middle).unwrap(), 2);
    assert_eq!(&middle, b"45");
    random.close().unwrap();

    let mut append = filesystem
        .open_append_stream(&format!("{root}/a.bin"), None)
        .unwrap();
    assert_eq!(append.tell(), 6);
    assert_eq!(append.write(b"67").unwrap(), 2);
    append.flush().unwrap();
    append.close().unwrap();
    append.close().unwrap();
    assert_eq!(
        read(filesystem.as_ref(), &format!("{root}/a.bin")).unwrap(),
        b"01234567"
    );

    let mut created_by_append = filesystem
        .open_append_stream(&format!("{root}/new.bin"), None)
        .unwrap();
    assert_eq!(created_by_append.tell(), 0);
    assert_eq!(created_by_append.write(b"new").unwrap(), 3);
    created_by_append.close().unwrap();
    assert_eq!(
        read(filesystem.as_ref(), &format!("{root}/new.bin")).unwrap(),
        b"new"
    );

    let immediate = filesystem
        .list(&FileSelector::new(root, false, false))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let paths: Vec<_> = immediate.iter().map(|info| info.path.as_str()).collect();
    let listed_root = root.replace('\\', "/");
    assert_eq!(
        paths,
        [
            format!("{listed_root}/a.bin"),
            format!("{listed_root}/b.bin"),
            format!("{listed_root}/new.bin"),
            format!("{listed_root}/sub")
        ]
    );
    let recursive = filesystem
        .list(&FileSelector::new(root, true, false))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert!(recursive.windows(2).all(|pair| pair[0].path < pair[1].path));

    assert!(
        filesystem
            .list(&FileSelector::new(format!("{root}/absent"), false, true))
            .next()
            .is_none()
    );
    let mut missing = filesystem.list(&FileSelector::new(format!("{root}/absent"), false, false));
    assert!(missing.next().unwrap().unwrap_err().is_absent());
    assert!(missing.next().is_none());

    let error = filesystem.delete_file(&format!("{root}/sub")).unwrap_err();
    assert!(matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::IsADirectory));
    let error = filesystem.delete_dir(root).unwrap_err();
    assert!(
        matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty)
    );

    filesystem
        .delete_dir_contents(&format!("{root}/sub"), false)
        .unwrap();
    assert_eq!(
        filesystem.file_info(&format!("{root}/sub")).unwrap().kind,
        IOKind::Directory
    );
    filesystem.delete_dir(&format!("{root}/sub")).unwrap();
    assert_eq!(
        filesystem.file_info(&format!("{root}/sub")).unwrap().kind,
        IOKind::Unknown
    );
    filesystem.delete_file(&format!("{root}/a.bin")).unwrap();
    filesystem.delete_file(&format!("{root}/b.bin")).unwrap();
    filesystem.delete_file(&format!("{root}/new.bin")).unwrap();
    filesystem
        .create_dir(&format!("{root}/recursive"), false)
        .unwrap();
    write(
        filesystem.as_ref(),
        &format!("{root}/recursive/child.bin"),
        b"child",
    )
    .unwrap();
    filesystem
        .delete_dir_recursive(&format!("{root}/recursive"))
        .unwrap();
    assert_eq!(
        filesystem
            .file_info(&format!("{root}/recursive"))
            .unwrap()
            .kind,
        IOKind::Unknown
    );
    filesystem.delete_dir(root).unwrap();
}

#[test]
fn memory_implements_the_complete_reference_contract() {
    reference_conformance(Arc::new(MemoryFileSystem::new()), "suite");
}

#[test]
fn memory_recursive_directory_creation_preserves_leading_and_repeated_slashes() {
    let filesystem = MemoryFileSystem::new();
    filesystem.create_dir("/a//b", true).unwrap();
    for path in ["/a", "/a/", "/a//b"] {
        assert_eq!(
            filesystem.file_info(path).unwrap().kind,
            IOKind::Directory,
            "{path}"
        );
    }
    assert_eq!(filesystem.file_info("a/b").unwrap().kind, IOKind::Unknown);
}

#[test]
fn reference_filesystems_do_not_invent_output_parents() {
    let memory = MemoryFileSystem::new();
    assert!(
        memory
            .open_output_stream("missing/file.bin", None)
            .err()
            .unwrap()
            .is_absent()
    );
    assert_eq!(memory.file_info("missing").unwrap().kind, IOKind::Unknown);
}

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yggdryl-fs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn local_implements_the_complete_reference_contract() {
    let temporary = TestDirectory::new();
    let root = temporary.0.join("suite").to_string_lossy().into_owned();
    reference_conformance(Arc::new(LocalFileSystem::new()), &root);
}

#[test]
fn recursive_reference_listings_are_globally_sorted() {
    for filesystem in [
        Arc::new(MemoryFileSystem::new()) as Arc<dyn FileSystem>,
        Arc::new(LocalFileSystem::new()) as Arc<dyn FileSystem>,
    ] {
        let temporary = TestDirectory::new();
        let root = if filesystem.type_name() == "file" {
            temporary.0.join("order").to_string_lossy().into_owned()
        } else {
            "order".to_owned()
        };
        filesystem.create_dir(&format!("{root}/a"), true).unwrap();
        write(filesystem.as_ref(), &format!("{root}/a/z"), b"z").unwrap();
        write(filesystem.as_ref(), &format!("{root}/a-thing"), b"-").unwrap();
        write(filesystem.as_ref(), &format!("{root}/a.child"), b".").unwrap();
        let paths = filesystem
            .list(&FileSelector::new(&root, true, false))
            .map(|entry| entry.unwrap().path)
            .collect::<Vec<_>>();
        assert!(paths.windows(2).all(|pair| pair[0] < pair[1]), "{paths:?}");
    }
}

#[test]
fn memory_writer_failures_remain_visible_through_close() {
    let filesystem = MemoryFileSystem::new();
    let mut writer = filesystem.open_output_stream("lost.bin", None).unwrap();
    filesystem.delete_file("lost.bin").unwrap();
    assert!(writer.write(b"x").unwrap_err().is_absent());
    assert!(writer.close().unwrap_err().is_absent());
    assert!(writer.closed());
    assert!(writer.close().unwrap_err().is_absent());

    let mut successful = filesystem.open_output_stream("closed.bin", None).unwrap();
    successful.close().unwrap();
    successful.close().unwrap();
    assert!(successful.closed());
}

#[test]
fn bound_locations_keep_raw_path_uri_and_filesystem_identity_separate() {
    let memory = MemoryFileSystem::new();
    let filesystem: Arc<dyn FileSystem> = Arc::new(memory.clone());
    let equal: Arc<dyn FileSystem> = Arc::new(memory);
    let different: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    let uri = "s3://access:secret@bucket/v=a%2Fb.bin?session_token=hidden";
    let location = BoundLocation::new(
        Arc::clone(&filesystem),
        "bucket/v=a%2Fb.bin",
        Some(uri.to_owned()),
    )
    .unwrap();
    let same = BoundLocation::new(equal, "bucket/v=a%2Fb.bin", None::<String>).unwrap();
    let other = BoundLocation::new(different, "bucket/v=a%2Fb.bin", None::<String>).unwrap();

    assert_eq!(location.path(), "bucket/v=a%2Fb.bin");
    assert_eq!(location.uri(), Some(uri));
    assert!(location.same_location(&same));
    assert_eq!(location.identity(), same.identity());
    assert!(!location.same_location(&other));
    assert_ne!(location.identity(), other.identity());
    for diagnostic in [
        location.masked_uri().unwrap().to_owned(),
        format!("{location}"),
        format!("{location:?}"),
        format!("{:?}", location.identity()),
    ] {
        assert!(!diagnostic.contains("secret"));
        assert!(!diagnostic.contains("hidden"));
    }

    let parent = location.parent().unwrap().unwrap();
    assert_eq!(parent.path(), "bucket");
    assert!(parent.filesystem().equals(filesystem.as_ref()));
    let child = parent.child("v=a%2Fb//x%25+y://z").unwrap();
    assert_eq!(child.path(), "bucket/v=a%2Fb//x%25+y://z");
    assert!(child.filesystem().equals(filesystem.as_ref()));

    let absolute = BoundLocation::new(
        Arc::clone(&filesystem),
        "/tmp/value",
        Some("file:///tmp/value".to_owned()),
    )
    .unwrap();
    assert_eq!(absolute.parent().unwrap().unwrap().path(), "/tmp");
    assert_eq!(
        absolute.parent().unwrap().unwrap().uri(),
        Some("file:///tmp")
    );
    let absolute_root = absolute
        .parent()
        .unwrap()
        .unwrap()
        .parent()
        .unwrap()
        .unwrap();
    assert_eq!(absolute_root.path(), "/");
    assert_eq!(absolute_root.uri(), Some("file:///"));

    filesystem.create_dir("bucket", false).unwrap();
    let mut file = File::new(location.clone());
    file.write_all_bytes(b"literal").unwrap();
    assert_eq!(file.read_all_bytes().unwrap(), b"literal");
    let folder = Folder::from_path(
        Arc::clone(&filesystem),
        "bucket",
        Some("s3://access:secret@bucket?session_token=hidden".to_owned()),
    )
    .unwrap();
    let listed = folder.ls(false, true).collect::<Result<Vec<_>>>().unwrap();
    assert_eq!(listed.len(), 1);
    let listed_bound = listed[0].bound_location().unwrap();
    assert_eq!(listed_bound.path(), "bucket/v=a%2Fb.bin");
    assert!(listed_bound.filesystem().equals(filesystem.as_ref()));
    assert_eq!(
        listed_bound.uri(),
        Some("s3://access:secret@bucket/v=a%2Fb.bin?session_token=hidden")
    );
    let globbed = folder
        .glob("v=a%2F*.bin", true)
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(globbed.len(), 1);
    assert_eq!(
        globbed[0].bound_location().unwrap().path(),
        "bucket/v=a%2Fb.bin"
    );
}

#[test]
fn native_filesystem_errors_never_expose_credentials_in_opaque_paths() {
    let filesystem = MemoryFileSystem::new();
    let path = "s3://access:do-not-leak@bucket/missing?session_token=also-hidden";
    let error = match filesystem.open_input_file(path) {
        Ok(_) => panic!("missing path unexpectedly opened"),
        Err(error) => error,
    };
    for rendered in [error.to_string(), format!("{error:?}")] {
        assert!(!rendered.contains("do-not-leak"), "{rendered}");
        assert!(!rendered.contains("also-hidden"), "{rendered}");
    }
}

#[test]
fn bound_glob_preserves_repeated_separator_paths() {
    let memory = MemoryFileSystem::new();
    memory.create_dir("root/foo/", true).unwrap();
    write(&memory, "root/foo//bar.txt", b"value").unwrap();
    let filesystem: Arc<dyn FileSystem> = Arc::new(memory);
    let root = Folder::from_path(filesystem, "root", None).unwrap();

    let paths = root
        .glob("foo//*.txt", true)
        .unwrap()
        .map(|entry| entry.unwrap().bound_location().unwrap().path().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["root/foo//bar.txt"]);
}

#[derive(Default)]
struct Calls {
    file_info: AtomicUsize,
    create_dir: AtomicUsize,
    input_file: AtomicUsize,
    input_stream: AtomicUsize,
    output_stream: AtomicUsize,
    append_stream: AtomicUsize,
    copy_file: AtomicUsize,
    move_file: AtomicUsize,
    metadata: Mutex<Option<OutputMetadata>>,
}

struct InstrumentedFileSystem {
    inner: MemoryFileSystem,
    identity: Arc<()>,
    calls: Arc<Calls>,
    fail_after: Option<usize>,
}

impl InstrumentedFileSystem {
    fn new(inner: MemoryFileSystem) -> Self {
        Self {
            inner,
            identity: Arc::new(()),
            calls: Arc::new(Calls::default()),
            fail_after: None,
        }
    }

    fn failing(inner: MemoryFileSystem, fail_after: usize) -> Self {
        Self {
            fail_after: Some(fail_after),
            ..Self::new(inner)
        }
    }
}

impl FileSystem for InstrumentedFileSystem {
    fn type_name(&self) -> &str {
        "instrumented"
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| Arc::ptr_eq(&self.identity, &other.identity))
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        self.inner.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.calls.file_info.fetch_add(1, Ordering::Relaxed);
        self.inner.file_info(path)
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.inner.list(selector)
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        self.calls.create_dir.fetch_add(1, Ordering::Relaxed);
        self.inner.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        self.inner.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        self.inner.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        self.inner.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.inner.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        self.calls.copy_file.fetch_add(1, Ordering::Relaxed);
        self.inner.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        self.calls.move_file.fetch_add(1, Ordering::Relaxed);
        self.inner.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        self.calls.input_file.fetch_add(1, Ordering::Relaxed);
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        self.calls.input_stream.fetch_add(1, Ordering::Relaxed);
        let reader = self.inner.open_input_stream(path)?;
        Ok(match self.fail_after {
            Some(limit) => Box::new(FailingReader {
                inner: reader,
                remaining: limit,
            }),
            None => reader,
        })
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.calls.output_stream.fetch_add(1, Ordering::Relaxed);
        *self.calls.metadata.lock().unwrap() = metadata.cloned();
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.calls.append_stream.fetch_add(1, Ordering::Relaxed);
        self.inner.open_append_stream(path, metadata)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn high_level_writes_repair_a_missing_parent_once_without_probing() {
    let instrumented = Arc::new(InstrumentedFileSystem::new(MemoryFileSystem::new()));
    let calls = Arc::clone(&instrumented.calls);
    let filesystem: Arc<dyn FileSystem> = instrumented;
    let mut file =
        File::from_path(Arc::clone(&filesystem), "missing/deep/value.bin", None).unwrap();

    file.write_all_bytes(b"value").unwrap();

    assert_eq!(calls.file_info.load(Ordering::Relaxed), 0);
    assert_eq!(calls.output_stream.load(Ordering::Relaxed), 2);
    assert_eq!(calls.create_dir.load(Ordering::Relaxed), 1);
    assert_eq!(
        filesystem.file_info("missing/deep").unwrap().kind,
        IOKind::Directory
    );
    assert_eq!(
        read(filesystem.as_ref(), "missing/deep/value.bin").unwrap(),
        b"value"
    );
}

struct FailingReader {
    inner: Box<dyn ByteReader>,
    remaining: usize,
}

impl ByteReader for FailingReader {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        if self.remaining == 0 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "injected transport failure",
            )));
        }
        let length = buffer.len().min(self.remaining);
        let count = self.inner.read(&mut buffer[..length])?;
        self.remaining -= count;
        Ok(count)
    }

    fn tell(&self) -> u64 {
        self.inner.tell()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn closed(&self) -> bool {
        self.inner.closed()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

#[test]
fn streaming_retains_one_open_and_output_metadata_reaches_the_backend() {
    let inner = MemoryFileSystem::new();
    write(&inner, "ten.bin", b"0123456789").unwrap();
    let instrumented = Arc::new(InstrumentedFileSystem::new(inner));
    let calls = Arc::clone(&instrumented.calls);
    let filesystem: Arc<dyn FileSystem> = instrumented;
    let file = File::from_path(Arc::clone(&filesystem), "ten.bin", None).unwrap();
    let chunks = file
        .pstream_bytes(0, 3)
        .unwrap()
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        chunks,
        [
            b"012".to_vec(),
            b"345".to_vec(),
            b"678".to_vec(),
            b"9".to_vec()
        ]
    );
    assert_eq!(calls.input_stream.load(Ordering::Relaxed), 1);
    assert!(calls.file_info.load(Ordering::Relaxed) <= 1);

    let metadata = OutputMetadata::from_entries([("content-type", "application/octet-stream")]);
    let mut output = filesystem
        .open_output_stream("metadata.bin", Some(&metadata))
        .unwrap();
    output.write(b"x").unwrap();
    output.close().unwrap();
    assert_eq!(*calls.metadata.lock().unwrap(), Some(metadata));
}

#[test]
fn whole_and_range_reads_use_streams_without_a_metadata_probe() {
    let inner = MemoryFileSystem::new();
    write(&inner, "value.bin", b"0123456789").unwrap();
    let instrumented = Arc::new(InstrumentedFileSystem::new(inner));
    let calls = Arc::clone(&instrumented.calls);
    let filesystem: Arc<dyn FileSystem> = instrumented;
    let file = File::from_path(filesystem, "value.bin", None).unwrap();

    assert_eq!(file.read_all_bytes().unwrap(), b"0123456789");
    assert_eq!(file.read_range_bytes(3, 4).unwrap(), b"3456");
    assert_eq!(calls.file_info.load(Ordering::Relaxed), 0);
    assert_eq!(calls.input_stream.load(Ordering::Relaxed), 1);
    assert_eq!(calls.input_file.load(Ordering::Relaxed), 1);
}

#[test]
fn same_filesystem_copy_and_move_each_use_one_native_operation_and_no_streams() {
    let inner = MemoryFileSystem::new();
    write(&inner, "source.bin", b"payload").unwrap();
    let instrumented = Arc::new(InstrumentedFileSystem::new(inner));
    let calls = Arc::clone(&instrumented.calls);
    let filesystem: Arc<dyn FileSystem> = instrumented;
    let source = BoundLocation::new(Arc::clone(&filesystem), "source.bin", None::<String>).unwrap();
    let copied = BoundLocation::new(Arc::clone(&filesystem), "copied.bin", None::<String>).unwrap();
    let moved = BoundLocation::new(Arc::clone(&filesystem), "moved.bin", None::<String>).unwrap();

    assert_eq!(copy_bound(&source, &copied).unwrap(), 7);
    assert_eq!(move_bound(&copied, &moved).unwrap(), 7);
    assert_eq!(calls.copy_file.load(Ordering::Relaxed), 1);
    assert_eq!(calls.move_file.load(Ordering::Relaxed), 1);
    assert_eq!(calls.input_file.load(Ordering::Relaxed), 0);
    assert_eq!(calls.input_stream.load(Ordering::Relaxed), 0);
    assert_eq!(calls.output_stream.load(Ordering::Relaxed), 0);
}

#[test]
fn cross_filesystem_copy_never_publishes_a_missing_or_partial_source() {
    let missing_source: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    let target_memory = MemoryFileSystem::new();
    write(&target_memory, "target.bin", b"original").unwrap();
    let target: Arc<dyn FileSystem> = Arc::new(target_memory.clone());
    let missing = BoundLocation::new(missing_source, "missing.bin", None::<String>).unwrap();
    let existing = BoundLocation::new(Arc::clone(&target), "target.bin", None::<String>).unwrap();
    assert!(copy_bound(&missing, &existing).unwrap_err().is_absent());
    assert_eq!(read(&target_memory, "target.bin").unwrap(), b"original");
    let absent_target = BoundLocation::new(Arc::clone(&target), "new.bin", None::<String>).unwrap();
    assert!(
        copy_bound(&missing, &absent_target)
            .unwrap_err()
            .is_absent()
    );
    assert_eq!(
        target_memory.file_info("new.bin").unwrap().kind,
        IOKind::Unknown
    );

    let source_memory = MemoryFileSystem::new();
    write(&source_memory, "source.bin", b"0123456789").unwrap();
    let failing: Arc<dyn FileSystem> = Arc::new(InstrumentedFileSystem::failing(source_memory, 4));
    let source = BoundLocation::new(failing, "source.bin", None::<String>).unwrap();
    let error = copy_bound(&source, &existing).unwrap_err();
    assert!(
        matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::ConnectionReset)
    );
    assert_eq!(read(&target_memory, "target.bin").unwrap(), b"original");
    let names = target_memory
        .list(&FileSelector::new("", true, false))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert!(
        names
            .iter()
            .all(|info| !info.path.contains(".yggdryl-transfer-"))
    );
}

#[test]
fn root_content_deletion_is_explicit_and_listing_fuses_after_error() {
    let memory = MemoryFileSystem::new();
    write(&memory, "a.bin", b"a").unwrap();
    memory.create_dir("folder", false).unwrap();
    let bound_filesystem: Arc<dyn FileSystem> = Arc::new(memory.clone());
    let non_root = Folder::new(
        BoundLocation::new(Arc::clone(&bound_filesystem), "folder", None::<String>).unwrap(),
    );
    assert!(
        non_root
            .delete_root_dir_contents()
            .unwrap_err()
            .is_unsupported()
    );
    let error = memory.delete_dir_contents("", false).unwrap_err();
    assert!(error.is_unsupported());
    let root = Folder::new(BoundLocation::new(bound_filesystem, "", None::<String>).unwrap());
    root.delete_root_dir_contents().unwrap();
    assert!(
        memory
            .list(&FileSelector::new("", false, false))
            .next()
            .is_none()
    );

    let failure = Error::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied",
    ));
    let mut entries = FileInfos::new(
        vec![
            Ok(FileInfo::file("a", 1, None)),
            Err(failure),
            Ok(FileInfo::file("b", 1, None)),
        ]
        .into_iter(),
    );
    assert_eq!(entries.next().unwrap().unwrap().path, "a");
    assert!(
        matches!(entries.next().unwrap(), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
    assert!(entries.next().is_none());
}

#[test]
fn explicit_uri_options_override_query_without_exposing_secrets() {
    let mut options = BTreeMap::new();
    options.insert("region".to_owned(), "us-east-2".to_owned());
    options.insert("anonymous".to_owned(), "true".to_owned());
    options.insert("force_path_style".to_owned(), "true".to_owned());
    let resolved = ResolvedFileSystemUri::from_uri(
        "s3://access:never-show-this@bucket/v=a%2Fb?region=eu-west-1",
        Some(&options),
    )
    .unwrap();
    let ResolvedFileSystem::S3(configuration) = resolved.filesystem() else {
        panic!("expected S3")
    };
    assert_eq!(resolved.path(), "bucket/v=a%2Fb");
    assert_eq!(configuration.region(), Some("us-east-2"));
    assert!(configuration.anonymous());
    assert_eq!(configuration.addressing_style(), S3AddressingStyle::Path);
    assert!(!format!("{resolved:?}").contains("never-show-this"));
}
