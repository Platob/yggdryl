//! The settings a record read or write takes, shared across encodings.
//!
//! Reading rows out of an Arrow IPC stream and reading them out of a Parquet
//! file need the same handful of answers - what schema, what to call an
//! inferred root, how strict a cast may be, how many rows per batch, how much
//! may flow at all, how hard to compress, and what makes two rows the same
//! row - and each encoding then
//! adds its own. [`IORecordOptions`] is that shared surface, each encoding
//! stores those settings as its own flat fields, and [`RecordOptions`] is the
//! enum naming every encoding's options.
//!
//! An encoding is never guessed: [`RecordOptions::for_media_type`] derives it
//! from the handle's media type, which is what [`crate::io::IOBase`]'s record
//! methods use when a caller does not supply options of their own.
//!
//! ```
//! use yggdryl::generic::{IORecordOptions, RecordOptions};
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
//!     .required_field("row");
//!
//! // Arrow IPC is the encoding every build implements, so this example reads
//! // the same in a default build as in one with the `parquet` feature on.
//! let options = RecordOptions::for_media_type(&Url::from_str("file:///t.arrows")?.media_type())?
//!     .with_schema(schema.clone())
//!     .with_batch_size(1024);
//!
//! assert_eq!(options.schema(), Some(&schema));
//! assert_eq!(options.batch_size(), Some(1024));
//! # Ok(())
//! # }
//! ```

use smol_str::SmolStr;

use crate::Level;
use crate::ipc::IpcOptions;
use crate::{Error, Field, MediaType, MimeType, Result};

/// The read and write settings shared by every record encoding.
///
/// Each encoding stores these as its own fields - there is no shared settings
/// struct to thread through - and the builders here are what every caller uses,
/// so the encodings cannot drift apart in what a shared setting means.
pub trait IORecordOptions: Sized {
    /// Borrow the declared canonical schema, if any.
    fn schema(&self) -> Option<&Field>;

    /// Declare the canonical schema.
    fn set_schema(&mut self, schema: Field);

    /// Borrow the root Field name used when a schema is inferred.
    fn root_name(&self) -> &str;

    /// Set the root Field name used when a schema is inferred.
    fn set_root_name(&mut self, root_name: SmolStr);

    /// Return whether a cast may null a value it cannot convert.
    fn safe(&self) -> bool;

    /// Set whether a cast may null a value it cannot convert.
    fn set_safe(&mut self, safe: bool);

    /// Return the row-per-batch bound, if any.
    fn batch_size(&self) -> Option<usize>;

    /// Set the row-per-batch bound.
    fn set_batch_size(&mut self, batch_size: Option<usize>);

    /// Return the bound on how many result rows flow in total, if any - a
    /// **count of rows**, never a per-row byte cap, because the name reads
    /// both ways.
    ///
    /// The limit is the last transform of the shaping pipeline - declared
    /// schema, then selection, then completion cast, then partition filter,
    /// then the limit - so it counts *result* rows: a limit of ten combined
    /// with a filter means the first ten matching rows. `Some(0)` yields a
    /// reader with the shaped schema and no batches rather than an error;
    /// `None` is unlimited. The bound is exact: the batch it lands inside is
    /// cut with [`RecordBatch::slice`](arrow_array::RecordBatch::slice), a
    /// view over the same buffers rather than a copy.
    fn max_row_size(&self) -> Option<u64>;

    /// Set the bound on how many result rows flow in total.
    fn set_max_row_size(&mut self, max_row_size: Option<u64>);

    /// Return the bound on the result rows' Arrow in-memory bytes, if any.
    ///
    /// Bytes are counted as
    /// [`get_array_memory_size`](arrow_array::RecordBatch::get_array_memory_size)
    /// counts them - the same accounting the Iceberg target-file-size rolling
    /// uses - never as encoded bytes, so a Parquet file written under a byte
    /// limit lands well under it: the format compresses what this measures
    /// uncompressed. The flow stops at the last row that keeps the running
    /// total at or under the limit, and a non-zero limit always yields at
    /// least one row rather than silently losing everything to one wide row -
    /// only `Some(0)` yields nothing. When
    /// [`max_row_size`](Self::max_row_size) is also set, whichever bound binds
    /// first wins.
    fn max_byte_size(&self) -> Option<u64>;

    /// Set the bound on the result rows' Arrow in-memory bytes.
    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>);

    /// Return the compression level applied to a declared content coding.
    fn level(&self) -> Level;

    /// Set the compression level.
    fn set_level(&mut self, level: Level);

    /// Borrow the column names whose values form a write's match key.
    ///
    /// An empty list is an overwrite. A non-empty one names the columns a
    /// [`write`](crate::io::IOBase::write_arrow_batch_reader) matches incoming
    /// rows against: a row whose key is already stored updates it, and a row
    /// whose key is not appends.
    fn merge_by_names(&self) -> &[String];

    /// Set the column names whose values form a write's match key.
    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>);

    /// Borrow the column names a read or write is narrowed to.
    ///
    /// An empty list selects everything. A non-empty one names the columns, in
    /// the order they are wanted: a read yields exactly those columns of the
    /// stored rows, and a write keeps exactly those columns of the incoming
    /// rows. Names match ASCII case-insensitively, the way every cast selects,
    /// and a name the rows do not have is an error rather than a null column,
    /// because a selection is a claim about what is there.
    fn select_by_names(&self) -> &[String];

    /// Set the column names a read or write is narrowed to.
    fn set_select_by_names(&mut self, select_by_names: Vec<String>);

    /// Borrow the partition equalities a read is pruned and filtered by.
    ///
    /// An empty list keeps every row. A non-empty one names
    /// `(column, value)` pairs, values spelled as
    /// [`partition_text`](crate::io::partition::partition_text) spells them:
    /// a folder read skips every leaf whose path names a different value -
    /// nothing under it is listed or decoded - and rows whose data carries
    /// the column are filtered to the named values, so path-partitioned and
    /// data-partitioned layouts answer the same question the same way.
    fn filter_partitions(&self) -> &[(String, String)];

    /// Set the partition equalities a read is pruned and filtered by.
    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>);

    /// The predicate these options' partition equalities spell about a *path*.
    ///
    /// One expression, built from the pairs, asked of the holder rather than
    /// of the rows: `&holder.partition['year'] = '2024'`. This is what prunes
    /// a listing before anything is opened, and it is the same predicate type
    /// the rows are filtered with - the pairs are sugar over one
    /// representation, not a second filter.
    fn partition_filter(&self) -> crate::Expression {
        crate::Expression::all_holder_partitions_equal(
            self.filter_partitions()
                .iter()
                .map(|(column, value)| (column, value)),
        )
    }

    /// The predicate these options' partition equalities spell about *rows*.
    ///
    /// The same pairs, read through the schema's own datatypes, which is what
    /// makes `("price", "20")` an integer comparison on an `int32` column. A
    /// pair naming a column the schema does not declare is left out: the path
    /// answered for it already.
    fn partition_predicate(&self, schema: &crate::Field) -> crate::Expression {
        crate::Expression::all_partitions_equal(
            schema,
            self.filter_partitions()
                .iter()
                .map(|(column, value)| (column, value)),
        )
    }

    /// Borrow the declared schema, or say that one is required.
    ///
    /// # Errors
    ///
    /// Returns an error naming the builder that declares a schema.
    fn require_schema(&self) -> Result<&Field> {
        self.schema().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static(
                "expected a declared schema to write records; call with_schema first",
            ),
        })
    }

    /// Return these options with a declared canonical schema.
    #[must_use]
    fn with_schema(mut self, schema: Field) -> Self {
        self.set_schema(schema);
        self
    }

    /// Return these options with a different inferred-root Field name.
    #[must_use]
    fn with_root_name(mut self, root_name: impl Into<SmolStr>) -> Self {
        self.set_root_name(root_name.into());
        self
    }

    /// Return these options with a different cast strictness.
    #[must_use]
    fn with_safe(mut self, safe: bool) -> Self {
        self.set_safe(safe);
        self
    }

    /// Return these options with a row-per-batch bound.
    #[must_use]
    fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.set_batch_size(Some(batch_size));
        self
    }

    /// Return these options with a bound on how many result rows flow.
    #[must_use]
    fn with_max_row_size(mut self, max_row_size: u64) -> Self {
        self.set_max_row_size(Some(max_row_size));
        self
    }

    /// Return these options with a bound on the result rows' Arrow bytes.
    #[must_use]
    fn with_max_byte_size(mut self, max_byte_size: u64) -> Self {
        self.set_max_byte_size(Some(max_byte_size));
        self
    }

    /// Return these options with a different compression level.
    #[must_use]
    fn with_level(mut self, level: Level) -> Self {
        self.set_level(level);
        self
    }

    /// Return these options with a match key for a write.
    #[must_use]
    fn with_merge_by_names<I, S>(mut self, merge_by_names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_merge_by_names(merge_by_names.into_iter().map(Into::into).collect());
        self
    }

    /// Return these options narrowed to the named columns, for reads and writes.
    #[must_use]
    fn with_select_by_names<I, S>(mut self, select_by_names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.set_select_by_names(select_by_names.into_iter().map(Into::into).collect());
        self
    }

    /// Return these options pruned and filtered to the named partitions.
    #[must_use]
    fn with_filter_partitions<I, C, V>(mut self, filter_partitions: I) -> Self
    where
        I: IntoIterator<Item = (C, V)>,
        C: Into<String>,
        V: Into<String>,
    {
        self.set_filter_partitions(
            filter_partitions
                .into_iter()
                .map(|(column, value)| (column.into(), value.into()))
                .collect(),
        );
        self
    }

    /// Cast one batch the way these options say, completed by what is stored.
    ///
    /// This is the one definition of option-driven casting, in three layers
    /// applied in order: the declared [`schema`](Self::schema) says what the
    /// rows are meant to be, [`select_by_names`](Self::select_by_names)
    /// narrows and orders the columns, and `existing` - a holder's stored
    /// shape - is what the batch is finally cast onto, always safely, so a
    /// value that will not convert into a stored column becomes null rather
    /// than quietly redefining that column for every reader of the resource.
    /// Each absent layer costs nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned or a selected name is
    /// not a column of the rows.
    fn cast_arrow_batch(
        &self,
        batch: arrow_array::RecordBatch,
        existing: Option<&Field>,
    ) -> Result<arrow_array::RecordBatch> {
        let safe = self.safe();
        let batch = match self.schema() {
            Some(declared) => crate::field::cast::cast_record_batch(declared, batch, safe)?,
            None => batch,
        };
        let root =
            crate::arrow::record_schema_from_arrow(self.root_name(), batch.schema().as_ref())?;
        let batch =
            match crate::arrow::selected_root(&root, self.select_by_names(), self.root_name())? {
                Some(target) => crate::field::cast::cast_record_batch(&target, batch, safe)?,
                None => batch,
            };
        match existing {
            Some(stored) => Ok(crate::field::cast::cast_record_batch(stored, batch, true)?),
            None => Ok(batch),
        }
    }

    /// Cast a whole reader as [`cast_arrow_batch`](Self::cast_arrow_batch)
    /// casts one batch, streaming - nothing is collected, each batch is cast
    /// as it is pulled, and a layer whose target already matches costs
    /// nothing at all.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned or a selected name is
    /// not a column of the reader.
    fn cast_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
        existing: Option<&Field>,
    ) -> Result<crate::arrow::BatchReader> {
        let safe = self.safe();
        let reader = match self.schema() {
            Some(declared) => crate::arrow::cast_reader(reader, declared, safe)?,
            None => reader,
        };
        let root =
            crate::arrow::record_schema_from_arrow(self.root_name(), reader.schema().as_ref())?;
        let reader =
            match crate::arrow::selected_root(&root, self.select_by_names(), self.root_name())? {
                Some(target) => crate::arrow::cast_reader(reader, &target, safe)?,
                None => reader,
            };
        match existing {
            Some(stored) => Ok(crate::arrow::cast_reader(reader, stored, true)?),
            None => Ok(reader),
        }
    }

    /// Bound a reader by [`max_row_size`](Self::max_row_size) and
    /// [`max_byte_size`](Self::max_byte_size).
    ///
    /// This is one more transform of the same option-driven shaping seam as
    /// [`cast_arrow_reader`](Self::cast_arrow_reader), applied *last*: the
    /// order is declared schema, then selection, then completion cast, then
    /// partition filter, then the limit, so the limit counts result rows and
    /// never rows an earlier layer dropped or reshaped. No media implements a
    /// limit and none checks one - the record methods wrap the shaped reader
    /// here, exactly once per call.
    ///
    /// The wrapper holds at most one batch and stops pulling the moment it is
    /// satisfied, so the rest of the source is never decoded. With neither
    /// bound set the reader is returned as it stands.
    ///
    /// # Errors
    ///
    /// Returns an error naming both settings when a limit is combined with a
    /// non-empty [`merge_by_names`](Self::merge_by_names): a truncated merge
    /// would update the matched keys it kept and silently drop the rest,
    /// which corrupts the resource rather than shortening the write.
    fn limit_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
    ) -> Result<crate::arrow::BatchReader> {
        let max_rows = self.max_row_size();
        let max_bytes = self.max_byte_size();
        if max_rows.is_none() && max_bytes.is_none() {
            return Ok(reader);
        }
        if !self.merge_by_names().is_empty() {
            let mut limits = String::new();
            if let Some(rows) = max_rows {
                limits.push_str(&format!("max_row_size = {rows}"));
            }
            if let Some(bytes) = max_bytes {
                if !limits.is_empty() {
                    limits.push_str(" and ");
                }
                limits.push_str(&format!("max_byte_size = {bytes}"));
            }
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    "max_row_size and max_byte_size without merge_by_names - a truncated merge \
                     updates the matched keys it kept and silently drops the rest, corrupting \
                     rather than shortening",
                    format!("{limits} with merge_by_names {:?}", self.merge_by_names()),
                ),
            });
        }
        use arrow_array::RecordBatchReader as _;
        let schema = reader.schema();
        Ok(Box::new(Limited {
            inner: reader,
            schema,
            remaining_rows: max_rows,
            remaining_bytes: max_bytes,
            yielded: false,
            // `Some(0)` on either bound admits nothing: the reader still
            // reports its shaped schema, and the first `next` is `None`
            // without the source ever being touched.
            satisfied: max_rows == Some(0) || max_bytes == Some(0),
        }))
    }
}

/// A reader bounding the rows and Arrow bytes that flow through it.
///
/// One batch is held at a time - pulled, cut if a bound lands inside it,
/// handed on - and the moment the bounds are met the source is never pulled
/// again, because a limit exists precisely so the rest of a file is not
/// decoded.
struct Limited {
    inner: crate::arrow::BatchReader,
    schema: arrow_schema::SchemaRef,
    /// Rows the row bound still admits; `None` when no row bound is set.
    remaining_rows: Option<u64>,
    /// Bytes the byte bound still admits; `None` when no byte bound is set.
    remaining_bytes: Option<u64>,
    /// Whether any row went out yet, for the at-least-one-row rule.
    yielded: bool,
    /// Set the moment the bounds are met, so `next` never touches `inner`
    /// again.
    satisfied: bool,
}

impl Iterator for Limited {
    type Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.satisfied {
            return None;
        }
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => {
                // Fused after the first error, per the listing contract.
                self.satisfied = true;
                return Some(Err(error));
            }
        };
        let rows = batch.num_rows() as u64;
        if rows == 0 {
            // An empty batch carries no rows to count and no row bytes to
            // charge, and swallowing it would end the stream early.
            return Some(Ok(batch));
        }
        // How many leading rows each bound admits; the smaller answer wins.
        let mut take = rows;
        if let Some(remaining) = self.remaining_rows {
            take = take.min(remaining);
        }
        // Priced only when a byte bound exists, because sizing a batch walks
        // every one of its arrays.
        let mut size = 0_u64;
        if let Some(remaining) = self.remaining_bytes {
            size = u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX);
            if size > remaining {
                // The whole batch does not fit, so the rows that keep the
                // running total at or under the limit are prorated from the
                // batch's own size - Arrow cannot price one row alone.
                let fits = u64::try_from(
                    u128::from(remaining) * u128::from(rows) / u128::from(size.max(1)),
                )
                .unwrap_or(u64::MAX);
                // A non-zero byte limit always yields at least one row: a
                // first row wider than the whole budget would otherwise turn
                // a bounded read into a silent total loss.
                let fits = if fits == 0 && !self.yielded { 1 } else { fits };
                take = take.min(fits);
            }
        }
        if take == 0 {
            self.satisfied = true;
            return None;
        }
        if take < rows {
            // A bound landed inside this batch, so nothing after it can fit:
            // the cut is a `slice`, a view over the same buffers, never a
            // copy.
            self.satisfied = true;
            self.yielded = true;
            let take = usize::try_from(take).unwrap_or(usize::MAX);
            return Some(Ok(batch.slice(0, take)));
        }
        if let Some(remaining) = &mut self.remaining_rows {
            *remaining -= rows;
            if *remaining == 0 {
                self.satisfied = true;
            }
        }
        if let Some(remaining) = &mut self.remaining_bytes {
            *remaining = remaining.saturating_sub(size);
            if *remaining == 0 {
                self.satisfied = true;
            }
        }
        self.yielded = true;
        Some(Ok(batch))
    }
}

impl arrow_array::RecordBatchReader for Limited {
    fn schema(&self) -> arrow_schema::SchemaRef {
        std::sync::Arc::clone(&self.schema)
    }
}

/// Implement [`IORecordOptions`] over one struct's own fields.
///
/// Every encoding stores the same shared settings under the same names, so the
/// accessors are mechanical; what differs is the fields an encoding adds.
#[macro_export]
macro_rules! record_options_fields {
    () => {
        fn schema(&self) -> Option<&$crate::Field> {
            self.schema.as_ref()
        }

        fn set_schema(&mut self, schema: $crate::Field) {
            self.schema = Some(schema);
        }

        fn root_name(&self) -> &str {
            self.root_name.as_str()
        }

        fn set_root_name(&mut self, root_name: smol_str::SmolStr) {
            self.root_name = root_name;
        }

        fn safe(&self) -> bool {
            self.safe
        }

        fn set_safe(&mut self, safe: bool) {
            self.safe = safe;
        }

        fn batch_size(&self) -> Option<usize> {
            self.batch_size
        }

        fn set_batch_size(&mut self, batch_size: Option<usize>) {
            self.batch_size = batch_size;
        }

        fn max_row_size(&self) -> Option<u64> {
            self.max_row_size
        }

        fn set_max_row_size(&mut self, max_row_size: Option<u64>) {
            self.max_row_size = max_row_size;
        }

        fn max_byte_size(&self) -> Option<u64> {
            self.max_byte_size
        }

        fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
            self.max_byte_size = max_byte_size;
        }

        fn level(&self) -> $crate::Level {
            self.level
        }

        fn set_level(&mut self, level: $crate::Level) {
            self.level = level;
        }

        fn merge_by_names(&self) -> &[String] {
            &self.merge_by_names
        }

        fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
            self.merge_by_names = merge_by_names;
        }

        fn select_by_names(&self) -> &[String] {
            &self.select_by_names
        }

        fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
            self.select_by_names = select_by_names;
        }

        fn filter_partitions(&self) -> &[(String, String)] {
            &self.filter_partitions
        }

        fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
            self.filter_partitions = filter_partitions;
        }
    };
}

/// One value naming every record encoding's options.
///
/// The variant *is* the encoding: a record call takes `RecordOptions` and
/// needs no separate format argument.
#[derive(Clone, Debug)]
pub enum RecordOptions {
    /// Arrow IPC stream options.
    Ipc(IpcOptions),
    /// Apache Parquet file options.
    #[cfg(feature = "parquet")]
    Parquet(crate::parquet::ParquetOptions),
    /// Apache Avro container options.
    Avro(crate::avro::AvroOptions),
}

impl RecordOptions {
    /// Derive the options for the encoding a media type names.
    ///
    /// Content codings are ignored here: they are the handle's business, not
    /// the record encoding's.
    ///
    /// # Errors
    ///
    /// Returns an error when no encoding in this build covers `media_type`,
    /// naming what was found.
    pub fn for_media_type(media_type: &MediaType) -> Result<Self> {
        Self::for_mime_type(media_type.base())
    }

    /// Derive the options for the encoding a MIME type names.
    ///
    /// # Errors
    ///
    /// Returns an error when no encoding in this build covers `base`.
    pub fn for_mime_type(base: &MimeType) -> Result<Self> {
        if base == &MimeType::ARROW_STREAM || base == &MimeType::ARROW_FILE {
            return Ok(Self::Ipc(IpcOptions::new()));
        }
        #[cfg(feature = "parquet")]
        if base == &MimeType::PARQUET {
            return Ok(Self::Parquet(crate::parquet::ParquetOptions::new()));
        }
        if base == &MimeType::AVRO {
            return Ok(Self::Avro(crate::avro::AvroOptions::new()));
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                if cfg!(feature = "parquet") {
                    "a record encoding this build implements (application/vnd.apache.arrow.stream, application/vnd.apache.parquet, application/avro)"
                } else {
                    "a record encoding this build implements (application/vnd.apache.arrow.stream, application/avro; the `parquet` feature is not enabled)"
                },
                base,
            ),
        })
    }

    /// Return the MIME type of the encoding these options describe.
    pub const fn mime_type(&self) -> MimeType {
        match self {
            Self::Ipc(_) => MimeType::ARROW_STREAM,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => MimeType::PARQUET,
            Self::Avro(_) => MimeType::AVRO,
        }
    }
}

impl IORecordOptions for RecordOptions {
    fn schema(&self) -> Option<&Field> {
        match self {
            Self::Ipc(options) => options.schema(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.schema(),
            Self::Avro(options) => options.schema(),
        }
    }

    fn set_schema(&mut self, schema: Field) {
        match self {
            Self::Ipc(options) => options.set_schema(schema),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_schema(schema),
            Self::Avro(options) => options.set_schema(schema),
        }
    }

    fn root_name(&self) -> &str {
        match self {
            Self::Ipc(options) => options.root_name(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.root_name(),
            Self::Avro(options) => options.root_name(),
        }
    }

    fn set_root_name(&mut self, root_name: SmolStr) {
        match self {
            Self::Ipc(options) => options.set_root_name(root_name),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_root_name(root_name),
            Self::Avro(options) => options.set_root_name(root_name),
        }
    }

    fn safe(&self) -> bool {
        match self {
            Self::Ipc(options) => options.safe(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.safe(),
            Self::Avro(options) => options.safe(),
        }
    }

    fn set_safe(&mut self, safe: bool) {
        match self {
            Self::Ipc(options) => options.set_safe(safe),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_safe(safe),
            Self::Avro(options) => options.set_safe(safe),
        }
    }

    fn batch_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.batch_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.batch_size(),
            Self::Avro(options) => options.batch_size(),
        }
    }

    fn set_batch_size(&mut self, batch_size: Option<usize>) {
        match self {
            Self::Ipc(options) => options.set_batch_size(batch_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_batch_size(batch_size),
            Self::Avro(options) => options.set_batch_size(batch_size),
        }
    }

    fn max_row_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.max_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.max_row_size(),
            Self::Avro(options) => options.max_row_size(),
        }
    }

    fn set_max_row_size(&mut self, max_row_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_max_row_size(max_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_max_row_size(max_row_size),
            Self::Avro(options) => options.set_max_row_size(max_row_size),
        }
    }

    fn max_byte_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.max_byte_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.max_byte_size(),
            Self::Avro(options) => options.max_byte_size(),
        }
    }

    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_max_byte_size(max_byte_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_max_byte_size(max_byte_size),
            Self::Avro(options) => options.set_max_byte_size(max_byte_size),
        }
    }

    fn level(&self) -> Level {
        match self {
            Self::Ipc(options) => options.level(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.level(),
            Self::Avro(options) => options.level(),
        }
    }

    fn set_level(&mut self, level: Level) {
        match self {
            Self::Ipc(options) => options.set_level(level),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_level(level),
            Self::Avro(options) => options.set_level(level),
        }
    }

    fn merge_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.merge_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.merge_by_names(),
            Self::Avro(options) => options.merge_by_names(),
        }
    }

    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_merge_by_names(merge_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_merge_by_names(merge_by_names),
            Self::Avro(options) => options.set_merge_by_names(merge_by_names),
        }
    }

    fn select_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.select_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.select_by_names(),
            Self::Avro(options) => options.select_by_names(),
        }
    }

    fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_select_by_names(select_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_select_by_names(select_by_names),
            Self::Avro(options) => options.set_select_by_names(select_by_names),
        }
    }

    fn filter_partitions(&self) -> &[(String, String)] {
        match self {
            Self::Ipc(options) => options.filter_partitions(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.filter_partitions(),
            Self::Avro(options) => options.filter_partitions(),
        }
    }

    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
        match self {
            Self::Ipc(options) => options.set_filter_partitions(filter_partitions),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_filter_partitions(filter_partitions),
            Self::Avro(options) => options.set_filter_partitions(filter_partitions),
        }
    }
}

impl From<IpcOptions> for RecordOptions {
    fn from(value: IpcOptions) -> Self {
        Self::Ipc(value)
    }
}

#[cfg(feature = "parquet")]
impl From<crate::parquet::ParquetOptions> for RecordOptions {
    fn from(value: crate::parquet::ParquetOptions) -> Self {
        Self::Parquet(value)
    }
}

impl From<crate::avro::AvroOptions> for RecordOptions {
    fn from(value: crate::avro::AvroOptions) -> Self {
        Self::Avro(value)
    }
}

#[cfg(test)]
mod tests;
