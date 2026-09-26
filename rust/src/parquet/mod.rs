//! Apache Parquet data files over [`IOBase`] handles.
//!
//! Parquet is a footer-first columnar container with its own internal
//! compression, so unlike [`crate::ipc`] this module does **not** apply the
//! handle's content coding over the file: compression is a
//! [`ParquetOptions::compression`] setting that the reader recovers from the
//! footer. A handle declaring an outer coding is rejected rather than silently
//! double-compressed, because the result would be a file no Parquet reader
//! could open.
//!
//! The encoding lives in free functions - [`read_field`], [`read_batch_reader`],
//! [`overwrite_arrow_reader`] - over any [`IOBase`] handle, which is what
//! [`crate::IOMedia::read_arrow_reader`] and its two siblings call. They are the
//! encoding and nothing more: the `field` they take is a column pushdown, and
//! the casting, merging, and partition routing a caller sees belong to
//! [`IOBase`]'s three record methods above them.
//!
//! Yggdryl field identifiers survive the round trip. A [`Field`] carrying
//! `PARQUET:field_id` writes that id into the Parquet schema and reads it back,
//! which is what lets a downstream Iceberg or Delta layer resolve columns by
//! id rather than by position.
//!
//! # Geospatial and variant columns
//!
//! A column whose Arrow field metadata declares the `geoarrow.wkb` extension
//! writes Parquet's own `GEOMETRY` or `GEOGRAPHY` logical type over
//! `BYTE_ARRAY` WKB - CRS and edge algorithm included - and one declaring
//! `arrow.parquet.variant` writes the group the format states for a variant:
//! two required `BYTE_ARRAY` children, `metadata` and `value`, annotated
//! `VARIANT(1)` and carrying no field id. (GeoArrow is a community specification
//! whose own documents say it is not finalized; the `geoarrow.wkb` spelling
//! here is revisitable if it changes.) The writer then refuses min/max value
//! bounds for geospatial columns - their sort order is undefined, so a bound
//! would be a lie - and records the format's own geospatial statistics
//! instead: bounding box and geometry types, computed from the WKB bytes by
//! [`crate::wkb`] and readable back through
//! [`ColumnStatistics::geospatial`] or recomputable by
//! [`read_geospatial_statistics`]. Geography columns record no bounding box:
//! a planar fold of the vertices under-covers non-planar edges.
//!
//! One named limit remains. Reading a *foreign* file whose columns carry
//! `GEOMETRY` or `GEOGRAPHY` surfaces plain `Binary` Arrow types without
//! extension metadata, because the pinned parquet crate only maps those
//! logical types to Arrow extensions behind a crate feature that pulls new
//! dependencies; files written here round-trip their extension metadata
//! through the embedded Arrow schema. A foreign `VARIANT` group does not
//! share that limit: the annotation is read back and the column imports as
//! the variant it is, whatever Arrow schema the file carries.
//!
//! ```
//! use yggdryl::{IOBase, IOMedia, StructType, holder::Buffer};
//! use yggdryl::parquet::Parquet;
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = DataType::from(StructType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::utf8().nullable_field("symbol"),
//! ])?)
//! .required_field("row");
//!
//! let handle =
//!     Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
//! let mut media = Parquet::new(handle);
//!
//! // One instance owns the handle and every write option.
//! let options = media.record_options()?;
//! media.overwrite_arrow_reader(
//!     yggdryl::arrow::batch_reader(field.into_arrow_schema()?, []),
//!     &options,
//! )?;
//! assert_eq!(media.read_arrow_reader(&options)?.count(), 0);
//! # Ok(())
//! # }
//! ```

use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use arrow_array::{RecordBatch, RecordBatchIterator};
use arrow_schema::{ArrowError, Schema};
use bytes::Bytes;
use parquet::arrow::ArrowWriter;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{
    ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder,
};
use parquet::arrow::arrow_writer::{
    ArrowColumnWriter, ArrowRowGroupWriterFactory, ArrowWriterOptions, compute_leaves,
};
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::metadata::{ParquetMetaData, ParquetMetaDataReader};
use parquet::file::properties::WriterProperties;
use parquet::file::writer::SerializedFileWriter;

use crate::IOBase;
use crate::arrow::arrow_schema_from_field;
use crate::arrow::{
    BatchReader, Error, Result, field_from_arrow_schema, from_reader_error, projection_indices,
};
use crate::media::{IORecordOptions, RecordOptions};
use crate::{Error as CoreError, Field};

pub(crate) mod geospatial;
mod metadata;

pub use geospatial::{GeospatialStatistics, read_geospatial_statistics};
pub use metadata::{ColumnStatistics, FileStatistics, RowGroupStatistics};

/// The settings a Parquet read or write takes.
///
/// The shared settings (root name, datatype, metadata, cast strictness, batch
/// row size) are flat fields here, alongside the ones only Parquet has.
/// The shared compression level is deliberately unused: Parquet compresses
/// pages internally through [`Self::compression`], and an outer content coding
/// would produce a file no Parquet reader can open.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ParquetOptions {
    /// Page compression applied inside the file.
    pub compression: Compression,
    /// Maximum rows per row group.
    pub max_row_group_size: usize,
    /// File-level key/value metadata written into the footer.
    pub key_value_metadata: Vec<(String, String)>,
    /// Root Field name; the declared field's when one is declared.
    pub name: smol_str::SmolStr,
    /// The declared root; `None` infers the shape.
    pub field: Option<crate::Field>,
    /// The rows a read or write keeps.
    pub filter: crate::Filter,
    /// The columns a read or write publishes.
    pub select: crate::Selector,
    /// The columns forming an explicit merge's match key.
    pub merge_by: crate::Selector,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Bytes per batch, whichever of this and `batch_row_size` binds first.
    ///
    /// A target rather than a ceiling, and a non-zero bound always yields at
    /// least one row.
    pub batch_byte_size: Option<u64>,
    /// Rows per batch; `None` reads
    /// [`DEFAULT_RECORD_BATCH_ROW_SIZE`](crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE)
    /// rows at a time, or fewer when an unfiltered read's `max_row_size`
    /// asks for fewer.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total - a count of rows, not a per-row byte cap.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Unused: Parquet compresses pages internally through `compression`.
    pub level: crate::Level,
    /// The threads one file's columns decode or encode on; `None` is what
    /// the host offers.
    ///
    /// Crate-internal and outside the options' identity, because it changes
    /// how fast a file is read or written and never what is: a table that
    /// already works on several files at once hands each its share, so the
    /// two levels of parallelism never multiply past what it resolved.
    pub(crate) threads: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct CompressionIdentity {
    codec: u8,
    level: i64,
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct ParquetOptionsIdentity<'a> {
    compression: CompressionIdentity,
    max_row_group_size: usize,
    key_value_metadata: &'a [(String, String)],
    name: &'a smol_str::SmolStr,
    field: &'a Option<crate::Field>,
    filter: &'a crate::Filter,
    select: &'a crate::Selector,
    merge_by: &'a crate::Selector,
    safe: bool,
    batch_byte_size: Option<u64>,
    batch_row_size: Option<usize>,
    max_row_size: Option<u64>,
    max_byte_size: Option<u64>,
    commit_row_size: Option<usize>,
    level: crate::Level,
}

impl ParquetOptions {
    fn identity(&self) -> ParquetOptionsIdentity<'_> {
        ParquetOptionsIdentity {
            compression: compression_identity(self.compression),
            max_row_group_size: self.max_row_group_size,
            key_value_metadata: &self.key_value_metadata,
            name: &self.name,
            field: &self.field,
            filter: &self.filter,
            select: &self.select,
            merge_by: &self.merge_by,
            safe: self.safe,
            batch_byte_size: self.batch_byte_size,
            batch_row_size: self.batch_row_size,
            max_row_size: self.max_row_size,
            max_byte_size: self.max_byte_size,
            commit_row_size: self.commit_row_size,
            level: self.level,
        }
    }

    /// Balanced defaults: Zstandard pages and 1,048,576-row groups.
    pub fn new() -> Self {
        Self {
            compression: Compression::ZSTD(ZstdLevel::default()),
            max_row_group_size: 1_048_576,
            key_value_metadata: Vec::new(),
            name: smol_str::SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: crate::Filter::always_true(),
            select: crate::Selector::all(),
            merge_by: crate::Selector::all(),
            safe: false,
            batch_byte_size: None,
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: crate::Level::DEFAULT,
            threads: None,
        }
    }

    /// The threads one file's columns may decode or encode on.
    fn column_threads(&self) -> usize {
        self.threads.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        })
    }

    /// Return these options with a different page compression.
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Return these options with a different row-group size.
    pub fn with_max_row_group_size(mut self, rows: usize) -> Self {
        self.max_row_group_size = rows;
        self
    }

    /// Return these options with one added footer metadata entry.
    pub fn with_key_value(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.key_value_metadata.push((key.into(), value.into()));
        self
    }

    /// Return the page compression in the spelling accepted by
    /// [`set_compression_name`](Self::set_compression_name).
    pub fn compression_name(&self) -> String {
        match self.compression {
            Compression::UNCOMPRESSED => "uncompressed".to_owned(),
            Compression::SNAPPY => "snappy".to_owned(),
            Compression::GZIP(level) => format!("gzip({})", level.compression_level()),
            Compression::LZO => "lzo".to_owned(),
            Compression::BROTLI(level) => format!("brotli({})", level.compression_level()),
            Compression::LZ4 => "lz4".to_owned(),
            Compression::ZSTD(level) => format!("zstd({})", level.compression_level()),
            Compression::LZ4_RAW => "lz4_raw".to_owned(),
        }
    }

    /// Parse and set the page compression from Parquet's canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns a typed record-option error when `compression` is not one of
    /// the spellings the Parquet writer accepts.
    pub fn set_compression_name(&mut self, compression: &str) -> crate::Result<()> {
        self.compression =
            compression
                .parse::<Compression>()
                .map_err(|error| crate::Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$.compression"),
                    reason: smol_str::SmolStr::new(error.to_string()),
                })?;
        Ok(())
    }

    /// Replace the maximum number of rows in one row group.
    pub const fn set_max_row_group_size(&mut self, rows: usize) {
        self.max_row_group_size = rows;
    }

    /// Replace the file-level key/value metadata written into the footer.
    pub fn set_key_value_metadata(&mut self, metadata: Vec<(String, String)>) {
        self.key_value_metadata = metadata;
    }

    /// Add one file-level key/value entry to the footer.
    pub fn push_key_value(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.key_value_metadata.push((key.into(), value.into()));
    }

    fn writer_properties(&self) -> WriterProperties {
        let mut builder = WriterProperties::builder()
            .set_compression(self.compression)
            .set_max_row_group_row_count(Some(self.max_row_group_size));
        if !self.key_value_metadata.is_empty() {
            builder = builder.set_key_value_metadata(Some(
                self.key_value_metadata
                    .iter()
                    .map(|(key, value)| {
                        parquet::file::metadata::KeyValue::new(key.clone(), value.clone())
                    })
                    .collect(),
            ));
        }
        builder.build()
    }
}

impl PartialEq for ParquetOptions {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for ParquetOptions {}

impl PartialOrd for ParquetOptions {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParquetOptions {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for ParquetOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

fn compression_identity(compression: Compression) -> CompressionIdentity {
    let (codec, level) = match compression {
        Compression::UNCOMPRESSED => (0, 0),
        Compression::SNAPPY => (1, 0),
        Compression::GZIP(level) => (2, i64::from(level.compression_level())),
        Compression::LZO => (3, 0),
        Compression::BROTLI(level) => (4, i64::from(level.compression_level())),
        Compression::LZ4 => (5, 0),
        Compression::ZSTD(level) => (6, i64::from(level.compression_level())),
        Compression::LZ4_RAW => (7, 0),
    };
    CompressionIdentity { codec, level }
}

impl Default for ParquetOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for ParquetOptions {
    crate::record_options_fields!();
}

/// Reject a handle whose media type declares a content coding.
///
/// Parquet compresses internally, so an outer coding would produce a file no
/// Parquet reader can open.
fn reject_outer_coding<H: IOBase + ?Sized>(handle: &H) -> Result<()> {
    let codec = handle.codec();
    if codec.is_identity() {
        return Ok(());
    }
    Err(Error::Core(CoreError::Codec {
        format: "parquet",
        position: 0,
        reason: smol_str::format_smolstr!(
            "expected an uncompressed parquet handle, got {codec} coding; parquet compresses \
             internally, so set ParquetOptions::compression instead of a {codec} suffix"
        ),
    }))
}

/// Refuse a union column before the writer is built: Parquet has no union
/// layout, and the Arrow writer's schema conversion has no answer for one.
fn reject_unions(schema: &Schema) -> Result<()> {
    fn union_path(field: &arrow_schema::Field, path: &str) -> Option<String> {
        use arrow_schema::DataType as D;
        match field.data_type() {
            D::Union(..) => Some(path.to_owned()),
            D::Struct(children) => children
                .iter()
                .find_map(|child| union_path(child, &format!("{path}.{}", child.name()))),
            D::List(item)
            | D::LargeList(item)
            | D::ListView(item)
            | D::LargeListView(item)
            | D::FixedSizeList(item, _)
            | D::Map(item, _) => union_path(item, &format!("{path}[]")),
            D::Dictionary(_, value) => union_path(
                &arrow_schema::Field::new(field.name(), value.as_ref().clone(), true),
                path,
            ),
            D::RunEndEncoded(_, values) => union_path(values, path),
            _ => None,
        }
    }
    match schema
        .fields()
        .iter()
        .find_map(|field| union_path(field, field.name()))
    {
        None => Ok(()),
        Some(path) => Err(Error::Core(CoreError::Codec {
            format: "parquet",
            position: 0,
            reason: smol_str::format_smolstr!(
                "expected columns parquet can store, got the union column {path:?}; parquet has \
                 no union layout, so write it to Arrow IPC or declare the column a variant"
            ),
        })),
    }
}

/// Read the Arrow schema of the file `handle` holds.
///
/// # Errors
///
/// Returns a read or footer failure.
pub fn read_arrow_schema<H: IOBase + ?Sized>(handle: &H) -> Result<Arc<Schema>> {
    schema_from_metadata(load_metadata(handle)?)
}

/// Read the exact non-null Struct root Field of the file `handle` holds.
///
/// A declared schema in `options` is returned as-is; otherwise the footer
/// supplies one, named by the options' root name.
///
/// # Errors
///
/// Returns a read, footer, or schema-projection failure.
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &ParquetOptions) -> Result<Field> {
    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    let schema = read_arrow_schema(handle)?;
    field_from_arrow_schema(options.name(), schema.as_ref())
}

/// Read the file `handle` holds, keeping only the columns `field` names.
///
/// A `field` naming a subset of the stored columns becomes a Parquet projection
/// mask, which is the format's own column pushdown: the column chunks it leaves
/// out are never located, decompressed, or decoded. This is the encoding where
/// a projection really does move less data, because a Parquet column chunk is
/// separately addressable while an Arrow IPC record batch is one message. A
/// `field` naming anything the file does not store is ignored, because a mask
/// can only drop columns, never invent them.
///
/// # Errors
///
/// Returns a read, footer, or decoding failure.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &ParquetOptions,
) -> Result<BatchReader> {
    let columns = options.apply_columns();
    let size = handle.size();
    if size == 0 {
        // Per the laziness contract, a missing file holds no batches.
        let schema = match options.field() {
            Some(field) => arrow_schema_from_field(&field)?,
            None => Arc::new(Schema::empty()),
        };
        let schema = match projection_indices(field, columns.as_deref(), &schema) {
            Some(indices) => Arc::new(schema.project(&indices)?),
            None => schema,
        };
        return Ok(Box::new(RecordBatchIterator::new(
            std::iter::empty(),
            schema,
        )));
    }
    let mut source = open_footer(handle, size, Some(options))?;
    // Every array a batch holds is allocated, decoded into and handed on per
    // batch, so an unbounded read takes the crate's batch size - the one the
    // other record encodings read at - rather than the Parquet crate's 1,024
    // rows. A limited read with no filter decodes no more than its limit asks
    // for; under a filter the limit counts result rows, not stored ones, so
    // it does not size the batches.
    let batch_rows = options.batch_row_size().unwrap_or_else(|| {
        options
            .max_row_size()
            .filter(|_| options.filter().is_always_true())
            .and_then(|rows| usize::try_from(rows).ok())
            .map_or(crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE, |rows| {
                rows.clamp(1, crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE)
            })
    });
    let projection = projection_indices(field, columns.as_deref(), source.metadata.schema());
    source.push_filter(options);
    source.fetch(handle, projection.as_deref())?;
    // A limited read stays lazy on one thread: the threads would decode
    // whole row groups ahead of a consumer that stops at its limit.
    let threads = if options.max_row_size().is_some() {
        1
    } else {
        options.column_threads()
    };
    if let Some(reader) = ParallelRead::open(&source, projection.as_deref(), batch_rows, threads)? {
        return Ok(reader);
    }
    let builder = source.builder().with_batch_size(batch_rows);
    let builder = match projection {
        // Root indices, not leaf indices: a nested column is one root, and its
        // whole subtree comes along with it.
        Some(indices) => {
            let mask = ProjectionMask::roots(builder.parquet_schema(), indices);
            builder.with_projection(mask)
        }
        None => builder,
    };
    Ok(Box::new(builder.build()?))
}

/// Replace the file `handle` holds with every batch `batches` yields.
///
/// The reader's own schema is the file's schema, so a caller replacing a file
/// with a declared root builds the reader over that root's Arrow projection -
/// which is what carries `PARQUET:field_id` into the file.
///
/// # Errors
///
/// Returns a schema, encoding, or write failure, or an error when the handle
/// declares an outer content coding.
pub fn overwrite_arrow_reader<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &ParquetOptions,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    overwrite_buffered(handle, batches, options, WRITE_BUFFER_BYTES)
}

/// [`overwrite_arrow_reader`] holding at most `buffer` Arrow bytes of input
/// before it feeds the column writers.
fn overwrite_buffered<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &ParquetOptions,
    buffer: usize,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    reject_outer_coding(handle)?;
    let schema = batches.schema();
    reject_unions(schema.as_ref())?;

    let mut encoded = Vec::new();
    let mut writer_options = ArrowWriterOptions::new().with_properties(options.writer_properties());
    if let Some(descriptor) = geospatial::extension_schema(schema.as_ref())? {
        // A geospatial or variant extension column: hand the writer a Parquet
        // schema carrying the matching logical type, and make sure the WKB
        // statistics factory is in place before the first geospatial page.
        geospatial::install_wkb_statistics();
        writer_options = writer_options.with_parquet_schema(descriptor);
    }
    let (mut file, columns) =
        ArrowWriter::try_new_with_options(&mut encoded, Arc::clone(&schema), writer_options)?
            .into_serialized_writer()?;
    let group_rows = options.max_row_group_size.max(1);
    let threads = options.column_threads();
    let mut group: Option<RowGroupEncoder> = None;
    let mut buffered = 0;
    // Arrow schema equality includes nullability and metadata, so comparing
    // would refuse a batch differing from the writer's only in a nullable flag
    // that holds no null, an `ARROW:extension:name`, or a schema-level entry -
    // all of which are the same rows. A batch naming the same columns is
    // reconciled instead, strictly; one naming different columns is different
    // data and is still refused, because a cast would invent the columns it is
    // missing. The plan is built once per layout the batches carry, and an
    // exact batch never reaches it.
    let root = crate::arrow::field_from_arrow_schema("row", schema.as_ref())?;
    let options = crate::ArrowCastOptions::new()
        .with_safe(false)
        .with_nullability(crate::Nullability::Strict);
    let mut plans = crate::cast::PlanCache::new();
    for (index, batch) in batches.enumerate() {
        let batch = batch.map_err(from_reader_error)?;
        let stored = batch.schema();
        let mismatch = |reason: &dyn std::fmt::Display| Error::SchemaMismatch {
            index: Some(index),
            path: smol_str::SmolStr::new_static("$"),
            diff: format!(
                "\u{2260} batch {index} {reason}\n  \u{2212} {}\n  + {}",
                crate::text::elide_display(&schema),
                crate::text::elide_display(&stored)
            ),
        };
        let batch = if Arc::ptr_eq(&stored, &schema) {
            batch
        } else if crate::arrow::same_columns(&schema, &stored) {
            plans
                .get_or_compile(stored.fields(), || {
                    crate::cast::ArrowCastPlan::compile_schema(
                        &stored,
                        &root,
                        options,
                        crate::cast::Deferred::default(),
                    )
                })
                .and_then(|plan| plan.reconcile_batch(batch))
                .map_err(|error| mismatch(&error))?
        } else {
            return Err(mismatch(&"names different columns than the written root"));
        };
        // A row group closes at exactly `max_row_group_size` rows, cutting a
        // batch that crosses the boundary, as `ArrowWriter` cuts it.
        let mut offset = 0;
        while offset < batch.num_rows() {
            let length = (group_rows - buffered).min(batch.num_rows() - offset);
            let encoder = match &mut group {
                Some(encoder) => encoder,
                None => group.insert(RowGroupEncoder::open(&file, &columns, threads, buffer)?),
            };
            encoder.push(&schema, &batch.slice(offset, length))?;
            buffered += length;
            offset += length;
            if buffered == group_rows {
                if let Some(encoder) = group.take() {
                    encoder.close(&mut file, &schema)?;
                }
                buffered = 0;
            }
        }
    }
    if let Some(encoder) = group.take() {
        encoder.close(&mut file, &schema)?;
    }
    file.close()?;
    handle.write_all_bytes(&encoded)?;
    Ok(())
}

/// The Arrow bytes worth spreading one feed of the column writers over
/// threads.
///
/// Below this, spawning costs more than it saves: a thread is tens of
/// microseconds, and a megabyte of columns encodes in a few milliseconds.
const PARALLEL_ROW_GROUP_BYTES: usize = 1024 * 1024;

/// The Arrow bytes a write holds before it feeds them to the column writers.
///
/// A row group's column writers stay open for the whole group, so input is
/// encoded as it arrives rather than when the group closes; on several
/// threads it is gathered first, so each column's share of one feed is worth
/// a thread. What a write holds is therefore this much Arrow input - the
/// slices themselves, their levels computed only as each is fed - plus the
/// batch that crossed it, beside the pages already encoded, however many
/// rows a row group takes.
const WRITE_BUFFER_BYTES: usize = 32 * 1024 * 1024;

/// One open row group: a writer per leaf column and, per root column, the
/// input its writers have yet to be fed.
///
/// The writers are the ones [`ArrowWriter`] would use, fed the same leaves
/// in the same order, and the chunks are appended in schema order, so the
/// file is byte for byte the one a sequential writer produces - only the
/// encoding runs side by side.
struct RowGroupEncoder {
    /// One writer per leaf column, in schema order.
    writers: Vec<ArrowColumnWriter>,
    /// How many leaf writers each root column has, in schema order.
    leaves: Vec<usize>,
    /// Each root column's slices not yet fed, in arrival order.
    pending: Vec<Vec<arrow_array::ArrayRef>>,
    /// The Arrow bytes of each root column's pending slices.
    weights: Vec<usize>,
    /// The Arrow bytes pending in all.
    buffered: usize,
    /// The threads a feed may use; one feeds each slice as it arrives.
    threads: usize,
    /// The pending bytes that trigger a feed.
    buffer: usize,
}

impl RowGroupEncoder {
    /// Open the column writers of the file's next row group.
    fn open<W: std::io::Write + Send>(
        file: &SerializedFileWriter<W>,
        columns: &ArrowRowGroupWriterFactory,
        threads: usize,
        buffer: usize,
    ) -> Result<Self> {
        let writers = columns.create_column_writers(file.flushed_row_groups().len())?;
        let descriptor = file.schema_descr();
        let roots = descriptor.root_schema().get_fields().len();
        let mut leaves = vec![0; roots];
        for leaf in 0..descriptor.num_columns() {
            if let Some(count) = leaves.get_mut(descriptor.get_column_root_idx(leaf)) {
                *count += 1;
            }
        }
        Ok(Self {
            writers,
            leaves,
            pending: (0..roots).map(|_| Vec::new()).collect(),
            weights: vec![0; roots],
            buffered: 0,
            threads,
            buffer,
        })
    }

    /// Take one batch's columns: fed at once on one thread, gathered for a
    /// feed on several.
    fn push(&mut self, schema: &Schema, batch: &RecordBatch) -> Result<()> {
        if self.threads <= 1 {
            let mut writers = self.writers.as_mut_slice();
            for ((field, column), count) in schema
                .fields()
                .iter()
                .zip(batch.columns())
                .zip(&self.leaves)
            {
                let (own, rest) = writers.split_at_mut((*count).min(writers.len()));
                write_root(field, column, own)?;
                writers = rest;
            }
            return Ok(());
        }
        for (root, column) in batch.columns().iter().enumerate() {
            let size = crate::arrow::sliced_array_size(column);
            if let (Some(pending), Some(weight)) =
                (self.pending.get_mut(root), self.weights.get_mut(root))
            {
                pending.push(Arc::clone(column));
                *weight += size;
                self.buffered += size;
            }
        }
        if self.buffered >= self.buffer {
            self.feed(schema)?;
        }
        Ok(())
    }

    /// Feed every pending slice to its root column's writers, the roots side
    /// by side when what is pending is worth the threads; the heaviest are
    /// claimed first, so the last to finish is a light one.
    fn feed(&mut self, schema: &Schema) -> Result<()> {
        let threads = if self.buffered < PARALLEL_ROW_GROUP_BYTES {
            1
        } else {
            self.threads.min(self.leaves.len())
        };
        self.buffered = 0;
        let mut work: Vec<RootWork<'_>> = Vec::with_capacity(self.leaves.len());
        let mut writers = self.writers.as_mut_slice();
        for (((field, count), pending), weight) in schema
            .fields()
            .iter()
            .zip(&self.leaves)
            .zip(self.pending.iter_mut())
            .zip(self.weights.iter_mut())
        {
            let (own, rest) = writers.split_at_mut((*count).min(writers.len()));
            writers = rest;
            work.push((std::mem::take(weight), field, own, std::mem::take(pending)));
        }
        if threads <= 1 {
            for (_, field, writers, slices) in work {
                for column in &slices {
                    write_root(field, column, writers)?;
                }
            }
            return Ok(());
        }
        // Claimed from the back, so the heaviest root goes first.
        work.sort_by_key(|(weight, _, _, _)| *weight);
        let queue = std::sync::Mutex::new(work);
        let failure = std::sync::Mutex::new(None);
        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| {
                    loop {
                        let Some((_, field, writers, slices)) =
                            queue.lock().ok().and_then(|mut queue| queue.pop())
                        else {
                            return;
                        };
                        let fed = slices
                            .iter()
                            .try_for_each(|column| write_root(field, column, writers));
                        if let Err(error) = fed {
                            // The row group is lost either way; the first
                            // failure is the one reported, and the other
                            // threads stop at their next claim.
                            if let Ok(mut failure) = failure.lock() {
                                failure.get_or_insert(error);
                            }
                            if let Ok(mut queue) = queue.lock() {
                                queue.clear();
                            }
                            return;
                        }
                    }
                });
            }
        });
        match failure.into_inner() {
            Ok(None) => Ok(()),
            Ok(Some(error)) => Err(error),
            Err(_) => Err(parquet::errors::ParquetError::General(
                "expected every column encoder to finish, got a poisoned failure".to_owned(),
            )
            .into()),
        }
    }

    /// Feed what is pending, close every writer, and append the row group.
    fn close<W: std::io::Write + Send>(
        mut self,
        file: &mut SerializedFileWriter<W>,
        schema: &Schema,
    ) -> Result<()> {
        self.feed(schema)?;
        let mut row_group = file.next_row_group()?;
        for writer in self.writers {
            writer.close()?.append_to_row_group(&mut row_group)?;
        }
        row_group.close()?;
        Ok(())
    }
}

/// One root column's share of a feed: its weight, its field, its leaf
/// writers, and the slices they have yet to be fed.
type RootWork<'a> = (
    usize,
    &'a arrow_schema::FieldRef,
    &'a mut [ArrowColumnWriter],
    Vec<arrow_array::ArrayRef>,
);

/// Feed one root column's slice to its leaf writers.
///
/// # Errors
///
/// Returns the leaves' computation or encoding failure, or a slice whose
/// leaves do not match the writers the schema gave the root.
fn write_root(
    field: &arrow_schema::FieldRef,
    column: &arrow_array::ArrayRef,
    writers: &mut [ArrowColumnWriter],
) -> Result<()> {
    let leaves = compute_leaves(field.as_ref(), column)?;
    if leaves.len() != writers.len() {
        return Err(parquet::errors::ParquetError::General(format!(
            "expected {} leaf columns under {:?}, got {}",
            writers.len(),
            field.name(),
            leaves.len()
        ))
        .into());
    }
    for (leaf, writer) in leaves.iter().zip(writers) {
        writer.write(leaf)?;
    }
    Ok(())
}

/// Read the footer statistics of the file `handle` holds.
///
/// This is the input a query planner or an Iceberg manifest writer needs:
/// per-row-group counts, sizes, null counts, and value bounds.
///
/// # Errors
///
/// Returns a read or footer failure.
pub fn read_statistics<H: IOBase + ?Sized>(handle: &H) -> Result<FileStatistics> {
    Ok(FileStatistics::from_metadata(
        load_metadata(handle)?.as_ref(),
    ))
}

/// Return the whole file's logical row count from its footer.
///
/// Projection, partition filters, and read limits in `options` deliberately do
/// not affect this metadata answer.
pub(crate) fn row_size<H: IOBase + ?Sized>(
    handle: &H,
    _options: &ParquetOptions,
) -> crate::Result<u64> {
    if handle.is_empty() {
        return Ok(0);
    }
    metadata_row_size(load_metadata(handle)?.as_ref())
}

/// Convert Parquet's signed footer count into the public unsigned dimension.
fn metadata_row_size(metadata: &ParquetMetaData) -> crate::Result<u64> {
    u64::try_from(metadata.file_metadata().num_rows()).map_err(|_| crate::Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$"),
        reason: smol_str::SmolStr::new_static(
            "Parquet footer contains a negative logical row count",
        ),
    })
}

/// Parse a file's footer without caching it.
fn load_metadata<H: IOBase + ?Sized>(handle: &H) -> Result<Arc<ParquetMetaData>> {
    use parquet::errors::ParquetError;

    reject_outer_coding(handle)?;
    const TAIL: u64 = 8;
    let size = handle.size();
    if size < TAIL {
        return Err(ParquetError::EOF(format!(
            "expected an eight-byte Parquet footer tail, got {size} bytes"
        ))
        .into());
    }
    let tail = handle.read_range_bytes(size - TAIL, TAIL as usize)?;
    if tail.len() != TAIL as usize {
        return Err(ParquetError::EOF(format!(
            "expected an eight-byte Parquet footer tail, got {} bytes",
            tail.len()
        ))
        .into());
    }
    if &tail[4..] != b"PAR1" {
        return Err(ParquetError::General(
            "expected Parquet magic at the end of the file".to_owned(),
        )
        .into());
    }
    let footer_length = u64::from(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]));
    let footer_start = (size - TAIL).checked_sub(footer_length).ok_or_else(|| {
        Error::from(ParquetError::EOF(format!(
            "footer declares {footer_length} metadata bytes in a {size}-byte file"
        )))
    })?;
    let footer_length = usize::try_from(footer_length).map_err(|_| {
        Error::from(ParquetError::General(
            "Parquet footer length does not fit this address space".to_owned(),
        ))
    })?;
    let footer = handle.read_range_bytes(footer_start, footer_length)?;
    if footer.len() != footer_length {
        return Err(ParquetError::EOF(format!(
            "expected {footer_length} Parquet footer bytes, got {} bytes",
            footer.len()
        ))
        .into());
    }
    Ok(Arc::new(ParquetMetaDataReader::decode_metadata(&footer)?))
}

/// Recover the embedded Arrow schema from a footer already in hand.
fn schema_from_metadata(metadata: Arc<ParquetMetaData>) -> Result<Arc<Schema>> {
    let metadata = reader_metadata(metadata)?;
    Ok(Arc::clone(metadata.schema()))
}

/// The reader metadata for one footer, with the variant extension attached
/// wherever the Parquet schema says `VARIANT` and the Arrow schema does not.
///
/// A file written here already declares it, so this is the read of a
/// *foreign* file: the annotation is what says the two binaries are one
/// variant, and reading it is what makes the column import as one.
fn reader_metadata(metadata: Arc<ParquetMetaData>) -> Result<ArrowReaderMetadata> {
    let read = ArrowReaderMetadata::try_new(Arc::clone(&metadata), ArrowReaderOptions::new())?;
    let Some(schema) = geospatial::variant_schema(
        metadata.file_metadata().schema_descr(),
        read.schema().as_ref(),
    ) else {
        return Ok(read);
    };
    Ok(ArrowReaderMetadata::try_new(
        metadata,
        ArrowReaderOptions::new().with_schema(Arc::new(schema)),
    )?)
}

/// The bytes one read decodes, the footer they end in, and the row groups it
/// keeps - everything a reader builder is made from, so a read can make
/// several over one fetch.
struct ParquetSource {
    /// The fetched column chunks: empty until [`Self::fetch`], then the
    /// ranges the kept row groups and projected columns occupy.
    data: FetchedRanges,
    /// The decoded footer and the Arrow schema it describes.
    metadata: ArrowReaderMetadata,
    /// The row groups the read keeps, when a row bound or the filter spares
    /// the rest.
    row_groups: Option<Vec<usize>>,
}

impl ParquetSource {
    /// Keep only the row groups the read's filter could find a row in.
    ///
    /// Each conjunct is bound against the root the rows are filtered as, and
    /// every row group's footer statistics become the same
    /// [`Bounds`](crate::expression::Bounds) an Iceberg manifest entry or a
    /// Hive path becomes, so one pruning rule answers all three. Only a
    /// column stored as the type the filter reads takes part - a declared
    /// root that casts a column filters the cast value, which the stored
    /// bounds do not bound. A filter that runs after the selection, a
    /// conjunct that does not bind, and a statistic the file does not carry
    /// prune nothing, and every row of every group that survives is still
    /// filtered afterwards, so pruning only ever saves a read.
    fn push_filter(&mut self, options: &ParquetOptions) {
        let filter = options.filter();
        if filter.is_always_true() {
            return;
        }
        let stored = Arc::clone(self.metadata.schema());
        if crate::expression::filter_after_select(
            filter,
            options.select(),
            stored.fields().iter().map(|field| field.name().as_str()),
        ) {
            return;
        }
        let root = match options.field() {
            Some(field) => field,
            None => match field_from_arrow_schema(options.name(), stored.as_ref()) {
                Ok(field) => field,
                Err(_) => return,
            },
        };
        self.prune(&filter.simplify().conjuncts(), &root, &stored);
    }

    /// Keep only the row groups the bound conjuncts could find a row in.
    fn prune(&mut self, conjuncts: &[crate::Filter], root: &Field, stored: &Schema) {
        use parquet::arrow::arrow_reader::statistics::StatisticsConverter;

        let bound: Vec<crate::expression::Bound> = conjuncts
            .iter()
            .filter_map(|conjunct| conjunct.bind(root).ok())
            .collect();
        if bound.is_empty() {
            return;
        }
        let groups = self.metadata.metadata().row_groups();
        let kept: Vec<usize> = match &self.row_groups {
            Some(kept) => kept.clone(),
            None => (0..groups.len()).collect(),
        };
        let parquet_schema = self.metadata.parquet_schema();
        let mut names: Vec<String> = conjuncts.iter().flat_map(crate::Filter::columns).collect();
        names.dedup();
        let mut columns = Vec::new();
        for name in names {
            let Some((index, declared)) = stored_column(root, stored, &name) else {
                continue;
            };
            if columns
                .iter()
                .any(|column: &StoredBounds| column.field.name() == declared.name())
            {
                continue;
            }
            let Ok(converter) =
                StatisticsConverter::try_new(stored.field(index).name(), stored, parquet_schema)
            else {
                continue;
            };
            // A count the writer did not record is unknown, never zero: an
            // `is null` must not prune a group that may hold nulls.
            let converter = converter.with_missing_null_counts_as_zero(false);
            let Some(leaf) = converter.parquet_column_index() else {
                continue;
            };
            let chosen = || kept.iter().filter_map(|index| groups.get(*index));
            let (Ok(minimums), Ok(maximums), Ok(nulls)) = (
                converter.row_group_mins(chosen()),
                converter.row_group_maxes(chosen()),
                converter.row_group_null_counts(chosen()),
            ) else {
                continue;
            };
            let use_extremes = extremes_bound(&declared);
            // A declared non-null column over a stored nullable one reads each
            // stored null as the type's default, which no statistic describes.
            let fills_nulls = !declared.is_nullable() && stored.field(index).is_nullable();
            // Each statistic column lands once under the field the filter
            // reads, and a group's bound is one cell of it; a column that
            // does not land - another layout, a value the field refuses -
            // bounds nothing.
            let field = Arc::new(declared.with_nullable(true));
            let landed = |values: arrow_array::ArrayRef| {
                crate::serie::land(Arc::clone(&field), values, &crate::serie::Proof::Unproven).ok()
            };
            columns.push(StoredBounds {
                minimums: landed(minimums),
                maximums: landed(maximums),
                field,
                leaf,
                use_extremes,
                fills_nulls,
                nulls,
            });
        }

        let bound_at = |values: Option<&crate::Serie>, position: usize| {
            values?
                .scalar(position)
                .ok()
                .filter(|value| !value.is_null())
        };
        let mut survivors = Vec::with_capacity(kept.len());
        for (position, index) in kept.iter().enumerate() {
            let Some(group) = groups.get(*index) else {
                continue;
            };
            let mut bounds = crate::expression::Bounds::new(u64::try_from(group.num_rows()).ok());
            for column in &columns {
                use arrow_array::Array as _;
                let nulls = (!column.nulls.is_null(position)).then(|| column.nulls.value(position));
                if column.fills_nulls && nulls != Some(0) {
                    // Some rows read as a default the statistics never saw.
                    continue;
                }
                // Writers before parquet-mr 1.10 ordered strings, bytes and
                // unsigned integers as signed bytes in the deprecated fields;
                // those extremes do not bound the values in this crate's
                // order, as parquet-mr and Arrow C++ also refuse them.
                let legacy = group.column(column.leaf).statistics().is_some_and(|stats| {
                    stats.is_min_max_deprecated()
                        && parquet_schema.column(column.leaf).sort_order()
                            != parquet::basic::SortOrder::SIGNED
                });
                let (minimum, maximum) = if column.use_extremes && !legacy {
                    (
                        bound_at(column.minimums.as_ref(), position),
                        bound_at(column.maximums.as_ref(), position),
                    )
                } else {
                    (None, None)
                };
                bounds = bounds.with_column(column.field.name(), minimum, maximum, nulls);
            }
            if bound
                .iter()
                .all(|conjunct| conjunct.statistics_prune(&bounds))
            {
                survivors.push(*index);
            }
        }
        if survivors.len() < kept.len() {
            self.row_groups = Some(survivors);
        }
    }

    /// Fetch the column chunks the kept row groups hold for the projected
    /// roots.
    ///
    /// A source [`open_footer`] read whole already holds them. Otherwise each
    /// chunk's footer byte range is read through the handle into owned memory,
    /// ranges no more than [`FETCH_GAP_BYTES`] apart coalesced into one read:
    /// a pruned row group or an unprojected column is read only when it lies
    /// in such a gap between kept ones, and on an object store a gap costs
    /// less than the request it saves. The copies are the reader's own:
    /// nothing a batch holds points back into the handle's storage, so
    /// rewriting the file while a reader or its batches live is safe.
    ///
    /// # Errors
    ///
    /// Returns the handle's read failure.
    fn fetch<H: IOBase + ?Sized>(
        &mut self,
        handle: &H,
        projection: Option<&[usize]>,
    ) -> Result<()> {
        if self.data.whole {
            return Ok(());
        }
        let metadata = self.metadata.metadata();
        let parquet_schema = metadata.file_metadata().schema_descr();
        let leaves: Vec<usize> = (0..parquet_schema.num_columns())
            .filter(|leaf| {
                projection
                    .is_none_or(|roots| roots.contains(&parquet_schema.get_column_root_idx(*leaf)))
            })
            .collect();
        let groups = metadata.row_groups();
        let kept: Vec<usize> = match &self.row_groups {
            Some(kept) => kept.clone(),
            None => (0..groups.len()).collect(),
        };
        let mut wanted: Vec<(u64, u64)> = Vec::new();
        for group in kept.iter().filter_map(|index| groups.get(*index)) {
            for leaf in &leaves {
                let Some(column) = group.columns().get(*leaf) else {
                    continue;
                };
                // `byte_range` asserts what a crafted footer can violate, so
                // the range is read here, and a negative one - or one past the
                // end of the file - is fetched only as far as the file goes:
                // the reader then reports the short chunk as the error it is.
                let start = column
                    .dictionary_page_offset()
                    .unwrap_or_else(|| column.data_page_offset());
                let (Ok(start), Ok(length)) = (
                    u64::try_from(start),
                    u64::try_from(column.compressed_size()),
                ) else {
                    continue;
                };
                let end = start.saturating_add(length).min(self.data.length);
                if start < end {
                    wanted.push((start, end));
                }
            }
        }
        wanted.sort_unstable();
        let mut coalesced: Vec<(u64, u64)> = Vec::with_capacity(wanted.len());
        for (start, end) in wanted {
            match coalesced.last_mut() {
                Some(last) if start <= last.1.saturating_add(FETCH_GAP_BYTES) => {
                    last.1 = last.1.max(end);
                }
                _ => coalesced.push((start, end)),
            }
        }
        let mut ranges = Vec::with_capacity(coalesced.len());
        for (start, end) in coalesced {
            let length = usize::try_from(end - start).map_err(|_| {
                CoreError::from(crate::arrow::Error::from(ArrowError::ExternalError(
                    format!(
                        "expected a Parquet column chunk this platform can address, got {} bytes",
                        end - start
                    )
                    .into(),
                )))
            })?;
            ranges.push((start, Bytes::from(handle.read_range_bytes(start, length)?)));
        }
        self.data.ranges = ranges.into();
        Ok(())
    }

    /// A reader builder over the fetched ranges, sharing them rather than
    /// copying.
    fn builder(&self) -> ParquetRecordBatchReaderBuilder<FetchedRanges> {
        let builder = ParquetRecordBatchReaderBuilder::new_with_metadata(
            self.data.clone(),
            self.metadata.clone(),
        );
        match &self.row_groups {
            Some(groups) => builder.with_row_groups(groups.clone()),
            None => builder,
        }
    }
}

/// One stored column's footer statistics, across the kept row groups.
struct StoredBounds {
    /// The field the filter reads the column as.
    field: Arc<Field>,
    /// The column's leaf in the Parquet schema.
    leaf: usize,
    /// Whether the minimums and maximums bound the values the filter reads.
    use_extremes: bool,
    /// Whether the read turns the stored nulls into the type's default.
    fills_nulls: bool,
    /// Each kept group's minimum, or `None` where the statistics do not
    /// land under the field.
    minimums: Option<crate::Serie>,
    /// Each kept group's maximum, likewise.
    maximums: Option<crate::Serie>,
    /// Each kept group's null count; a null entry is a count not recorded.
    nulls: arrow_array::UInt64Array,
}

/// Whether a column's footer minimum and maximum bound the values a filter
/// reads it as.
///
/// A floating-point column's do not: Parquet writers leave NaN out of them,
/// while this crate orders NaN past every number, above or below by its sign,
/// so a group whose extremes rule a comparison out can still hold a NaN row
/// it keeps. Its null count still prunes.
fn extremes_bound(field: &Field) -> bool {
    !field.clone().into_arrow_field().is_ok_and(|arrow| {
        matches!(
            arrow.data_type(),
            arrow_schema::DataType::Float16
                | arrow_schema::DataType::Float32
                | arrow_schema::DataType::Float64
        )
    })
}

/// The stored root column a filter column reads, with the field the filter
/// reads it as - when it is stored as that type, and only then.
///
/// A declared child that derives its value - a partition, digest or
/// transform column - reads something other than what is stored under its
/// name, so its statistics bound nothing the filter sees.
fn stored_column(root: &Field, stored: &Schema, name: &str) -> Option<(usize, Field)> {
    let declared = root
        .fields()
        .iter()
        .find(|child| child.name().eq_ignore_ascii_case(name))?;
    if declared.metadata_iter().any(|(key, _)| {
        key.starts_with("PARTITION:") || key.starts_with("DIGEST:") || key.starts_with("TRANSFORM:")
    }) {
        return None;
    }
    let (index, column) = stored
        .fields()
        .iter()
        .enumerate()
        .find(|(_, column)| column.name().eq_ignore_ascii_case(declared.name()))?;
    declared
        .clone()
        .into_arrow_field()
        .is_ok_and(|filtered| filtered.data_type() == column.data_type())
        .then(|| (index, declared.clone()))
}

/// Open a reader builder over a handle's complete bytes.
///
/// The whole-value door that geospatial statistics and the `internals`
/// forwarder read through: a builder over bytes the caller may project any
/// way it likes afterwards.
fn open_builder<H: IOBase + ?Sized>(handle: &H) -> Result<ParquetRecordBatchReaderBuilder<Bytes>> {
    reject_outer_coding(handle)?;
    let bytes = Bytes::from(handle.read_all_bytes()?);
    let metadata = ParquetMetaDataReader::new().parse_and_finish(&bytes)?;
    Ok(ParquetRecordBatchReaderBuilder::new_with_metadata(
        bytes,
        reader_metadata(Arc::new(metadata))?,
    ))
}

/// Gap between two needed byte ranges below which one read covers both.
///
/// Reading a short gap costs less than a second request - a copy of a
/// megabyte is some fifty microseconds, and an object-store request tens of
/// milliseconds - so column chunks that nearly touch are fetched as one.
const FETCH_GAP_BYTES: u64 = 1024 * 1024;

/// The byte ranges of one Parquet file a read has fetched, each owned.
///
/// The Parquet reader asks for a column chunk by its offset in the file;
/// this answers it from the one fetched range that holds it. Clones share
/// the ranges.
#[derive(Clone, Debug)]
struct FetchedRanges {
    /// The file's length, which the footer's offsets are counted in.
    length: u64,
    /// Whether the one range is the whole file, fetched before the footer.
    whole: bool,
    /// The fetched ranges in file order, each with the offset it starts at.
    ranges: Arc<[(u64, Bytes)]>,
}

impl FetchedRanges {
    /// The fetched range holding `length` bytes from `start`.
    fn holding(&self, start: u64, length: usize) -> parquet::errors::Result<(usize, &Bytes)> {
        let after = self.ranges.partition_point(|(offset, _)| *offset <= start);
        after
            .checked_sub(1)
            .and_then(|index| self.ranges.get(index))
            .and_then(|(offset, bytes)| {
                let from = usize::try_from(start - offset).ok()?;
                (from.checked_add(length)? <= bytes.len()).then_some((from, bytes))
            })
            .ok_or_else(|| {
                parquet::errors::ParquetError::EOF(format!(
                    "expected {length} bytes at offset {start} of the Parquet file to be fetched"
                ))
            })
    }
}

impl parquet::file::reader::Length for FetchedRanges {
    fn len(&self) -> u64 {
        self.length
    }
}

impl parquet::file::reader::ChunkReader for FetchedRanges {
    type T = bytes::buf::Reader<Bytes>;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        use bytes::Buf as _;
        let (from, bytes) = self.holding(start, 0)?;
        Ok(bytes.slice(from..).reader())
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        let (from, bytes) = self.holding(start, length)?;
        Ok(bytes.slice(from..from + length))
    }
}

/// Files up to this size are fetched whole, in one read: a second request
/// costs an object store more than the bytes a footer-first plan could skip.
const WHOLE_READ_BYTES: u64 = 1024 * 1024;

/// Bytes a larger file's first read takes from its end, so the footer
/// usually arrives with the eight-byte tail rather than in a second request.
const FOOTER_PREFETCH_BYTES: u64 = 64 * 1024;

/// Read a file's footer, and the leading row groups a row bound keeps.
///
/// A file of at most [`WHOLE_READ_BYTES`] is read whole, in one request.
/// A larger one - or any file under a row bound that may spare most of it -
/// has its footer read first and its column chunks only through
/// [`ParquetSource::fetch`], once pruning and projection have said which.
/// With a row bound and no filter - stored rows then are result rows - only
/// the leading row groups whose counts cover
/// [`max_row_size`](IORecordOptions::max_row_size) are kept. The bound is a
/// fetch plan, not the limit itself: the record methods above still trim the
/// result to the exact row count, so this changes what is *read*, never what
/// a limited read yields.
///
/// A tail that is not a Parquet footer is read the whole-value way instead,
/// so every malformed file is reported by the Parquet crate's own parser.
///
/// # Errors
///
/// Returns a read failure, a malformed footer, or a footer whose embedded
/// Arrow schema cannot be interpreted.
fn open_footer<H: IOBase + ?Sized>(
    handle: &H,
    size: u64,
    options: Option<&ParquetOptions>,
) -> Result<ParquetSource> {
    reject_outer_coding(handle)?;
    let bound = options
        .filter(|options| options.filter().is_always_true())
        .and_then(IORecordOptions::max_row_size);
    let footer = if size < FOOTER_TAIL {
        None
    } else if size <= WHOLE_READ_BYTES {
        match bound {
            // One read of the whole file is the cheapest plan it has.
            None => None,
            // The tail alone, so a bound that spares most of the file reads
            // only the footer and the rows it keeps.
            Some(_) => {
                let tail = handle.read_range_bytes(size - FOOTER_TAIL, FOOTER_TAIL as usize)?;
                footer_metadata(handle, size, &tail)?
            }
        }
    } else {
        let prefetch = size.min(FOOTER_PREFETCH_BYTES);
        let end =
            handle.read_range_bytes(size - prefetch, usize::try_from(prefetch).unwrap_or(0))?;
        footer_metadata(handle, size, &end)?
    };
    let Some(metadata) = footer else {
        let bytes = Bytes::from(handle.read_all_bytes()?);
        let metadata = ParquetMetaDataReader::new().parse_and_finish(&bytes)?;
        return Ok(ParquetSource {
            data: FetchedRanges {
                length: bytes.len() as u64,
                whole: true,
                ranges: Arc::from([(0, bytes)]),
            },
            metadata: reader_metadata(Arc::new(metadata))?,
            row_groups: None,
        });
    };
    let mut row_groups = None;
    if let Some(max_rows) = bound {
        let mut selected = Vec::new();
        let mut covered = 0_u64;
        for (index, group) in metadata.row_groups().iter().enumerate() {
            if covered >= max_rows {
                break;
            }
            selected.push(index);
            covered = covered.saturating_add(u64::try_from(group.num_rows()).unwrap_or(0));
        }
        if selected.len() < metadata.num_row_groups() {
            row_groups = Some(selected);
        }
    }
    Ok(ParquetSource {
        data: FetchedRanges {
            length: size,
            whole: false,
            ranges: Arc::from([]),
        },
        metadata: reader_metadata(Arc::new(metadata))?,
        row_groups,
    })
}

/// The footer length and the closing magic every Parquet file ends in.
const FOOTER_TAIL: u64 = 8;

/// Decode the footer that a file's last bytes, `end`, finish with.
///
/// Answers from `end` when it holds the whole footer, and reads the footer's
/// own range otherwise. `None` is a tail that is not a Parquet footer, or a
/// footer that does not decode - left for the whole-value parser to report.
///
/// # Errors
///
/// Returns the handle's read failure.
fn footer_metadata<H: IOBase + ?Sized>(
    handle: &H,
    size: u64,
    end: &[u8],
) -> Result<Option<ParquetMetaData>> {
    let tail = FOOTER_TAIL as usize;
    let Some(closing) = end.len().checked_sub(tail).map(|at| &end[at..]) else {
        return Ok(None);
    };
    if &closing[4..] != b"PAR1" {
        return Ok(None);
    }
    let length = u64::from(u32::from_le_bytes([
        closing[0], closing[1], closing[2], closing[3],
    ]));
    let (Some(start), Ok(length)) = (
        (size - FOOTER_TAIL).checked_sub(length),
        usize::try_from(length),
    ) else {
        return Ok(None);
    };
    let held_from = size - end.len() as u64;
    if start >= held_from {
        let from = usize::try_from(start - held_from).unwrap_or(usize::MAX);
        return Ok(end
            .get(from..end.len() - tail)
            .and_then(|footer| ParquetMetaDataReader::decode_metadata(footer).ok()));
    }
    let footer = handle.read_range_bytes(start, length)?;
    Ok(ParquetMetaDataReader::decode_metadata(&footer).ok())
}

/// The compressed column bytes a read decodes before it splits across
/// threads.
///
/// Below this, spawning costs more than it saves: a thread is tens of
/// microseconds, and a megabyte of pages decodes in a few milliseconds.
const PARALLEL_READ_BYTES: u64 = 1024 * 1024;

/// Batches one unit decodes ahead of the consumer before it waits.
///
/// Sixteen batches of the default 65,536 rows cover a row group of the
/// million rows most writers cut, so decoders of later row groups rarely
/// wait on the front one, and a single huge row group is held a bounded
/// number of batches ahead rather than whole.
const READ_AHEAD_BATCHES: usize = 16;

/// One decoded batch, or the failure that ended a unit.
type Decoded = std::result::Result<RecordBatch, ArrowError>;

/// A read decoded on several threads, handed out in file order.
///
/// The work is cut into units of one row group and one group of projected
/// root columns, each its own Parquet reader over the same shared bytes and
/// batch size. Row groups come first: a read of at least as many row groups
/// as it has threads decodes whole row groups side by side, and a read of
/// fewer deals each row group's columns into groups of roughly equal
/// uncompressed size, so a single-row-group file still uses every thread.
/// The column groups of one row group are cut into batches at the same
/// rows, so the reader joins their i-th batches side by side in file column
/// order. A batch never spans two row groups.
///
/// At most `threads` units decode at once, and each runs at most
/// [`READ_AHEAD_BATCHES`] batches ahead of the consumer before it waits, so
/// what the reader holds is bounded by the batch size, never by how large a
/// writer made its row groups; the next row group starts as the consumer
/// finishes one.
///
/// **Dropping the reader detaches the decoders rather than joining them**,
/// as a parallel table scan's workers are: each owns its reader, the copied
/// column chunks it decodes, and its sender, so nothing borrowed outlives the
/// drop, and each stops at its next send.
struct ParallelRead {
    /// The schema a single reader over every projected column reports.
    schema: Arc<Schema>,
    /// The shared bytes and footer every unit reads.
    source: ParquetSource,
    /// Rows per batch, the same in every unit.
    batch_rows: usize,
    /// The projected root columns of each column group, in file order.
    column_groups: Vec<Vec<usize>>,
    /// For each output column, its column group and its position there.
    layout: Vec<(usize, usize)>,
    /// The row groups not yet started, in file order.
    pending: std::collections::VecDeque<usize>,
    /// The row groups started and not yet drained, in file order, each with
    /// one receiver per column group.
    running: std::collections::VecDeque<Vec<std::sync::mpsc::Receiver<Decoded>>>,
    /// How many row groups decode at once.
    window: usize,
    /// Whether the reader has finished, cleanly or not.
    done: bool,
}

impl ParallelRead {
    /// Split a read across threads, when it is worth them.
    ///
    /// Answers `None` - the caller reads with one reader - when it may use
    /// one thread, when there is only one row group and one column to split,
    /// or when its column chunks are below [`PARALLEL_READ_BYTES`].
    fn open(
        source: &ParquetSource,
        projection: Option<&[usize]>,
        batch_rows: usize,
        threads: usize,
    ) -> Result<Option<BatchReader>> {
        let roots: Vec<usize> = match projection {
            Some(indices) => indices.to_vec(),
            None => (0..source.metadata.schema().fields().len()).collect(),
        };
        let groups = source.metadata.metadata().row_groups();
        let kept: Vec<usize> = match &source.row_groups {
            Some(kept) => kept.clone(),
            None => (0..groups.len()).collect(),
        };
        if threads < 2 || roots.is_empty() || kept.is_empty() {
            return Ok(None);
        }
        let parquet_schema = source.metadata.parquet_schema();
        let mut compressed = 0_u64;
        let mut weights = vec![0_u64; source.metadata.schema().fields().len()];
        for group in kept.iter().filter_map(|index| groups.get(*index)) {
            for (leaf, column) in group.columns().iter().enumerate() {
                let root = parquet_schema.get_column_root_idx(leaf);
                if roots.contains(&root) {
                    compressed += u64::try_from(column.compressed_size()).unwrap_or(0);
                    if let Some(weight) = weights.get_mut(root) {
                        *weight += u64::try_from(column.uncompressed_size()).unwrap_or(0);
                    }
                }
            }
        }
        // Whole row groups first; what the row groups leave of the threads
        // splits each one's columns.
        let per_group = (threads / kept.len().min(threads)).min(roots.len()).max(1);
        if compressed < PARALLEL_READ_BYTES || (kept.len() < 2 && per_group < 2) {
            return Ok(None);
        }

        // Largest first, each to the lightest column group so far.
        let mut order = roots.clone();
        order.sort_by_key(|root| std::cmp::Reverse(weights[*root]));
        let mut dealt: Vec<(u64, Vec<usize>)> = (0..per_group).map(|_| (0, Vec::new())).collect();
        for root in order {
            if let Some(lightest) = dealt.iter_mut().min_by_key(|(weight, _)| *weight) {
                lightest.0 += weights[root];
                lightest.1.push(root);
            }
        }
        let column_groups: Vec<Vec<usize>> = dealt
            .into_iter()
            .map(|(_, mut members)| {
                members.sort_unstable();
                members
            })
            .filter(|members| !members.is_empty())
            .collect();

        // What one reader over every projected column would report.
        let schema = source
            .builder()
            .with_batch_size(batch_rows)
            .with_projection(ProjectionMask::roots(parquet_schema, roots.iter().copied()))
            .build()
            .map(|reader| arrow_array::RecordBatchReader::schema(&reader))?;
        let mut sorted = roots;
        sorted.sort_unstable();
        let layout = sorted
            .iter()
            .map(|root| {
                column_groups
                    .iter()
                    .enumerate()
                    .find_map(|(group, members)| {
                        members
                            .iter()
                            .position(|member| member == root)
                            .map(|position| (group, position))
                    })
                    .unwrap_or_default()
            })
            .collect();
        let window = (threads / column_groups.len()).max(1);
        let mut read = Self {
            schema,
            source: ParquetSource {
                data: source.data.clone(),
                metadata: source.metadata.clone(),
                row_groups: None,
            },
            batch_rows,
            column_groups,
            layout,
            pending: kept.into(),
            running: std::collections::VecDeque::with_capacity(window),
            window,
            done: false,
        };
        for _ in 0..read.window {
            read.start_next()?;
        }
        Ok(Some(Box::new(read)))
    }

    /// Start decoding the next row group, one thread per column group.
    fn start_next(&mut self) -> Result<()> {
        let Some(row_group) = self.pending.pop_front() else {
            return Ok(());
        };
        let mut receivers = Vec::with_capacity(self.column_groups.len());
        for members in &self.column_groups {
            let reader = self
                .source
                .builder()
                .with_row_groups(vec![row_group])
                .with_batch_size(self.batch_rows)
                .with_projection(ProjectionMask::roots(
                    self.source.metadata.parquet_schema(),
                    members.iter().copied(),
                ))
                .build()?;
            let (sender, receiver) = std::sync::mpsc::sync_channel(READ_AHEAD_BATCHES);
            // Deliberately detached: see the type docs for why drop does not join.
            let _ = std::thread::spawn(move || decode_unit(reader, &sender));
            receivers.push(receiver);
        }
        self.running.push_back(receivers);
        Ok(())
    }

    /// The next batch of the row group in front, or `None` when it is done.
    fn next_joined(&mut self) -> Option<Decoded> {
        let units = self.running.front()?;
        let mut batches = Vec::with_capacity(units.len());
        let mut ended = 0;
        for unit in units {
            match unit.recv() {
                Ok(Ok(batch)) => batches.push(batch),
                Ok(Err(error)) => return Some(Err(error)),
                // A decoder that is done has dropped its sender.
                Err(_) => ended += 1,
            }
        }
        if ended == units.len() {
            return None;
        }
        if ended > 0 {
            return Some(Err(ArrowError::ComputeError(
                "expected every Parquet column group to yield the same batches, got one that \
                 ended early"
                    .to_owned(),
            )));
        }
        let rows = batches[0].num_rows();
        if batches.iter().any(|batch| batch.num_rows() != rows) {
            return Some(Err(ArrowError::ComputeError(format!(
                "expected every Parquet column group to yield {rows} rows, got {:?}",
                batches
                    .iter()
                    .map(RecordBatch::num_rows)
                    .collect::<Vec<_>>()
            ))));
        }
        if batches.len() == 1 {
            return batches.pop().map(Ok);
        }
        let columns = self
            .layout
            .iter()
            .map(|(group, position)| Arc::clone(batches[*group].column(*position)))
            .collect();
        Some(RecordBatch::try_new_with_options(
            Arc::clone(&self.schema),
            columns,
            &arrow_array::RecordBatchOptions::new().with_row_count(Some(rows)),
        ))
    }
}

/// Decode one unit on its own thread.
///
/// Every send doubles as the liveness check, and the body is unwind-guarded
/// so a panicking decoder reports an error rather than leaving the consumer
/// to read its silence as the end of the row group.
fn decode_unit(
    reader: parquet::arrow::arrow_reader::ParquetRecordBatchReader,
    sender: &std::sync::mpsc::SyncSender<Decoded>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for batch in reader {
            if sender.send(batch).is_err() {
                return;
            }
        }
    }));
    if outcome.is_err() {
        let _ = sender.send(Err(ArrowError::ExternalError(
            "expected a Parquet row group to decode, got a panicking decoder".into(),
        )));
    }
}

impl Iterator for ParallelRead {
    type Item = Decoded;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            match self.next_joined() {
                Some(Ok(batch)) => return Some(Ok(batch)),
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(error));
                }
                None if self.running.is_empty() => self.done = true,
                None => {
                    // The front row group is drained, so the window has
                    // room for one more.
                    self.running.pop_front();
                    if let Err(error) = self.start_next() {
                        self.done = true;
                        return Some(Err(ArrowError::ExternalError(Box::new(error))));
                    }
                }
            }
        }
        None
    }
}

impl arrow_array::RecordBatchReader for ParallelRead {
    fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }
}

/// An Apache Parquet file bound to one [`IOBase`] handle.
///
/// Every read and write goes through this type, so the handle, the options,
/// and the cached footer live in one place instead of being repeated at each
/// call. [`IOBase::open`] materializes the handle and caches the footer, so
/// repeated schema or statistics reads do not re-parse it; [`IOBase::close`]
/// releases both.
#[derive(Debug)]
pub struct Parquet<H: IOBase> {
    handle: H,
    options: ParquetOptions,
    /// Explicit lifecycle state. An opened empty file has no footer, so cache
    /// presence cannot truthfully answer whether the wrapper is open.
    opened: bool,
    /// `Some(None)` is the stable opened-session answer for an empty handle.
    cached: OnceLock<Option<Arc<ParquetMetaData>>>,
    /// The schema conversion is also metadata-only, but materially more
    /// expensive than returning its result. Cache the derived width only for
    /// the explicitly opened session, under the same invalidation rules as
    /// the footer.
    cached_column_size: OnceLock<usize>,
}

impl<H: IOBase> Parquet<H> {
    /// Bind a Parquet file to a handle.
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: ParquetOptions::new(),
            opened: false,
            cached: OnceLock::new(),
            cached_column_size: OnceLock::new(),
        }
    }

    /// Return this file with different options.
    #[must_use]
    pub fn with_options(mut self, options: ParquetOptions) -> Self {
        self.options = options;
        self.invalidate_metadata();
        self
    }

    /// Return this file with a different root Field name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<smol_str::SmolStr>) -> Self {
        self.options.set_name(name.into());
        self.invalidate_metadata();
        self
    }

    /// Return this file with an explicit canonical schema.
    ///
    /// Record writes use it, and record reads materialize rows against it
    /// instead of against the schema stored in the footer.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self.invalidate_metadata();
        self
    }

    /// Borrow the underlying handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        self.invalidate_metadata();
        &mut self.handle
    }

    /// Consume the file and return its handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Borrow the options this file reads and writes with.
    pub const fn options(&self) -> &ParquetOptions {
        &self.options
    }

    /// Borrow the options mutably.
    pub fn options_mut(&mut self) -> &mut ParquetOptions {
        self.invalidate_metadata();
        &mut self.options
    }

    /// Discard footer metadata after an in-place mutation while retaining the
    /// explicit open state. The next metadata ask repopulates an open cache.
    fn invalidate_metadata(&mut self) {
        self.cached.take();
        self.cached_column_size.take();
    }

    /// Return opened-session footer metadata, or a fresh uncached closed read.
    fn metadata(&self) -> Result<Option<Arc<ParquetMetaData>>> {
        if !self.opened {
            return if self.handle.is_empty() {
                Ok(None)
            } else {
                load_metadata(&self.handle).map(Some)
            };
        }
        if let Some(cached) = self.cached.get() {
            return Ok(cached.clone());
        }
        let loaded = if self.handle.is_empty() {
            None
        } else {
            Some(load_metadata(&self.handle)?)
        };
        // Concurrent immutable asks may race to refill an invalidated cache;
        // whichever answer wins defines this opened session consistently.
        let _ = self.cached.set(loaded.clone());
        Ok(self.cached.get().cloned().unwrap_or(loaded))
    }

    /// Refresh the footer after publication without implicitly opening a
    /// closed wrapper.
    fn refresh_metadata(&mut self) -> crate::Result<()> {
        self.invalidate_metadata();
        if self.opened {
            let loaded = if self.handle.is_empty() {
                None
            } else {
                Some(load_metadata(&self.handle)?)
            };
            let _ = self.cached.set(loaded);
        }
        Ok(())
    }

    /// Best-effort refresh after an error that may follow a partial commit.
    /// The original write error remains authoritative.
    fn refresh_metadata_after_error(&mut self) {
        self.invalidate_metadata();
        if self.opened {
            let loaded = if self.handle.is_empty() {
                Some(None)
            } else {
                load_metadata(&self.handle).ok().map(Some)
            };
            if let Some(loaded) = loaded {
                let _ = self.cached.set(loaded);
            }
        }
    }

    /// Refuse options for a different encoding before a write can pull its
    /// first incoming batch.
    fn require_record_options<'a>(
        &self,
        options: &'a RecordOptions,
    ) -> crate::Result<&'a ParquetOptions> {
        match options {
            RecordOptions::Parquet(options) => Ok(options),
            _ => Err(crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("Parquet record options", options.mime_type()),
            }),
        }
    }

    /// Read the file's Arrow schema without decoding any rows.
    ///
    /// Field identifiers written by [`overwrite_arrow_reader`] are present in
    /// the returned schema's field metadata.
    ///
    /// # Errors
    ///
    /// Returns a read or footer failure.
    pub fn read_arrow_schema(&self) -> Result<Arc<Schema>> {
        match self.metadata()? {
            Some(metadata) => schema_from_metadata(metadata),
            None => read_arrow_schema(&self.handle),
        }
    }

    /// Read the file's footer statistics without decoding any rows.
    ///
    /// # Errors
    ///
    /// Returns a read or footer failure.
    pub fn read_statistics(&self) -> Result<FileStatistics> {
        match self.metadata()? {
            Some(metadata) => Ok(FileStatistics::from_metadata(metadata.as_ref())),
            None => read_statistics(&self.handle),
        }
    }

    /// Compute one geospatial column's statistics by scanning its stored WKB.
    ///
    /// [`Self::read_statistics`] exposes what the footer recorded; this
    /// decodes the named column and computes the same bounding-box-and-types
    /// answer from the values, so it also serves files whose writer recorded
    /// no geospatial statistics.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, when `column` does not
    /// name a stored WKB binary column, or when a stored value is malformed.
    pub fn read_geospatial_statistics(&self, column: &str) -> Result<GeospatialStatistics> {
        read_geospatial_statistics(&self.handle, column)
    }
}

/// A `Parquet` mirrors the bytes of the handle it owns, so the encoded file is
/// reachable directly - to copy it, upload it, or hand it to another reader -
/// without unwrapping the media type first.
///
/// [`IOBase::open`] additionally caches the footer and [`IOBase::close`]
/// releases it, which is what a scoped context binds to.
impl<H: IOBase> crate::IOMedia for Parquet<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> crate::Result<u64> {
        match self.metadata()? {
            Some(metadata) => metadata_row_size(metadata.as_ref()),
            None => Ok(0),
        }
    }

    fn column_size(&self) -> crate::Result<usize> {
        if self.opened {
            if let Some(column_size) = self.cached_column_size.get() {
                return Ok(*column_size);
            }
        }
        let column_size = if let Some(field) = self.options.field() {
            field.field_len()
        } else if let Some(metadata) = self.metadata()? {
            schema_from_metadata(metadata)?.fields().len()
        } else {
            0
        };
        if self.opened {
            let _ = self.cached_column_size.set(column_size);
        }
        Ok(*self.cached_column_size.get().unwrap_or(&column_size))
    }

    /// Return this wrapper's Parquet options even when the wrapped byte handle
    /// has no informative media type of its own.
    fn record_options(&self) -> crate::Result<RecordOptions> {
        Ok(RecordOptions::Parquet(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> crate::Result<Field> {
        let options = self.require_record_options(options)?;
        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        let schema = self.read_arrow_schema()?;
        Ok(field_from_arrow_schema(options.name(), schema.as_ref())?)
    }

    fn read_parquet_statistics(&self) -> crate::Result<FileStatistics> {
        Ok(self.read_statistics()?)
    }

    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> crate::Result<GeospatialStatistics> {
        Ok(self.read_geospatial_statistics(column)?)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        let result = crate::iobase::overwrite_arrow_reader_default(self, batches, options);
        // Publication may have changed the visible file before a later source
        // or storage failure. Never retain a footer from before the attempt.
        if let Err(error) = result {
            self.refresh_metadata_after_error();
            return Err(error);
        }
        self.refresh_metadata()?;
        Ok(())
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        self.require_record_options(options)?;
        let result = crate::iobase::leaf_writer(self, batches, options);
        if let Err(error) = result {
            self.refresh_metadata_after_error();
            return Err(error);
        }
        self.refresh_metadata()?;
        Ok(())
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

impl<H: IOBase> IOBase for Parquet<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        size, capacity, reserve, uri, url,
        bound_location, mtime, media_type, set_media_type, flush, parent, child_by_path, ls, kind);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.invalidate_metadata();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.invalidate_metadata();
        self.handle.truncate(size)
    }

    /// Parquet is a record encoding, so this handle holds rows whatever media
    /// type the bytes underneath happen to carry - no probe, no listing, no
    /// read.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Materialize the handle and cache the footer.
    fn open(&mut self) -> crate::Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        self.invalidate_metadata();
        let metadata = if self.handle.is_empty() {
            None
        } else {
            Some(load_metadata(&self.handle)?)
        };
        let _ = self.cached.set(metadata);
        self.opened = true;
        Ok(())
    }

    /// Return explicit lifecycle state, including for an empty file.
    fn opened(&self) -> bool {
        self.opened
    }

    /// Flush the handle and drop the cached footer.
    fn close(&mut self) -> crate::Result<()> {
        self.opened = false;
        self.invalidate_metadata();
        self.handle.close()
    }

    /// Empty the encoded resource and drop the cached footer with it.
    ///
    /// Invalidation is part of the call, not deferred to the next `open`: a
    /// cached footer describing bytes that are gone is a stale answer, and a
    /// stale answer after an emptying is a bug.
    fn clear(&mut self) -> crate::Result<()> {
        self.invalidate_metadata();
        let result = self.handle.clear();
        if self.opened {
            if result.is_ok() {
                let _ = self.cached.set(None);
            } else {
                self.refresh_metadata_after_error();
            }
        }
        result
    }

    /// Delete the encoded resource, and every cached footer it filled.
    ///
    /// A media handle removes what it wraps, not merely its own view: the
    /// resource behind the handle goes, and the footer cache goes with it.
    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        self.opened = false;
        self.invalidate_metadata();
        self.handle.remove(recursive)
    }
}

impl From<parquet::errors::ParquetError> for Error {
    fn from(value: parquet::errors::ParquetError) -> Self {
        Self::external(value)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/parquet/mod_.rs` pins and a caller cannot reach.
    //!
    //! `open_builder` is the file-private whole-value reader builder, and the
    //! logical type a column publishes - what a reader outside this crate
    //! sees - is visible only on it. Forwarding it changes no visibility: the
    //! builder itself is the `parquet` crate's own public type. The thread
    //! bound a table hands each file is crate-internal; pinning it is what
    //! lets a test run the threaded paths, or the one-thread path, whatever
    //! the host offers.

    use bytes::Bytes;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    use crate::IOBase;
    use crate::arrow::Result;

    /// Open the reader builder a Parquet read goes through.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where the handle declares an outer content
    /// coding, or where the footer does not read.
    pub fn open_builder<H: IOBase + ?Sized>(
        handle: &H,
    ) -> Result<ParquetRecordBatchReaderBuilder<Bytes>> {
        super::open_builder(handle)
    }

    /// Replace the file `handle` holds as [`super::overwrite_arrow_reader`]
    /// does, feeding the column writers every `buffer` Arrow bytes - small
    /// enough, in a test, that one row group takes several feeds.
    ///
    /// # Errors
    ///
    /// Returns what the public write returns.
    pub fn overwrite_with_write_buffer<H: IOBase + ?Sized>(
        handle: &mut H,
        batches: crate::arrow::BatchReader,
        options: &super::ParquetOptions,
        buffer: usize,
    ) -> Result<()> {
        super::overwrite_buffered(handle, batches, options, buffer)
    }

    /// These options with the threads one file decodes or encodes on pinned.
    #[must_use]
    pub fn with_threads(
        mut options: super::ParquetOptions,
        threads: usize,
    ) -> super::ParquetOptions {
        options.threads = Some(threads.max(1));
        options
    }
}
