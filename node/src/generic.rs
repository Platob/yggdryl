//! The record settings every encoding shares, as one JavaScript value.
//!
//! The variant *is* the encoding, so a record call takes `RecordOptions` and no
//! separate format argument. The encoding is never guessed: it is derived from a
//! media type, which is what [`crate::io::JsIOBase::record_options`] reads off
//! the handle.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::Level;
use yggdryl::generic::{IORecordOptions, RecordOptions as CoreRecordOptions};

use crate::exact_u8;
use crate::field::{JsField, MetadataEntry};
use crate::media::{
    JsMimeType, MediaTypeInput, MimeTypeInput, media_type_from_input, mime_type_from_input,
};
use crate::napi_error;

/// Name a page compression the way the Parquet parser accepts it.
///
/// The codec's own `Display` prints the level as its internal type, which its
/// own `FromStr` then refuses, so the accepted spelling is written here rather
/// than recovered from that text.
fn compression_name(compression: parquet::basic::Compression) -> String {
    use parquet::basic::Compression as C;

    match compression {
        C::UNCOMPRESSED => "uncompressed".to_owned(),
        C::SNAPPY => "snappy".to_owned(),
        C::GZIP(level) => format!("gzip({})", level.compression_level()),
        C::LZO => "lzo".to_owned(),
        C::BROTLI(level) => format!("brotli({})", level.compression_level()),
        C::LZ4 => "lz4".to_owned(),
        C::ZSTD(level) => format!("zstd({})", level.compression_level()),
        C::LZ4_RAW => "lz4_raw".to_owned(),
    }
}

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

    /// Borrow the Parquet settings, or name the encoding that has none.
    fn parquet_mut(&mut self, setting: &str) -> Result<&mut yggdryl::parquet::ParquetOptions> {
        let encoding = self.inner.mime_type();
        match &mut self.inner {
            CoreRecordOptions::Parquet(options) => Ok(options),
            CoreRecordOptions::Ipc(_) | CoreRecordOptions::Avro(_) => Err(napi_error(format!(
                "expected Parquet options to set {setting}, got {encoding}"
            ))),
        }
    }

    /// Resolve the options a call was given, or the ones a handle names.
    pub(crate) fn resolved(
        value: Option<&Self>,
        handle: &yggdryl::generic::Holder,
    ) -> Result<CoreRecordOptions> {
        use yggdryl::io::IOBase as _;

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

    /// The declared canonical schema, when one was declared.
    #[napi(getter)]
    pub fn schema(&self) -> Option<JsField> {
        self.inner.schema().cloned().map(JsField::from_core)
    }

    /// Declare the canonical schema.
    #[napi(setter)]
    pub fn set_schema(&mut self, schema: &JsField) {
        self.inner.set_schema(schema.inner.clone());
    }

    /// The root Field name used when a schema is inferred.
    #[napi(getter)]
    pub fn root_name(&self) -> String {
        self.inner.root_name().to_owned()
    }

    /// Set the root Field name used when a schema is inferred.
    #[napi(setter)]
    pub fn set_root_name(&mut self, root_name: String) {
        self.inner.set_root_name(root_name.into());
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
    pub fn batch_size(&self) -> Option<u32> {
        self.inner.batch_size().and_then(|size| {
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
    pub fn set_batch_size(&mut self, batch_size: Option<u32>) -> Result<()> {
        if batch_size == Some(0) {
            return Err(napi::Error::from_reason(
                "expected a positive row count for batchSize, got 0; pass null for no bound",
            ));
        }
        self.inner
            .set_batch_size(batch_size.map(|size| size as usize));
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

    /// The page compression applied inside a Parquet file, if this is one.
    ///
    /// A setting one encoding has is absent on the others rather than invented,
    /// so this is `null` for an Arrow IPC stream, whose coding belongs to the
    /// handle instead.
    #[napi(getter)]
    pub fn compression(&self) -> Option<String> {
        match &self.inner {
            CoreRecordOptions::Parquet(options) => Some(compression_name(options.compression)),
            CoreRecordOptions::Ipc(_) | CoreRecordOptions::Avro(_) => None,
        }
    }

    /// Set the page compression applied inside a Parquet file.
    #[napi(setter)]
    pub fn set_compression(&mut self, compression: String) -> Result<()> {
        let options = self.parquet_mut("a page compression")?;
        // The target field names the type, so the parquet crate's own parser is
        // what accepts `uncompressed`, `snappy`, `zstd(3)`, and the rest.
        options.compression = compression.parse().map_err(napi_error)?;
        Ok(())
    }

    /// The maximum rows per row group of a Parquet file, if this is one.
    #[napi(getter)]
    pub fn max_row_group_size(&self) -> Option<u32> {
        match &self.inner {
            CoreRecordOptions::Parquet(options) => u32::try_from(options.max_row_group_size).ok(),
            CoreRecordOptions::Ipc(_) | CoreRecordOptions::Avro(_) => None,
        }
    }

    /// Set the maximum rows per row group of a Parquet file.
    #[napi(setter)]
    pub fn set_max_row_group_size(&mut self, rows: u32) -> Result<()> {
        self.parquet_mut("a row-group size")?.max_row_group_size = rows as usize;
        Ok(())
    }

    /// The footer key/value entries a Parquet write adds.
    #[napi(getter)]
    pub fn key_value_metadata(&self) -> Vec<MetadataEntry> {
        match &self.inner {
            CoreRecordOptions::Parquet(options) => options
                .key_value_metadata
                .iter()
                .map(|(key, value)| MetadataEntry {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect(),
            CoreRecordOptions::Ipc(_) | CoreRecordOptions::Avro(_) => Vec::new(),
        }
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
            .parquet_mut("footer metadata")?
            .key_value_metadata
            .push((key, value));
        Ok(options)
    }

    /// Return these options with a declared canonical schema.
    #[napi]
    pub fn with_schema(&self, schema: &JsField) -> Self {
        let mut options = self.clone();
        options.set_schema(schema);
        options
    }

    /// Return these options with a different inferred-root Field name.
    #[napi]
    pub fn with_root_name(&self, root_name: String) -> Self {
        let mut options = self.clone();
        options.set_root_name(root_name);
        options
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
    pub fn with_batch_size(&self, batch_size: u32) -> Result<Self> {
        let mut options = self.clone();
        options.set_batch_size(Some(batch_size))?;
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

    /// Return the encoding these options describe, so they print as what they
    /// encode rather than as an opaque object.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner.mime_type().to_string()
    }
}
