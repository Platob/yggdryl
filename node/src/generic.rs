//! The record settings every encoding shares, as one JavaScript value.
//!
//! The variant *is* the encoding, so a record call takes `RecordOptions` and no
//! separate format argument. The encoding is never guessed: it is derived from a
//! media type, which is what [`crate::io::JsIOBase::record_options`] reads off
//! the handle.

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;
use yggdryl::media::{
    DEFAULT_RECORD_BATCH_ROW_SIZE, IORecordOptions, RecordOptions as CoreRecordOptions,
};
use yggdryl::{IOMode, Level, Metadata};

use crate::datatype::{DataTypeInput, JsDataType, dtype_from_input};
use crate::exact_u8;
use crate::field::{JsField, MetadataEntry, MetadataInput, metadata_pairs};
use crate::media::{
    JsMimeType, MediaTypeInput, MimeTypeInput, media_type_from_input, mime_type_from_input,
};
use crate::napi_error;
use crate::timezone::{JsTimezone, TimezoneInput, timezone_from_input};

/// The settings one record read or write takes.
#[napi(js_name = "RecordOptions")]
pub struct JsRecordOptions {
    pub(crate) inner: CoreRecordOptions,
}

impl Clone for JsRecordOptions {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl JsRecordOptions {
    pub(crate) const fn from_core(inner: CoreRecordOptions) -> Self {
        Self { inner }
    }

    /// Resolve the options a call was given, or the ones a handle names.
    pub(crate) fn resolved(
        value: Option<&Self>,
        handle: &yggdryl::holder::Holder,
    ) -> Result<CoreRecordOptions> {
        use yggdryl::IOMedia as _;

        match value {
            Some(options) => Ok(options.inner.clone()),
            None => handle.record_options().map_err(napi_error),
        }
    }
}

#[napi]
impl JsRecordOptions {
    /// Derive the options for the encoding a media type names.
    #[napi(constructor)]
    pub fn new(value: MediaTypeInput<'_>) -> Result<Self> {
        Self::for_media_type(value)
    }

    /// Infer from a native media wrapper or a media/extension string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: MediaTypeInput<'_>) -> Result<Self> {
        Self::for_media_type(value)
    }

    /// Derive the options for the encoding a media type names.
    #[napi(factory)]
    pub fn for_media_type(value: MediaTypeInput<'_>) -> Result<Self> {
        CoreRecordOptions::for_media_type(&media_type_from_input(value)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Derive the options for the encoding a MIME type names.
    #[napi(factory)]
    pub fn for_mime_type(value: MimeTypeInput<'_>) -> Result<Self> {
        CoreRecordOptions::for_mime_type(&mime_type_from_input(value)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The MIME type of the encoding these options describe.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(self.inner.mime_type())
    }

    /// The declared root Field, built from `name`, `dtype`, and `metadata`;
    /// `null` until a datatype is declared.
    #[napi(getter)]
    pub fn field(&self) -> Option<JsField> {
        self.inner.field().map(JsField::from_core)
    }

    /// Validate explicit write intent before JavaScript converts or pulls input.
    ///
    /// The loader captures and removes this private bridge, then calls it ahead
    /// of every representation adapter so an invalid mode never consumes a
    /// one-shot reader, iterable, or async iterable.
    #[napi(js_name = "_requireWritePreflightNative", skip_typescript)]
    pub fn require_write_preflight(&self, intent: String) -> Result<u32> {
        let mode = IOMode::from_str(&intent).map_err(napi_error)?;
        self.inner.require_write_mode(mode).map_err(napi_error)?;
        self.inner.require_commit_row_size().map_err(napi_error)?;
        self.inner.require_write_limits().map_err(napi_error)?;
        u32::try_from(DEFAULT_RECORD_BATCH_ROW_SIZE).map_err(napi_error)
    }

    /// Declare the root Field: its name, datatype, and metadata become the
    /// three declared parts; nullability and dictionary options are dropped.
    #[napi(setter)]
    pub fn set_field(&mut self, field: &JsField) {
        self.inner.set_field(field.inner.clone());
    }

    /// The root Field name, declared or given to an inferred schema.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// Set the root Field name.
    #[napi(setter)]
    pub fn set_name(&mut self, name: String) {
        self.inner.set_name(name.into());
    }

    /// The declared root datatype; `null` when the shape is inferred.
    #[napi(getter)]
    pub fn dtype(&self) -> Option<JsDataType> {
        self.inner.dtype().cloned().map(JsDataType::from_core)
    }

    /// Declare the root datatype from a `DataType` or a type expression;
    /// `null` clears it.
    #[napi(setter)]
    pub fn set_dtype(&mut self, dtype: Option<DataTypeInput<'_>>) -> Result<()> {
        let dtype = dtype.map(dtype_from_input).transpose()?;
        self.inner.set_dtype(dtype);
        Ok(())
    }

    /// The root metadata entries in lexical key order; empty unless declared.
    #[napi(getter)]
    pub fn metadata(&self) -> Vec<MetadataEntry> {
        self.inner
            .metadata()
            .iter()
            .map(|(key, value)| MetadataEntry {
                key: key.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    }

    /// Declare the root metadata from entries or a plain object; empty clears.
    #[napi(setter)]
    pub fn set_metadata(&mut self, values: MetadataInput) -> Result<()> {
        let metadata = Metadata::from_entries(metadata_pairs(values)).map_err(napi_error)?;
        self.inner.set_metadata(metadata);
        Ok(())
    }

    /// Whether a cast may null a value it cannot convert.
    #[napi(getter)]
    pub fn safe(&self) -> bool {
        self.inner.safe()
    }

    /// Set whether a cast may null a value it cannot convert.
    #[napi(setter)]
    pub fn set_safe(&mut self, safe: bool) {
        self.inner.set_safe(safe);
    }

    /// The rows-per-batch bound, when one is set.
    #[napi(getter)]
    pub fn batch_row_size(&self) -> Option<u32> {
        self.inner.batch_row_size().and_then(|size| {
            // A bound past 2^32 rows per batch is a caller mistake rather than a
            // number to round, so it reads as unset instead of truncated.
            u32::try_from(size).ok()
        })
    }

    /// Set the rows-per-batch bound.
    ///
    /// A bound of zero is refused rather than stored: the readers chunk by this
    /// number, so it turns a read of a hundred rows into a successful read of
    /// none. `null` is how "no bound" is spelled.
    #[napi(setter)]
    pub fn set_batch_row_size(&mut self, batch_row_size: Option<u32>) -> Result<()> {
        if batch_row_size == Some(0) {
            return Err(napi::Error::from_reason(
                "expected a positive row count for batchRowSize, got 0; pass null for no bound",
            ));
        }
        self.inner
            .set_batch_row_size(batch_row_size.map(|size| size as usize));
        Ok(())
    }

    /// The bound on how many result rows flow in total, when one is set.
    ///
    /// A count of rows, applied last - after the declared schema, selection,
    /// completion cast, and partition filter - so `0` is a valid ask: the
    /// shaped schema with no batches, rather than an error.
    #[napi(getter)]
    pub fn max_row_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.max_row_size().map(|rows| rows as f64)
    }

    /// Set the bound on how many result rows flow in total.
    #[napi(setter)]
    pub fn set_max_row_size(&mut self, max_row_size: Option<f64>) -> Result<()> {
        let bound = match max_row_size {
            Some(rows) => Some(crate::exact_u64(rows, "maxRowSize")?),
            None => None,
        };
        self.inner.set_max_row_size(bound);
        Ok(())
    }

    /// The bound on the result rows' Arrow in-memory bytes, when one is set.
    ///
    /// Counted uncompressed, never as encoded bytes; a non-zero bound always
    /// yields at least one row, and only `0` yields nothing.
    #[napi(getter)]
    pub fn max_byte_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.max_byte_size().map(|bytes| bytes as f64)
    }

    /// Set the bound on the result rows' Arrow in-memory bytes.
    #[napi(setter)]
    pub fn set_max_byte_size(&mut self, max_byte_size: Option<f64>) -> Result<()> {
        let bound = match max_byte_size {
            Some(bytes) => Some(crate::exact_u64(bytes, "maxByteSize")?),
            None => None,
        };
        self.inner.set_max_byte_size(bound);
        Ok(())
    }

    /// Rows published per streamed-write commit, when one is set.
    #[napi(getter)]
    pub fn commit_row_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.commit_row_size().map(|rows| rows as f64)
    }

    /// Set the streamed-write publication cadence.
    ///
    /// Zero is retained so the write preflight can reject it before touching a
    /// one-shot JavaScript source. `null` restores one publication at the end.
    #[napi(setter)]
    pub fn set_commit_row_size(&mut self, commit_row_size: Option<f64>) -> Result<()> {
        let rows = match commit_row_size {
            Some(rows) => {
                let rows = crate::exact_u64(rows, "commitRowSize")?;
                Some(usize::try_from(rows).map_err(|_| {
                    napi_error(format!(
                        "commitRowSize {rows} exceeds this platform's row-count range"
                    ))
                })?)
            }
            None => None,
        };
        self.inner.set_commit_row_size(rows);
        Ok(())
    }

    /// The compression level on the shared 0-to-9 scale.
    #[napi(getter)]
    pub fn level(&self) -> u8 {
        self.inner.level().get()
    }

    /// Set the compression level on the shared 0-to-9 scale.
    #[napi(setter)]
    pub fn set_level(&mut self, level: f64) -> Result<()> {
        self.inner.set_level(Level::new(exact_u8(level, "level")?));
        Ok(())
    }

    /// The column names a write matches rows on; empty means overwrite.
    #[napi(getter)]
    pub fn merge_by_names(&self) -> Vec<String> {
        self.inner.merge_by_names().to_vec()
    }

    /// Set the column names a write matches rows on.
    #[napi(setter)]
    pub fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
        self.inner.set_merge_by_names(merge_by_names);
    }

    /// The column names a read or write is narrowed to; empty selects all.
    #[napi(getter)]
    pub fn select_by_names(&self) -> Vec<String> {
        self.inner.select_by_names().to_vec()
    }

    /// Set the column names a read or write is narrowed to.
    #[napi(setter)]
    pub fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
        self.inner.set_select_by_names(select_by_names);
    }

    /// The partition equalities a read is pruned and filtered by; empty
    /// keeps every row. Values are spelled as partition paths spell them.
    #[napi(getter)]
    pub fn filter_partitions(&self) -> Vec<(String, String)> {
        self.inner.filter_partitions().to_vec()
    }

    /// Set the partition equalities a read is pruned and filtered by.
    #[napi(setter)]
    pub fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
        self.inner.set_filter_partitions(filter_partitions);
    }

    /// The timezone applied while autotyping offset-free timestamps.
    #[napi(getter)]
    pub fn timezone(&self) -> Option<JsTimezone> {
        self.inner.timezone().cloned().map(JsTimezone::from_core)
    }

    /// Set or clear the timezone for autotyped timestamps.
    #[napi(setter)]
    pub fn set_timezone(&mut self, value: Option<TimezoneInput<'_>>) -> Result<()> {
        let timezone = value.map(timezone_from_input).transpose()?;
        self.inner.set_timezone(timezone).map_err(napi_error)
    }

    /// The Avro block codec name, or `null` for another encoding.
    #[napi(getter)]
    pub fn block_codec(&self) -> Option<String> {
        self.inner.avro_block_codec().map(ToOwned::to_owned)
    }

    /// Validate and set the Avro block codec name.
    #[napi(setter)]
    pub fn set_block_codec(&mut self, block_codec: String) -> Result<()> {
        self.inner
            .set_avro_block_codec(&block_codec)
            .map_err(napi_error)
    }

    /// The fixed sixteen-byte Avro synchronization marker, when one is set.
    #[napi(getter)]
    pub fn sync_marker(&self) -> Option<Buffer> {
        self.inner
            .avro_sync_marker()
            .map(|marker| marker.to_vec().into())
    }

    /// Set or clear the fixed Avro synchronization marker.
    #[napi(setter)]
    pub fn set_sync_marker(&mut self, marker: Option<Buffer>) -> Result<()> {
        self.inner
            .set_avro_sync_marker(marker.as_deref())
            .map_err(napi_error)
    }

    /// The page compression applied inside a Parquet file, if this is one.
    ///
    /// A setting one encoding has is absent on the others rather than invented,
    /// so this is `null` for an Arrow IPC stream, whose coding belongs to the
    /// handle instead.
    #[napi(getter)]
    pub fn compression(&self) -> Option<String> {
        self.inner.parquet_compression_name()
    }

    /// Set the page compression applied inside a Parquet file.
    #[napi(setter)]
    pub fn set_compression(&mut self, compression: String) -> Result<()> {
        self.inner
            .set_parquet_compression_name(&compression)
            .map_err(napi_error)
    }

    /// The maximum rows per row group of a Parquet file, if this is one.
    #[napi(getter)]
    pub fn max_row_group_size(&self) -> Option<u32> {
        self.inner
            .parquet_max_row_group_size()
            .and_then(|rows| u32::try_from(rows).ok())
    }

    /// Set the maximum rows per row group of a Parquet file.
    #[napi(setter)]
    pub fn set_max_row_group_size(&mut self, rows: u32) -> Result<()> {
        self.inner
            .set_parquet_max_row_group_size(rows as usize)
            .map_err(napi_error)
    }

    /// The footer key/value entries a Parquet write adds.
    #[napi(getter)]
    pub fn key_value_metadata(&self) -> Vec<MetadataEntry> {
        self.inner
            .parquet_key_value_metadata()
            .unwrap_or_default()
            .iter()
            .map(|(key, value)| MetadataEntry {
                key: key.clone(),
                value: value.clone(),
            })
            .collect()
    }

    /// Return these options with a different Parquet page compression.
    #[napi]
    pub fn with_compression(&self, compression: String) -> Result<Self> {
        let mut options = self.clone();
        options.set_compression(compression)?;
        Ok(options)
    }

    /// Return these options with a different Parquet row-group size.
    #[napi]
    pub fn with_max_row_group_size(&self, rows: u32) -> Result<Self> {
        let mut options = self.clone();
        options.set_max_row_group_size(rows)?;
        Ok(options)
    }

    /// Return these options with one added Parquet footer entry.
    #[napi]
    pub fn with_key_value(&self, key: String, value: String) -> Result<Self> {
        let mut options = self.clone();
        options
            .inner
            .push_parquet_key_value(key, value)
            .map_err(napi_error)?;
        Ok(options)
    }

    /// Return these options with a validated Avro block codec.
    #[napi]
    pub fn with_block_codec(&self, block_codec: String) -> Result<Self> {
        let mut options = self.clone();
        options.set_block_codec(block_codec)?;
        Ok(options)
    }

    /// Return these options with a fixed Avro marker, or `null` to clear it.
    #[napi]
    pub fn with_sync_marker(&self, marker: Option<Buffer>) -> Result<Self> {
        let mut options = self.clone();
        options.set_sync_marker(marker)?;
        Ok(options)
    }

    /// Return these options with a declared canonical root Field.
    #[napi]
    pub fn with_field(&self, field: &JsField) -> Self {
        let mut options = self.clone();
        options.set_field(field);
        options
    }

    /// Return these options with a different root Field name.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        let mut options = self.clone();
        options.set_name(name);
        options
    }

    /// Return these options with a declared root datatype.
    #[napi]
    pub fn with_dtype(&self, dtype: DataTypeInput<'_>) -> Result<Self> {
        let mut options = self.clone();
        options.set_dtype(Some(dtype))?;
        Ok(options)
    }

    /// Return these options with declared root metadata.
    #[napi]
    pub fn with_metadata(&self, values: MetadataInput) -> Result<Self> {
        let mut options = self.clone();
        options.set_metadata(values)?;
        Ok(options)
    }

    /// Return these options with a different cast strictness.
    #[napi]
    pub fn with_safe(&self, safe: bool) -> Self {
        let mut options = self.clone();
        options.set_safe(safe);
        options
    }

    /// Return these options with a rows-per-batch bound.
    #[napi]
    pub fn with_batch_row_size(&self, batch_row_size: u32) -> Result<Self> {
        let mut options = self.clone();
        options.set_batch_row_size(Some(batch_row_size))?;
        Ok(options)
    }

    /// Return these options with a bound on how many result rows flow.
    #[napi]
    pub fn with_max_row_size(&self, max_row_size: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_max_row_size(Some(max_row_size))?;
        Ok(options)
    }

    /// Return these options with a bound on the result rows' Arrow bytes.
    #[napi]
    pub fn with_max_byte_size(&self, max_byte_size: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_max_byte_size(Some(max_byte_size))?;
        Ok(options)
    }

    /// Return these options with a streamed-write publication cadence.
    #[napi]
    pub fn with_commit_row_size(&self, commit_row_size: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_commit_row_size(Some(commit_row_size))?;
        Ok(options)
    }

    /// Return these options with a different compression level.
    #[napi]
    pub fn with_level(&self, level: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_level(level)?;
        Ok(options)
    }

    /// Return these options with a match key for a write.
    #[napi]
    pub fn with_merge_by_names(&self, merge_by_names: Vec<String>) -> Self {
        let mut options = self.clone();
        options.set_merge_by_names(merge_by_names);
        options
    }

    /// Return these options narrowed to the named columns, on reads and writes.
    #[napi]
    pub fn with_select_by_names(&self, select_by_names: Vec<String>) -> Self {
        let mut options = self.clone();
        options.set_select_by_names(select_by_names);
        options
    }

    /// Return these options pruned and filtered to the named partitions.
    #[napi]
    pub fn with_filter_partitions(&self, filter_partitions: Vec<(String, String)>) -> Self {
        let mut options = self.clone();
        options.set_filter_partitions(filter_partitions);
        options
    }

    /// Return whether the encoding variant and every current setting are equal.
    #[napi]
    pub fn equals(&self, other: &JsRecordOptions) -> bool {
        self.inner == other.inner
    }

    /// Compare the complete options through the core's total order.
    #[napi]
    pub fn compare(&self, other: &JsRecordOptions) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete core options value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a detached copy whose later mutations do not affect this value.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return the encoding these options describe, so they print as what they
    /// encode rather than as an opaque object.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.mime_type().to_string()
    }
}
