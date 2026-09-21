//! `rust/src/fs/file.rs`: a byte leaf over one bound location.

mod fs {
    use std::any::Any;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use yggdryl::fs::*;
    use yggdryl::{Error, IOBase, IOKind, Result};

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
}
