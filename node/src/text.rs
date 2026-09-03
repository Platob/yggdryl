//! Native JavaScript view of flat plain-text record options.

use napi::bindgen_prelude::{Buffer, Either, Result, Uint8Array};
use napi_derive::napi;
use yggdryl::generic::IORecordOptions;
use yggdryl::text::TextOptions as CoreTextOptions;
use yggdryl::{Level, Metadata, MimeType};

use crate::datatype::{DataTypeInput, JsDataType, dtype_from_input};
use crate::exact_u8;
use crate::field::{JsField, MetadataEntry, MetadataInput, metadata_pairs};
use crate::generic::JsRecordOptions;
use crate::media::JsMimeType;
use crate::napi_error;
use crate::timezone::{JsTimezone, TimezoneInput, timezone_from_input};

/// Flat settings for physical-line `text/plain` records.
#[napi(js_name = "TextOptions")]
pub struct JsTextOptions {
    pub(crate) inner: CoreTextOptions,
}

impl Clone for JsTextOptions {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl Default for JsTextOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl JsTextOptions {
    pub(crate) const fn from_core(inner: CoreTextOptions) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTextOptions {
    /// Build default plain-text record settings.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::from_core(CoreTextOptions::new())
    }

    /// Convert this text configuration into the generic dispatch value.
    #[napi(js_name = "_recordOptionsNative", skip_typescript)]
    pub fn record_options_native(&self) -> JsRecordOptions {
        JsRecordOptions::from_core(self.inner.clone().into())
    }

    /// Return the fixed `text/plain` media type.
    #[napi(getter)]
    pub fn mime_type(&self) -> JsMimeType {
        JsMimeType::from_core(MimeType::PLAIN_TEXT)
    }

    /// Return the declared root field, if any.
    #[napi(getter)]
    pub fn field(&self) -> Option<JsField> {
        self.inner.field().map(JsField::from_core)
    }

    /// Replace the declared root field.
    #[napi(setter)]
    pub fn set_field(&mut self, field: &JsField) {
        self.inner.set_field(field.inner.clone());
    }

    /// Return the inferred or declared root name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// Replace the root name.
    #[napi(setter)]
    pub fn set_name(&mut self, name: String) {
        self.inner.set_name(name.into());
    }

    /// Return the declared root datatype, if any.
    #[napi(getter)]
    pub fn dtype(&self) -> Option<JsDataType> {
        self.inner.dtype().cloned().map(JsDataType::from_core)
    }

    /// Replace or clear the declared root datatype.
    #[napi(setter)]
    pub fn set_dtype(&mut self, dtype: Option<DataTypeInput<'_>>) -> Result<()> {
        self.inner
            .set_dtype(dtype.map(dtype_from_input).transpose()?);
        Ok(())
    }

    /// Return root metadata entries in key order.
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

    /// Replace root metadata.
    #[napi(setter)]
    pub fn set_metadata(&mut self, values: MetadataInput) -> Result<()> {
        self.inner
            .set_metadata(Metadata::from_entries(metadata_pairs(values)).map_err(napi_error)?);
        Ok(())
    }

    /// Return whether casts may null incompatible values.
    #[napi(getter)]
    pub fn safe(&self) -> bool {
        self.inner.safe()
    }

    /// Set whether casts may null incompatible values.
    #[napi(setter)]
    pub fn set_safe(&mut self, safe: bool) {
        self.inner.set_safe(safe);
    }

    /// Return the row-per-batch bound.
    #[napi(getter)]
    pub fn batch_row_size(&self) -> Option<u32> {
        self.inner
            .batch_row_size()
            .and_then(|size| u32::try_from(size).ok())
    }

    /// Set or clear the row-per-batch bound.
    #[napi(setter)]
    pub fn set_batch_row_size(&mut self, size: Option<u32>) -> Result<()> {
        if size == Some(0) {
            return Err(napi::Error::from_reason(
                "expected a positive row count for batchRowSize, got 0; pass null for no bound",
            ));
        }
        self.inner
            .set_batch_row_size(size.map(|size| size as usize));
        Ok(())
    }

    /// Return the streamed-write commit cadence.
    #[napi(getter)]
    pub fn commit_row_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.commit_row_size().map(|rows| rows as f64)
    }

    /// Set or clear the streamed-write commit cadence.
    #[napi(setter)]
    pub fn set_commit_row_size(&mut self, value: Option<f64>) -> Result<()> {
        let rows = match value {
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

    /// Return the total result-row bound.
    #[napi(getter)]
    pub fn max_row_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.max_row_size().map(|rows| rows as f64)
    }

    /// Set or clear the total result-row bound.
    #[napi(setter)]
    pub fn set_max_row_size(&mut self, value: Option<f64>) -> Result<()> {
        self.inner.set_max_row_size(
            value
                .map(|rows| crate::exact_u64(rows, "maxRowSize"))
                .transpose()?,
        );
        Ok(())
    }

    /// Return the Arrow-memory byte bound.
    #[napi(getter)]
    pub fn max_byte_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.max_byte_size().map(|bytes| bytes as f64)
    }

    /// Set or clear the Arrow-memory byte bound.
    #[napi(setter)]
    pub fn set_max_byte_size(&mut self, value: Option<f64>) -> Result<()> {
        self.inner.set_max_byte_size(
            value
                .map(|bytes| crate::exact_u64(bytes, "maxByteSize"))
                .transpose()?,
        );
        Ok(())
    }

    /// Return the outer content-coding level.
    #[napi(getter)]
    pub fn level(&self) -> u8 {
        self.inner.level().get()
    }

    /// Set the outer content-coding level.
    #[napi(setter)]
    pub fn set_level(&mut self, level: f64) -> Result<()> {
        self.inner.set_level(Level::new(exact_u8(level, "level")?));
        Ok(())
    }

    /// Return write match-key column names.
    #[napi(getter)]
    pub fn merge_by_names(&self) -> Vec<String> {
        self.inner.merge_by_names().to_vec()
    }

    /// Replace write match-key column names.
    #[napi(setter)]
    pub fn set_merge_by_names(&mut self, names: Vec<String>) {
        self.inner.set_merge_by_names(names);
    }

    /// Return selected column names.
    #[napi(getter)]
    pub fn select_by_names(&self) -> Vec<String> {
        self.inner.select_by_names().to_vec()
    }

    /// Replace selected column names.
    #[napi(setter)]
    pub fn set_select_by_names(&mut self, names: Vec<String>) {
        self.inner.set_select_by_names(names);
    }

    /// Return partition filters.
    #[napi(getter)]
    pub fn filter_partitions(&self) -> Vec<(String, String)> {
        self.inner.filter_partitions().to_vec()
    }

    /// Replace partition filters.
    #[napi(setter)]
    pub fn set_filter_partitions(&mut self, partitions: Vec<(String, String)>) {
        self.inner.set_filter_partitions(partitions);
    }

    /// The regex searched for a row header in each physical line.
    #[napi(getter)]
    pub fn rowheader(&self) -> Option<String> {
        self.inner.rowheader().map(ToOwned::to_owned)
    }

    /// Compile or clear the row-header regex.
    #[napi(setter)]
    pub fn set_rowheader(&mut self, rowheader: Option<String>) -> Result<()> {
        self.inner
            .set_rowheader(rowheader.as_deref())
            .map_err(napi_error)
    }

    /// Return the left-edge stripping regex.
    #[napi(getter)]
    pub fn lstrip(&self) -> Option<String> {
        self.inner.lstrip().map(ToOwned::to_owned)
    }

    /// Compile or clear the left-edge stripping regex.
    #[napi(setter)]
    pub fn set_lstrip(&mut self, lstrip: Option<String>) -> Result<()> {
        self.inner.set_lstrip(lstrip.as_deref()).map_err(napi_error)
    }

    /// Return the right-edge stripping regex.
    #[napi(getter)]
    pub fn rstrip(&self) -> Option<String> {
        self.inner.rstrip().map(ToOwned::to_owned)
    }

    /// Compile or clear the right-edge stripping regex.
    #[napi(setter)]
    pub fn set_rstrip(&mut self, rstrip: Option<String>) -> Result<()> {
        self.inner.set_rstrip(rstrip.as_deref()).map_err(napi_error)
    }

    /// Return the pinned physical-line terminator.
    #[napi(getter)]
    pub fn linesep(&self) -> Option<Buffer> {
        self.inner
            .linesep()
            .map(|linesep| linesep.as_bytes().to_vec().into())
    }

    /// Set or clear the physical-line terminator.
    #[napi(setter)]
    pub fn set_linesep(&mut self, value: Option<Either<String, Uint8Array>>) -> Result<()> {
        let linesep = value
            .map(|value| match value {
                Either::A(value) => value.parse(),
                Either::B(value) => yggdryl::text::LineSep::new(value.as_ref()),
            })
            .transpose()
            .map_err(napi_error)?;
        self.inner.set_linesep(linesep);
        Ok(())
    }

    /// Return whether first-batch capture autotyping is enabled.
    #[napi(getter)]
    pub fn autotype(&self) -> bool {
        self.inner.autotype()
    }

    /// Enable or disable first-batch capture autotyping.
    #[napi(setter)]
    pub fn set_autotype(&mut self, autotype: bool) {
        self.inner.set_autotype(autotype);
    }

    /// Return the timezone for offset-free autotyped timestamps.
    #[napi(getter)]
    pub fn timezone(&self) -> Option<JsTimezone> {
        self.inner.timezone().cloned().map(JsTimezone::from_core)
    }

    /// Set or clear the autotyping timezone.
    #[napi(setter)]
    pub fn set_timezone(&mut self, value: Option<TimezoneInput<'_>>) -> Result<()> {
        self.inner
            .set_timezone(value.map(timezone_from_input).transpose()?);
        Ok(())
    }

    /// Return a copy with a declared root field.
    #[napi]
    pub fn with_field(&self, field: &JsField) -> Self {
        let mut options = self.clone();
        options.set_field(field);
        options
    }

    /// Return a copy with a different root name.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        let mut options = self.clone();
        options.set_name(name);
        options
    }

    /// Return a copy with a declared root datatype.
    #[napi]
    pub fn with_dtype(&self, dtype: DataTypeInput<'_>) -> Result<Self> {
        let mut options = self.clone();
        options.set_dtype(Some(dtype))?;
        Ok(options)
    }

    /// Return a copy with declared root metadata.
    #[napi]
    pub fn with_metadata(&self, values: MetadataInput) -> Result<Self> {
        let mut options = self.clone();
        options.set_metadata(values)?;
        Ok(options)
    }

    /// Return a copy with different cast strictness.
    #[napi]
    pub fn with_safe(&self, safe: bool) -> Self {
        let mut options = self.clone();
        options.set_safe(safe);
        options
    }

    /// Return a copy with a row-per-batch bound.
    #[napi]
    pub fn with_batch_row_size(&self, size: u32) -> Result<Self> {
        let mut options = self.clone();
        options.set_batch_row_size(Some(size))?;
        Ok(options)
    }

    /// Return a copy with a streamed-write commit cadence.
    #[napi]
    pub fn with_commit_row_size(&self, rows: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_commit_row_size(Some(rows))?;
        Ok(options)
    }

    /// Return a copy with a total result-row bound.
    #[napi]
    pub fn with_max_row_size(&self, rows: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_max_row_size(Some(rows))?;
        Ok(options)
    }

    /// Return a copy with an Arrow-memory byte bound.
    #[napi]
    pub fn with_max_byte_size(&self, bytes: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_max_byte_size(Some(bytes))?;
        Ok(options)
    }

    /// Return a copy with a different content-coding level.
    #[napi]
    pub fn with_level(&self, level: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_level(level)?;
        Ok(options)
    }

    /// Return a copy with write match-key columns.
    #[napi]
    pub fn with_merge_by_names(&self, names: Vec<String>) -> Self {
        let mut options = self.clone();
        options.set_merge_by_names(names);
        options
    }

    /// Return a copy with selected columns.
    #[napi]
    pub fn with_select_by_names(&self, names: Vec<String>) -> Self {
        let mut options = self.clone();
        options.set_select_by_names(names);
        options
    }

    /// Return a copy with partition filters.
    #[napi]
    pub fn with_filter_partitions(&self, partitions: Vec<(String, String)>) -> Self {
        let mut options = self.clone();
        options.set_filter_partitions(partitions);
        options
    }

    /// Return whether every setting is equal.
    #[napi]
    pub fn equals(&self, other: &JsTextOptions) -> bool {
        self.inner == other.inner
    }

    /// Compare every setting through the core total order.
    #[napi]
    pub fn compare(&self, other: &JsTextOptions) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Return a detached copy.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return the media type spelling.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        MimeType::PLAIN_TEXT.to_string()
    }
}
