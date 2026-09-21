//! `rust/src/fs/stream.rs`: the object-safe byte streams a read and a write go
//! through, and what they cost the backend.

mod fs {
    use std::any::Any;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use yggdryl::fs::*;
    use yggdryl::{Error, IOBase, Result};

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
}
