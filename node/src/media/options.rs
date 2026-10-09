//! The record settings every encoding shares, as one JavaScript value.
//!
//! The variant *is* the encoding, so a record call takes `RecordOptions` and no
//! separate format argument. The encoding is never guessed: it is derived from a
//! media type, which is what [`crate::iobase::JsIOBase::record_options`] reads off
//! the handle.

use napi::bindgen_prelude::{Buffer, Either, Null, Result};
use napi_derive::napi;
use yggdryl::avro::AvroOptions;
use yggdryl::excel::ExcelOptions;
use yggdryl::media::{
    DEFAULT_RECORD_BATCH_ROW_SIZE, IORecordOptions, RecordOptions as CoreRecordOptions,
};
use yggdryl::parquet::ParquetOptions;
use yggdryl::{IOMode, Level};

use crate::enums::{
    JsMimeType, MediaTypeInput, MimeTypeInput, media_type_from_input, mime_type_from_input,
};
use crate::exact_u8;
use crate::expression::{
    JsFilter, JsPlan, JsSelector, filter_from_input, plan_from_input, selector_from_input,
};
use crate::field::{JsField, MetadataEntry};
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

/// A `commitBatchNum` value as the batch count the core counts: an exact
/// integer in this platform's range. Zero is kept, so the write preflight
/// refuses it by name before a one-shot source is touched.
pub(crate) fn batch_count(batches: f64) -> Result<usize> {
    let batches = crate::exact_u64(batches, "commitBatchNum")?;
    usize::try_from(batches).map_err(|_| {
        napi_error(format!(
            "commitBatchNum {batches} exceeds this platform's batch-count range"
        ))
    })
}

/// A `numThreads` value as the thread count the core counts: an exact
/// integer in this platform's range. Zero is kept, so the write preflight
/// refuses it by name before a one-shot source is touched.
pub(crate) fn thread_count(threads: f64) -> Result<usize> {
    let threads = crate::exact_u64(threads, "numThreads")?;
    usize::try_from(threads).map_err(|_| {
        napi_error(format!(
            "numThreads {threads} exceeds this platform's thread-count range"
        ))
    })
}

/// A CSV role byte as the one-character string JavaScript spells it.
fn byte_text(byte: u8) -> String {
    char::from(byte).to_string()
}

/// The one byte a CSV role is spelled as, read by the core's one spelling
/// ([`yggdryl::csv::CsvOptions::byte_from_text`]); whether that byte may
/// play the role is the core setter's judgement.
fn byte_of(text: &str, name: &str) -> Result<u8> {
    yggdryl::csv::CsvOptions::byte_from_text(text, name).map_err(crate::napi_error)
}

/// The byte an optional CSV role is set to, or `None` where `null` clears
/// it. `undefined` is an argument not given, never a `null`: the `Either`
/// refuses it before it reaches here, so nothing is cleared by omission.
fn optional_byte_of(value: Either<String, Null>, name: &str) -> Result<Option<u8>> {
    match value {
        Either::A(text) => byte_of(&text, name).map(Some),
        Either::B(Null) => Ok(None),
    }
}

#[napi]
impl JsRecordOptions {
    /// The options for the encoding `value` names, each of `properties` set
    /// by its own setter - applied by the JavaScript class, as a record
    /// call's property bag is.
    #[napi(
        constructor,
        ts_args_type = "value: MediaTypeInput, properties?: Record<string, unknown> | null"
    )]
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

    /// The declared root Field - the `create` section of the plan - or
    /// `null` when the shape is inferred from the rows.
    #[napi(getter)]
    pub fn field(&self) -> Option<JsField> {
        self.inner.field().map(JsField::from_core)
    }

    /// The write mode `mode` spells, read through the core's `IOMode`
    /// vocabulary - trimmed, in any case - as its canonical name: one of
    /// `overwrite`, `append` and `merge`. The loader captures and removes
    /// this private bridge, and reads every generic write's mode through it
    /// before an input is touched; a mode that writes nothing (`readonly`,
    /// `random`) is refused here.
    #[napi(js_name = "_writeModeNative", skip_typescript)]
    pub fn write_mode_native(mode: String) -> Result<String> {
        let mode = IOMode::from_str(&mode).map_err(napi_error)?;
        if IOMode::WRITE.contains(&mode) {
            Ok(mode.as_str().to_owned())
        } else {
            Err(napi_error(format!(
                "expected a write mode - {} - got {mode}",
                IOMode::WRITE.map(IOMode::as_str).join(", ")
            )))
        }
    }

    /// Validate explicit write intent before JavaScript converts or pulls input.
    ///
    /// The loader captures and removes this private bridge, then calls it ahead
    /// of every representation adapter so an invalid mode never consumes a
    /// one-shot reader, iterable, or async iterable. Where the destination is
    /// given, a merge naming no key is checked against its own key
    /// (`IOMedia::merge_by`: an Iceberg table's identity partition columns,
    /// then its identifier columns) and refused only where it states none.
    #[napi(js_name = "_requireWritePreflightNative", skip_typescript)]
    pub fn require_write_preflight(
        &self,
        intent: String,
        handle: Option<&crate::iobase::JsIOBase>,
    ) -> Result<u32> {
        let mode = IOMode::from_str(&intent).map_err(napi_error)?;
        let options = if let Some(handle) = handle {
            yggdryl::IOMedia::write_options(handle.core(), mode, &self.inner).map_err(napi_error)?
        } else {
            self.inner.require_write_mode(mode).map_err(napi_error)?;
            std::borrow::Cow::Borrowed(&self.inner)
        };
        options.require_commit_batch_num().map_err(napi_error)?;
        options.require_num_threads().map_err(napi_error)?;
        options.require_write_limits().map_err(napi_error)?;
        u32::try_from(DEFAULT_RECORD_BATCH_ROW_SIZE).map_err(napi_error)
    }

    /// Declare the root Field, or clear it with `null`; the stored root is
    /// always required, whatever nullability the value carried.
    #[napi(setter)]
    pub fn set_field(&mut self, field: Option<&JsField>) {
        match field {
            Some(field) => self.inner.set_field(field.inner.clone()),
            None => self.inner.set_declared(None),
        }
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

    /// Whether a declared or stored nullable column takes a value it cannot
    /// convert as null, `true` by default; a not-null column refuses it by name
    /// either way.
    #[napi(getter)]
    pub fn safe(&self) -> bool {
        self.inner.safe()
    }

    /// Set whether a declared or stored nullable column takes a value it cannot
    /// convert as null; `false` refuses it too.
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

    /// How many leading result rows a read or write skips, when set - the
    /// plan's `offset`; the row bound counts the rows after it.
    #[napi(getter)]
    pub fn row_offset(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.row_offset().map(|rows| rows as f64)
    }

    /// Set how many leading result rows a read or write skips.
    #[napi(setter)]
    pub fn set_row_offset(&mut self, row_offset: Option<f64>) -> Result<()> {
        let skip = match row_offset {
            Some(rows) => Some(crate::exact_u64(rows, "rowOffset")?),
            None => None,
        };
        self.inner.set_row_offset(skip);
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

    /// Whole batches published per streamed-write commit, when one is set.
    ///
    /// A positive count publishes every that many batches of the shaped
    /// stream, then the final remainder; a batch is one the source yields,
    /// cut by `batchRowSize` where records are converted, never by the
    /// cadence. `null` is the destination's own cadence: a file, a folder
    /// and an Iceberg table each publish once after the source ends, what
    /// they hold in between kept under the process spill bound.
    #[napi(getter)]
    pub fn commit_batch_num(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.commit_batch_num().map(|batches| batches as f64)
    }

    /// Set the streamed-write publication cadence, in whole batches.
    ///
    /// Zero is retained so the write preflight can reject it before touching a
    /// one-shot JavaScript source. `null` restores the destination's own
    /// cadence.
    #[napi(setter)]
    pub fn set_commit_batch_num(&mut self, commit_batch_num: Option<f64>) -> Result<()> {
        let batches = match commit_batch_num {
            Some(batches) => Some(crate::media::options::batch_count(batches)?),
            None => None,
        };
        self.inner.set_commit_batch_num(batches);
        Ok(())
    }

    /// The threads a write of several parts runs on at once, when set.
    ///
    /// The parts are an Iceberg commit's partition groups, written side by
    /// side; a leaf of one file reads it as the bound on its encoding's
    /// threads. `null` is the destination's own answer: an Iceberg table's
    /// `write.parallelism`, else its `read.parallelism`, else every thread
    /// the host offers.
    #[napi(getter)]
    pub fn num_threads(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.num_threads().map(|threads| threads as f64)
    }

    /// Set the threads a write of several parts runs on at once.
    ///
    /// Zero is retained so the write preflight refuses it by name, naming
    /// `$.num_threads`, before a one-shot source is touched. `null` restores
    /// the destination's own answer.
    #[napi(setter)]
    pub fn set_num_threads(&mut self, num_threads: Option<f64>) -> Result<()> {
        let threads = match num_threads {
            Some(threads) => Some(crate::media::options::thread_count(threads)?),
            None => None,
        };
        self.inner.set_num_threads(threads);
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

    /// The keys a write matches stored rows on - the plan's `upsert by`;
    /// `select *` (empty) means overwrite or append.
    #[napi(getter)]
    pub fn merge_by(&self) -> JsSelector {
        JsSelector::from_core(self.inner.merge_by().clone())
    }

    /// Set the keys a write matches stored rows on: a `Selector`, the text
    /// of one, or the key column names.
    #[napi(setter)]
    pub fn set_merge_by(
        &mut self,
        merge_by: Option<
            napi::bindgen_prelude::Either4<
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                String,
                Vec<
                    napi::bindgen_prelude::Either<
                        napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                        String,
                    >,
                >,
            >,
        >,
    ) -> Result<()> {
        // `null` is a value, and clears: no key, so an overwrite or append.
        let merge_by = match merge_by {
            Some(merge_by) => selector_from_input(merge_by)?,
            None => yggdryl::Selector::from_scalar(&yggdryl::Scalar::Null).map_err(napi_error)?,
        };
        self.inner.set_merge_by(merge_by);
        Ok(())
    }

    /// The `select` section a read or write is shaped by; `select *` keeps
    /// every column.
    #[napi(getter)]
    pub fn select(&self) -> JsSelector {
        JsSelector::from_core(self.inner.select().clone())
    }

    /// Set the `select` section: a `Selector`, the text of one, a `Term`, or
    /// the column names.
    #[napi(setter)]
    pub fn set_select(
        &mut self,
        select: Option<
            napi::bindgen_prelude::Either4<
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                String,
                Vec<
                    napi::bindgen_prelude::Either<
                        napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                        String,
                    >,
                >,
            >,
        >,
    ) -> Result<()> {
        // `null` is a value, and clears: `select *`.
        let select = match select {
            Some(select) => selector_from_input(select)?,
            None => yggdryl::Selector::from_scalar(&yggdryl::Scalar::Null).map_err(napi_error)?,
        };
        self.inner.set_select(select);
        Ok(())
    }

    /// The `where` section a read is pruned and filtered by; always true
    /// keeps every row.
    #[napi(getter)]
    pub fn filter(&self) -> JsFilter {
        JsFilter::from_core(self.inner.filter().clone())
    }

    /// Set the `where` section: a `Filter`, a `Term`, or the text of a
    /// predicate.
    #[napi(setter)]
    pub fn set_filter(
        &mut self,
        filter: Option<
            napi::bindgen_prelude::Either3<
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsFilter>,
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                String,
            >,
        >,
    ) -> Result<()> {
        // `null` is a value, and clears: always true.
        let filter = match filter {
            Some(filter) => filter_from_input(filter)?,
            None => yggdryl::Filter::from_scalar(&yggdryl::Scalar::Null).map_err(napi_error)?,
        };
        self.inner.set_filter(filter);
        Ok(())
    }

    /// The whole plan these options run: `create` from the declared field,
    /// `upsert by` from the merge keys, `select`, `where`, and `limit` from
    /// `maxRowSize`.
    #[napi(getter)]
    pub fn plan(&self) -> JsPlan {
        JsPlan::from_core(self.inner.plan())
    }

    /// Split a `Plan`, its text, a clause, or a `Field` back into the
    /// sections, replacing every one of them.
    #[napi(setter)]
    pub fn set_plan(
        &mut self,
        plan: Option<
            napi::bindgen_prelude::Either5<
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsPlan>,
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
                napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsFilter>,
                napi::bindgen_prelude::ClassInstance<'_, crate::field::JsField>,
                String,
            >,
        >,
    ) -> Result<()> {
        // `null` is a value, and clears: the plan with no section.
        let plan = match plan {
            Some(plan) => plan_from_input(plan)?,
            None => yggdryl::Plan::from_scalar(&yggdryl::Scalar::Null).map_err(napi_error)?,
        };
        self.inner.set_plan(plan).map_err(napi_error)
    }

    /// The partition equalities the filter pins, `[column, value]` pairs
    /// spelled as partition paths spell them; what prunes a listing before
    /// anything is opened.
    #[napi]
    pub fn partition_pairs(&self) -> Vec<(String, String)> {
        self.inner.partition_pairs()
    }

    /// The timezone applied while autotyping offset-free timestamps.
    #[napi(getter)]
    pub fn timezone(&self) -> Option<JsTimezone> {
        self.inner.timezone().copied().map(JsTimezone::from_core)
    }

    /// Set or clear the timezone for autotyped timestamps.
    #[napi(setter)]
    pub fn set_timezone(&mut self, value: Option<TimezoneInput<'_>>) -> Result<()> {
        let timezone = value.map(timezone_from_input).transpose()?;
        self.inner.set_timezone(timezone).map_err(napi_error)
    }

    /// The worksheet a workbook read or write addresses, `null` for the
    /// first worksheet - or for another encoding.
    #[napi(getter)]
    pub fn sheet(&self) -> Option<String> {
        self.inner
            .settings::<ExcelOptions>()
            .and_then(|options| options.sheet())
            .map(ToOwned::to_owned)
    }

    /// Address the worksheet `sheet`, or the first worksheet for `null`.
    #[napi(setter)]
    pub fn set_sheet(&mut self, sheet: Option<String>) -> Result<()> {
        self.inner
            .require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")
            .and_then(|options| options.set_sheet(sheet.as_deref()))
            .map_err(napi_error)
    }

    /// The cells a workbook read or write addresses, `null` for the whole
    /// sheet - or for another encoding.
    #[napi(getter)]
    pub fn range(&self) -> Option<crate::excel::JsCellRange> {
        self.inner
            .settings::<ExcelOptions>()
            .and_then(ExcelOptions::range)
            .map(|inner| crate::excel::JsCellRange { inner })
    }

    /// Address the cells of `range`, or the whole sheet for `null`.
    #[napi(setter)]
    pub fn set_range(&mut self, range: Option<crate::excel::CellRangeInput<'_>>) -> Result<()> {
        let range = range.map(crate::excel::cell_range_from).transpose()?;
        self.inner
            .require_settings_mut::<ExcelOptions>("$.range", "a cell range")
            .map(|options| options.set_range(range))
            .map_err(napi_error)
    }

    /// These options addressing the sheet `sheet`.
    #[napi]
    pub fn with_sheet(&self, sheet: Option<String>) -> Result<Self> {
        let mut options = self.clone();
        options.set_sheet(sheet)?;
        Ok(options)
    }

    /// These options addressing the cells of `range`.
    #[napi]
    pub fn with_range(&self, range: Option<crate::excel::CellRangeInput<'_>>) -> Result<Self> {
        let mut options = self.clone();
        options.set_range(range)?;
        Ok(options)
    }

    /// The Avro block codec name, or `null` for another encoding.
    #[napi(getter)]
    pub fn block_codec(&self) -> Option<String> {
        self.inner
            .settings::<AvroOptions>()
            .map(|options| options.block_codec().to_owned())
    }

    /// Validate and set the Avro block codec name.
    #[napi(setter)]
    pub fn set_block_codec(&mut self, block_codec: String) -> Result<()> {
        self.inner
            .require_settings_mut::<AvroOptions>("$.block_codec", "a block codec")
            .and_then(|options| options.set_block_codec(&block_codec))
            .map_err(napi_error)
    }

    /// The fixed sixteen-byte Avro synchronization marker, when one is set.
    #[napi(getter)]
    pub fn sync_marker(&self) -> Option<Buffer> {
        self.inner
            .settings::<AvroOptions>()
            .and_then(|options| options.sync_marker())
            .map(|marker| marker.to_vec().into())
    }

    /// Set or clear the fixed Avro synchronization marker.
    #[napi(setter)]
    pub fn set_sync_marker(&mut self, marker: Option<Buffer>) -> Result<()> {
        self.inner
            .require_settings_mut::<AvroOptions>("$.sync_marker", "a synchronization marker")
            .and_then(|options| options.set_sync_marker(marker.as_deref()))
            .map_err(napi_error)
    }

    /// The page compression applied inside a Parquet file, if this is one.
    ///
    /// A setting one encoding has is absent on the others rather than invented,
    /// so this is `null` for an Arrow IPC stream, whose coding belongs to the
    /// handle instead.
    #[napi(getter)]
    pub fn compression(&self) -> Option<String> {
        self.inner
            .settings::<ParquetOptions>()
            .map(ParquetOptions::compression_name)
    }

    /// Set the page compression applied inside a Parquet file.
    #[napi(setter)]
    pub fn set_compression(&mut self, compression: String) -> Result<()> {
        self.inner
            .require_settings_mut::<ParquetOptions>("$.compression", "a page compression")
            .and_then(|options| options.set_compression_name(&compression))
            .map_err(napi_error)
    }

    /// The maximum rows per row group of a Parquet file, if this is one.
    #[napi(getter)]
    pub fn max_row_group_size(&self) -> Option<u32> {
        self.inner
            .settings::<ParquetOptions>()
            .and_then(|options| u32::try_from(options.max_row_group_size).ok())
    }

    /// Set the maximum rows per row group of a Parquet file.
    #[napi(setter)]
    pub fn set_max_row_group_size(&mut self, rows: u32) -> Result<()> {
        self.inner
            .require_settings_mut::<ParquetOptions>("$.max_row_group_size", "a row-group size")
            .map(|options| options.set_max_row_group_size(rows as usize))
            .map_err(napi_error)
    }

    /// The footer key/value entries a Parquet write adds.
    #[napi(getter)]
    pub fn key_value_metadata(&self) -> Vec<MetadataEntry> {
        self.inner
            .settings::<ParquetOptions>()
            .map_or(&[][..], |options| options.key_value_metadata.as_slice())
            .iter()
            .map(|(key, value)| MetadataEntry {
                key: key.clone(),
                value: value.clone(),
            })
            .collect()
    }

    /// The CSV byte between two cells, as the one-character string it is;
    /// `null` for another encoding.
    #[napi(getter)]
    pub fn separator(&self) -> Option<String> {
        self.inner.csv_separator().map(byte_text)
    }

    /// Set the CSV byte between two cells: one ASCII character, neither a
    /// line break nor a byte another role holds.
    #[napi(setter)]
    pub fn set_separator(&mut self, separator: String) -> Result<()> {
        self.inner
            .set_csv_separator(byte_of(&separator, "separator")?)
            .map_err(napi_error)
    }

    /// The CSV quote byte as a one-character string; `null` where the
    /// dialect quotes nothing, or for another encoding.
    #[napi(getter)]
    pub fn quote(&self) -> Option<String> {
        self.inner.csv_quote().flatten().map(byte_text)
    }

    /// Set the CSV quote byte, or clear it with `null` so nothing is quoted
    /// on write and a quote reads as content; `undefined`, an argument not
    /// given, clears nothing and is refused.
    #[napi(setter)]
    pub fn set_quote(&mut self, quote: Either<String, Null>) -> Result<()> {
        let quote = optional_byte_of(quote, "quote")?;
        self.inner.set_csv_quote(quote).map_err(napi_error)
    }

    /// The CSV escape byte as a one-character string; `null` where a quote
    /// inside a quoted cell is doubled instead (RFC 4180), or for another
    /// encoding.
    #[napi(getter)]
    pub fn escape(&self) -> Option<String> {
        self.inner.csv_escape().flatten().map(byte_text)
    }

    /// Set the CSV escape byte, or clear it with `null`; `undefined` clears
    /// nothing and is refused.
    #[napi(setter)]
    pub fn set_escape(&mut self, escape: Either<String, Null>) -> Result<()> {
        let escape = optional_byte_of(escape, "escape")?;
        self.inner.set_csv_escape(escape).map_err(napi_error)
    }

    /// The CSV comment byte - a record opening with it is skipped - as a
    /// one-character string; `null` where none is, or for another encoding.
    #[napi(getter)]
    pub fn comment(&self) -> Option<String> {
        self.inner.csv_comment().flatten().map(byte_text)
    }

    /// Set the CSV comment byte, or clear it with `null`; `undefined` clears
    /// nothing and is refused.
    #[napi(setter)]
    pub fn set_comment(&mut self, comment: Either<String, Null>) -> Result<()> {
        let comment = optional_byte_of(comment, "comment")?;
        self.inner.set_csv_comment(comment).map_err(napi_error)
    }

    /// Whether the first record names the columns - a CSV's first record, a
    /// workbook's first row; `null` for another encoding.
    #[napi(getter)]
    pub fn header(&self) -> Option<bool> {
        self.inner.header()
    }

    /// Set whether the first record names the columns: a CSV's first record,
    /// a workbook's first row.
    #[napi(setter)]
    pub fn set_header(&mut self, header: bool) -> Result<()> {
        self.inner.set_header(header).map_err(napi_error)
    }

    /// The CSV spellings of an absent value - an unquoted cell spelling one
    /// is null, a null is written as the first; `null` for another encoding.
    #[napi(getter)]
    pub fn null_values(&self) -> Option<Vec<String>> {
        self.inner
            .csv_null_values()
            .map(|spellings| spellings.iter().map(ToString::to_string).collect())
    }

    /// Set the CSV spellings of an absent value, each listed once.
    #[napi(setter)]
    pub fn set_null_values(&mut self, null_values: Vec<String>) -> Result<()> {
        self.inner
            .set_csv_null_values(null_values)
            .map_err(napi_error)
    }

    /// Whether the CSV drops the blanks around an unquoted cell; `null` for
    /// another encoding.
    #[napi(getter)]
    pub fn trim(&self) -> Option<bool> {
        self.inner.csv_trim()
    }

    /// Set whether the CSV drops the blanks around an unquoted cell.
    #[napi(setter)]
    pub fn set_trim(&mut self, trim: bool) -> Result<()> {
        self.inner.set_csv_trim(trim).map_err(napi_error)
    }

    /// The records a CSV read samples to infer a column's datatype when no
    /// field is declared; `null` for another encoding.
    #[napi(getter)]
    pub fn infer_row_size(&self) -> Option<f64> {
        #[allow(clippy::cast_precision_loss)]
        self.inner.csv_infer_row_size().map(|rows| rows as f64)
    }

    /// Set the records a CSV read samples to infer a column's datatype; zero
    /// is refused.
    #[napi(setter)]
    pub fn set_infer_row_size(&mut self, infer_row_size: f64) -> Result<()> {
        let rows = crate::exact_u64(infer_row_size, "inferRowSize")?;
        let rows = usize::try_from(rows).map_err(|_| {
            napi_error(format!(
                "inferRowSize {rows} exceeds this platform's row-count range"
            ))
        })?;
        self.inner.set_csv_infer_row_size(rows).map_err(napi_error)
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
            .require_settings_mut::<ParquetOptions>("$.key_value_metadata", "footer metadata")
            .map(|options| options.push_key_value(key, value))
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

    /// Return these options with another CSV byte between two cells.
    #[napi]
    pub fn with_separator(&self, separator: String) -> Result<Self> {
        let mut options = self.clone();
        options.set_separator(separator)?;
        Ok(options)
    }

    /// Return these options with another CSV quote byte, or `null` for none.
    #[napi]
    pub fn with_quote(&self, quote: Either<String, Null>) -> Result<Self> {
        let mut options = self.clone();
        options.set_quote(quote)?;
        Ok(options)
    }

    /// Return these options with another CSV escape byte, or `null` for none.
    #[napi]
    pub fn with_escape(&self, escape: Either<String, Null>) -> Result<Self> {
        let mut options = self.clone();
        options.set_escape(escape)?;
        Ok(options)
    }

    /// Return these options with another CSV comment byte, or `null` for none.
    #[napi]
    pub fn with_comment(&self, comment: Either<String, Null>) -> Result<Self> {
        let mut options = self.clone();
        options.set_comment(comment)?;
        Ok(options)
    }

    /// Return these options with or without a header record: a CSV's first
    /// record, a workbook's first row.
    #[napi]
    pub fn with_header(&self, header: bool) -> Result<Self> {
        let mut options = self.clone();
        options.set_header(header)?;
        Ok(options)
    }

    /// Return these options with other CSV spellings of an absent value.
    #[napi]
    pub fn with_null_values(&self, null_values: Vec<String>) -> Result<Self> {
        let mut options = self.clone();
        options.set_null_values(null_values)?;
        Ok(options)
    }

    /// Return these options trimming, or keeping, the blanks around a CSV
    /// cell.
    #[napi]
    pub fn with_trim(&self, trim: bool) -> Result<Self> {
        let mut options = self.clone();
        options.set_trim(trim)?;
        Ok(options)
    }

    /// Return these options sampling another number of CSV records to infer
    /// a column's datatype.
    #[napi]
    pub fn with_infer_row_size(&self, infer_row_size: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_infer_row_size(infer_row_size)?;
        Ok(options)
    }

    /// Return these options with a declared canonical root Field.
    #[napi]
    pub fn with_field(&self, field: &JsField) -> Self {
        let mut options = self.clone();
        options.set_field(Some(field));
        options
    }

    /// Return these options with a different root Field name.
    #[napi]
    pub fn with_name(&self, name: String) -> Self {
        let mut options = self.clone();
        options.set_name(name);
        options
    }

    /// Return a copy whose declared or stored nullable columns take a value
    /// they cannot convert as null (`true`) or refuse it (`false`).
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

    /// Return these options skipping the given leading result rows.
    #[napi]
    pub fn with_row_offset(&self, row_offset: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_row_offset(Some(row_offset))?;
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

    /// Return these options with a publication every `commitBatchNum` batches.
    #[napi]
    pub fn with_commit_batch_num(&self, commit_batch_num: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_commit_batch_num(Some(commit_batch_num))?;
        Ok(options)
    }

    /// Return these options running a write of several parts on `numThreads`.
    #[napi]
    pub fn with_num_threads(&self, num_threads: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_num_threads(Some(num_threads))?;
        Ok(options)
    }

    /// Return these options with a different compression level.
    #[napi]
    pub fn with_level(&self, level: f64) -> Result<Self> {
        let mut options = self.clone();
        options.set_level(level)?;
        Ok(options)
    }

    /// Return these options with the keys a write matches stored rows on.
    #[napi]
    pub fn with_merge_by(
        &self,
        merge_by: napi::bindgen_prelude::Either4<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
            Vec<
                napi::bindgen_prelude::Either<
                    napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                    String,
                >,
            >,
        >,
    ) -> Result<Self> {
        let mut options = self.clone();
        options.set_merge_by(Some(merge_by))?;
        Ok(options)
    }

    /// Return these options shaped by a `select` section, on reads and writes.
    #[napi]
    pub fn with_select(
        &self,
        select: napi::bindgen_prelude::Either4<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
            Vec<
                napi::bindgen_prelude::Either<
                    napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
                    String,
                >,
            >,
        >,
    ) -> Result<Self> {
        let mut options = self.clone();
        options.set_select(Some(select))?;
        Ok(options)
    }

    /// Return these options pruned and filtered by a `where` section.
    #[napi]
    pub fn with_filter(
        &self,
        filter: napi::bindgen_prelude::Either3<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsFilter>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsTerm>,
            String,
        >,
    ) -> Result<Self> {
        let mut options = self.clone();
        options.set_filter(Some(filter))?;
        Ok(options)
    }

    /// Return these options with every section a plan spells.
    #[napi]
    pub fn with_plan(
        &self,
        plan: napi::bindgen_prelude::Either5<
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsPlan>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsSelector>,
            napi::bindgen_prelude::ClassInstance<'_, crate::expression::JsFilter>,
            napi::bindgen_prelude::ClassInstance<'_, crate::field::JsField>,
            String,
        >,
    ) -> Result<Self> {
        let mut options = self.clone();
        options.set_plan(Some(plan))?;
        Ok(options)
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
