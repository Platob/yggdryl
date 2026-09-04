//! Arrow IPC streams over any byte handle.
//!
//! The encoding lives in free functions - [`read_field`], [`read_batch_reader`],
//! [`overwrite_arrow_reader`] - that take any [`IOBase`] handle and one
//! [`IpcOptions`]. That is what [`crate::IOMedia::read_arrow_reader`] and its
//! three write siblings call, so reading an IPC stream needs nothing but a handle whose
//! media type says `arrow.stream`. Streaming is the only shape here: a read
//! returns a [`BatchReader`] and a write consumes one, never a collected
//! vector. These functions are the encoding and nothing more - the `field` they
//! take is a column pushdown, and the casting, merging, and partition routing a
//! caller sees belong to [`crate::IOMedia`]'s record methods above them.
//!
//! [`Ipc`] is the stateful form of the same thing: it owns the handle and the
//! options, and caches the stream's schema and dimensions between calls.
//! [`IOBase::open`] fills those caches and [`IOBase::close`] releases them,
//! which is what a scoped context binds to in the bindings.
//!
//! Content coding comes from the handle's media type, so a handle named
//! `trades.arrows.gz` round-trips compressed with no extra argument.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{Int64Array, RecordBatch};
//! use yggdryl::{IOBase, IOMedia, holder::Buffer};
//! use yggdryl::ipc::Ipc;
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // A struct field is the schema of the batches it describes.
//! let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
//!     .required_field("row");
//! let arrow_schema = schema.clone().into_arrow_schema()?;
//! let batch = RecordBatch::try_new(
//!     Arc::clone(&arrow_schema),
//!     vec![Arc::new(Int64Array::from(vec![1, 2]))],
//! )?;
//!
//! // The name carries the coding, so the stream round-trips compressed.
//! let handle =
//!     Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
//! let mut media = Ipc::new(handle).with_field(schema.clone());
//!
//! let options = media.record_options()?;
//! media.overwrite_arrow_reader(
//!     yggdryl::arrow::batch_reader(arrow_schema, [batch]),
//!     &options,
//! )?;
//! assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
//! # Ok(())
//! # }
//! ```

use std::io::{BufRead, BufReader, Read};
use std::sync::{Arc, OnceLock};

use arrow_array::RecordBatchIterator;
use arrow_ipc::MessageHeader;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{ArrowError, Schema};

use crate::IOBase;
use crate::Level;
use crate::arrow::{
    BatchReader, Result, arrow_schema_from_field, field_from_arrow_schema, from_reader_error,
    projection_indices,
};
use crate::generic::{IORecordOptions, RecordOptions};
use crate::{DataType, Field, Metadata};
use smol_str::SmolStr;

/// The settings an Arrow IPC read or write takes.
///
/// IPC adds nothing to the shared settings: a stream carries its own schema,
/// and its content coding comes from the handle.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IpcOptions {
    /// Root Field name; [`DEFAULT_ROOT_NAME`](crate::generic::DEFAULT_ROOT_NAME) unless set.
    pub name: SmolStr,
    /// Declared root datatype; inferred from the stream when absent.
    pub dtype: Option<DataType>,
    /// Root metadata; empty unless declared.
    pub metadata: Metadata,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per batch, when a reader should bound them.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total - a count of rows, not a per-row byte cap.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// Column names forming a write's match key; empty means overwrite.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to; empty selects everything.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by; empty keeps all.
    pub filter_partitions: Vec<(String, String)>,
}

impl IpcOptions {
    /// Build the default IPC options.
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::generic::DEFAULT_ROOT_NAME),
            dtype: None,
            metadata: Metadata::new(),
            safe: false,
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
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
    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    let schema = read_schema(handle)?
        .ok_or_else(|| ipc_metadata_error("an IPC stream has no schema message"))?;
    field_from_arrow_schema(options.name(), &schema)
}

/// Count the rows in an IPC stream from message metadata alone.
///
/// Record-batch and dictionary bodies are skipped by their declared byte
/// lengths; no Arrow arrays are constructed. An uncoded positional handle does
/// not read those body ranges at all. An outer content coding must decompress
/// them to advance its stream, but discards each bounded chunk immediately.
pub(crate) fn row_size<H: IOBase + ?Sized>(
    handle: &H,
    _options: &IpcOptions,
) -> crate::Result<u64> {
    Ok(read_metadata(handle)?.rows)
}

/// Schema and logical row count carried by the IPC message metadata.
#[derive(Debug, Default)]
struct IpcMetadata {
    schema: Option<Schema>,
    rows: u64,
}

/// How far a metadata pass must advance through the stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetadataScan {
    /// Return immediately after the schema message metadata is parsed.
    SchemaOnly,
    /// Visit every message so record-batch row counts are available.
    All,
}

/// Read only the stream schema through a bounded decoded byte stream.
///
/// This deliberately uses `pstream_bytes` for identity storage too. A page
/// cache is valuable for random access, but retaining its first page merely to
/// inspect an IPC schema would make a metadata query populate state nobody
/// asked to retain.
fn read_schema<H: IOBase + ?Sized>(handle: &H) -> Result<Option<Schema>> {
    let mut source = handle.pstream_bytes(0, crate::DEFAULT_STREAM_BATCH_SIZE)?;
    if handle.codec().is_identity() {
        return Ok(read_metadata_from(StreamInput::new(source), MetadataScan::SchemaOnly)?.schema);
    }

    // An absent resource is an empty stream, while an empty byte slice is not
    // valid gzip/zlib/Zstandard framing. Pull one normal transport window and
    // replay it into the decoder, without a size request or retained page.
    let Some(prefix) = source.next().transpose()? else {
        return Ok(None);
    };
    let decoded =
        handle
            .codec()
            .reader(std::io::Cursor::new(prefix).chain(BufReader::with_capacity(
                crate::DEFAULT_STREAM_BATCH_SIZE,
                source,
            )));
    Ok(read_metadata_from(StreamInput::new(decoded), MetadataScan::SchemaOnly)?.schema)
}

/// Read metadata through the cheapest input the handle permits.
fn read_metadata<H: IOBase + ?Sized>(handle: &H) -> Result<IpcMetadata> {
    if handle.codec().is_identity() {
        return read_metadata_from(HandleInput::new(handle), MetadataScan::All);
    }
    let mut source = handle.pstream_bytes(0, crate::DEFAULT_STREAM_BATCH_SIZE)?;
    let Some(prefix) = source.next().transpose()? else {
        return Ok(IpcMetadata::default());
    };
    read_metadata_from(
        StreamInput::new(handle.codec().reader(std::io::Cursor::new(prefix).chain(
            BufReader::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE, source),
        ))),
        MetadataScan::All,
    )
}

/// Parse message FlatBuffers and advance over each body without decoding it.
fn read_metadata_from(mut input: impl MetadataInput, scan: MetadataScan) -> Result<IpcMetadata> {
    let mut metadata = IpcMetadata::default();
    let mut saw_framing = false;
    while let Some(mut metadata_length) = read_stream_u32(&mut input)? {
        saw_framing = true;
        if metadata_length == u32::MAX {
            metadata_length = read_stream_u32(&mut input)?
                .ok_or_else(|| ipc_metadata_error("truncated IPC continuation-marker prefix"))?;
        }
        if metadata_length == 0 {
            break;
        }

        let metadata_length = metadata_length as usize;
        let mut message_bytes = Vec::new();
        message_bytes
            .try_reserve_exact(metadata_length)
            .map_err(|source| {
                crate::arrow::Error::allocation("IPC metadata bytes", metadata_length, source)
            })?;
        message_bytes.resize(metadata_length, 0);
        input
            .read_exact_or_eof(&mut message_bytes)?
            .then_some(())
            .ok_or_else(|| ipc_metadata_error("IPC message metadata ends past the stream"))?;
        let message = arrow_ipc::root_as_message(&message_bytes).map_err(|error| {
            ipc_metadata_error(format!("invalid IPC message metadata: {error}"))
        })?;

        let body_length = usize::try_from(message.bodyLength())
            .map_err(|_| ipc_metadata_error("IPC message body length is negative"))?;

        match message.header_type() {
            MessageHeader::Schema => {
                if metadata.schema.is_some() {
                    return Err(ipc_metadata_error(
                        "an IPC stream cannot declare its schema more than once",
                    ));
                }
                let schema = message.header_as_schema().ok_or_else(|| {
                    ipc_metadata_error("an IPC schema message has no schema header")
                })?;
                metadata.schema = Some(arrow_ipc::convert::fb_to_schema(schema));
                if scan == MetadataScan::SchemaOnly {
                    return Ok(metadata);
                }
            }
            MessageHeader::RecordBatch => {
                if metadata.schema.is_none() {
                    return Err(ipc_metadata_error(
                        "an IPC record batch appeared before the stream schema",
                    ));
                }
                let batch = message.header_as_record_batch().ok_or_else(|| {
                    ipc_metadata_error("an IPC record-batch message has no record-batch header")
                })?;
                let batch_rows = u64::try_from(batch.length())
                    .map_err(|_| ipc_metadata_error("an IPC record batch has negative length"))?;
                metadata.rows = metadata
                    .rows
                    .checked_add(batch_rows)
                    .ok_or_else(|| ipc_metadata_error("the IPC row count exceeds u64::MAX"))?;
            }
            MessageHeader::DictionaryBatch => {
                if metadata.schema.is_none() {
                    return Err(ipc_metadata_error(
                        "an IPC dictionary appeared before the stream schema",
                    ));
                }
                if message.header_as_dictionary_batch().is_none() {
                    return Err(ipc_metadata_error(
                        "an IPC dictionary message has no dictionary header",
                    ));
                }
            }
            MessageHeader::NONE => {}
            other => {
                return Err(ipc_metadata_error(format!(
                    "unsupported IPC stream message type {other:?}"
                )));
            }
        }
        input.skip_exact(body_length)?;
    }

    if metadata.schema.is_none() && saw_framing {
        return Err(ipc_metadata_error("an IPC stream has no schema message"));
    }
    Ok(metadata)
}

/// Read one little-endian stream framing word without accepting truncation.
fn read_stream_u32(input: &mut impl MetadataInput) -> Result<Option<u32>> {
    let mut bytes = [0_u8; 4];
    if !input.read_exact_or_eof(&mut bytes)? {
        return Ok(None);
    }
    Ok(Some(u32::from_le_bytes(bytes)))
}

/// Keep framing failures in Arrow's typed IPC error channel.
fn ipc_metadata_error(reason: impl Into<String>) -> crate::arrow::Error {
    ArrowError::IpcError(reason.into()).into()
}

/// The two operations metadata parsing needs: one bounded read and one exact
/// skip. Identity storage implements `skip_exact` by moving its positional
/// offset; a decoded stream drains into one stack buffer.
trait MetadataInput {
    fn read_exact_or_eof(&mut self, bytes: &mut [u8]) -> Result<bool>;
    fn skip_exact(&mut self, length: usize) -> Result<()>;
}

/// Positional bytes over an `IOBase`, with a skip that performs no read.
struct HandleInput<'handle, H: IOBase + ?Sized> {
    handle: &'handle H,
    offset: u64,
}

impl<'handle, H: IOBase + ?Sized> HandleInput<'handle, H> {
    const fn new(handle: &'handle H) -> Self {
        Self { handle, offset: 0 }
    }
}

impl<H: IOBase + ?Sized> MetadataInput for HandleInput<'_, H> {
    fn read_exact_or_eof(&mut self, bytes: &mut [u8]) -> Result<bool> {
        let mut filled = 0_usize;
        while filled < bytes.len() {
            let read = self.handle.pread(self.offset, &mut bytes[filled..])?;
            if read == 0 {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(crate::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "truncated IPC stream value",
                ))
                .into());
            }
            filled += read;
            self.offset = self
                .offset
                .checked_add(read as u64)
                .ok_or_else(|| ipc_metadata_error("IPC stream offset exceeds u64::MAX"))?;
        }
        Ok(true)
    }

    fn skip_exact(&mut self, length: usize) -> Result<()> {
        let end = self
            .offset
            .checked_add(length as u64)
            .filter(|end| *end <= self.handle.size())
            .ok_or_else(|| ipc_metadata_error("IPC message body ends past the stream"))?;
        self.offset = end;
        Ok(())
    }
}

impl<H: IOBase + ?Sized> Read for HandleInput<'_, H> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let read = self
            .handle
            .pread(self.offset, bytes)
            .map_err(std::io::Error::other)?;
        self.offset = self
            .offset
            .checked_add(read as u64)
            .ok_or_else(|| std::io::Error::other("IPC stream offset exceeds u64::MAX"))?;
        Ok(read)
    }
}

/// A streaming content decoder. Skipping consumes bounded chunks because a
/// compressed stream cannot seek to a decoded offset.
struct StreamInput<R> {
    reader: R,
}

impl<R> StreamInput<R> {
    const fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R: Read> MetadataInput for StreamInput<R> {
    fn read_exact_or_eof(&mut self, bytes: &mut [u8]) -> Result<bool> {
        if bytes.is_empty() {
            return Ok(true);
        }
        let first = loop {
            match self.reader.read(bytes) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(crate::Error::Io(error).into()),
                Ok(read) => break read,
            }
        };
        if first == 0 {
            return Ok(false);
        }
        self.reader
            .read_exact(&mut bytes[first..])
            .map_err(crate::Error::Io)?;
        Ok(true)
    }

    fn skip_exact(&mut self, mut length: usize) -> Result<()> {
        let mut discarded = [0_u8; 8 * 1024];
        while length > 0 {
            let chunk = length.min(discarded.len());
            self.reader
                .read_exact(&mut discarded[..chunk])
                .map_err(crate::Error::Io)?;
            length -= chunk;
        }
        Ok(())
    }
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
    let stored = match field {
        Some(_) => read_schema(handle)?,
        // The Arrow reader consumes the schema from the same owned stream;
        // this marker avoids a separate absence probe when no projection asks
        // for the stored schema up front.
        None => Some(Schema::empty()),
    };
    if stored.is_none() {
        return empty_batch_reader(field, options);
    }

    finish_batch_reader(owned_decoded_reader(handle)?, stored, field, options)
}

/// Read from a handle the returned Arrow reader can own directly.
///
/// A decoding view uses this after capturing an unlocated encoded value, so
/// the captured bytes move into Arrow rather than being copied a second time.
pub(crate) fn read_owned_batch_reader<H: IOBase + 'static>(
    handle: H,
    field: Option<&Field>,
    options: &IpcOptions,
) -> Result<BatchReader> {
    let stored = match field {
        Some(_) => read_schema(&handle)?,
        None => Some(Schema::empty()),
    };
    if stored.is_none() {
        return empty_batch_reader(field, options);
    }
    let codec = handle.codec();
    let source = decoded_prefix_reader(codec, crate::Cursor::new(handle))?;
    finish_batch_reader(source, stored, field, options)
}

/// Finish projection and construct Arrow over an optional decoded source.
fn finish_batch_reader(
    source: Option<Box<dyn Read + Send>>,
    stored: Option<Schema>,
    field: Option<&Field>,
    options: &IpcOptions,
) -> Result<BatchReader> {
    let indices = field.and_then(|field| {
        stored
            .as_ref()
            .and_then(|stored| projection_indices(field, stored))
    });
    let Some(source) = source else {
        return empty_batch_reader(field, options);
    };
    let reader = StreamReader::try_new(source, indices.clone())?;
    let Some(indices) = indices else {
        return Ok(Box::new(reader));
    };
    let projected = Arc::new(
        stored
            .as_ref()
            .ok_or_else(|| ipc_metadata_error("an IPC stream has no schema message"))?
            .project(&indices)?,
    );
    // A projected `StreamReader` yields projected batches but still reports the
    // full stream schema, so the projected one is restated here rather than
    // leaving a reader whose schema disagrees with its batches.
    Ok(Box::new(RecordBatchIterator::new(reader, projected)))
}

/// Build the typed empty reader used for an absent IPC resource.
fn empty_batch_reader(field: Option<&Field>, options: &IpcOptions) -> Result<BatchReader> {
    let schema = match options.field() {
        Some(field) => arrow_schema_from_field(&field)?,
        None => Arc::new(Schema::empty()),
    };
    let schema = match field.and_then(|field| projection_indices(field, &schema)) {
        Some(indices) => Arc::new(schema.project(&indices)?),
        None => schema,
    };
    Ok(Box::new(RecordBatchIterator::new(
        std::iter::empty(),
        schema,
    )))
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
pub fn overwrite_arrow_reader<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &IpcOptions,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    let schema = batches.schema();
    let mut encoded = Vec::new();
    {
        let encoder = handle
            .codec()
            .writer_with_level(&mut encoded, options.level());
        let mut writer = StreamWriter::try_new(encoder, schema.as_ref())?;
        for batch in batches {
            writer.write(&batch.map_err(from_reader_error)?)?;
        }
        // Arrow finishes its EOS marker and returns the codec writer; the
        // codec then writes its own trailer. The encoded value is still staged
        // whole, so a failed batch never publishes a partial IPC resource.
        writer.into_inner()?.finish()?;
    }
    Ok(handle.write_all_bytes(&encoded)?)
}

/// Own a decoded stream so the returned Arrow reader can outlive this call.
///
/// Located resources are reopened lazily and retain only the decoder's window.
/// An unlocated handle (principally an in-memory buffer) snapshots its encoded
/// bytes because the public `BatchReader` is owning and `'static`; decoding is
/// still lazy, so compressed input never also allocates the full plain value.
fn owned_decoded_reader<H: IOBase + ?Sized>(handle: &H) -> Result<Option<Box<dyn Read + Send>>> {
    let codec = handle.codec();
    // A raw `trades.arrows.gz` holder can be reopened under the same coding.
    // An explicit `Coding` view presents decoded bytes while retaining that raw
    // URL, so reopening its child would silently replace the view with gzip
    // bytes mislabeled as identity; snapshot the presented stream in that case.
    let decoded_view_over_coded_url = codec.is_identity()
        && handle
            .url()
            .is_some_and(|url| !crate::Codec::from_url(url).is_identity());
    if !decoded_view_over_coded_url {
        if let Some(parent) = handle.parent() {
            if let Some(name) = handle.url().and_then(crate::Url::file_name) {
                let mut child = parent.child_by_path(name)?;
                child.set_media_type(handle.media_type().clone());
                return decoded_prefix_reader(codec, crate::Cursor::new(child));
            }
        }
    }
    let mut encoded = Vec::new();
    let mut source = handle.pstream_bytes(0, crate::DEFAULT_STREAM_BATCH_SIZE)?;
    loop {
        let start = encoded.len();
        let end = start
            .checked_add(crate::DEFAULT_STREAM_BATCH_SIZE)
            .ok_or_else(|| ipc_metadata_error("IPC stream snapshot exceeds addressable memory"))?;
        encoded.try_reserve_exact(end - start).map_err(|source| {
            crate::arrow::Error::allocation("IPC stream snapshot", end - start, source)
        })?;
        encoded.resize(end, 0);
        let read = source
            .read(&mut encoded[start..end])
            .map_err(crate::Error::Io)?;
        encoded.truncate(start + read);
        if read == 0 {
            break;
        }
    }
    decoded_prefix_reader(codec, std::io::Cursor::new(encoded))
}

/// Decode one owned source lazily and replay a useful prefix into Arrow.
///
/// The 64-byte decoded prefix is the absence test and the start of IPC framing
/// at once. `BufReader` gives every located encoded source the standard
/// transport window, so the decoder never creates a one-byte range request.
fn decoded_prefix_reader<R>(codec: crate::Codec, source: R) -> Result<Option<Box<dyn Read + Send>>>
where
    R: Read + Send + 'static,
{
    let mut reader = EmptySafeDecoder::new(codec, source);
    let mut prefix = [0_u8; 64];
    let mut filled = 0_usize;
    while filled < prefix.len() {
        match reader.read(&mut prefix[filled..]) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(crate::Error::Io(error).into()),
            Ok(0) => break,
            Ok(read) => filled += read,
        }
    }
    if filled == 0 {
        return Ok(None);
    }
    Ok(Some(Box::new(
        std::io::Cursor::new(prefix)
            .take(filled as u64)
            .chain(reader),
    )))
}

/// Delay decoder construction until the encoded transport proves non-empty.
struct EmptySafeDecoder<R> {
    codec: crate::Codec,
    source: Option<BufReader<R>>,
    decoder: Option<Box<dyn Read + Send>>,
}

impl<R: Read + Send + 'static> EmptySafeDecoder<R> {
    fn new(codec: crate::Codec, source: R) -> Self {
        Self {
            codec,
            source: Some(BufReader::with_capacity(
                crate::DEFAULT_STREAM_BATCH_SIZE,
                source,
            )),
            decoder: None,
        }
    }
}

impl<R: Read + Send + 'static> Read for EmptySafeDecoder<R> {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        if self.decoder.is_none() {
            let Some(mut source) = self.source.take() else {
                return Ok(0);
            };
            if source.fill_buf()?.is_empty() {
                return Ok(0);
            }
            self.decoder = Some(self.codec.reader_send(source));
        }
        self.decoder
            .as_mut()
            .map_or(Ok(0), |decoder| decoder.read(target))
    }
}

/// An Arrow IPC stream bound to one [`IOBase`] handle.
///
/// Every read and write goes through this type, so the handle, the options,
/// and the opened-session metadata caches live in one place rather than being
/// repeated at each call.
#[derive(Debug)]
pub struct Ipc<H: IOBase> {
    handle: H,
    options: IpcOptions,
    /// Whether the caller explicitly opened this media, including when empty.
    opened: bool,
    /// Schema cached only for an explicitly opened session.
    cached_schema: OnceLock<Field>,
    /// Metadata-only row count cached only for an explicitly opened session.
    cached_row_size: OnceLock<u64>,
    /// Canonical column count cached only for an explicitly opened session.
    cached_column_size: OnceLock<usize>,
}

impl<H: IOBase> Ipc<H> {
    /// Bind an IPC stream to a handle.
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: IpcOptions::new(),
            opened: false,
            cached_schema: OnceLock::new(),
            cached_row_size: OnceLock::new(),
            cached_column_size: OnceLock::new(),
        }
    }

    /// Return this stream with different options.
    #[must_use]
    pub fn with_options(mut self, options: IpcOptions) -> Self {
        self.options = options;
        self.invalidate_cached_metadata();
        self
    }

    /// Return this stream with an explicit canonical schema.
    ///
    /// Reads validate against it instead of inferring one, and writes use it.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self.invalidate_cached_metadata();
        self
    }

    /// Return this stream with a different root Field name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<smol_str::SmolStr>) -> Self {
        self.options.set_name(name.into());
        self.invalidate_cached_metadata();
        self
    }

    /// Return this stream with a different compression level.
    #[must_use]
    pub fn with_level(mut self, level: Level) -> Self {
        self.options.set_level(level);
        self.invalidate_cached_metadata();
        self
    }

    /// Borrow the options this stream reads and writes with.
    pub const fn options(&self) -> &IpcOptions {
        &self.options
    }

    /// Borrow the options mutably, invalidating opened-session metadata first.
    pub fn options_mut(&mut self) -> &mut IpcOptions {
        self.invalidate_cached_metadata();
        &mut self.options
    }

    /// Refuse options for a different encoding before a write can pull its
    /// first incoming batch.
    fn require_record_options<'a>(
        &self,
        options: &'a RecordOptions,
    ) -> crate::Result<&'a IpcOptions> {
        match options {
            RecordOptions::Ipc(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("Arrow IPC record options", options.mime_type()),
            }),
        }
    }

    /// Borrow the underlying handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying handle mutably, invalidating opened-session
    /// metadata before any byte mutation can occur.
    pub fn handle_mut(&mut self) -> &mut H {
        self.invalidate_cached_metadata();
        &mut self.handle
    }

    /// Consume the stream and return its handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Drop metadata derived from bytes or options without closing the handle.
    fn invalidate_cached_metadata(&mut self) {
        self.cached_schema.take();
        self.cached_row_size.take();
        self.cached_column_size.take();
    }

    /// Read schema and dimensions in one metadata pass for `open`.
    fn fresh_metadata(&self) -> Result<(Option<Field>, u64, usize)> {
        let metadata = read_metadata(&self.handle)?;
        let rows = metadata.rows;
        let field = match (self.options.field(), metadata.schema) {
            (Some(field), _) => Some(field.clone()),
            (None, Some(schema)) => Some(field_from_arrow_schema(self.options.name(), &schema)?),
            (None, None) => None,
        };
        let columns = field.as_ref().map_or(0, Field::field_len);
        Ok((field, rows, columns))
    }

    /// Populate every opened-session metadata cache atomically after parsing.
    fn cache_metadata(&self, field: Option<Field>, rows: u64, columns: usize) {
        if let Some(field) = field {
            let _ = self.cached_schema.set(field);
        }
        let _ = self.cached_row_size.set(rows);
        let _ = self.cached_column_size.set(columns);
    }
}

/// An `Ipc` mirrors the bytes of the handle it owns, so a caller can reach the
/// raw stream - to copy it, compress it, or hand it to another reader - without
/// unwrapping the media type first.
///
/// [`IOBase::open`] additionally caches the stream's schema and dimensions;
/// [`IOBase::close`] releases them.
impl<H: IOBase> crate::IOMedia for Ipc<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> crate::Result<u64> {
        if self.opened {
            if let Some(rows) = self.cached_row_size.get() {
                return Ok(*rows);
            }
        }
        let rows = row_size(&self.handle, &self.options)?;
        if self.opened {
            let _ = self.cached_row_size.set(rows);
            return Ok(*self.cached_row_size.get().unwrap_or(&rows));
        }
        Ok(rows)
    }

    fn column_size(&self) -> crate::Result<usize> {
        if self.opened {
            if let Some(columns) = self.cached_column_size.get() {
                return Ok(*columns);
            }
        }
        let columns = if let Some(field) = self.options.field() {
            field.field_len()
        } else if self.handle.is_empty() {
            0
        } else {
            let options = RecordOptions::Ipc(self.options.clone());
            crate::IOMedia::read_arrow_field(self, &options)?.field_len()
        };
        if self.opened {
            let _ = self.cached_column_size.set(columns);
            return Ok(*self.cached_column_size.get().unwrap_or(&columns));
        }
        Ok(columns)
    }

    /// Return this wrapper's IPC options even when the wrapped byte handle has
    /// no informative media type of its own.
    fn record_options(&self) -> crate::Result<RecordOptions> {
        Ok(RecordOptions::Ipc(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> crate::Result<Field> {
        let options = self.require_record_options(options)?;
        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        // An explicit held Field makes the opened cache a logical declaration,
        // not the stored schema. A caller supplying different options must then
        // derive the bytes afresh rather than receive that unrelated Field.
        if self.opened && self.options.field().is_none() {
            if let Some(cached) = self.cached_schema.get() {
                return Ok(cached.clone().with_name(options.name()));
            }
        }
        let field = read_field(&self.handle, options)?;
        if self.opened && self.options.field().is_none() {
            let cached = field.clone().with_name(self.options.name());
            let _ = self.cached_schema.set(cached);
        }
        Ok(field)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        let opened = self.opened;
        match crate::iobase::overwrite_arrow_reader_default_with_field(self, batches, options) {
            Ok(published) => {
                // Closed media never begin caching as a side effect of a
                // write. An already-open one keeps its cache coherent with the
                // final field after all shaping and stored completion.
                self.invalidate_cached_metadata();
                if opened {
                    if let Some(published) = published {
                        let _ = self.cached_schema.set(published);
                    }
                }
                Ok(())
            }
            Err(error) => {
                // A later cadence may already be visible. The old cached field
                // is therefore never retained after a failed publication. If
                // this handle was open, keep that lifecycle state only when
                // the visible prefix still answers a valid fresh field; never
                // mask the original write error when it does not.
                self.invalidate_cached_metadata();
                Err(error)
            }
        }
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        let opened = self.opened;
        let published = if opened {
            Some(field_from_arrow_schema(
                options.name(),
                batches.schema().as_ref(),
            )?)
        } else {
            None
        };
        match crate::iobase::leaf_writer(self, batches, options) {
            Ok(()) => {
                self.invalidate_cached_metadata();
                if let Some(published) = published {
                    let _ = self.cached_schema.set(published);
                }
                Ok(())
            }
            Err(error) => {
                self.invalidate_cached_metadata();
                Err(error)
            }
        }
    }

    fn append_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        crate::iobase::append_arrow_reader_default(self, batches, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        crate::iobase::merge_arrow_reader_default(self, batches, options)
    }
}

impl<H: IOBase> IOBase for Ipc<H> {
    crate::delegate_iobase!(handle: pread, pstream_bytes, size, capacity, reserve, url, media_type, flush,
        parent, child_by_path, ls, kind);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.invalidate_cached_metadata();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.invalidate_cached_metadata();
        self.handle.truncate(size)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.invalidate_cached_metadata();
        self.handle.set_media_type(media_type);
    }

    /// An Arrow IPC stream is a record encoding, so this handle holds rows
    /// whatever media type the bytes underneath happen to carry - no probe, no
    /// listing, no read.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Materialize the handle and cache the stream's schema.
    ///
    /// Repeated reads then reuse the cached schema instead of re-deriving it.
    /// Opening an empty or missing stream succeeds and caches zero dimensions.
    fn open(&mut self) -> crate::Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        let (field, rows, columns) = self.fresh_metadata()?;
        self.invalidate_cached_metadata();
        self.cache_metadata(field, rows, columns);
        self.opened = true;
        Ok(())
    }

    /// Return whether a schema is currently cached.
    fn opened(&self) -> bool {
        self.opened
    }

    /// Flush the handle and drop the cached schema.
    fn close(&mut self) -> crate::Result<()> {
        self.invalidate_cached_metadata();
        self.opened = false;
        self.handle.close()
    }

    /// Empty the encoded resource and drop the cached schema with it.
    ///
    /// Invalidation is part of the call, not deferred to the next `open`: a
    /// cached schema describing bytes that are gone is a stale answer, and a
    /// stale answer after an emptying is a bug.
    fn clear(&mut self) -> crate::Result<()> {
        self.invalidate_cached_metadata();
        self.handle.clear()
    }

    /// Delete the encoded resource, and every cached schema it filled.
    ///
    /// A media handle removes what it wraps, not merely its own view: the
    /// resource behind the handle goes, and the schema cache goes with it.
    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        self.invalidate_cached_metadata();
        self.opened = false;
        self.handle.remove(recursive)
    }
}

#[cfg(test)]
mod tests;
