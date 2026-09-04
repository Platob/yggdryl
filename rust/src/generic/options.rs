//! The settings a record read or write takes, shared across encodings.
//!
//! Reading rows out of an Arrow IPC stream and reading them out of a Parquet
//! file need the same handful of answers - what to call the root, what
//! datatype and metadata it declares, how strict a cast may be, how many rows
//! per batch, how much may flow at all, how hard to compress, and what makes
//! two rows the same row - and each encoding then adds its own.
//! [`IORecordOptions`] is that shared surface, each encoding stores those
//! settings as its own flat fields, and [`RecordOptions`] is the enum naming
//! every encoding's options.
//!
//! The declared root is three parts - `name`, `dtype`, `metadata` - and
//! [`field`](IORecordOptions::field) builds the non-null Struct [`Field`] from
//! them on every ask, so one part changes without the others: a datatype
//! swapped under the same name, a name given to a datatype, metadata added to
//! either. Only the datatype is optional: without one nothing is declared and
//! the shape is inferred, while the name and metadata always have a value -
//! [`DEFAULT_ROOT_NAME`](crate::generic::DEFAULT_ROOT_NAME) and no entries -
//! so one declaration has one spelling.
//!
//! An encoding is never guessed: [`RecordOptions::for_media_type`] derives it
//! from the handle's media type, which is what [`crate::IOBase`]'s record
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
//!     .with_field(schema.clone())
//!     .with_batch_row_size(1024)
//!     .with_commit_row_size(10_000);
//!
//! assert_eq!(options.field(), Some(schema.clone()));
//! assert_eq!(options.name(), "row");
//! assert_eq!(options.dtype(), Some(schema.dtype()));
//! assert!(options.metadata().is_empty());
//! assert_eq!(options.batch_row_size(), Some(1024));
//! assert_eq!(options.commit_row_size(), Some(10_000));
//!
//! // Each part mutates on its own: the same datatype under another root name.
//! let renamed = options.with_name("trade").require_field()?;
//! assert_eq!(renamed.name(), "trade");
//! assert_eq!(renamed.dtype(), schema.dtype());
//! # Ok(())
//! # }
//! ```

use smol_str::SmolStr;

use crate::Level;
use crate::ipc::IpcOptions;
use crate::{DataType, Error, Field, IOMode, MediaType, Metadata, MimeType, Result};

/// Default rows materialized in one native-record conversion batch.
///
/// Runtime bindings use this same value when their host-language rows must be
/// widened into Arrow before entering the core reader surface.
pub const DEFAULT_RECORD_BATCH_ROW_SIZE: usize = 65_536;

/// The read and write settings shared by every record encoding.
///
/// Each encoding stores these as its own fields - there is no shared settings
/// struct to thread through - and the builders here are what every caller uses,
/// so the encodings cannot drift apart in what a shared setting means.
pub trait IORecordOptions: Sized {
    /// Borrow the root Field name.
    ///
    /// The name is one of the three parts [`field`](Self::field) is built
    /// from, and it names an inferred root as well, so a stream read without
    /// a schema and one read under a declared datatype answer the same root
    /// name. It defaults to
    /// [`DEFAULT_ROOT_NAME`](crate::generic::DEFAULT_ROOT_NAME).
    fn name(&self) -> &str;

    /// Set the root Field name.
    fn set_name(&mut self, name: SmolStr);

    /// Borrow the declared root datatype, if any.
    ///
    /// A declared datatype is what makes a field declared at all: reads
    /// project onto it and writes cast onto it, while `None` infers the shape
    /// from the encoding or the incoming rows.
    fn dtype(&self) -> Option<&DataType>;

    /// Declare or clear the root datatype.
    fn set_dtype(&mut self, dtype: Option<DataType>);

    /// Borrow the root metadata; empty unless declared.
    ///
    /// Metadata reaches a read or write only through the field a declared
    /// datatype builds: on its own it declares nothing.
    fn metadata(&self) -> &Metadata;

    /// Set the root metadata; an empty snapshot clears it.
    fn set_metadata(&mut self, metadata: Metadata);

    /// Build the declared canonical field from its parts, if a datatype is
    /// declared.
    ///
    /// The field is built on every ask - the non-null Struct root that
    /// [`name`](Self::name), [`dtype`](Self::dtype), and
    /// [`metadata`](Self::metadata) spell - so it is never stale against a
    /// part changed after it. The parts are shared handles, so the build
    /// clones no datatype tree and no metadata map; a caller casting many
    /// batches builds it once and keeps it.
    fn field(&self) -> Option<Field> {
        let dtype = self.dtype()?.clone();
        Some(Field::new_with_metadata(
            self.name(),
            dtype,
            false,
            self.metadata().clone(),
        ))
    }

    /// Declare the canonical field, part by part.
    ///
    /// The field's name, datatype, and metadata become the three parts. Its
    /// nullability and dictionary options are not part of a declaration and
    /// are dropped: a row root is a non-null Struct, which is what the build
    /// answers.
    fn set_field(&mut self, field: Field) {
        self.set_name(SmolStr::new(field.name()));
        self.set_dtype(Some(field.dtype().clone()));
        self.set_metadata(field.as_metadata().clone());
    }

    /// Remove and return the declared canonical field, if any.
    ///
    /// The datatype and metadata are cleared either way; the name stays,
    /// because it still names the root a delegated write infers. Write
    /// combinators use this after casting an incoming stream: the delegated
    /// overwrite then receives rows already in the declared shape and cannot
    /// cast them a second time.
    fn take_field(&mut self) -> Option<Field> {
        let field = self.field();
        self.set_dtype(None);
        self.set_metadata(Metadata::new());
        field
    }

    /// Return whether a cast may null a value it cannot convert.
    fn safe(&self) -> bool;

    /// Set whether a cast may null a value it cannot convert.
    fn set_safe(&mut self, safe: bool);

    /// Return the row-per-batch bound, if any.
    fn batch_row_size(&self) -> Option<usize>;

    /// Set the row-per-batch bound.
    fn set_batch_row_size(&mut self, batch_row_size: Option<usize>);

    /// Return the row materialization bound for a native-record write.
    ///
    /// Row conversion must never run past the next publication boundary: a
    /// conversion error at row `N + 1` must not erase the complete `N`-row
    /// prefix waiting to commit. The smaller of `batch_row_size` and
    /// `commit_row_size` is therefore the writer's batch size; either setting
    /// alone supplies the bound.
    fn write_batch_row_size(&self) -> Option<usize> {
        match self.commit_row_size() {
            Some(commit) => Some(
                self.batch_row_size()
                    .unwrap_or(DEFAULT_RECORD_BATCH_ROW_SIZE)
                    .max(1)
                    .min(commit),
            ),
            None => self.batch_row_size(),
        }
    }

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

    /// Return the publication cadence for a streamed write, in rows.
    ///
    /// `None` publishes once after the source ends. `Some(N)` publishes every
    /// complete group of `N` incoming rows and then the final remainder. Zero
    /// is not a cadence and is rejected before a write pulls its source.
    fn commit_row_size(&self) -> Option<usize>;

    /// Set the publication cadence for a streamed write.
    fn set_commit_row_size(&mut self, commit_row_size: Option<usize>);

    /// Return the compression level applied to a declared content coding.
    fn level(&self) -> Level;

    /// Set the compression level.
    fn set_level(&mut self, level: Level);

    /// Borrow the column names whose values form an explicit merge's match key.
    ///
    /// A non-empty list is required by
    /// [`merge_arrow_reader`](crate::IOMedia::merge_arrow_reader): a row
    /// whose key is already stored updates it, and a row whose key is not
    /// appends. The option never selects an operation; overwrite and append
    /// reject it, and merge rejects an empty list.
    fn merge_by_names(&self) -> &[String];

    /// Set the column names whose values form an explicit merge's match key.
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

    /// Build the declared field, or say that one is required.
    ///
    /// # Errors
    ///
    /// Returns an error naming the builders that declare a field.
    fn require_field(&self) -> Result<Field> {
        self.field().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static(
                "expected a declared field to write records; call with_field or with_dtype first",
            ),
        })
    }

    /// Return these options with a declared canonical field.
    #[must_use]
    fn with_field(mut self, field: Field) -> Self {
        self.set_field(field);
        self
    }

    /// Return these options with a different root Field name.
    #[must_use]
    fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.set_name(name.into());
        self
    }

    /// Return these options with a declared root datatype.
    #[must_use]
    fn with_dtype(mut self, dtype: DataType) -> Self {
        self.set_dtype(Some(dtype));
        self
    }

    /// Return these options with root metadata.
    #[must_use]
    fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.set_metadata(metadata);
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
    fn with_batch_row_size(mut self, batch_row_size: usize) -> Self {
        self.set_batch_row_size(Some(batch_row_size));
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

    /// Return these options with a publication every `commit_row_size` rows.
    ///
    /// A zero value is retained so the write can return a typed error before
    /// touching a one-shot input. Use `None` through
    /// [`set_commit_row_size`](Self::set_commit_row_size) for one publication
    /// at the end.
    #[must_use]
    fn with_commit_row_size(mut self, commit_row_size: usize) -> Self {
        self.set_commit_row_size(Some(commit_row_size));
        self
    }

    /// Return these options with a different compression level.
    #[must_use]
    fn with_level(mut self, level: Level) -> Self {
        self.set_level(level);
        self
    }

    /// Return these options with a match key for an explicit merge.
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
    /// applied in order: the declared [`field`](Self::field) says what the
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
        let batch = match self.field() {
            Some(declared) => crate::field::cast::cast_record_batch(&declared, batch, safe)?,
            None => batch,
        };
        let root = crate::arrow::field_from_arrow_schema(self.name(), batch.schema().as_ref())?;
        let batch = match crate::arrow::selected_root(&root, self.select_by_names(), self.name())? {
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
        let reader = match self.field() {
            Some(declared) => crate::arrow::cast_reader(reader, &declared, safe)?,
            None => reader,
        };
        let root = crate::arrow::field_from_arrow_schema(self.name(), reader.schema().as_ref())?;
        let reader = match crate::arrow::selected_root(&root, self.select_by_names(), self.name())?
        {
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
    /// limit - the record methods wrap the shaped reader here, exactly once
    /// per call. A media may read a row bound as a fetch plan (Parquet
    /// fetches only the leading row groups that cover it), but the trim to
    /// the exact count happens only here.
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
        self.require_write_limits()?;
        use arrow_array::RecordBatchReader as _;
        let schema = reader.schema();
        Ok(Box::new(Limited {
            inner: reader,
            schema,
            state: WriteLimitState::new(max_rows, max_bytes),
        }))
    }

    /// Validate deterministic write-limit combinations without an input.
    ///
    /// This preflight runs before append/merge peek at a one-shot reader. A
    /// truncated keyed merge is always invalid, independently of the rows it
    /// would receive, so its error must not consume one merely to discover the
    /// same configuration failure.
    fn require_write_limits(&self) -> Result<()> {
        let max_rows = self.max_row_size();
        let max_bytes = self.max_byte_size();
        if max_rows.is_none() && max_bytes.is_none() {
            return Ok(());
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
        Ok(())
    }

    /// Return whether a zero write bound admits no incoming row.
    fn write_limit_is_zero(&self) -> bool {
        self.max_row_size() == Some(0) || self.max_byte_size() == Some(0)
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
    state: WriteLimitState,
}

/// Stateful write limits shared by a pull reader and an incremental runtime
/// bridge.
///
/// The JavaScript binding has to leave the Rust stack while it awaits the next
/// asynchronous record chunk. Keeping these counters outside `Limited` lets
/// that bridge resume the exact same logical limit: byte accounting and the
/// one global "at least one row" decision never restart at a chunk boundary.
#[derive(Clone, Debug)]
pub(crate) struct WriteLimitState {
    /// Rows the row bound still admits; `None` when no row bound is set.
    remaining_rows: Option<u64>,
    /// Bytes the byte bound still admits; `None` when no byte bound is set.
    remaining_bytes: Option<u64>,
    /// Whether any row went out yet, for the at-least-one-row rule.
    yielded: bool,
    /// Set the moment the bounds are met, so no caller touches its source
    /// again.
    satisfied: bool,
}

impl WriteLimitState {
    pub(crate) const fn new(remaining_rows: Option<u64>, remaining_bytes: Option<u64>) -> Self {
        Self {
            remaining_rows,
            remaining_bytes,
            yielded: false,
            // `Some(0)` on either bound admits nothing: a reader still reports
            // its shaped schema, and no input batch is pulled.
            satisfied: matches!(remaining_rows, Some(0)) || matches!(remaining_bytes, Some(0)),
        }
    }

    pub(crate) const fn satisfied(&self) -> bool {
        self.satisfied
    }

    /// Admit the leading rows one logical write limit still allows.
    ///
    /// `None` means the limit is complete. A zero-row batch passes through as
    /// `Some`, because it carries schema continuity without spending either
    /// budget.
    pub(crate) fn apply(
        &mut self,
        batch: arrow_array::RecordBatch,
    ) -> Option<arrow_array::RecordBatch> {
        if self.satisfied {
            return None;
        }
        let rows = batch.num_rows() as u64;
        if rows == 0 {
            return Some(batch);
        }
        let mut take = rows;
        if let Some(remaining) = self.remaining_rows {
            take = take.min(remaining);
        }
        let mut size = 0_u64;
        if let Some(remaining) = self.remaining_bytes {
            size = u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX);
            if size > remaining {
                let fits = u64::try_from(
                    u128::from(remaining) * u128::from(rows) / u128::from(size.max(1)),
                )
                .unwrap_or(u64::MAX);
                let fits = if fits == 0 && !self.yielded { 1 } else { fits };
                take = take.min(fits);
            }
        }
        if take == 0 {
            self.satisfied = true;
            return None;
        }
        if take < rows {
            self.satisfied = true;
            self.yielded = true;
            let take = usize::try_from(take).unwrap_or(usize::MAX);
            return Some(batch.slice(0, take));
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
        Some(batch)
    }
}

impl Iterator for Limited {
    type Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state.satisfied() {
            return None;
        }
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => {
                // Fused after the first error, per the listing contract.
                self.state.satisfied = true;
                return Some(Err(error));
            }
        };
        self.state.apply(batch).map(Ok)
    }
}

impl arrow_array::RecordBatchReader for Limited {
    fn schema(&self) -> arrow_schema::SchemaRef {
        std::sync::Arc::clone(&self.schema)
    }
}

/// The one streaming splitter for row-count publication boundaries.
///
/// A bounded instance owns at most one complete cadence plus the unconsumed
/// remainder of the current input batch. Batch slices are Arrow views over the
/// same buffers. It never pulls a following batch after the current cadence is
/// full, which is what makes a successful prefix observable before a later
/// source failure. An unbounded instance yields the original reader once.
pub(crate) struct CommitReaders {
    schema: arrow_schema::SchemaRef,
    batches: Option<crate::arrow::BatchReader>,
    commit_row_size: Option<usize>,
    buffer: Option<CommitBuffer>,
    done: bool,
}

/// One owned, bounded publication window.
///
/// Both the ordinary pull reader and a runtime binding that pushes batches
/// between awaits use this object. It owns only the current cadence; slices are
/// Arrow views over their source buffers.
pub(crate) struct CommitBuffer {
    schema: arrow_schema::SchemaRef,
    row_size: usize,
    rows: usize,
    batches: Vec<arrow_array::RecordBatch>,
    /// The one input batch whose leading rows completed the last cadence.
    /// Its remaining rows are not sliced until the caller asks for the next
    /// cadence, after it has had a chance to publish the one just returned.
    current: Option<(arrow_array::RecordBatch, usize)>,
}

impl CommitBuffer {
    pub(crate) fn new(schema: arrow_schema::SchemaRef, row_size: usize) -> Self {
        debug_assert!(row_size > 0);
        Self {
            schema,
            row_size,
            rows: 0,
            batches: Vec::new(),
            current: None,
        }
    }

    /// Add one batch and return at most the first cadence it completes.
    ///
    /// A remainder stays in `current`; callers must publish the returned
    /// reader before asking [`next_ready`](Self::next_ready) to advance it.
    pub(crate) fn push(
        &mut self,
        batch: arrow_array::RecordBatch,
    ) -> Option<crate::arrow::BatchReader> {
        debug_assert!(self.current.is_none());
        self.current = Some((batch, 0));
        self.next_ready()
    }

    /// Advance only the retained input batch, yielding at most one cadence.
    pub(crate) fn next_ready(&mut self) -> Option<crate::arrow::BatchReader> {
        let (batch, offset) = self.current.take()?;
        let available = batch.num_rows().saturating_sub(offset);
        if available == 0 {
            return None;
        }
        let take = (self.row_size - self.rows).min(available);
        if offset == 0 && take == batch.num_rows() {
            self.batches.push(batch);
        } else {
            self.batches.push(batch.slice(offset, take));
            if take < available {
                self.current = Some((batch, offset + take));
            }
        }
        self.rows += take;
        if self.rows != self.row_size {
            return None;
        }
        self.rows = 0;
        Some(crate::arrow::batch_reader(
            std::sync::Arc::clone(&self.schema),
            std::mem::take(&mut self.batches),
        ))
    }

    /// Take the successful final remainder, if this window holds one.
    pub(crate) fn finish(&mut self) -> Option<crate::arrow::BatchReader> {
        if self.rows == 0 {
            return None;
        }
        self.rows = 0;
        Some(crate::arrow::batch_reader(
            std::sync::Arc::clone(&self.schema),
            std::mem::take(&mut self.batches),
        ))
    }

    /// Discard an incomplete cadence after a source or conversion failure.
    pub(crate) fn clear(&mut self) {
        self.rows = 0;
        self.batches.clear();
        self.current = None;
    }
}

impl Iterator for CommitReaders {
    type Item = Result<crate::arrow::BatchReader>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let Some(commit_row_size) = self.commit_row_size else {
            self.done = true;
            return self.batches.take().map(Ok);
        };
        loop {
            if let Some(reader) = self.buffer.as_mut().and_then(CommitBuffer::next_ready) {
                return Some(Ok(reader));
            }
            let batches = self
                .batches
                .as_mut()
                .expect("a live commit splitter owns its source");
            match batches.next() {
                Some(Ok(batch)) => {
                    if batch.num_rows() == 0 {
                        continue;
                    }
                    let buffer = self.buffer.get_or_insert_with(|| {
                        CommitBuffer::new(std::sync::Arc::clone(&self.schema), commit_row_size)
                    });
                    if let Some(reader) = buffer.push(batch) {
                        return Some(Ok(reader));
                    }
                }
                Some(Err(error)) => {
                    // The partial cadence has not been published. Previous
                    // complete cadences remain visible by design.
                    self.done = true;
                    if let Some(buffer) = &mut self.buffer {
                        buffer.clear();
                    }
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => {
                    self.done = true;
                    return self.buffer.as_mut().and_then(CommitBuffer::finish).map(Ok);
                }
            }
        }
    }
}

/// Implement [`IORecordOptions`] over one struct's own fields.
///
/// Every encoding stores the same shared settings under the same names, so the
/// accessors are mechanical; what differs is the fields an encoding adds.
#[macro_export]
macro_rules! record_options_fields {
    () => {
        fn name(&self) -> &str {
            self.name.as_str()
        }

        fn set_name(&mut self, name: smol_str::SmolStr) {
            self.name = name;
        }

        fn dtype(&self) -> Option<&$crate::DataType> {
            self.dtype.as_ref()
        }

        fn set_dtype(&mut self, dtype: Option<$crate::DataType>) {
            self.dtype = dtype;
        }

        fn metadata(&self) -> &$crate::Metadata {
            &self.metadata
        }

        fn set_metadata(&mut self, metadata: $crate::Metadata) {
            self.metadata = metadata;
        }

        fn safe(&self) -> bool {
            self.safe
        }

        fn set_safe(&mut self, safe: bool) {
            self.safe = safe;
        }

        fn batch_row_size(&self) -> Option<usize> {
            self.batch_row_size
        }

        fn set_batch_row_size(&mut self, batch_row_size: Option<usize>) {
            self.batch_row_size = batch_row_size;
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

        fn commit_row_size(&self) -> Option<usize> {
            self.commit_row_size
        }

        fn set_commit_row_size(&mut self, commit_row_size: Option<usize>) {
            self.commit_row_size = commit_row_size;
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
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RecordOptions {
    /// Arrow IPC stream options.
    Ipc(IpcOptions),
    /// Apache Parquet file options.
    #[cfg(feature = "parquet")]
    Parquet(crate::parquet::ParquetOptions),
    /// Apache Avro container options.
    Avro(crate::avro::AvroOptions),
    /// Plain-text row options.
    Text(Box<crate::text::TextOptions>),
}

impl RecordOptions {
    /// Return a deterministic hash of the encoding and its complete options.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        match self {
            Self::Ipc(options) => crate::stable_hash_of(&("ipc", options)),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => crate::stable_hash_of(&("parquet", options)),
            Self::Avro(options) => crate::stable_hash_of(&("avro", options)),
            Self::Text(options) => crate::stable_hash_of(&("text", options)),
        }
    }

    fn text_mut(
        &mut self,
        path: &'static str,
        setting: &'static str,
    ) -> Result<&mut crate::text::TextOptions> {
        let media_type = self.mime_type();
        match self {
            Self::Text(options) => Ok(options),
            Self::Ipc(_) | Self::Avro(_) => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected text options to set {setting}, got {media_type} options"
                ),
            }),
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected text options to set {setting}, got {media_type} options"
                ),
            }),
        }
    }

    /// Borrow the text autotyping timezone, or `None` when unset or not text.
    pub const fn timezone(&self) -> Option<&crate::Timezone> {
        match self {
            Self::Text(options) => options.timezone(),
            Self::Ipc(_) | Self::Avro(_) => None,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => None,
        }
    }

    /// Set or clear the text autotyping timezone.
    pub fn set_timezone(&mut self, timezone: Option<crate::Timezone>) -> Result<()> {
        self.text_mut("$.timezone", "an autotyping timezone")?
            .set_timezone(timezone);
        Ok(())
    }

    fn avro_mut(
        &mut self,
        path: &'static str,
        setting: &'static str,
    ) -> Result<&mut crate::avro::AvroOptions> {
        let media_type = self.mime_type();
        match self {
            Self::Avro(options) => Ok(options),
            Self::Ipc(_) | Self::Text(_) => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected Avro options to set {setting}, got {media_type} options"
                ),
            }),
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected Avro options to set {setting}, got {media_type} options"
                ),
            }),
        }
    }

    /// Return the Avro block codec name, or `None` for another encoding.
    pub fn avro_block_codec(&self) -> Option<&str> {
        match self {
            Self::Avro(options) => Some(options.codec.as_str()),
            Self::Ipc(_) | Self::Text(_) => None,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => None,
        }
    }

    /// Validate and set the Avro block codec.
    ///
    /// Validation uses the codec vocabulary the container encoder itself
    /// dispatches through, so a binding can reject a bad name before it pulls
    /// a one-shot record source.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-Avro variant or a codec this build does not
    /// implement.
    pub fn set_avro_block_codec(&mut self, codec: &str) -> Result<()> {
        let options = self.avro_mut("$.block_codec", "a block codec")?;
        crate::avro::container::BlockCoding::from_name(codec)?;
        options.codec = SmolStr::new(codec);
        Ok(())
    }

    /// Borrow the optional fixed Avro synchronization marker.
    ///
    /// `None` means either that a fresh marker will be generated for an Avro
    /// write or that these options describe another encoding. A setter remains
    /// encoding-checked, so the two cases cannot be confused while mutating.
    pub const fn avro_sync_marker(&self) -> Option<&[u8; 16]> {
        match self {
            Self::Avro(options) => options.sync_marker.as_ref(),
            Self::Ipc(_) | Self::Text(_) => None,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => None,
        }
    }

    /// Set or clear the fixed Avro synchronization marker.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-Avro variant or a marker whose length is not
    /// exactly sixteen bytes.
    pub fn set_avro_sync_marker(&mut self, marker: Option<&[u8]>) -> Result<()> {
        let options = self.avro_mut("$.sync_marker", "a synchronization marker")?;
        let marker = marker
            .map(|marker| {
                marker.try_into().map_err(|_| Error::InvalidRecord {
                    path: SmolStr::new_static("$.sync_marker"),
                    reason: smol_str::format_smolstr!(
                        "expected exactly 16 bytes, got {}",
                        marker.len()
                    ),
                })
            })
            .transpose()?;
        options.sync_marker = marker;
        Ok(())
    }

    #[cfg(feature = "parquet")]
    fn parquet_mut(
        &mut self,
        path: &'static str,
        setting: &'static str,
    ) -> Result<&mut crate::parquet::ParquetOptions> {
        let media_type = self.mime_type();
        match self {
            Self::Parquet(options) => Ok(options),
            Self::Ipc(_) | Self::Avro(_) | Self::Text(_) => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected Parquet options to set {setting}, got {media_type} options"
                ),
            }),
        }
    }

    /// Return the Parquet page-compression name, or `None` for another encoding.
    #[cfg(feature = "parquet")]
    pub fn parquet_compression_name(&self) -> Option<String> {
        match self {
            Self::Parquet(options) => Some(options.compression_name()),
            Self::Ipc(_) | Self::Avro(_) | Self::Text(_) => None,
        }
    }

    /// Parse and set the Parquet page compression.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-Parquet variant or an invalid compression.
    #[cfg(feature = "parquet")]
    pub fn set_parquet_compression_name(&mut self, compression: &str) -> Result<()> {
        self.parquet_mut("$.compression", "a page compression")?
            .set_compression_name(compression)
    }

    /// Return the Parquet row-group bound, or `None` for another encoding.
    #[cfg(feature = "parquet")]
    pub const fn parquet_max_row_group_size(&self) -> Option<usize> {
        match self {
            Self::Parquet(options) => Some(options.max_row_group_size),
            Self::Ipc(_) | Self::Avro(_) | Self::Text(_) => None,
        }
    }

    /// Set the maximum rows in one Parquet row group.
    ///
    /// # Errors
    ///
    /// Returns an error when these are not Parquet options.
    #[cfg(feature = "parquet")]
    pub fn set_parquet_max_row_group_size(&mut self, rows: usize) -> Result<()> {
        self.parquet_mut("$.max_row_group_size", "a row-group size")?
            .set_max_row_group_size(rows);
        Ok(())
    }

    /// Borrow Parquet footer metadata, or return `None` for another encoding.
    #[cfg(feature = "parquet")]
    pub fn parquet_key_value_metadata(&self) -> Option<&[(String, String)]> {
        match self {
            Self::Parquet(options) => Some(&options.key_value_metadata),
            Self::Ipc(_) | Self::Avro(_) | Self::Text(_) => None,
        }
    }

    /// Replace Parquet footer metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when these are not Parquet options.
    #[cfg(feature = "parquet")]
    pub fn set_parquet_key_value_metadata(
        &mut self,
        metadata: Vec<(String, String)>,
    ) -> Result<()> {
        self.parquet_mut("$.key_value_metadata", "footer metadata")?
            .set_key_value_metadata(metadata);
        Ok(())
    }

    /// Add one Parquet footer metadata entry.
    ///
    /// # Errors
    ///
    /// Returns an error when these are not Parquet options.
    #[cfg(feature = "parquet")]
    pub fn push_parquet_key_value(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<()> {
        self.parquet_mut("$.key_value_metadata", "footer metadata")?
            .push_key_value(key, value);
        Ok(())
    }

    /// Validate one explicit write mode before a runtime binding consumes input.
    ///
    /// The mode is authoritative. Match keys refine `merge` and never select
    /// it implicitly: merge requires at least one key, while overwrite and
    /// append refuse every key.
    #[doc(hidden)]
    pub fn require_write_mode(&self, mode: IOMode) -> Result<()> {
        let keyed = mode == IOMode::Merge;
        let keys = self.merge_by_names();
        if keyed != keys.is_empty() {
            return Ok(());
        }
        let reason = if keyed {
            format!("write mode {mode} requires at least one merge_by_names column")
        } else {
            format!(
                "write mode {mode} does not accept merge_by_names; use merge mode for keyed writes"
            )
        };
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.merge_by_names"),
            reason: SmolStr::new(reason),
        })
    }

    /// Validate the optional streamed-write publication cadence.
    ///
    /// This is public only for the workspace bindings, which must reject a
    /// zero cadence before converting or pulling a runtime iterator.
    #[doc(hidden)]
    pub fn require_commit_row_size(&self) -> Result<Option<usize>> {
        match self.commit_row_size() {
            Some(0) => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.commit_row_size"),
                reason: SmolStr::new_static(
                    "expected commit_row_size to be a non-zero row count, got 0",
                ),
            }),
            commit_row_size => Ok(commit_row_size),
        }
    }

    /// Split one already-shaped stream into bounded publication readers.
    ///
    /// The caller validates write intent and shapes the stream before entering
    /// here. Every yielded reader contains exactly one complete cadence, or
    /// the final remainder after end-of-stream. A source error discards only
    /// the incomplete cadence it interrupted.
    pub(crate) fn commit_arrow_readers(
        &self,
        batches: crate::arrow::BatchReader,
    ) -> Result<CommitReaders> {
        Ok(CommitReaders {
            schema: batches.schema(),
            batches: Some(batches),
            commit_row_size: self.require_commit_row_size()?,
            buffer: None,
            done: false,
        })
    }

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
        // Plain text reads and writes as lines: the projection is the
        // encoding, so a `.log` answers the record surface out of the box.
        if base == &MimeType::PLAIN_TEXT {
            return Ok(Self::Text(Box::default()));
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                if cfg!(feature = "parquet") {
                    "a record encoding this build implements (application/vnd.apache.arrow.stream, application/vnd.apache.parquet, application/avro, text/plain)"
                } else {
                    "a record encoding this build implements (application/vnd.apache.arrow.stream, application/avro, text/plain; the `parquet` feature is not enabled)"
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
            Self::Text(_) => MimeType::PLAIN_TEXT,
        }
    }
}

impl IORecordOptions for RecordOptions {
    fn name(&self) -> &str {
        match self {
            Self::Ipc(options) => options.name(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.name(),
            Self::Avro(options) => options.name(),
            Self::Text(options) => options.name(),
        }
    }

    fn set_name(&mut self, name: SmolStr) {
        match self {
            Self::Ipc(options) => options.set_name(name),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_name(name),
            Self::Avro(options) => options.set_name(name),
            Self::Text(options) => options.set_name(name),
        }
    }

    fn dtype(&self) -> Option<&DataType> {
        match self {
            Self::Ipc(options) => options.dtype(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.dtype(),
            Self::Avro(options) => options.dtype(),
            Self::Text(options) => options.dtype(),
        }
    }

    fn set_dtype(&mut self, dtype: Option<DataType>) {
        match self {
            Self::Ipc(options) => options.set_dtype(dtype),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_dtype(dtype),
            Self::Avro(options) => options.set_dtype(dtype),
            Self::Text(options) => options.set_dtype(dtype),
        }
    }

    fn metadata(&self) -> &Metadata {
        match self {
            Self::Ipc(options) => options.metadata(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.metadata(),
            Self::Avro(options) => options.metadata(),
            Self::Text(options) => options.metadata(),
        }
    }

    fn set_metadata(&mut self, metadata: Metadata) {
        match self {
            Self::Ipc(options) => options.set_metadata(metadata),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_metadata(metadata),
            Self::Avro(options) => options.set_metadata(metadata),
            Self::Text(options) => options.set_metadata(metadata),
        }
    }

    fn safe(&self) -> bool {
        match self {
            Self::Ipc(options) => options.safe(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.safe(),
            Self::Avro(options) => options.safe(),
            Self::Text(options) => options.safe(),
        }
    }

    fn set_safe(&mut self, safe: bool) {
        match self {
            Self::Ipc(options) => options.set_safe(safe),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_safe(safe),
            Self::Avro(options) => options.set_safe(safe),
            Self::Text(options) => options.set_safe(safe),
        }
    }

    fn batch_row_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.batch_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.batch_row_size(),
            Self::Avro(options) => options.batch_row_size(),
            Self::Text(options) => options.batch_row_size(),
        }
    }

    fn set_batch_row_size(&mut self, batch_row_size: Option<usize>) {
        match self {
            Self::Ipc(options) => options.set_batch_row_size(batch_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_batch_row_size(batch_row_size),
            Self::Avro(options) => options.set_batch_row_size(batch_row_size),
            Self::Text(options) => options.set_batch_row_size(batch_row_size),
        }
    }

    fn max_row_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.max_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.max_row_size(),
            Self::Avro(options) => options.max_row_size(),
            Self::Text(options) => options.max_row_size(),
        }
    }

    fn set_max_row_size(&mut self, max_row_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_max_row_size(max_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_max_row_size(max_row_size),
            Self::Avro(options) => options.set_max_row_size(max_row_size),
            Self::Text(options) => options.set_max_row_size(max_row_size),
        }
    }

    fn max_byte_size(&self) -> Option<u64> {
        match self {
            Self::Ipc(options) => options.max_byte_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.max_byte_size(),
            Self::Avro(options) => options.max_byte_size(),
            Self::Text(options) => options.max_byte_size(),
        }
    }

    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
        match self {
            Self::Ipc(options) => options.set_max_byte_size(max_byte_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_max_byte_size(max_byte_size),
            Self::Avro(options) => options.set_max_byte_size(max_byte_size),
            Self::Text(options) => options.set_max_byte_size(max_byte_size),
        }
    }

    fn commit_row_size(&self) -> Option<usize> {
        match self {
            Self::Ipc(options) => options.commit_row_size(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.commit_row_size(),
            Self::Avro(options) => options.commit_row_size(),
            Self::Text(options) => options.commit_row_size(),
        }
    }

    fn set_commit_row_size(&mut self, commit_row_size: Option<usize>) {
        match self {
            Self::Ipc(options) => options.set_commit_row_size(commit_row_size),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_commit_row_size(commit_row_size),
            Self::Avro(options) => options.set_commit_row_size(commit_row_size),
            Self::Text(options) => options.set_commit_row_size(commit_row_size),
        }
    }

    fn level(&self) -> Level {
        match self {
            Self::Ipc(options) => options.level(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.level(),
            Self::Avro(options) => options.level(),
            Self::Text(options) => options.level(),
        }
    }

    fn set_level(&mut self, level: Level) {
        match self {
            Self::Ipc(options) => options.set_level(level),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_level(level),
            Self::Avro(options) => options.set_level(level),
            Self::Text(options) => options.set_level(level),
        }
    }

    fn merge_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.merge_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.merge_by_names(),
            Self::Avro(options) => options.merge_by_names(),
            Self::Text(options) => options.merge_by_names(),
        }
    }

    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_merge_by_names(merge_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_merge_by_names(merge_by_names),
            Self::Avro(options) => options.set_merge_by_names(merge_by_names),
            Self::Text(options) => options.set_merge_by_names(merge_by_names),
        }
    }

    fn select_by_names(&self) -> &[String] {
        match self {
            Self::Ipc(options) => options.select_by_names(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.select_by_names(),
            Self::Avro(options) => options.select_by_names(),
            Self::Text(options) => options.select_by_names(),
        }
    }

    fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
        match self {
            Self::Ipc(options) => options.set_select_by_names(select_by_names),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_select_by_names(select_by_names),
            Self::Avro(options) => options.set_select_by_names(select_by_names),
            Self::Text(options) => options.set_select_by_names(select_by_names),
        }
    }

    fn filter_partitions(&self) -> &[(String, String)] {
        match self {
            Self::Ipc(options) => options.filter_partitions(),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.filter_partitions(),
            Self::Avro(options) => options.filter_partitions(),
            Self::Text(options) => options.filter_partitions(),
        }
    }

    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
        match self {
            Self::Ipc(options) => options.set_filter_partitions(filter_partitions),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.set_filter_partitions(filter_partitions),
            Self::Avro(options) => options.set_filter_partitions(filter_partitions),
            Self::Text(options) => options.set_filter_partitions(filter_partitions),
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

impl From<crate::text::TextOptions> for RecordOptions {
    fn from(value: crate::text::TextOptions) -> Self {
        Self::Text(Box::new(value))
    }
}

#[cfg(test)]
mod tests;
