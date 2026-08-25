//! Avro object containers as a record encoding over any byte handle.
//!
//! The encoding lives in free functions - [`read_field`], [`read_batch_reader`],
//! [`overwrite_arrow_reader`] - that take any [`IOBase`] handle and one
//! [`AvroOptions`]. That is what [`crate::io::IOMedia::read_arrow_reader`] and its
//! two siblings call, so reading a container needs nothing but a handle whose
//! media type says `avro`. These functions are the encoding and nothing more -
//! the `field` they take is a column pushdown, and the casting, merging, and
//! partition routing a caller sees belong to [`IOBase`]'s three record methods
//! above them.
//!
//! Decoding is columnar: one builder per leaf, appended per record, with no
//! intermediate [`Scalar`](crate::Scalar) tree. A pushed-down projection turns
//! every unselected top-level column into a skip - length-prefixed values jump
//! by their prefix, size-carrying blocks jump whole - so a projection saves
//! decode, allocation, *and* the bytes of the skipped fields; what it cannot
//! save is reading the row, because Avro interleaves columns per record.
//!
//! Avro compresses its blocks internally, so - like Parquet and unlike IPC - a
//! handle declaring an outer content coding is rejected rather than silently
//! double-compressed.

use std::sync::{Arc, OnceLock};

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Decimal128Builder, FixedSizeBinaryBuilder, PrimitiveBuilder,
    StringBuilder,
};
use arrow_array::types::{
    Date32Type, Float32Type, Float64Type, Int32Type, Int64Type, IntervalMonthDayNanoType,
    Time32MillisecondType, Time64MicrosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType,
};
use arrow_array::{
    Array, ArrayRef, ListArray, MapArray, RecordBatch, RecordBatchIterator, RecordBatchOptions,
    StructArray, cast::AsArray,
};
use arrow_buffer::{IntervalMonthDayNano, NullBufferBuilder, OffsetBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType as ArrowDataType, FieldRef, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use crate::arrow::{BatchReader, Result, arrow_schema_from_field, field_from_arrow_schema};
use crate::generic::{IORecordOptions, RecordOptions};
use crate::io::IOBase;
use crate::{Field, Level, Limits};

use super::arrow::{field_from_schema, schema_json_from_field};
use super::container::{
    BlockCoding, CODEC_KEY, MAGIC, SCHEMA_KEY, SYNC_LEN, check_magic, parse_header, sync_marker,
};
use super::datum::{Cursor, DatumCodec, block_count, codec, invalid, put_bytes, put_long};
use super::schema::{Node, Schema};

/// The default root Field name used when a schema is inferred.
pub const DEFAULT_ROOT_NAME: &str = "row";

/// The settings an Avro record read or write takes.
///
/// Avro adds two settings to the shared surface: the block compression codec,
/// named in Avro's own vocabulary because that is what the header stores, and
/// an optional fixed synchronization marker for byte-reproducible output.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AvroOptions {
    /// Declared canonical schema; inferred from the container when absent.
    pub field: Option<Field>,
    /// Root Field name used for an inferred schema.
    pub root_name: SmolStr,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per batch a reader yields.
    pub batch_size: Option<usize>,
    /// Most result rows in total - a count of rows, not a per-row byte cap.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level for the block codec.
    pub level: Level,
    /// Column names forming a write's match key; empty means overwrite.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to; empty selects everything.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by; empty keeps all.
    pub filter_partitions: Vec<(String, String)>,
    /// The Avro codec name blocks are written with: `null`, `deflate`,
    /// `zstandard`, or - with the `parquet` feature - `snappy`.
    pub codec: SmolStr,
    /// A fixed synchronization marker, for writes that must be reproducible
    /// byte for byte; a fresh random marker is used when absent.
    pub sync_marker: Option<[u8; 16]>,
}

impl AvroOptions {
    /// Build the default Avro options.
    pub fn new() -> Self {
        Self {
            field: None,
            root_name: SmolStr::new_static(DEFAULT_ROOT_NAME),
            safe: false,
            batch_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
            codec: SmolStr::new_static("deflate"),
            sync_marker: None,
        }
    }

    /// Return these options with a different block codec name.
    #[must_use]
    pub fn with_codec(mut self, codec: impl Into<SmolStr>) -> Self {
        self.codec = codec.into();
        self
    }

    /// Return these options with a fixed synchronization marker.
    #[must_use]
    pub fn with_sync_marker(mut self, sync_marker: [u8; 16]) -> Self {
        self.sync_marker = Some(sync_marker);
        self
    }
}

impl Default for AvroOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for AvroOptions {
    crate::record_options_fields!();
}

/// How many rows one batch carries when the caller does not say.
const DEFAULT_BATCH_ROWS: usize = crate::generic::DEFAULT_RECORD_BATCH_SIZE;

/// Read the schema of the container `handle` holds.
///
/// Only the header is read, over `pread`, so asking a large container for its
/// schema never decodes a row.
///
/// # Errors
///
/// Returns a read, decoding, or schema failure.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &AvroOptions) -> Result<Field> {
    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    reject_outer_coding(handle)?;
    let blocks = super::container::read_blocks(handle)?;
    Ok(field_from_schema(blocks.schema(), options.root_name())?)
}

/// Return the number of rows declared by an Avro container's block headers.
///
/// Selection, partition filters, and read limits are deliberately ignored:
/// this describes the whole logical media value. Compressed payloads are
/// skipped positionally, so no datum is decompressed or decoded.
pub(crate) fn row_size<H: IOBase + ?Sized>(
    handle: &H,
    _options: &AvroOptions,
) -> crate::Result<u64> {
    if handle.is_empty() {
        return Ok(0);
    }
    reject_outer_coding(handle)?;
    let mut blocks = super::container::read_blocks(handle)?;
    let mut rows = 0_u64;
    while let Some(count) = blocks.next_block_count()? {
        rows = rows.checked_add(count).ok_or_else(row_count_overflow)?;
    }
    Ok(rows)
}

/// Schema and row count cached for one explicitly opened container.
#[derive(Clone, Debug)]
struct AvroDimensions {
    field: Field,
    rows: u64,
}

/// Read both pieces of Avro metadata without decoding a row.
fn read_dimensions<H: IOBase + ?Sized>(
    handle: &H,
    options: &AvroOptions,
) -> crate::Result<Option<AvroDimensions>> {
    if handle.is_empty() {
        return Ok(None);
    }
    reject_outer_coding(handle)?;
    let mut blocks = super::container::read_blocks(handle)?;
    let field = field_from_schema(blocks.schema(), options.root_name())?;
    let mut rows = 0_u64;
    while let Some(count) = blocks.next_block_count()? {
        rows = rows.checked_add(count).ok_or_else(row_count_overflow)?;
    }
    Ok(Some(AvroDimensions { field, rows }))
}

/// Report that the sum of block counts cannot be represented by the surface.
fn row_count_overflow() -> crate::Error {
    crate::Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: SmolStr::new_static("logical row count exceeds u64::MAX"),
    }
}

/// Read the container `handle` holds, keeping only the columns `field` names.
///
/// Batches are returned exactly as they were written, without being cast to a
/// declared schema. A `field` naming a subset of the stored columns becomes
/// skip steps in the decoder: an unselected column's bytes are jumped, not
/// decoded, so the projection saves decode, allocation, and those bytes. A
/// `field` naming anything the container does not carry is ignored, because a
/// projection can only drop columns, never invent them.
///
/// # Errors
///
/// Returns a read or decoding failure.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &AvroOptions,
) -> Result<BatchReader> {
    reject_outer_coding(handle)?;
    let bytes = handle.read_all_bytes()?;
    if bytes.is_empty() {
        // Per the laziness contract, a missing container holds no batches.
        let schema = match options.field() {
            Some(field) => arrow_schema_from_field(field)?,
            None => Arc::new(arrow_schema::Schema::empty()),
        };
        return Ok(Box::new(RecordBatchIterator::new(
            std::iter::empty(),
            schema,
        )));
    }

    let limits = Limits::default();
    let mut cursor = Cursor::new(&bytes);
    check_magic(cursor.take(MAGIC.len())?)?;
    let header = parse_header(&mut cursor, limits)?;
    let blocks_at = cursor.position;

    let keep: Option<Vec<&str>> =
        field.map(|field| field.fields().iter().map(|child| child.name()).collect());
    let (root, arrow_schema) =
        RootReader::new(&header.schema, options.root_name(), keep.as_deref())?;

    Ok(Box::new(AvroBatchReader {
        bytes,
        position: blocks_at,
        schema: arrow_schema,
        writer: header.schema,
        coding: header.coding,
        sync: header.sync,
        limits,
        root,
        batch_size: options.batch_size().unwrap_or(DEFAULT_BATCH_ROWS).max(1),
        block: None,
        failed: false,
    }))
}

/// Replace the container `handle` holds with every batch `batches` yields.
///
/// The container's schema is derived from the reader's schema, refusing any
/// datatype Avro cannot spell by name; each incoming batch becomes one block,
/// compressed with the options' codec.
///
/// # Errors
///
/// Returns a schema, encoding, or write failure.
pub fn overwrite_arrow_reader<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &AvroOptions,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    reject_outer_coding(handle)?;
    let root = field_from_arrow_schema(options.root_name(), batches.schema().as_ref())?;
    let schema_json = schema_json_from_field(&root)?;
    let schema = Schema::from_json(&schema_json)?;
    // The canonical shape is what the encoder walks: casting each batch onto
    // it first means the encoder only ever sees the arrow types the mapping
    // produces, and an exact batch passes through untouched.
    let canonical = field_from_schema(&schema, root.name())?;

    let coding = BlockCoding::from_name(&options.codec)?;
    let sync = options.sync_marker.unwrap_or_else(sync_marker);
    let encoded_schema = crate::json::into_bytes(&schema_json)?;

    let mut output = Vec::with_capacity(1024);
    output.extend_from_slice(&MAGIC);
    put_long(&mut output, 2);
    put_bytes(&mut output, SCHEMA_KEY.as_bytes());
    put_bytes(&mut output, &encoded_schema);
    put_bytes(&mut output, CODEC_KEY.as_bytes());
    put_bytes(&mut output, coding.name().as_bytes());
    put_long(&mut output, 0);
    output.extend_from_slice(&sync);

    let mut payload = Vec::new();
    for batch in batches {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        if batch.num_rows() == 0 {
            continue;
        }
        let batch = crate::field::cast::cast_record_batch(&canonical, batch, false)?;
        payload.clear();
        encode_batch(&schema.node, &schema, &batch, &mut payload)?;
        let compressed = coding.dump(&payload, options.level())?;
        put_long(&mut output, batch.num_rows() as i64);
        put_bytes(&mut output, &compressed);
        output.extend_from_slice(&sync);
    }

    handle.write_all_bytes(&output)?;
    Ok(())
}

/// Refuse a handle whose media type declares an outer content coding.
///
/// Avro compresses inside its blocks, so an outer coding would produce a file
/// no Avro reader could open without knowing about it.
fn reject_outer_coding<H: IOBase + ?Sized>(handle: &H) -> Result<()> {
    let media_type = handle.media_type();
    if let Some(encoding) = media_type.encodings().last() {
        return Err(crate::arrow::Error::from(invalid(format_smolstr!(
            "expected an Avro container without an outer content coding, got {encoding}"
        ))));
    }
    Ok(())
}

/// The lazy batch iterator over one container's blocks.
struct AvroBatchReader {
    /// The whole container. DESIGN: owned because a `BatchReader` outlives the
    /// borrow of the handle it came from; decompression and decoding stay
    /// lazy, one block at a time.
    bytes: Vec<u8>,
    /// The offset of the next unread block.
    position: usize,
    /// The projected batch schema.
    schema: SchemaRef,
    /// The writer schema, for skips and reference resolution.
    writer: Schema,
    /// The block compression.
    coding: BlockCoding,
    /// The marker that closes every block.
    sync: [u8; SYNC_LEN],
    /// The decode bounds.
    limits: Limits,
    /// The columnar decoder.
    root: RootReader,
    /// Rows per yielded batch.
    batch_size: usize,
    /// The block being decoded: decompressed payload, offset, rows left.
    block: Option<(Vec<u8>, usize, u64)>,
    /// Whether an error ended the stream.
    failed: bool,
}

impl AvroBatchReader {
    /// Pull the next block into `self.block`, or report the end.
    fn next_block(&mut self) -> crate::Result<bool> {
        let mut cursor = Cursor::new(&self.bytes);
        cursor.position = self.position;
        if cursor.is_exhausted() {
            return Ok(false);
        }
        let count = cursor.long()?;
        let count = u64::try_from(count).map_err(|_| {
            codec(
                cursor.position,
                format_smolstr!("expected a non-negative Avro block count, got {count}"),
            )
        })?;
        let payload = cursor.bytes()?;
        let marker = cursor.take(SYNC_LEN)?;
        if marker != self.sync {
            return Err(codec(
                cursor.position,
                SmolStr::new_static(
                    "expected the header's synchronization marker after an Avro block",
                ),
            ));
        }
        if count as usize > self.limits.max_nodes() {
            return Err(invalid(format_smolstr!(
                "expected at most {} rows in a block",
                self.limits.max_nodes()
            )));
        }
        let decoded = self.coding.load(payload, self.limits)?;
        self.position = cursor.position;
        self.block = Some((decoded, 0, count));
        Ok(true)
    }

    /// Decode up to one batch of rows.
    fn next_batch(&mut self) -> crate::Result<Option<RecordBatch>> {
        let mut rows = 0_usize;
        while rows < self.batch_size {
            let Some((payload, mut offset, mut left)) = self.block.take() else {
                if !self.next_block()? {
                    break;
                }
                continue;
            };
            while rows < self.batch_size && left > 0 {
                let mut cursor = Cursor::new(&payload);
                cursor.position = offset;
                let mut budget = self.limits.max_nodes();
                let datum = DatumCodec {
                    names: &self.writer.names,
                    limits: self.limits,
                };
                self.root.append(&mut cursor, &datum, &mut budget)?;
                offset = cursor.position;
                left -= 1;
                rows += 1;
            }
            if left > 0 {
                self.block = Some((payload, offset, left));
            } else if offset < payload.len() {
                return Err(codec(
                    offset,
                    SmolStr::new_static("expected the block to end after its declared rows"),
                ));
            }
        }
        if rows == 0 {
            return Ok(None);
        }
        Ok(Some(self.root.finish(&self.schema, rows)?))
    }
}

impl Iterator for AvroBatchReader {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.next_batch() {
            Ok(batch) => batch.map(Ok),
            Err(error) => {
                self.failed = true;
                Some(Err(ArrowError::ExternalError(Box::new(error))))
            }
        }
    }
}

impl arrow_array::RecordBatchReader for AvroBatchReader {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }
}

/// The decoder for one row's top level.
struct RootReader {
    /// One step per writer field, or one reader for a wrapped scalar root.
    steps: Vec<RootStep>,
}

/// What to do with one top-level writer field.
enum RootStep {
    /// Decode into a column.
    Read(ColumnReader),
    /// Jump the bytes without decoding.
    Skip(Node),
}

impl RootReader {
    /// Build the decoder and the projected batch schema.
    fn new(
        schema: &Schema,
        root_name: &str,
        keep: Option<&[&str]>,
    ) -> crate::Result<(Self, SchemaRef)> {
        let root_field = field_from_schema(schema, root_name)?;
        let full = arrow_schema_from_field(&root_field)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;

        let mut steps = Vec::new();
        let mut kept_fields = Vec::new();
        if let Node::Record(record) = &schema.node {
            for (index, field) in record.fields.iter().enumerate() {
                let wanted = keep.is_none_or(|names| {
                    names
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&field.name))
                });
                if wanted {
                    let arrow_field = full.field(index).clone();
                    steps.push(RootStep::Read(ColumnReader::new(
                        &field.schema,
                        schema,
                        arrow_field.data_type(),
                    )?));
                    kept_fields.push(arrow_field);
                } else {
                    steps.push(RootStep::Skip(field.schema.clone()));
                }
            }
        } else {
            // A non-record root reads as one column called `value`.
            let arrow_field = full.field(0).clone();
            steps.push(RootStep::Read(ColumnReader::new(
                &schema.node,
                schema,
                arrow_field.data_type(),
            )?));
            kept_fields.push(arrow_field);
        }
        let projected = Arc::new(arrow_schema::Schema::new(kept_fields));
        Ok((Self { steps }, projected))
    }

    /// Decode one row.
    fn append(
        &mut self,
        cursor: &mut Cursor<'_>,
        datum: &DatumCodec<'_>,
        budget: &mut usize,
    ) -> crate::Result<()> {
        for step in &mut self.steps {
            match step {
                RootStep::Read(reader) => reader.append(cursor, datum, budget)?,
                RootStep::Skip(node) => datum.skip(node, cursor, 0, budget)?,
            }
        }
        Ok(())
    }

    /// Assemble the accumulated rows into one batch.
    fn finish(&mut self, schema: &SchemaRef, rows: usize) -> crate::Result<RecordBatch> {
        let mut arrays = Vec::new();
        for step in &mut self.steps {
            if let RootStep::Read(reader) = step {
                arrays.push(reader.finish()?);
            }
        }
        RecordBatch::try_new_with_options(
            schema.clone(),
            arrays,
            &RecordBatchOptions::new().with_row_count(Some(rows)),
        )
        .map_err(|error| invalid(format_smolstr!("{error}")))
    }
}

/// One column's decoder: a builder per leaf, appended per record.
enum ColumnReader {
    /// The empty type: rows counted, no bytes.
    Null { length: usize },
    /// One byte per value.
    Boolean(BooleanBuilder),
    /// A zig-zag 32-bit integer.
    Int32(PrimitiveBuilder<Int32Type>),
    /// A zig-zag 64-bit integer.
    Int64(PrimitiveBuilder<Int64Type>),
    /// Four little-endian bytes.
    Float32(PrimitiveBuilder<Float32Type>),
    /// Eight little-endian bytes.
    Float64(PrimitiveBuilder<Float64Type>),
    /// A length-prefixed byte run.
    Binary(BinaryBuilder),
    /// A length-prefixed UTF-8 run.
    Utf8(StringBuilder),
    /// A `date` int.
    Date32(PrimitiveBuilder<Date32Type>),
    /// A `time-millis` int.
    Time32(PrimitiveBuilder<Time32MillisecondType>),
    /// A `time-micros` long.
    Time64(PrimitiveBuilder<Time64MicrosecondType>),
    /// A `timestamp-millis` or `local-timestamp-millis` long.
    TimestampMillis(PrimitiveBuilder<TimestampMillisecondType>, bool),
    /// A `timestamp-micros` or `local-timestamp-micros` long.
    TimestampMicros(PrimitiveBuilder<TimestampMicrosecondType>, bool),
    /// A `timestamp-nanos` or `local-timestamp-nanos` long.
    TimestampNanos(PrimitiveBuilder<TimestampNanosecondType>, bool),
    /// A `duration` fixed(12): months, days, milliseconds.
    Interval(PrimitiveBuilder<IntervalMonthDayNanoType>),
    /// A `decimal` over bytes or fixed.
    Decimal {
        /// The unscaled integers.
        builder: Decimal128Builder,
        /// Declared precision.
        precision: u8,
        /// Declared scale.
        scale: i8,
        /// The fixed width, when not over bytes.
        size: Option<usize>,
    },
    /// A fixed-width byte run.
    Fixed {
        /// The values.
        builder: FixedSizeBinaryBuilder,
        /// The declared width.
        size: usize,
    },
    /// An enum index decoded to its symbol.
    Enum {
        /// The symbols as strings.
        builder: StringBuilder,
        /// The declared symbols, by index.
        symbols: Arc<[SmolStr]>,
    },
    /// Fields back to back.
    Struct {
        /// The arrow shape of the children.
        fields: arrow_schema::Fields,
        /// One decoder per child.
        children: Vec<ColumnReader>,
        /// Validity per row.
        nulls: NullBufferBuilder,
        /// Rows appended.
        length: usize,
    },
    /// Item blocks.
    List {
        /// The arrow item field.
        field: FieldRef,
        /// The item decoder.
        child: Box<ColumnReader>,
        /// Row offsets into the child.
        offsets: Vec<i32>,
        /// Items appended to the child.
        child_length: i32,
        /// Validity per row.
        nulls: NullBufferBuilder,
    },
    /// Entry blocks.
    Map {
        /// The arrow entries field.
        field: FieldRef,
        /// The entry keys.
        keys: StringBuilder,
        /// The entry values.
        values: Box<ColumnReader>,
        /// Row offsets into the entries.
        offsets: Vec<i32>,
        /// Entries appended.
        child_length: i32,
        /// Validity per row.
        nulls: NullBufferBuilder,
    },
    /// A union: the branch index is read and validated per value.
    Union {
        /// The branch index that means null, when the union has one.
        null_branch: Option<usize>,
        /// The branch index carrying the value, when the union has one.
        value_branch: Option<usize>,
        /// How many branches the union declares, for the failure message.
        width: usize,
        /// The value decoder; a null-only union decodes as a null column.
        inner: Box<ColumnReader>,
    },
}

impl ColumnReader {
    /// Build the decoder for one node against its mapped arrow type.
    fn new(node: &Node, schema: &Schema, arrow: &ArrowDataType) -> crate::Result<Self> {
        Ok(match node {
            Node::Null => Self::Null { length: 0 },
            Node::Boolean => Self::Boolean(BooleanBuilder::new()),
            Node::Int => Self::Int32(PrimitiveBuilder::new()),
            Node::Long => Self::Int64(PrimitiveBuilder::new()),
            Node::Float => Self::Float32(PrimitiveBuilder::new()),
            Node::Double => Self::Float64(PrimitiveBuilder::new()),
            Node::Bytes => Self::Binary(BinaryBuilder::new()),
            Node::String | Node::Uuid => Self::Utf8(StringBuilder::new()),
            Node::Date => Self::Date32(PrimitiveBuilder::new()),
            Node::TimeMillis => Self::Time32(PrimitiveBuilder::new()),
            Node::TimeMicros => Self::Time64(PrimitiveBuilder::new()),
            Node::TimestampMillis => Self::TimestampMillis(PrimitiveBuilder::new(), true),
            Node::TimestampMicros => Self::TimestampMicros(PrimitiveBuilder::new(), true),
            Node::TimestampNanos => Self::TimestampNanos(PrimitiveBuilder::new(), true),
            Node::LocalTimestampMillis => Self::TimestampMillis(PrimitiveBuilder::new(), false),
            Node::LocalTimestampMicros => Self::TimestampMicros(PrimitiveBuilder::new(), false),
            Node::LocalTimestampNanos => Self::TimestampNanos(PrimitiveBuilder::new(), false),
            Node::Duration(_) => Self::Interval(PrimitiveBuilder::new()),
            Node::Decimal(decimal) => Self::Decimal {
                builder: Decimal128Builder::new(),
                precision: u8::try_from(decimal.precision).unwrap_or(38),
                scale: i8::try_from(decimal.scale).unwrap_or(0),
                size: decimal.fixed.as_ref().map(|fixed| fixed.size),
            },
            Node::UuidFixed(fixed) | Node::Fixed(fixed) => Self::Fixed {
                builder: FixedSizeBinaryBuilder::new(i32::try_from(fixed.size).unwrap_or(0)),
                size: fixed.size,
            },
            Node::Enum(declared) => Self::Enum {
                builder: StringBuilder::new(),
                symbols: declared.symbols.clone(),
            },
            Node::Record(record) => {
                let ArrowDataType::Struct(fields) = arrow else {
                    return Err(shape_error(node, arrow));
                };
                let mut children = Vec::with_capacity(record.fields.len());
                for (child, arrow_field) in record.fields.iter().zip(fields.iter()) {
                    children.push(Self::new(&child.schema, schema, arrow_field.data_type())?);
                }
                Self::Struct {
                    fields: fields.clone(),
                    children,
                    nulls: NullBufferBuilder::new(1024),
                    length: 0,
                }
            }
            Node::Array(items) => {
                let ArrowDataType::List(field) = arrow else {
                    return Err(shape_error(node, arrow));
                };
                Self::List {
                    field: field.clone(),
                    child: Box::new(Self::new(items, schema, field.data_type())?),
                    offsets: vec![0],
                    child_length: 0,
                    nulls: NullBufferBuilder::new(1024),
                }
            }
            Node::Map(values) => {
                let ArrowDataType::Map(entries, _) = arrow else {
                    return Err(shape_error(node, arrow));
                };
                let ArrowDataType::Struct(pair) = entries.data_type() else {
                    return Err(shape_error(node, arrow));
                };
                let value_field = pair.get(1).ok_or_else(|| shape_error(node, arrow))?;
                Self::Map {
                    field: entries.clone(),
                    keys: StringBuilder::new(),
                    values: Box::new(Self::new(values, schema, value_field.data_type())?),
                    offsets: vec![0],
                    child_length: 0,
                    nulls: NullBufferBuilder::new(1024),
                }
            }
            Node::Union(branches) => {
                let null_branch = branches
                    .iter()
                    .position(|branch| matches!(branch, Node::Null));
                let mut values = branches
                    .iter()
                    .enumerate()
                    .filter(|(_, branch)| !matches!(branch, Node::Null));
                let value = values.next();
                if values.next().is_some() {
                    return Err(shape_error(node, arrow));
                }
                Self::Union {
                    null_branch,
                    value_branch: value.map(|(index, _)| index),
                    width: branches.len(),
                    inner: Box::new(match value {
                        Some((_, value)) => Self::new(value, schema, arrow)?,
                        // A null-only union still spends a branch index per
                        // value; the inner reader is the null column it fills.
                        None => Self::Null { length: 0 },
                    }),
                }
            }
            Node::Ref(name) => {
                let target = schema.names.get(name).cloned().ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a declared Avro type named {name:?}"
                    ))
                })?;
                Self::new(&target, schema, arrow)?
            }
        })
    }

    /// Decode one value into the column.
    fn append(
        &mut self,
        cursor: &mut Cursor<'_>,
        datum: &DatumCodec<'_>,
        budget: &mut usize,
    ) -> crate::Result<()> {
        datum.spend(budget)?;
        match self {
            Self::Null { length } => *length += 1,
            Self::Boolean(builder) => {
                builder.append_value(cursor.take(1)?.first().is_some_and(|byte| *byte != 0));
            }
            Self::Int32(builder) => builder.append_value(cursor.int()?),
            Self::Int64(builder) => builder.append_value(cursor.long()?),
            Self::Float32(builder) => builder.append_value(cursor.float()?),
            Self::Float64(builder) => builder.append_value(cursor.double()?),
            Self::Binary(builder) => builder.append_value(cursor.bytes()?),
            Self::Utf8(builder) => builder.append_value(cursor.string()?),
            Self::Date32(builder) => builder.append_value(cursor.int()?),
            Self::Time32(builder) => builder.append_value(cursor.int()?),
            Self::Time64(builder) => builder.append_value(cursor.long()?),
            Self::TimestampMillis(builder, _) => builder.append_value(cursor.long()?),
            Self::TimestampMicros(builder, _) => builder.append_value(cursor.long()?),
            Self::TimestampNanos(builder, _) => builder.append_value(cursor.long()?),
            Self::Interval(builder) => {
                let bytes = cursor.take(12)?;
                let part = |index: usize| {
                    u32::from_le_bytes(bytes[index..index + 4].try_into().unwrap_or_default())
                };
                // The wire counts are unsigned 32-bit; the arrow interval is
                // signed. A count outside 31 bits is refused, never clamped.
                let position = cursor.position;
                let bounded = |name: &'static str, value: u32| {
                    i32::try_from(value).map_err(|_| {
                        codec(
                            position,
                            format_smolstr!(
                                "expected a duration {name} within 31 bits, got {value}"
                            ),
                        )
                    })
                };
                let months = bounded("months", part(0))?;
                let days = bounded("days", part(4))?;
                let nanos = i64::from(part(8)) * 1_000_000;
                builder.append_value(IntervalMonthDayNano::new(months, days, nanos));
            }
            Self::Decimal { builder, size, .. } => {
                let bytes = match size {
                    Some(size) => cursor.take(*size)?,
                    None => cursor.bytes()?,
                };
                let unscaled = super::datum::decimal_from_bytes(bytes).ok_or_else(|| {
                    codec(
                        cursor.position,
                        format_smolstr!(
                            "expected a decimal of at most 38 digits, got {} bytes",
                            bytes.len()
                        ),
                    )
                })?;
                builder.append_value(unscaled);
            }
            Self::Fixed { builder, size } => {
                builder
                    .append_value(cursor.take(*size)?)
                    .map_err(|error| invalid(format_smolstr!("{error}")))?;
            }
            Self::Enum { builder, symbols } => {
                let index = cursor.long()?;
                let symbol = usize::try_from(index)
                    .ok()
                    .and_then(|index| symbols.get(index))
                    .ok_or_else(|| {
                        codec(
                            cursor.position,
                            format_smolstr!(
                                "expected an Avro enum index below {}, got {index}",
                                symbols.len()
                            ),
                        )
                    })?;
                builder.append_value(symbol);
            }
            Self::Struct {
                children,
                nulls,
                length,
                ..
            } => {
                for child in children {
                    child.append(cursor, datum, budget)?;
                }
                nulls.append_non_null();
                *length += 1;
            }
            Self::List {
                child,
                offsets,
                child_length,
                nulls,
                ..
            } => {
                loop {
                    let (count, _) = block_count(cursor)?;
                    if count == 0 {
                        break;
                    }
                    for _ in 0..count {
                        child.append(cursor, datum, budget)?;
                        *child_length += 1;
                    }
                }
                offsets.push(*child_length);
                nulls.append_non_null();
            }
            Self::Map {
                keys,
                values,
                offsets,
                child_length,
                nulls,
                ..
            } => {
                loop {
                    let (count, _) = block_count(cursor)?;
                    if count == 0 {
                        break;
                    }
                    for _ in 0..count {
                        datum.spend(budget)?;
                        keys.append_value(cursor.string()?);
                        values.append(cursor, datum, budget)?;
                        *child_length += 1;
                    }
                }
                offsets.push(*child_length);
                nulls.append_non_null();
            }
            Self::Union {
                null_branch,
                value_branch,
                width,
                inner,
            } => {
                let declared = cursor.long()?;
                let index = usize::try_from(declared).ok();
                if index.is_some() && index == *null_branch {
                    inner.append_null()?;
                } else if index.is_some() && index == *value_branch {
                    inner.append(cursor, datum, budget)?;
                } else {
                    return Err(codec(
                        cursor.position,
                        format_smolstr!(
                            "expected an Avro union branch below {width}, got {declared}"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Append one null.
    fn append_null(&mut self) -> crate::Result<()> {
        match self {
            Self::Null { length } => *length += 1,
            Self::Boolean(builder) => builder.append_null(),
            Self::Int32(builder) => builder.append_null(),
            Self::Int64(builder) => builder.append_null(),
            Self::Float32(builder) => builder.append_null(),
            Self::Float64(builder) => builder.append_null(),
            Self::Binary(builder) => builder.append_null(),
            Self::Utf8(builder) => builder.append_null(),
            Self::Date32(builder) => builder.append_null(),
            Self::Time32(builder) => builder.append_null(),
            Self::Time64(builder) => builder.append_null(),
            Self::TimestampMillis(builder, _) => builder.append_null(),
            Self::TimestampMicros(builder, _) => builder.append_null(),
            Self::TimestampNanos(builder, _) => builder.append_null(),
            Self::Interval(builder) => builder.append_null(),
            Self::Decimal { builder, .. } => builder.append_null(),
            Self::Fixed { builder, .. } => builder.append_null(),
            Self::Enum { builder, .. } => builder.append_null(),
            Self::Struct {
                children,
                nulls,
                length,
                ..
            } => {
                // A null struct still owes its children a physical slot.
                for child in children {
                    child.append_null()?;
                }
                nulls.append_null();
                *length += 1;
            }
            Self::List {
                offsets,
                child_length,
                nulls,
                ..
            } => {
                offsets.push(*child_length);
                nulls.append_null();
            }
            Self::Map {
                offsets,
                child_length,
                nulls,
                ..
            } => {
                offsets.push(*child_length);
                nulls.append_null();
            }
            Self::Union { inner, .. } => inner.append_null()?,
        }
        Ok(())
    }

    /// Assemble the accumulated values and reset for the next batch.
    fn finish(&mut self) -> crate::Result<ArrayRef> {
        Ok(match self {
            Self::Null { length } => {
                let array: ArrayRef = Arc::new(arrow_array::NullArray::new(*length));
                *length = 0;
                array
            }
            Self::Boolean(builder) => Arc::new(builder.finish()),
            Self::Int32(builder) => Arc::new(builder.finish()),
            Self::Int64(builder) => Arc::new(builder.finish()),
            Self::Float32(builder) => Arc::new(builder.finish()),
            Self::Float64(builder) => Arc::new(builder.finish()),
            Self::Binary(builder) => Arc::new(builder.finish()),
            Self::Utf8(builder) => Arc::new(builder.finish()),
            Self::Date32(builder) => Arc::new(builder.finish()),
            Self::Time32(builder) => Arc::new(builder.finish()),
            Self::Time64(builder) => Arc::new(builder.finish()),
            Self::TimestampMillis(builder, zoned) => {
                let array = builder.finish();
                Arc::new(if *zoned {
                    array.with_timezone("UTC")
                } else {
                    array
                })
            }
            Self::TimestampMicros(builder, zoned) => {
                let array = builder.finish();
                Arc::new(if *zoned {
                    array.with_timezone("UTC")
                } else {
                    array
                })
            }
            Self::TimestampNanos(builder, zoned) => {
                let array = builder.finish();
                Arc::new(if *zoned {
                    array.with_timezone("UTC")
                } else {
                    array
                })
            }
            Self::Interval(builder) => Arc::new(builder.finish()),
            Self::Decimal {
                builder,
                precision,
                scale,
                ..
            } => Arc::new(
                builder
                    .finish()
                    .with_precision_and_scale(*precision, *scale)
                    .map_err(|error| invalid(format_smolstr!("{error}")))?,
            ),
            Self::Fixed { builder, .. } => Arc::new(builder.finish()),
            Self::Enum { builder, .. } => Arc::new(builder.finish()),
            Self::Struct {
                fields,
                children,
                nulls,
                length,
            } => {
                let mut arrays = Vec::with_capacity(children.len());
                for child in children {
                    arrays.push(child.finish()?);
                }
                let array = if fields.is_empty() {
                    StructArray::new_empty_fields(*length, nulls.finish())
                } else {
                    StructArray::try_new(fields.clone(), arrays, nulls.finish())
                        .map_err(|error| invalid(format_smolstr!("{error}")))?
                };
                *length = 0;
                Arc::new(array)
            }
            Self::List {
                field,
                child,
                offsets,
                child_length,
                nulls,
            } => {
                let values = child.finish()?;
                let taken = std::mem::replace(offsets, vec![0]);
                *child_length = 0;
                let array = ListArray::try_new(
                    field.clone(),
                    OffsetBuffer::new(ScalarBuffer::from(taken)),
                    values,
                    nulls.finish(),
                )
                .map_err(|error| invalid(format_smolstr!("{error}")))?;
                Arc::new(array)
            }
            Self::Map {
                field,
                keys,
                values,
                offsets,
                child_length,
                nulls,
            } => {
                let keys: ArrayRef = Arc::new(keys.finish());
                let values = values.finish()?;
                let ArrowDataType::Struct(pair) = field.data_type() else {
                    return Err(invalid(SmolStr::new_static(
                        "expected a struct entries field on a map",
                    )));
                };
                let entries = StructArray::try_new(pair.clone(), vec![keys, values], None)
                    .map_err(|error| invalid(format_smolstr!("{error}")))?;
                let taken = std::mem::replace(offsets, vec![0]);
                *child_length = 0;
                let array = MapArray::try_new(
                    field.clone(),
                    OffsetBuffer::new(ScalarBuffer::from(taken)),
                    entries,
                    nulls.finish(),
                    false,
                )
                .map_err(|error| invalid(format_smolstr!("{error}")))?;
                Arc::new(array)
            }
            Self::Union { inner, .. } => inner.finish()?,
        })
    }
}

/// Report a schema node whose mapped arrow type does not line up.
fn shape_error(node: &Node, arrow: &ArrowDataType) -> crate::Error {
    invalid(format_smolstr!(
        "expected the arrow shape of an Avro {}, got {arrow}",
        node.kind()
    ))
}

/// Encode every row of one canonical batch.
fn encode_batch(
    node: &Node,
    schema: &Schema,
    batch: &RecordBatch,
    payload: &mut Vec<u8>,
) -> crate::Result<()> {
    let Node::Record(record) = node else {
        return Err(invalid(SmolStr::new_static(
            "expected a record schema to encode batches",
        )));
    };
    let columns = batch.columns();
    for row in 0..batch.num_rows() {
        for (field, column) in record.fields.iter().zip(columns) {
            encode_cell(&field.schema, schema, column.as_ref(), row, payload)
                .map_err(|error| locate_column(error, &field.name))?;
        }
    }
    Ok(())
}

/// Encode one cell of one canonical column.
fn encode_cell(
    node: &Node,
    schema: &Schema,
    column: &dyn Array,
    row: usize,
    payload: &mut Vec<u8>,
) -> crate::Result<()> {
    if let Node::Union(branches) = node {
        let null_branch = branches
            .iter()
            .position(|branch| matches!(branch, Node::Null))
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a union carrying null on the record surface",
                ))
            })?;
        if column.is_null(row) {
            put_long(payload, null_branch as i64);
            return Ok(());
        }
        let (index, value) = branches
            .iter()
            .enumerate()
            .find(|(_, branch)| !matches!(branch, Node::Null))
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a union carrying a value branch",
                ))
            })?;
        put_long(payload, index as i64);
        return encode_cell(value, schema, column, row, payload);
    }
    if column.is_null(row) {
        return Err(invalid(SmolStr::new_static(
            "expected a value for a required column, got null",
        )));
    }
    match node {
        Node::Null => {}
        Node::Boolean => payload.push(u8::from(column.as_boolean().value(row))),
        Node::Int => put_long(
            payload,
            i64::from(column.as_primitive::<Int32Type>().value(row)),
        ),
        Node::Long => put_long(payload, column.as_primitive::<Int64Type>().value(row)),
        Node::Float => payload.extend_from_slice(
            &column
                .as_primitive::<Float32Type>()
                .value(row)
                .to_le_bytes(),
        ),
        Node::Double => payload.extend_from_slice(
            &column
                .as_primitive::<Float64Type>()
                .value(row)
                .to_le_bytes(),
        ),
        Node::Bytes => put_bytes(payload, column.as_binary::<i32>().value(row)),
        Node::String | Node::Uuid => {
            put_bytes(payload, column.as_string::<i32>().value(row).as_bytes());
        }
        Node::Date => put_long(
            payload,
            i64::from(column.as_primitive::<Date32Type>().value(row)),
        ),
        Node::TimeMillis => put_long(
            payload,
            i64::from(column.as_primitive::<Time32MillisecondType>().value(row)),
        ),
        Node::TimeMicros => put_long(
            payload,
            column.as_primitive::<Time64MicrosecondType>().value(row),
        ),
        Node::TimestampMillis | Node::LocalTimestampMillis => put_long(
            payload,
            column.as_primitive::<TimestampMillisecondType>().value(row),
        ),
        Node::TimestampMicros | Node::LocalTimestampMicros => put_long(
            payload,
            column.as_primitive::<TimestampMicrosecondType>().value(row),
        ),
        Node::TimestampNanos | Node::LocalTimestampNanos => put_long(
            payload,
            column.as_primitive::<TimestampNanosecondType>().value(row),
        ),
        Node::Duration(_) => {
            let value = column.as_primitive::<IntervalMonthDayNanoType>().value(row);
            if value.nanoseconds % 1_000_000 != 0 {
                return Err(invalid(format_smolstr!(
                    "expected whole milliseconds for an Avro duration, got {} nanoseconds",
                    value.nanoseconds
                )));
            }
            // An Avro duration is three unsigned 32-bit counts; a component
            // outside that range is refused, never silently rewritten.
            let months = u32::try_from(value.months).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative duration months, got {}",
                    value.months
                ))
            })?;
            let days = u32::try_from(value.days).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a non-negative duration days, got {}",
                    value.days
                ))
            })?;
            let millis = u32::try_from(value.nanoseconds / 1_000_000).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a duration within u32 milliseconds, got {} nanoseconds",
                    value.nanoseconds
                ))
            })?;
            payload.extend_from_slice(&months.to_le_bytes());
            payload.extend_from_slice(&days.to_le_bytes());
            payload.extend_from_slice(&millis.to_le_bytes());
        }
        Node::Decimal(decimal) => {
            let unscaled = column
                .as_primitive::<arrow_array::types::Decimal128Type>()
                .value(row);
            match &decimal.fixed {
                Some(fixed) => {
                    let bytes =
                        super::datum::decimal_to_fixed(unscaled, fixed.size).ok_or_else(|| {
                            invalid(format_smolstr!(
                                "expected a decimal fitting {} fixed bytes, got {unscaled}",
                                fixed.size
                            ))
                        })?;
                    payload.extend_from_slice(&bytes);
                }
                None => put_bytes(payload, &super::datum::decimal_to_bytes(unscaled)),
            }
        }
        Node::UuidFixed(_) | Node::Fixed(_) => {
            payload.extend_from_slice(column.as_fixed_size_binary().value(row));
        }
        Node::Enum(declared) => {
            let symbol = column.as_string::<i32>().value(row);
            let index = declared
                .symbols
                .iter()
                .position(|candidate| candidate == symbol)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected one of the Avro enum symbols {:?}, got {symbol:?}",
                        declared.symbols
                    ))
                })?;
            put_long(payload, index as i64);
        }
        Node::Record(record) => {
            let entries = column.as_struct();
            for (field, child) in record.fields.iter().zip(entries.columns()) {
                encode_cell(&field.schema, schema, child.as_ref(), row, payload)
                    .map_err(|error| locate_column(error, &field.name))?;
            }
        }
        Node::Array(items) => {
            let list = column.as_list::<i32>();
            let values = list.value(row);
            if !values.is_empty() {
                put_long(payload, values.len() as i64);
                for index in 0..values.len() {
                    encode_cell(items, schema, values.as_ref(), index, payload)?;
                }
            }
            put_long(payload, 0);
        }
        Node::Map(value_node) => {
            let map = column.as_map();
            let entries = map.value(row);
            let keys = entries.column(0).as_string::<i32>();
            let values = entries.column(1);
            if !entries.is_empty() {
                put_long(payload, entries.len() as i64);
                for index in 0..entries.len() {
                    put_bytes(payload, keys.value(index).as_bytes());
                    encode_cell(value_node, schema, values.as_ref(), index, payload)?;
                }
            }
            put_long(payload, 0);
        }
        Node::Union(_) => unreachable!("handled above"),
        Node::Ref(name) => {
            let target = schema.names.get(name).cloned().ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a declared Avro type named {name:?}"
                ))
            })?;
            encode_cell(&target, schema, column, row, payload)?;
        }
    }
    Ok(())
}

/// Locate an encode failure at its column.
fn locate_column(error: crate::Error, column: &str) -> crate::Error {
    match error {
        crate::Error::Codec {
            format,
            position,
            reason,
        } => crate::Error::Codec {
            format,
            position,
            reason: format_smolstr!("{column}: {reason}"),
        },
        other => other,
    }
}

/// An Avro object container bound to one [`IOBase`] handle.
///
/// Every read and write goes through this type, so the handle, the options,
/// and the cached schema live in one place rather than being repeated at each
/// call.
#[derive(Debug)]
pub struct Avro<H: IOBase> {
    handle: H,
    options: AvroOptions,
    /// Explicit lifecycle state. An opened empty container has no metadata,
    /// so cache presence cannot truthfully answer this question.
    opened: bool,
    /// `Some(None)` is the stable opened-session answer for an empty handle.
    cached_dimensions: OnceLock<Option<AvroDimensions>>,
}

impl<H: IOBase> Avro<H> {
    /// Bind an Avro container to a handle.
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: AvroOptions::new(),
            opened: false,
            cached_dimensions: OnceLock::new(),
        }
    }

    /// Return this container with different options.
    #[must_use]
    pub fn with_options(mut self, options: AvroOptions) -> Self {
        self.options = options;
        self.invalidate_dimensions();
        self
    }

    /// Return this container with an explicit canonical schema.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self.invalidate_dimensions();
        self
    }

    /// Return this container with a different inferred-root Field name.
    #[must_use]
    pub fn with_root_name(mut self, root_name: impl Into<SmolStr>) -> Self {
        self.options.set_root_name(root_name.into());
        self.invalidate_dimensions();
        self
    }

    /// Return this container with a different compression level.
    #[must_use]
    pub fn with_level(mut self, level: Level) -> Self {
        self.options.set_level(level);
        self.invalidate_dimensions();
        self
    }

    /// Borrow the options this container reads and writes with.
    pub const fn options(&self) -> &AvroOptions {
        &self.options
    }

    /// Borrow the options mutably.
    pub fn options_mut(&mut self) -> &mut AvroOptions {
        self.invalidate_dimensions();
        &mut self.options
    }

    /// Refuse options for a different encoding before a write can pull its
    /// first incoming batch.
    fn require_record_options<'a>(
        &self,
        options: &'a RecordOptions,
    ) -> crate::Result<&'a AvroOptions> {
        match options {
            RecordOptions::Avro(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("Avro record options", options.mime_type()),
            }),
        }
    }

    /// Borrow the underlying handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        self.invalidate_dimensions();
        &mut self.handle
    }

    /// Consume the container and return its handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Discard metadata after an in-place mutation while retaining lifecycle
    /// state. The next dimension/schema ask in an open session repopulates it.
    fn invalidate_dimensions(&mut self) {
        self.cached_dimensions.take();
    }

    /// Return the opened-session metadata, or a fresh uncached closed answer.
    fn dimensions(&self) -> crate::Result<Option<AvroDimensions>> {
        if !self.opened {
            return read_dimensions(&self.handle, &self.options);
        }
        if let Some(cached) = self.cached_dimensions.get() {
            return Ok(cached.clone());
        }
        let loaded = read_dimensions(&self.handle, &self.options)?;
        // Concurrent immutable asks may race to fill an invalidated cache;
        // whichever answer wins defines this opened session consistently.
        let _ = self.cached_dimensions.set(loaded.clone());
        Ok(self.cached_dimensions.get().cloned().unwrap_or(loaded))
    }

    /// Refresh metadata after a successful publication while keeping an open
    /// session open. Closed operations never create a cache implicitly.
    fn refresh_dimensions(&mut self) -> crate::Result<()> {
        self.invalidate_dimensions();
        if self.opened {
            let loaded = read_dimensions(&self.handle, &self.options)?;
            let _ = self.cached_dimensions.set(loaded);
        }
        Ok(())
    }

    /// Best-effort refresh after a write error that may follow a published
    /// commit. The original failure must remain the reported failure.
    fn refresh_dimensions_after_error(&mut self) {
        self.invalidate_dimensions();
        if self.opened {
            if let Ok(loaded) = read_dimensions(&self.handle, &self.options) {
                let _ = self.cached_dimensions.set(loaded);
            }
        }
    }
}

/// An `Avro` mirrors the bytes of the handle it owns, so a caller can reach
/// the raw container - to copy it, upload it, or hand it to a foreign reader -
/// without unwrapping the media type first.
///
/// [`IOBase::open`] additionally caches the container's schema and
/// [`IOBase::close`] releases it.
impl<H: IOBase> crate::io::IOMedia for Avro<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> crate::Result<u64> {
        Ok(self.dimensions()?.map_or(0, |dimensions| dimensions.rows))
    }

    fn column_size(&self) -> crate::Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        if self.opened {
            return Ok(self
                .dimensions()?
                .map_or(0, |dimensions| dimensions.field.field_len()));
        }
        if self.handle.is_empty() {
            return Ok(0);
        }
        Ok(read_field(&self.handle, &self.options)?.field_len())
    }

    /// Return this wrapper's Avro options even when the wrapped byte handle
    /// has no informative media type of its own.
    fn record_options(&self) -> crate::Result<RecordOptions> {
        Ok(RecordOptions::Avro(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> crate::Result<Field> {
        let options = self.require_record_options(options)?;
        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        if self.opened {
            if let Some(dimensions) = self.dimensions()? {
                return Ok(dimensions.field.with_name(options.root_name()));
            }
        }
        Ok(read_field(&self.handle, options)?)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        match crate::io::overwrite_arrow_reader_default_with_field(self, batches, options) {
            Ok(_published) => {
                self.refresh_dimensions()?;
                Ok(())
            }
            Err(error) => {
                // A complete earlier cadence remains published by contract;
                // refresh an open handle from that visible container. A
                // truncated or invalid survivor merely drops the cache, and
                // never masks the original write failure.
                self.refresh_dimensions_after_error();
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
        match crate::io::leaf_writer(self, batches, options) {
            Ok(()) => {
                self.refresh_dimensions()?;
                Ok(())
            }
            Err(error) => {
                self.refresh_dimensions_after_error();
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
        crate::io::append_arrow_reader_default(self, batches, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        crate::io::merge_arrow_reader_default(self, batches, options)
    }
}

impl<H: IOBase> IOBase for Avro<H> {
    crate::delegate_iobase!(handle: pread, pstream_bytes, size, capacity, reserve, url, media_type,
        set_media_type, flush, parent, child_by_path, ls, kind);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.invalidate_dimensions();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.invalidate_dimensions();
        self.handle.truncate(size)
    }

    /// An Avro object container is a record encoding, so this handle holds
    /// rows whatever media type the bytes underneath happen to carry - no
    /// probe, no listing, no read.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Materialize the handle and cache its schema and block row counts.
    fn open(&mut self) -> crate::Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        self.invalidate_dimensions();
        let dimensions = read_dimensions(&self.handle, &self.options)?;
        let _ = self.cached_dimensions.set(dimensions);
        self.opened = true;
        Ok(())
    }

    /// Return explicit lifecycle state, including for an empty container.
    fn opened(&self) -> bool {
        self.opened
    }

    /// Flush the handle and drop the cached dimensions.
    fn close(&mut self) -> crate::Result<()> {
        self.opened = false;
        self.invalidate_dimensions();
        self.handle.close()
    }

    /// Empty the encoded resource and drop the cached schema with it.
    ///
    /// Invalidation is part of the call, not deferred to the next `open`: a
    /// cached schema describing bytes that are gone is a stale answer, and a
    /// stale answer after an emptying is a bug.
    fn clear(&mut self) -> crate::Result<()> {
        self.invalidate_dimensions();
        let result = self.handle.clear();
        if self.opened {
            if result.is_ok() {
                let _ = self.cached_dimensions.set(None);
            } else {
                self.refresh_dimensions_after_error();
            }
        }
        result
    }

    /// Delete the encoded resource, and every cached schema it filled.
    ///
    /// A media handle removes what it wraps, not merely its own view: the
    /// resource behind the handle goes, and the schema cache goes with it.
    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        self.opened = false;
        self.invalidate_dimensions();
        self.handle.remove(recursive)
    }
}
