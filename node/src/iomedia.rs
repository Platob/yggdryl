//! Arrow record batches crossing the JavaScript boundary as Arrow IPC.
//!
//! Arrow JS has no C Data consumer, so the boundary here is the standard copied
//! one: a batch leaves Rust as a self-contained Arrow IPC stream and arrives
//! back the same way. The stream shape is what the core already uses -
//! [`yggdryl::arrow::BatchReader`] is one batch at a time, never a materialized
//! table - so [`JsBatchReader`] is one-shot in JavaScript too: reading it or
//! handing it to a write consumes it.

use std::io::Cursor;
use std::sync::Arc;
use std::thread::ThreadId;

use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{ArrowError, SchemaRef};
use napi::bindgen_prelude::{Buffer, Env, Function, FunctionRef, Result, Uint8Array};
use napi_derive::napi;
use yggdryl::arrow::BatchReader;

use crate::media::iceberg::{FieldInput, field_from_input};
use yggdryl::media::DEFAULT_ROOT_NAME;

use crate::napi_error;
use crate::types::field::JsField;

/// Report a stream someone else has already read.
fn consumed() -> napi::Error {
    napi_error("this BatchReader has already been consumed; a stream is read once")
}

/// Encode `batches` as one Arrow IPC stream under `schema`.
fn encoded(schema: &SchemaRef, batches: &[RecordBatch]) -> Result<Buffer> {
    let mut writer = StreamWriter::try_new(Vec::new(), schema.as_ref()).map_err(napi_error)?;
    for batch in batches {
        writer.write(batch).map_err(napi_error)?;
    }
    writer.finish().map_err(napi_error)?;
    Ok(writer.into_inner().map_err(napi_error)?.into())
}

/// A native reader that asks JavaScript for one bounded IPC chunk at a time.
///
/// Arrow JS has no synchronous C Data stream. A bound JavaScript pull function
/// is therefore the narrow bridge that keeps record iterables lazy without
/// publishing one core write per chunk. It is called only on the isolate thread
/// that supplied it, exactly like the caller-supplied Arrow file-system vtable.
struct IpcPullReader {
    current: BatchReader,
    schema: SchemaRef,
    pull: FunctionRef<(), Option<Uint8Array>>,
    environment: usize,
    thread: ThreadId,
    finished: bool,
}

impl IpcPullReader {
    fn error(reason: impl Into<String>) -> ArrowError {
        ArrowError::ExternalError(Box::new(std::io::Error::other(reason.into())))
    }

    /// Ask for the next IPC chunk, copying its bytes before returning to Rust.
    fn pull(&self) -> std::result::Result<Option<Vec<u8>>, ArrowError> {
        if std::thread::current().id() != self.thread {
            return Err(Self::error(
                "a JavaScript record iterable can only be pulled on the isolate thread that supplied it",
            ));
        }
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        self.pull
            .borrow_back(&env)
            .and_then(|pull| pull.call(()))
            .map(|bytes| bytes.map(|bytes| bytes.to_vec()))
            .map_err(|error| Self::error(error.reason))
    }
}

impl Iterator for IpcPullReader {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            if let Some(batch) = self.current.next() {
                if batch.is_err() {
                    self.finished = true;
                }
                return Some(batch);
            }
            let bytes = match self.pull() {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    self.finished = true;
                    return None;
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(error));
                }
            };
            let next = match JsBatchReader::decoded(&bytes, DEFAULT_ROOT_NAME) {
                Ok(mut reader) => match reader.take() {
                    Ok(reader) => reader,
                    Err(error) => {
                        self.finished = true;
                        return Some(Err(Self::error(error.reason)));
                    }
                },
                Err(error) => {
                    self.finished = true;
                    return Some(Err(Self::error(error.reason)));
                }
            };
            if next.schema().as_ref() != self.schema.as_ref() {
                self.finished = true;
                return Some(Err(ArrowError::SchemaError(
                    "a streamed JavaScript record chunk inferred a different Arrow schema; declare options.field or use one stable record shape"
                        .to_owned(),
                )));
            }
            self.current = next;
        }
    }
}

impl RecordBatchReader for IpcPullReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// A one-shot stream of Arrow record batches.
///
/// The reader is the record shape at every level of this package: a read
/// returns one and a write consumes one. Each batch crosses as its own Arrow
/// IPC stream, which is what `apache-arrow` reads without a copy protocol of
/// its own.
#[napi(js_name = "BatchReader")]
pub struct JsBatchReader {
    /// The undrained core reader, taken by whatever consumes it.
    inner: Option<BatchReader>,
    /// The schema the reader declares, kept after the reader is taken.
    schema: SchemaRef,
    /// The root Field name an inferred schema is named by.
    root_name: String,
    /// Whether something else took the reader rather than draining it here.
    taken: bool,
}

impl JsBatchReader {
    pub(crate) fn from_core(inner: BatchReader, root_name: &str) -> Self {
        Self {
            schema: inner.schema(),
            inner: Some(inner),
            root_name: root_name.to_owned(),
            taken: false,
        }
    }

    /// Take the core reader, leaving the schema behind.
    ///
    /// A reader is a stream, so a second consumer gets the error rather than an
    /// empty stream that looks like a resource holding no rows.
    pub(crate) fn take(&mut self) -> Result<BatchReader> {
        let reader = self.inner.take().ok_or_else(consumed)?;
        self.taken = true;
        Ok(reader)
    }

    /// Build a reader over the batches an Arrow IPC stream carries.
    pub(crate) fn decoded(bytes: &[u8], root_name: &str) -> Result<Self> {
        if bytes.is_empty() {
            let schema = Arc::new(arrow_schema::Schema::empty());
            let empty: BatchReader = Box::new(RecordBatchIterator::new(std::iter::empty(), schema));
            return Ok(Self::from_core(empty, root_name));
        }
        let reader =
            StreamReader::try_new(Cursor::new(bytes.to_vec()), None).map_err(napi_error)?;
        Ok(Self::from_core(Box::new(reader), root_name))
    }
}

#[napi]
impl JsBatchReader {
    /// Read the batches an Arrow IPC stream holds.
    #[napi(factory)]
    pub fn from_ipc(bytes: Uint8Array, root_name: Option<String>) -> Result<Self> {
        Self::decoded(&bytes, root_name.as_deref().unwrap_or(DEFAULT_ROOT_NAME))
    }

    /// Continue this first IPC chunk from a bounded JavaScript pull function.
    ///
    /// The records loader captures and removes this private bridge. Keeping the
    /// callback behind one native `BatchReader` preserves one core write and
    /// one publication while a synchronous JavaScript iterable remains lazy.
    #[napi(js_name = "_chainIpcPullNative", skip_typescript)]
    pub fn chain_ipc_pull(
        &mut self,
        env: Env,
        pull: Function<'_, (), Option<Uint8Array>>,
    ) -> Result<Self> {
        let root_name = self.root_name.clone();
        let current = self.take()?;
        let schema = current.schema();
        let reader = IpcPullReader {
            current,
            schema,
            pull: pull.create_ref()?,
            environment: env.raw().expose_provenance(),
            thread: std::thread::current().id(),
            finished: false,
        };
        Ok(Self::from_core(Box::new(reader), &root_name))
    }

    /// The canonical non-null struct root `Field` these batches describe.
    #[napi(getter)]
    pub fn field(&self) -> Result<JsField> {
        yggdryl::Field::from_arrow_schema(&self.root_name, self.schema.as_ref())
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Return whether the stream has been read or handed to a write.
    #[napi(getter)]
    pub fn consumed(&self) -> bool {
        self.inner.is_none()
    }

    /// Pull the next batch as its own Arrow IPC stream, or `null` at the end.
    ///
    /// This is the native half of the iteration protocol; the loader wraps it so
    /// `for...of` yields Apache Arrow JS record batches.
    #[napi(js_name = "_nextIpcNative", skip_typescript)]
    pub fn next_ipc(&mut self) -> Result<Option<Buffer>> {
        if self.taken {
            // Draining this reader here ends it quietly; a write that took it
            // ended it somewhere else, and iterating that reads as no rows
            // unless it says so.
            return Err(consumed());
        }
        let Some(reader) = self.inner.as_mut() else {
            return Ok(None);
        };
        let Some(batch) = reader.next() else {
            self.inner = None;
            return Ok(None);
        };
        let batch = batch.map_err(napi_error)?;
        encoded(&self.schema, std::slice::from_ref(&batch)).map(Some)
    }

    /// Chain this reader and `other` onto the root their schemas merge into.
    ///
    /// Both readers are consumed, and the result stays lazy: the merge is
    /// derived from the two schemas alone, which a reader answers without
    /// pulling a batch.
    ///
    /// Columns unite by name (ASCII case-insensitively), this reader's order
    /// first and the other's extra columns after; a column present in only one
    /// side becomes nullable, because the other side's rows have no value for
    /// it; a shared column whose datatype or `PARQUET:field_id` disagrees is
    /// refused naming both sides rather than silently widened. Passing `schema`
    /// declares the root both sides cast onto instead of deriving one.
    #[napi]
    pub fn combined(
        &mut self,
        other: &mut JsBatchReader,
        schema: Option<FieldInput<'_>>,
        safe: Option<bool>,
    ) -> Result<Self> {
        let left = self.take()?;
        let right = other.take()?;
        let root_name = self.root_name.clone();
        let reader = match schema {
            Some(schema) => {
                let root = field_from_input(schema)?;
                yggdryl::arrow::combined_as(left, right, &root, safe.unwrap_or(true))
                    .map_err(napi_error)?
            }
            None => yggdryl::arrow::combined(left, right).map_err(napi_error)?,
        };
        Ok(Self::from_core(reader, &root_name))
    }

    /// Drain every remaining batch into one Arrow IPC stream.
    #[napi]
    pub fn into_ipc(&mut self) -> Result<Buffer> {
        let reader = self.take()?;
        let mut batches = Vec::new();
        for batch in reader {
            batches.push(batch.map_err(napi_error)?);
        }
        encoded(&self.schema, &batches)
    }
}
