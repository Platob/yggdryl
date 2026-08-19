//! Arrow IPC streams over any byte handle.
//!
//! The encoding lives in free functions - [`read_field`], [`read_batch_reader`],
//! [`write_batch_reader`] - that take any [`IOBase`] handle and one
//! [`IpcOptions`]. That is what [`IOBase::read_arrow_batch_reader`] and its two
//! siblings call, so reading an IPC stream needs nothing but a handle whose
//! media type says `arrow.stream`. Streaming is the only shape here: a read
//! returns a [`BatchReader`] and a write consumes one, never a collected
//! vector. These functions are the encoding and nothing more - the `field` they
//! take is a column pushdown, and the casting, merging, and partition routing a
//! caller sees belong to [`IOBase`]'s three record methods above them.
//!
//! [`Ipc`] is the stateful form of the same thing: it owns the handle and the
//! options, and caches the stream's schema between calls. [`IOBase::open`]
//! fills that cache and [`IOBase::close`] releases it, which is what a scoped
//! context binds to in the bindings.
//!
//! Content coding comes from the handle's media type, so a handle named
//! `trades.arrows.gz` round-trips compressed with no extra argument.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{Int64Array, RecordBatch};
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::ipc::Ipc;
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // A struct field is the schema of the batches it describes.
//! let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
//!     .required_field("row");
//! let arrow_schema = schema.to_arrow_schema()?;
//! let batch = RecordBatch::try_new(
//!     Arc::clone(&arrow_schema),
//!     vec![Arc::new(Int64Array::from(vec![1, 2]))],
//! )?;
//!
//! // The name carries the coding, so the stream round-trips compressed.
//! let handle =
//!     Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
//! let mut media = Ipc::new(handle).with_schema(schema.clone());
//!
//! media.write_batch_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]))?;
//! assert_eq!(media.read_batch_reader(None)?.count(), 1);
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use arrow_array::RecordBatchIterator;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::Schema;

use crate::Field;
use crate::Level;
use crate::arrow::{
    BatchReader, Result, from_reader_error, projection_indices, record_schema_from_arrow,
    schema_from_field,
};
use crate::generic::IORecordOptions;
use crate::io::IOBase;
use smol_str::SmolStr;

/// The default root Field name used when a stream's schema is inferred.
pub const DEFAULT_ROOT_NAME: &str = "row";

/// The settings an Arrow IPC read or write takes.
///
/// IPC adds nothing to the shared settings: a stream carries its own schema,
/// and its content coding comes from the handle.
#[derive(Clone, Debug)]
pub struct IpcOptions {
    /// Declared canonical schema; inferred from the stream when absent.
    pub schema: Option<Field>,
    /// Root Field name used for an inferred schema.
    pub root_name: SmolStr,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per batch, when a reader should bound them.
    pub batch_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// Column names forming a write's match key; empty means overwrite.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to; empty selects everything.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by; empty keeps all.
    pub filter_partitions: Vec<(String, String)>,
    /// The predicate a read is pruned and filtered by.
    ///
    /// This is the general form of `filter_partitions`, which stays as the
    /// sugar that builds one equality per pair.
    pub filter: Option<crate::Expr>,
    /// The projection a read or write is narrowed to.
    ///
    /// This is the general form of `select_by_names`, which stays as the sugar
    /// for a projection of bare columns.
    pub selection: Option<crate::Selection>,
}

impl IpcOptions {
    /// Build the default IPC options.
    pub fn new() -> Self {
        Self {
            schema: None,
            root_name: SmolStr::new_static(DEFAULT_ROOT_NAME),
            safe: false,
            batch_size: None,
            level: Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
            filter: None,
            selection: None,
        }
    }
}

impl Default for IpcOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for IpcOptions {
    crate::record_options_fields!();
}

/// Read the schema of the stream `handle` holds.
///
/// # Errors
///
/// Returns a read, decoding, or schema failure.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &IpcOptions) -> Result<Field> {
    if let Some(schema) = options.schema() {
        return Ok(schema.clone());
    }
    let decoded = decoded_bytes(handle)?;
    let reader = StreamReader::try_new(std::io::Cursor::new(decoded), None)?;
    record_schema_from_arrow(options.root_name(), reader.schema().as_ref())
}

/// Read the stream `handle` holds, keeping only the columns `field` names.
///
/// Batches are returned exactly as they were written, without being cast to a
/// declared schema. A `field` naming a subset of the stored columns is pushed
/// into the decoder as an Arrow IPC projection, so the columns it leaves out are
/// never turned into arrays. Be precise about what that saves: an IPC record
/// batch is one contiguous message, so its body is still read off the handle
/// whole - the projection removes the decode and the allocation, not the bytes.
/// Parquet, whose column chunks are addressable, is where a projection also
/// removes reading. A `field` naming anything the stream does not carry is
/// ignored, because a projection can only drop columns, never invent them.
///
/// # Errors
///
/// Returns a read or decoding failure.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &IpcOptions,
) -> Result<BatchReader> {
    let decoded = decoded_bytes(handle)?;
    if decoded.is_empty() {
        // Per the laziness contract, a missing stream holds no batches.
        let schema = match options.schema() {
            Some(schema) => schema_from_field(schema)?,
            None => Arc::new(Schema::empty()),
        };
        let schema = match field.and_then(|field| projection_indices(field, &schema)) {
            Some(indices) => Arc::new(schema.project(&indices)?),
            None => schema,
        };
        return Ok(Box::new(RecordBatchIterator::new(
            std::iter::empty(),
            schema,
        )));
    }

    let stored = StreamReader::try_new(std::io::Cursor::new(decoded.as_slice()), None)?.schema();
    let Some(indices) = field.and_then(|field| projection_indices(field, &stored)) else {
        return Ok(Box::new(StreamReader::try_new(
            std::io::Cursor::new(decoded),
            None,
        )?));
    };
    let projected = Arc::new(stored.project(&indices)?);
    let reader = StreamReader::try_new(std::io::Cursor::new(decoded), Some(indices))?;
    // A projected `StreamReader` yields projected batches but still reports the
    // full stream schema, so the projected one is restated here rather than
    // leaving a reader whose schema disagrees with its batches.
    Ok(Box::new(RecordBatchIterator::new(reader, projected)))
}

/// Replace the stream `handle` holds with every batch `batches` yields.
///
/// The reader's own schema is what the stream declares, so a caller replacing a
/// stream with a declared root builds the reader over that root's Arrow
/// projection.
///
/// # Errors
///
/// Returns a schema, encoding, or write failure.
pub fn write_batch_reader<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &IpcOptions,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    let schema = batches.schema();
    let mut stream = Vec::new();
    let mut writer = StreamWriter::try_new(&mut stream, schema.as_ref())?;
    for batch in batches {
        writer.write(&batch.map_err(from_reader_error)?)?;
    }
    writer.finish()?;
    store(handle, &stream, options.level())
}

/// Read a handle's bytes with any declared content coding removed.
fn decoded_bytes<H: IOBase + ?Sized>(handle: &H) -> Result<Vec<u8>> {
    let bytes = handle.read_all()?;
    if bytes.is_empty() {
        return Ok(bytes);
    }
    Ok(handle.codec().load(&bytes)?)
}

/// Apply the handle's content coding and replace its bytes.
fn store<H: IOBase + ?Sized>(handle: &mut H, stream: &[u8], level: Level) -> Result<()> {
    let encoded = handle.codec().dump_with_level(stream, level)?;
    handle.write_all_bytes(&encoded)?;
    Ok(())
}

/// An Arrow IPC stream bound to one [`IOBase`] handle.
///
/// Every read and write goes through this type, so the handle, the options,
/// and the cached schema live in one place rather than being repeated at each
/// call.
#[derive(Debug)]
pub struct Ipc<H: IOBase> {
    handle: H,
    options: IpcOptions,
    /// Schema cached by `open`, discarded by `close` or replaced by a write.
    cached_schema: Option<Field>,
}

impl<H: IOBase> Ipc<H> {
    /// Bind an IPC stream to a handle.
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: IpcOptions::new(),
            cached_schema: None,
        }
    }

    /// Return this stream with different options.
    #[must_use]
    pub fn with_options(mut self, options: IpcOptions) -> Self {
        self.options = options;
        self.cached_schema = None;
        self
    }

    /// Return this stream with an explicit canonical schema.
    ///
    /// Reads validate against it instead of inferring one, and writes use it.
    #[must_use]
    pub fn with_schema(mut self, schema: Field) -> Self {
        self.options.set_schema(schema);
        self.cached_schema = None;
        self
    }

    /// Return this stream with a different inferred-root Field name.
    #[must_use]
    pub fn with_root_name(mut self, root_name: impl Into<smol_str::SmolStr>) -> Self {
        self.options.set_root_name(root_name.into());
        self.cached_schema = None;
        self
    }

    /// Return this stream with a different compression level.
    #[must_use]
    pub fn with_level(mut self, level: Level) -> Self {
        self.options.set_level(level);
        self
    }

    /// Borrow the options this stream reads and writes with.
    pub const fn options(&self) -> &IpcOptions {
        &self.options
    }

    /// Borrow the options mutably.
    pub const fn options_mut(&mut self) -> &mut IpcOptions {
        &mut self.options
    }

    /// Borrow the underlying handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying handle mutably.
    pub const fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Consume the stream and return its handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Return the stream's canonical schema.
    ///
    /// A declared schema is returned as-is. An open stream answers from the
    /// cache [`IOBase::open`] filled; a closed one reads the stream fresh
    /// every time, because a cache nobody asked for is how a handle serves a
    /// stale schema after the resource changes underneath it. The scoped pair
    /// is how a caller opts into retention.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or schema failure.
    pub fn schema(&self) -> Result<Field> {
        if let Some(schema) = self.options.schema() {
            return Ok(schema.clone());
        }
        if let Some(cached) = &self.cached_schema {
            return Ok(cached.clone());
        }
        read_field(&self.handle, &self.options)
    }

    /// Read the stream, keeping only the columns `field` names.
    ///
    /// # Errors
    ///
    /// Returns a read or decoding failure.
    pub fn read_batch_reader(&self, field: Option<&Field>) -> Result<BatchReader> {
        read_batch_reader(&self.handle, field, &self.options)
    }

    /// Replace the stream with every batch `batches` yields.
    ///
    /// # Errors
    ///
    /// Returns a schema, encoding, or write failure.
    pub fn write_batch_reader(&mut self, batches: BatchReader) -> Result<()> {
        let written = batches.schema();
        write_batch_reader(&mut self.handle, batches, &self.options)?;
        // The batches just written are the stream's schema, so the cache is
        // refreshed rather than dropped. A schema that will not project back
        // into a Field drops the cache instead of failing a completed write -
        // the next read simply derives it again.
        self.cached_schema =
            record_schema_from_arrow(self.options.root_name(), written.as_ref()).ok();
        Ok(())
    }
}

/// An `Ipc` mirrors the bytes of the handle it owns, so a caller can reach the
/// raw stream - to copy it, compress it, or hand it to another reader - without
/// unwrapping the media type first.
///
/// [`IOBase::open`] additionally caches the stream's schema and
/// [`IOBase::close`] releases it.
impl<H: IOBase> IOBase for Ipc<H> {
    crate::delegate_iobase!(handle);

    /// Materialize the handle and cache the stream's schema.
    ///
    /// Repeated reads then reuse the cached schema instead of re-deriving it.
    /// Opening an empty or missing stream succeeds and caches nothing.
    fn open(&mut self) -> crate::Result<()> {
        self.handle.open()?;
        if self.cached_schema.is_none() && !self.handle.is_empty() {
            self.cached_schema = Some(read_field(&self.handle, &self.options)?);
        }
        Ok(())
    }

    /// Return whether a schema is currently cached.
    fn is_open(&self) -> bool {
        self.cached_schema.is_some()
    }

    /// Flush the handle and drop the cached schema.
    fn close(&mut self) -> crate::Result<()> {
        self.cached_schema = None;
        self.handle.close()
    }
}

#[cfg(test)]
mod tests;
