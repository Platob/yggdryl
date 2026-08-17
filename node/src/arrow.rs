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

use arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::SchemaRef;
use napi::bindgen_prelude::{Buffer, Result, Uint8Array};
use napi_derive::napi;
use yggdryl::arrow::{BatchReader, record_schema_from_arrow};
use yggdryl::ipc::DEFAULT_ROOT_NAME;

use crate::field::JsField;
use crate::napi_error;

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

    /// The canonical non-null struct root `Field` these batches describe.
    #[napi(getter)]
    pub fn field(&self) -> Result<JsField> {
        record_schema_from_arrow(&self.root_name, self.schema.as_ref())
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

    /// Drain every remaining batch into one Arrow IPC stream.
    #[napi]
    pub fn to_ipc(&mut self) -> Result<Buffer> {
        let reader = self.take()?;
        let mut batches = Vec::new();
        for batch in reader {
            batches.push(batch.map_err(napi_error)?);
        }
        encoded(&self.schema, &batches)
    }
}
