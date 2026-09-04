//! Record reads, writes, and dispatch over one IOBase.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
use arrow_schema::{ArrowError, SchemaRef};

use crate::arrow::BatchReader;
use crate::holder::Buffer;
use crate::media::{IORecordOptions, RecordOptions};
use crate::{ArrowWriteSession, IOBase, IOMedia};
use crate::{DataType, Error, Field, IOMode, MimeType, Scalar, Url};

fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

fn batch() -> RecordBatch {
    RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap()
}

/// The batches a write takes: one reader over one two-row batch.
fn reader() -> BatchReader {
    crate::arrow::batch_reader(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        [batch()],
    )
}

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

/// The total row count a handle currently holds.
fn rows(handle: &impl IOBase, options: &RecordOptions) -> usize {
    handle
        .read_arrow_reader(options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum()
}

fn rows_batch(ids: &[i64]) -> RecordBatch {
    RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(
                ids.iter().map(|_| Some("S")).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap()
}

/// A byte handle that observes each complete encoded publication.
struct PublicationProbe {
    handle: Buffer,
    publications: Arc<AtomicUsize>,
    source_pulls: Arc<AtomicUsize>,
    pulls_when_published: Arc<Mutex<Vec<usize>>>,
    destination_touches: Arc<AtomicUsize>,
    fail_publication: Option<usize>,
}

impl PublicationProbe {
    fn new(name: &str, source_pulls: Arc<AtomicUsize>) -> Self {
        Self {
            handle: handle(name),
            publications: Arc::new(AtomicUsize::new(0)),
            source_pulls,
            pulls_when_published: Arc::new(Mutex::new(Vec::new())),
            destination_touches: Arc::new(AtomicUsize::new(0)),
            fail_publication: None,
        }
    }

    fn reset_publications(&self) {
        self.publications.store(0, Ordering::SeqCst);
        self.pulls_when_published.lock().unwrap().clear();
    }

    fn fail_on_publication(&mut self, publication: usize) {
        self.fail_publication = Some(publication);
    }
}

impl crate::IOMedia for PublicationProbe {
    crate::impl_default_iomedia!();
}

impl IOBase for PublicationProbe {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.handle.pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.handle.pwrite(offset, bytes)
    }

    fn size(&self) -> u64 {
        self.handle.size()
    }

    fn capacity(&self) -> u64 {
        self.handle.capacity()
    }

    fn reserve(&mut self, capacity: u64) -> crate::Result<()> {
        self.handle.reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.handle.truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn media_type(&self) -> &crate::MediaType {
        self.handle.media_type()
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.handle.set_media_type(media_type);
    }

    fn kind(&self) -> crate::IOKind {
        self.destination_touches.fetch_add(1, Ordering::SeqCst);
        crate::IOKind::Memory
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> crate::Result<()> {
        let publication = self.publications.fetch_add(1, Ordering::SeqCst) + 1;
        self.pulls_when_published
            .lock()
            .unwrap()
            .push(self.source_pulls.load(Ordering::SeqCst));
        if self.fail_publication == Some(publication) {
            return Err(crate::Error::Io(std::io::Error::other(format!(
                "publication {publication} refused"
            ))));
        }
        self.handle.write_all_bytes(bytes)
    }
}

/// A fallible source whose exact pull frontier is observable.
struct CountedSource {
    schema: SchemaRef,
    batches: std::collections::VecDeque<std::result::Result<RecordBatch, ArrowError>>,
    pulls: Arc<AtomicUsize>,
}

impl Iterator for CountedSource {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.batches.pop_front()?;
        self.pulls.fetch_add(1, Ordering::SeqCst);
        Some(item)
    }
}

impl RecordBatchReader for CountedSource {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

fn counted_source(
    pulls: Arc<AtomicUsize>,
    batches: impl IntoIterator<Item = std::result::Result<RecordBatch, ArrowError>>,
) -> BatchReader {
    Box::new(CountedSource {
        schema: crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        batches: batches.into_iter().collect(),
        pulls,
    })
}

#[derive(Clone)]
struct NativeRow {
    id: i64,
    symbol: Option<&'static str>,
}

impl From<NativeRow> for Scalar {
    fn from(row: NativeRow) -> Self {
        Scalar::from_sequence([
            Scalar::from(row.id),
            row.symbol.map_or(Scalar::Null, Scalar::from),
        ])
    }
}

mod dispatch;
mod pushdown;
mod rows;
mod write;
