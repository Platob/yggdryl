use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use arrow_array::RecordBatch;
use arrow_ipc::writer::StreamWriter;

use super::super::folder_reader;
use super::prices;
use crate::holder::Holder;
use crate::holder::arrowfs::{ArrowFileSystem, File, FileInfo, FileInfos, MemoryFileSystem};
use crate::media::{IORecordOptions, RecordOptions};
use crate::{DataType, Error, IOKind, MediaType, MimeType, Result, Url};
use crate::{IOBase, Listing};

const LISTING_FAILURE: &str = "lazy folder listing failed";
const READ_FAILURE: &str = "lazy leaf read failed";

/// An in-memory foreign filesystem that records exactly which leaf bytes
/// were requested and can refuse one leaf's reads.
#[derive(Debug)]
struct ProbeFilesystem {
    inner: MemoryFileSystem,
    reads: Mutex<Vec<String>>,
    failing_path: Option<String>,
}

impl ProbeFilesystem {
    fn new(failing_path: Option<String>) -> Self {
        Self {
            inner: MemoryFileSystem::new(),
            reads: Mutex::new(Vec::new()),
            failing_path,
        }
    }

    fn read_count(&self, path: &str) -> usize {
        self.reads
            .lock()
            .expect("the read probe lock")
            .iter()
            .filter(|read| read.as_str() == path)
            .count()
    }

    fn total_reads(&self) -> usize {
        self.reads.lock().expect("the read probe lock").len()
    }
}

impl ArrowFileSystem for ProbeFilesystem {
    fn type_name(&self) -> &str {
        "probe"
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.inner.file_info(path)
    }

    fn list(&self, path: &str, recursive: bool) -> FileInfos {
        self.inner.list(path, recursive)
    }

    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.reads
            .lock()
            .expect("the read probe lock")
            .push(path.to_owned());
        if self.failing_path.as_deref() == Some(path) {
            return Err(Error::Io(std::io::Error::other(READ_FAILURE)));
        }
        self.inner.read_range(path, offset, buffer)
    }

    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.inner.write_full(path, bytes)
    }

    fn create_dir(&self, path: &str) -> Result<()> {
        self.inner.create_dir(path)
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.inner.delete_file(path)
    }
}

/// A synthetic folder whose listing builds one Arrow-filesystem leaf per
/// pull. The counter distinguishes a live listing from an eager vector.
struct ProbeFolder {
    url: Url,
    filesystem: Arc<ProbeFilesystem>,
    width: usize,
    pulls: Arc<AtomicUsize>,
    failing_entry: Option<usize>,
}

impl ProbeFolder {
    fn new(width: usize, failing_entry: Option<usize>, failing_read: Option<usize>) -> Self {
        let failing_path = failing_read.map(part_path);
        let filesystem = Arc::new(ProbeFilesystem::new(failing_path));
        let encoded = encoded_batch(&prices());
        for index in 0..width {
            filesystem
                .write_full(&part_path(index), &encoded)
                .expect("seed one IPC leaf");
        }
        Self {
            url: Url::from_str("probe://bucket/lake/").expect("a valid folder URL"),
            filesystem,
            width,
            pulls: Arc::new(AtomicUsize::new(0)),
            failing_entry,
        }
    }

    fn pulls(&self) -> usize {
        self.pulls.load(Ordering::Relaxed)
    }
}

impl crate::IOMedia for ProbeFolder {
    crate::impl_default_iomedia!();
}

impl IOBase for ProbeFolder {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        0
    }

    fn capacity(&self) -> u64 {
        0
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, _size: u64) -> Result<()> {
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        static DIRECTORY: OnceLock<MediaType> = OnceLock::new();
        DIRECTORY.get_or_init(|| MediaType::from(MimeType::DIRECTORY))
    }

    fn set_media_type(&mut self, _media_type: MediaType) {}

    fn kind(&self) -> IOKind {
        IOKind::Directory
    }

    fn children_where(&self, filters: &[(&str, &str)], include_private: bool) -> Result<Listing> {
        assert!(filters.is_empty());
        assert!(!include_private);
        let filesystem: Arc<dyn ArrowFileSystem> = self.filesystem.clone();
        let pulls = Arc::clone(&self.pulls);
        let width = self.width;
        let failing_entry = self.failing_entry;
        Ok(Listing::new((0..width).map(move |index| {
            pulls.fetch_add(1, Ordering::Relaxed);
            if failing_entry == Some(index) {
                return Err(Error::Io(std::io::Error::other(LISTING_FAILURE)));
            }
            File::from_location(Arc::clone(&filesystem), &part_path(index)).map(Holder::ArrowFile)
        })))
    }
}

fn part_path(index: usize) -> String {
    format!("bucket/lake/part-{index}.arrows")
}

fn encoded_batch(batch: &RecordBatch) -> Vec<u8> {
    let mut encoded = Vec::new();
    {
        let mut writer =
            StreamWriter::try_new(&mut encoded, batch.schema().as_ref()).expect("an IPC writer");
        writer.write(batch).expect("one IPC batch");
        writer.finish().expect("the IPC end marker");
    }
    encoded
}

fn options() -> RecordOptions {
    let field = DataType::from_fields([DataType::Int64.required_field("price")])
        .expect("one field")
        .required_field("row");
    RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
        .expect("IPC options")
        .with_field(field)
}

fn error_then_fused(mut reader: crate::arrow::BatchReader) -> String {
    let error = reader
        .next()
        .expect("one failure")
        .expect_err("the read must fail")
        .to_string();
    assert!(
        reader.next().is_none(),
        "the reader must fuse after failure"
    );
    assert!(reader.next().is_none(), "the reader must stay fused");
    error
}

#[test]
fn a_declared_schema_constructs_without_pulling_or_reading_a_leaf() {
    let folder = ProbeFolder::new(3, None, None);

    let reader = folder_reader(&folder, &options()).expect("a folder reader");

    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(folder.pulls(), 0, "the leaf listing stays untouched");
    assert_eq!(
        folder.filesystem.total_reads(),
        0,
        "no leaf bytes are inspected to construct a declared schema"
    );
}

#[test]
fn the_first_batch_opens_only_the_first_listed_leaf() {
    let folder = ProbeFolder::new(3, None, None);
    let mut reader = folder_reader(&folder, &options()).expect("a folder reader");

    let first = reader.next().expect("one batch").expect("a valid batch");

    assert_eq!(first.num_rows(), 3);
    assert_eq!(folder.pulls(), 1, "only one listing entry was demanded");
    assert!(folder.filesystem.read_count(&part_path(0)) > 0);
    assert_eq!(
        folder.filesystem.read_count(&part_path(1)),
        0,
        "the next leaf remains unopened"
    );
}

#[test]
fn a_listing_failure_is_yielded_once_then_the_reader_fuses() {
    let folder = ProbeFolder::new(3, Some(0), None);
    let reader = folder_reader(&folder, &options()).expect("a lazy folder reader");

    let error = error_then_fused(reader);

    assert!(error.contains(LISTING_FAILURE), "{error}");
    assert_eq!(folder.pulls(), 1, "nothing was pulled past the failure");
    assert_eq!(folder.filesystem.total_reads(), 0);
}

#[test]
fn a_leaf_read_failure_is_yielded_once_then_the_reader_fuses() {
    let folder = ProbeFolder::new(3, None, Some(0));
    let reader = folder_reader(&folder, &options()).expect("a lazy folder reader");

    let error = error_then_fused(reader);

    assert!(error.contains(READ_FAILURE), "{error}");
    assert_eq!(folder.pulls(), 1, "the next leaf was never requested");
    assert_eq!(folder.filesystem.read_count(&part_path(0)), 1);
    assert_eq!(folder.filesystem.read_count(&part_path(1)), 0);
}
