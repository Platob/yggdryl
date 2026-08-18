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
//! [`write_batch_reader`] - over any [`IOBase`] handle, which is what
//! [`IOBase::read_arrow_batch_reader`] and its two siblings call. They are the
//! encoding and nothing more: the `field` they take is a column pushdown, and
//! the casting, merging, and partition routing a caller sees belong to
//! [`IOBase`]'s three record methods above them.
//!
//! Yggdryl field identifiers survive the round trip. A [`Field`] carrying
//! `PARQUET:field_id` writes that id into the Parquet schema and reads it back,
//! which is what lets a downstream Iceberg or Delta layer resolve columns by
//! id rather than by position.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::parquet::Parquet;
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = DataType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::Utf8.nullable_field("symbol"),
//! ])?
//! .required_field("row");
//!
//! let handle =
//!     Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
//! let mut media = Parquet::new(handle);
//!
//! // One instance owns the handle and every write option.
//! media.write_batch_reader(yggdryl::arrow::batch_reader(field.to_arrow_schema()?, []))?;
//! assert_eq!(media.read_batch_reader(None)?.count(), 0);
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use arrow_array::RecordBatchIterator;
use arrow_schema::Schema;
use bytes::Bytes;
use parquet::arrow::ArrowWriter;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::{ArrowReaderOptions, ParquetRecordBatchReaderBuilder};
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::metadata::ParquetMetaData;
use parquet::file::properties::WriterProperties;

use crate::arrow::schema_from_field;
use crate::arrow::{
    BatchReader, Error, Result, from_reader_error, projection_indices, record_schema_from_arrow,
};
use crate::generic::IORecordOptions;
use crate::io::IOBase;
use crate::{Error as CoreError, Field};

mod metadata;

pub use metadata::{ColumnStatistics, FileStatistics, RowGroupStatistics};

/// The settings a Parquet read or write takes.
///
/// The shared settings (schema, root name, cast strictness, batch size) are
/// flat fields here, alongside the ones only Parquet has.
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
    /// Declared canonical schema; read from the footer when absent.
    pub schema: Option<Field>,
    /// Root Field name used for a schema read from the footer.
    pub root_name: smol_str::SmolStr,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per batch, when a reader should bound them.
    pub batch_size: Option<usize>,
    /// Unused: Parquet compresses pages internally through `compression`.
    pub level: crate::Level,
    /// Column names forming a write's match key; empty means overwrite.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to; empty selects everything.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by; empty keeps all.
    pub filter_partitions: Vec<(String, String)>,
}

impl ParquetOptions {
    /// Balanced defaults: Zstandard pages and 1,048,576-row groups.
    pub fn new() -> Self {
        Self {
            compression: Compression::ZSTD(ZstdLevel::default()),
            max_row_group_size: 1_048_576,
            key_value_metadata: Vec::new(),
            schema: None,
            root_name: smol_str::SmolStr::new_static("row"),
            safe: false,
            batch_size: None,
            level: crate::Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
        }
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

    fn to_properties(&self) -> WriterProperties {
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

/// Read the Arrow schema of the file `handle` holds.
///
/// # Errors
///
/// Returns a read or footer failure.
pub fn read_schema<H: IOBase + ?Sized>(handle: &H) -> Result<Arc<Schema>> {
    Ok(Arc::clone(open_builder(handle)?.schema()))
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
    if let Some(schema) = options.schema() {
        return Ok(schema.clone());
    }
    let schema = read_schema(handle)?;
    record_schema_from_arrow(options.root_name(), schema.as_ref())
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
    if handle.is_empty() {
        // Per the laziness contract, a missing file holds no batches.
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
    let builder = open_builder(handle)?;
    let builder = match options.batch_size() {
        Some(size) => builder.with_batch_size(size),
        None => builder,
    };
    let projection = field.and_then(|field| projection_indices(field, builder.schema()));
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
pub fn write_batch_reader<H>(
    handle: &mut H,
    batches: BatchReader,
    options: &ParquetOptions,
) -> Result<()>
where
    H: IOBase + ?Sized,
{
    reject_outer_coding(handle)?;
    let schema = batches.schema();

    let mut encoded = Vec::new();
    let mut writer = ArrowWriter::try_new(
        &mut encoded,
        Arc::clone(&schema),
        Some(options.to_properties()),
    )?;
    for (index, batch) in batches.enumerate() {
        let batch = batch.map_err(from_reader_error)?;
        if batch.schema() != schema {
            return Err(Error::SchemaMismatch {
                index: Some(index),
                path: smol_str::SmolStr::new_static("$"),
                diff: format!(
                    "\u{2260} batch {index} schema does not match the declared root\n  \u{2212} {}\n  + {}",
                    crate::text::elide_display(&schema),
                    crate::text::elide_display(&batch.schema())
                ),
            });
        }
        writer.write(&batch)?;
    }
    writer.close()?;
    handle.write_all_bytes(&encoded)?;
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

/// Parse a file's footer without caching it.
fn load_metadata<H: IOBase + ?Sized>(handle: &H) -> Result<Arc<ParquetMetaData>> {
    Ok(Arc::clone(open_builder(handle)?.metadata()))
}

/// Open a reader builder over a handle's complete bytes.
///
/// Parquet reads its footer last, so the value is fetched whole. A range-
/// reading `ChunkReader` over [`IOBase::pread`] is the optimization path;
/// until then this is honest about buffering.
fn open_builder<H: IOBase + ?Sized>(handle: &H) -> Result<ParquetRecordBatchReaderBuilder<Bytes>> {
    reject_outer_coding(handle)?;
    let bytes = Bytes::from(handle.read_all()?);
    Ok(ParquetRecordBatchReaderBuilder::try_new_with_options(
        bytes,
        ArrowReaderOptions::new(),
    )?)
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
    /// Footer cached by `open`, discarded by `close` or any write.
    cached: Option<Arc<ParquetMetaData>>,
}

impl<H: IOBase> Parquet<H> {
    /// Bind a Parquet file to a handle.
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: ParquetOptions::new(),
            cached: None,
        }
    }

    /// Return this file with different options.
    #[must_use]
    pub fn with_options(mut self, options: ParquetOptions) -> Self {
        self.options = options;
        self
    }

    /// Return this file with a different root Field name.
    #[must_use]
    pub fn with_root_name(mut self, root_name: impl Into<smol_str::SmolStr>) -> Self {
        self.options.set_root_name(root_name.into());
        self
    }

    /// Return this file with an explicit canonical schema.
    ///
    /// Record writes use it, and record reads materialize rows against it
    /// instead of against the schema stored in the footer.
    #[must_use]
    pub fn with_schema(mut self, schema: Field) -> Self {
        self.options.set_schema(schema);
        self
    }

    /// Borrow the underlying handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying handle mutably.
    pub const fn handle_mut(&mut self) -> &mut H {
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
    pub const fn options_mut(&mut self) -> &mut ParquetOptions {
        &mut self.options
    }

    /// Replace the file with every batch `batches` yields.
    ///
    /// # Errors
    ///
    /// Returns a schema, encoding, or write failure, or an error when the
    /// handle declares an outer content coding.
    pub fn write_batch_reader(&mut self, batches: BatchReader) -> Result<()> {
        write_batch_reader(&mut self.handle, batches, &self.options)?;
        // The file changed, so the cached footer is stale.
        self.cached = None;
        Ok(())
    }

    /// Read the file, keeping only the columns `field` names.
    ///
    /// Rows per batch come from [`ParquetOptions::batch_size`], so the bound
    /// lives with the rest of the settings rather than at each call.
    ///
    /// # Errors
    ///
    /// Returns a read, footer, or decoding failure.
    pub fn read_batch_reader(&self, field: Option<&Field>) -> Result<BatchReader> {
        read_batch_reader(&self.handle, field, &self.options)
    }

    /// Read the file's Arrow schema without decoding any rows.
    ///
    /// Field identifiers written by [`Self::write_batch_reader`] are present in the
    /// returned schema's field metadata.
    ///
    /// # Errors
    ///
    /// Returns a read or footer failure.
    pub fn read_schema(&self) -> Result<Arc<Schema>> {
        read_schema(&self.handle)
    }

    /// Read the exact non-null Struct root Field of the file.
    ///
    /// # Errors
    ///
    /// Returns a read, footer, or schema-projection failure.
    pub fn read_field(&self) -> Result<Field> {
        read_field(&self.handle, &self.options)
    }

    /// Return the file's canonical schema.
    ///
    /// A declared schema is returned as-is; otherwise the footer supplies one.
    ///
    /// # Errors
    ///
    /// Returns a read, footer, or schema-projection failure.
    pub fn schema(&mut self) -> Result<Field> {
        read_field(&self.handle, &self.options)
    }

    /// Read the file's footer statistics without decoding any rows.
    ///
    /// # Errors
    ///
    /// Returns a read or footer failure.
    pub fn read_statistics(&self) -> Result<FileStatistics> {
        match &self.cached {
            Some(metadata) => Ok(FileStatistics::from_metadata(metadata)),
            None => read_statistics(&self.handle),
        }
    }
}

/// A `Parquet` mirrors the bytes of the handle it owns, so the encoded file is
/// reachable directly - to copy it, upload it, or hand it to another reader -
/// without unwrapping the media type first.
///
/// [`IOBase::open`] additionally caches the footer and [`IOBase::close`]
/// releases it, which is what a scoped context binds to.
impl<H: IOBase> IOBase for Parquet<H> {
    crate::delegate_iobase!(handle, except_lifecycle);

    /// Materialize the handle and cache the footer.
    fn open(&mut self) -> crate::Result<()> {
        self.handle.open()?;
        if self.cached.is_none() && !self.handle.is_empty() {
            self.cached = Some(load_metadata(&self.handle)?);
        }
        Ok(())
    }

    /// Return whether a footer is currently cached.
    fn opened(&self) -> bool {
        self.cached.is_some()
    }

    /// Flush the handle and drop the cached footer.
    fn close(&mut self) -> crate::Result<()> {
        self.cached = None;
        self.handle.close()
    }

    /// Empty the encoded resource and drop the cached footer with it.
    ///
    /// Invalidation is part of the call, not deferred to the next `open`: a
    /// cached footer describing bytes that are gone is a stale answer, and a
    /// stale answer after an emptying is a bug.
    fn clear(&mut self) -> crate::Result<()> {
        self.cached = None;
        self.handle.clear()
    }

    /// Delete the encoded resource, and every cached footer it filled.
    ///
    /// A media handle removes what it wraps, not merely its own view: the
    /// resource behind the handle goes, and the footer cache goes with it.
    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        self.cached = None;
        self.handle.remove(recursive)
    }
}

impl From<parquet::errors::ParquetError> for Error {
    fn from(value: parquet::errors::ParquetError) -> Self {
        Self::external(value)
    }
}

#[cfg(test)]
mod tests;
