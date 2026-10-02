//! The settings a record read or write takes, shared across encodings.
//!
//! [`IORecordOptions`] is the shared surface for the plan a read or write
//! runs, casts, batch and flow limits, and compression. Each encoding stores
//! those settings as flat fields and adds its own; [`RecordOptions`] names the
//! selected encoding's options.
//!
//! Four properties say everything a read or write says about its rows, each
//! held on its own so one changes without touching the others: the
//! [`field`](IORecordOptions::field) it declares, the
//! [`filter`](IORecordOptions::filter) that keeps rows, the
//! [`select`](IORecordOptions::select) that publishes columns, and the
//! [`merge_by`](IORecordOptions::merge_by) keys a merge matches on. Together
//! they are the sections of one [`Plan`] - [`plan`](IORecordOptions::plan)
//! composes it and [`with_plan`](IORecordOptions::with_plan) splits one back
//! into them - so a caller declares what it means in either form and a media
//! extracts what it needs: a folder prunes its leaves by the equalities the
//! filter spells, an encoding decodes the columns the select clause names, a merge
//! matches by the keys.
//!
//! An encoding is never guessed: [`RecordOptions::for_media_type`] derives it
//! from the handle's media type, which is what [`crate::IOBase`]'s record
//! methods use when a caller does not supply options of their own.
//!
//! ```
//! use yggdryl::media::{IORecordOptions, RecordOptions};
//! use yggdryl::{DataType, StructType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
//!     .required_field("row");
//!
//! // Arrow IPC is available in every Arrow build.
//! let options = RecordOptions::for_media_type(&Url::from_str("file:///t.arrows")?.media_type())?
//!     .with_field(schema.clone())
//!     .with_filter("id > 10")?
//!     .with_batch_row_size(1024)
//!     .with_commit_batch_num(10);
//!
//! assert_eq!(options.field(), Some(schema.clone()));
//! assert_eq!(options.name(), "row");
//! assert_eq!(options.plan().to_string(), "create (id int64 not null) where id > 10");
//! assert_eq!(options.batch_row_size(), Some(1024));
//! assert_eq!(options.commit_batch_num(), Some(10));
//!
//! // The same plan, spelled as text.
//! let spelled = options.clone().with_plan("create trade (id int64 not null) where id > 10")?;
//! assert_eq!(spelled.name(), "trade");
//! assert_eq!(spelled.require_field()?.dtype(), schema.dtype());
//! # Ok(())
//! # }
//! ```

mod commit;
mod dispatch;
mod limits;

use commit::CommitReaders;
pub(crate) use commit::{Cadence, CommitBuffer};
use limits::Limited;
pub(crate) use limits::WriteLimitState;

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{Schema, SchemaRef};
use smol_str::SmolStr;

use crate::arrow::field_from_arrow_schema;
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred};
use crate::expression::{Bound, BoundSelector, IntoFilter, IntoPlan, IntoSelector, Plan, Term};
use crate::ipc::IpcOptions;
use crate::{
    DataType, Error, Field, Filter, IOMode, Level, MediaType, MimeType, Result, Scalar, Selector,
};

/// Default rows materialized in one native-record conversion batch.
///
/// Runtime bindings use this same value when their host-language rows must be
/// widened into Arrow before entering the core reader surface.
pub const DEFAULT_RECORD_BATCH_ROW_SIZE: usize = 65_536;

/// The bytes a resumable write session holds before it publishes, where
/// [`commit_batch_num`](IORecordOptions::commit_batch_num) is unset: 64 MiB,
/// measured as [`memory_size`](crate::arrow::memory_size) counts the held
/// batches.
///
/// A one-shot write with no cadence publishes once, after its source ends;
/// a session ([`ArrowWriteSession`](crate::ArrowWriteSession)) exists to
/// publish between the awaits of a runtime that pushes it batches, so it
/// publishes by this many bytes instead. A non-zero target always yields at
/// least one batch, and no batch is ever cut.
pub const DEFAULT_COMMIT_BYTE_SIZE: u64 = 64 * 1024 * 1024;

/// The threads one file may decode or encode on; `None` is what the host
/// offers.
///
/// Crate-internal, and outside every options value's identity: it changes
/// how fast a file is read or written and never what is, so two options that
/// differ only here compare, hash and order as equal.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FileThreads(pub(crate) Option<usize>);

impl FileThreads {
    /// The bound, or every thread the host offers when there is none.
    pub(crate) fn resolve(self) -> usize {
        self.0.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        })
    }
}

impl PartialEq for FileThreads {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for FileThreads {}

impl PartialOrd for FileThreads {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileThreads {
    fn cmp(&self, _: &Self) -> std::cmp::Ordering {
        std::cmp::Ordering::Equal
    }
}

impl std::hash::Hash for FileThreads {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

/// The read and write settings shared by every record encoding.
///
/// Each encoding stores these as its own fields - there is no shared settings
/// struct to thread through - and the builders here are what every caller uses,
/// so the encodings cannot drift apart in what a shared setting means.
pub trait IORecordOptions: Sized {
    /// Borrow the declared canonical field, if one is declared.
    ///
    /// The non-null Struct root a read projects onto and a write casts onto;
    /// `None` infers the shape from the encoding or the incoming rows. The
    /// field carries the root's name, datatype and metadata in one value.
    fn declared(&self) -> Option<&Field>;

    /// Declare, or clear, the canonical field.
    ///
    /// A field's own nullability and dictionary options are not part of a
    /// declaration: a row root is a non-null Struct, which is what is stored.
    fn set_declared(&mut self, field: Option<Field>);

    /// The declared canonical field, if one is declared.
    fn field(&self) -> Option<Field> {
        self.declared().cloned()
    }

    /// Declare the canonical field.
    fn set_field(&mut self, field: Field) {
        self.set_declared(Some(field));
    }

    /// Borrow the root Field name.
    ///
    /// The declared field's name, and the name an inferred root takes as
    /// well - an Avro container names its record after it - so a stream read
    /// without a schema and one read under a declared one answer the same
    /// root name. Declaring a field sets it; it defaults to
    /// [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME).
    fn name(&self) -> &str;

    /// Set the root Field name, renaming the declared field when there is
    /// one so the two never disagree.
    fn set_name(&mut self, name: SmolStr);

    /// Remove and return the declared canonical field, if any.
    ///
    /// Write combinators use this after casting an incoming stream: the
    /// delegated overwrite then receives rows already in the declared shape
    /// and cannot cast them a second time.
    fn take_field(&mut self) -> Option<Field> {
        let field = self.field();
        self.set_declared(None);
        field
    }

    /// Return whether a declared or stored nullable column takes a value it
    /// cannot convert as null; `true` unless a caller said otherwise.
    ///
    /// It never admits a null, an empty cell or a missing column into a
    /// not-null column, which refuses them - and a value it cannot convert -
    /// by name: the declared-column rule
    /// [`ArrowCastOptions`](crate::ArrowCastOptions) states once.
    fn safe(&self) -> bool;

    /// Set whether a declared or stored nullable column takes a value it
    /// cannot convert as null; `false` refuses it too.
    fn set_safe(&mut self, safe: bool);

    /// Return the row-per-batch bound, if any.
    fn batch_row_size(&self) -> Option<usize>;

    /// Set the row-per-batch bound.
    fn set_batch_row_size(&mut self, batch_row_size: Option<usize>);

    /// Return the byte-per-batch bound, if any.
    ///
    /// Whichever of this and [`Self::batch_row_size`] binds first closes the
    /// batch. Batching by rows alone makes a batch of heartbeats and a batch
    /// of market data differ by three orders of magnitude in memory for the
    /// same row count, which is what a byte bound exists to stop.
    ///
    /// It is a **target rather than a ceiling**: an in-progress builder cannot
    /// be measured the way a finished batch can, so the running estimate is
    /// what was appended plus a fixed per-row width. A non-zero bound always
    /// yields at least one row - the same guarantee the total byte bound
    /// gives - so a single enormous value can never produce an empty batch.
    fn batch_byte_size(&self) -> Option<u64> {
        None
    }

    /// Set the byte-per-batch bound.
    ///
    /// The default does nothing, for an encoding that does not hold one.
    fn set_batch_byte_size(&mut self, batch_byte_size: Option<u64>) {
        let _ = batch_byte_size;
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

    /// Return how many leading result rows a read or write skips, if any -
    /// the plan's `offset`.
    ///
    /// The skip is the same last transform as
    /// [`max_row_size`](Self::max_row_size), taken first: it counts *result*
    /// rows, and the row bound then counts the rows after it, so an offset of
    /// two under a bound of three yields result rows two to four. A batch the
    /// skip covers is dropped and the batch it lands inside is cut with
    /// [`RecordBatch::slice`](arrow_array::RecordBatch::slice), a view over
    /// the same buffers. `None` and `Some(0)` skip nothing.
    fn row_offset(&self) -> Option<u64>;

    /// Set how many leading result rows a read or write skips.
    fn set_row_offset(&mut self, row_offset: Option<u64>);

    /// Return the bound on the result rows' Arrow in-memory bytes, if any.
    ///
    /// Bytes are counted as [`memory_size`](crate::arrow::memory_size)
    /// counts them - each batch's own rows, a zero-copy slice its own
    /// extent rather than its parent's buffers, the one accounting every
    /// byte bound in the crate reads, the commit cadence and the Iceberg
    /// target-file-size rolling included - never as encoded bytes, so a
    /// Parquet file written under a byte limit lands well under it: the
    /// format compresses what this measures uncompressed. The flow stops at the last row that keeps the running
    /// total at or under the limit, and a non-zero limit always yields at
    /// least one row rather than silently losing everything to one wide row -
    /// only `Some(0)` yields nothing. When
    /// [`max_row_size`](Self::max_row_size) is also set, whichever bound binds
    /// first wins.
    fn max_byte_size(&self) -> Option<u64>;

    /// Set the bound on the result rows' Arrow in-memory bytes.
    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>);

    /// Return the publication cadence for a streamed write, in batches.
    ///
    /// `Some(N)` publishes every `N` batches of the shaped stream and then
    /// the final remainder; a batch is one `RecordBatch` the source yields,
    /// cut by [`batch_row_size`](Self::batch_row_size) and
    /// [`batch_byte_size`](Self::batch_byte_size) where a row adapter made
    /// it, never by the cadence, and an empty batch counts for nothing.
    /// Zero is not a cadence and is rejected before a write pulls its
    /// source.
    ///
    /// `None` is the destination's own best cadence. A leaf of any
    /// encoding and a partitioned folder publish once, after the source
    /// ends: a leaf append is a rewrite, so a periodic commit on one would
    /// be a rewrite per commit. An Iceberg table publishes a commit each
    /// time the batches it holds reach its target file size
    /// (`write.target-file-size-bytes`) as
    /// [`memory_size`](crate::arrow::memory_size) measures them, then the
    /// remainder, so a streamed write of any length holds at most one
    /// target file of rows before each commit. A resumable write session
    /// publishes by [`DEFAULT_COMMIT_BYTE_SIZE`].
    ///
    /// Whichever cadence applies, an overwrite's first commit replaces and
    /// every later one appends, an append appends on every commit, and every
    /// commit of a merge merges by its key - a merge an Iceberg table keys by
    /// its partition alone replaces a partition on the first commit of the
    /// write that reaches it and appends to it on every later one. The
    /// commits completed before a later failure stay published.
    fn commit_batch_num(&self) -> Option<usize>;

    /// Set the publication cadence for a streamed write, in batches.
    fn set_commit_batch_num(&mut self, commit_batch_num: Option<usize>);

    /// Return the compression level applied to a declared content coding.
    fn level(&self) -> Level;

    /// Set the compression level.
    fn set_level(&mut self, level: Level);

    /// Borrow the selector whose columns form an explicit merge's match key.
    ///
    /// A non-empty selector is required by
    /// [`merge_arrow_reader`](crate::IOMedia::merge_arrow_reader): a row
    /// whose key is already stored updates it, and a row whose key is not
    /// appends. Each projection is one key column - a stored column by name,
    /// or a term computed from the row, so `trade.id` and `lower(symbol)`
    /// are keys as much as `id` is. The option never selects an operation;
    /// overwrite and append reject it, and merge rejects an empty selector.
    fn merge_by(&self) -> &Selector;

    /// Set the selector whose columns form an explicit merge's match key.
    fn set_merge_by(&mut self, merge_by: Selector);

    /// Borrow the rows a read or write keeps: the `where` clause.
    ///
    /// Always true keeps every row. The clause binds once, and against the
    /// rows' own schema wherever it can: it runs before the selector, reading
    /// stored names, and after it exactly where it names a column only the
    /// selector publishes - which is what
    /// [`apply_arrow_expressions`](Self::apply_arrow_expressions) decides, once
    /// per stream.
    fn filter(&self) -> &Filter;

    /// Set the rows a read or write keeps.
    fn set_filter(&mut self, filter: Filter);

    /// Borrow the columns a read or write publishes: the `select` clause;
    /// `*` publishes every column unchanged.
    fn select(&self) -> &Selector;

    /// Set the columns a read or write publishes.
    fn set_select(&mut self, select: Selector);

    /// The plan these properties are the sections of.
    ///
    /// The declared field is its `create` section, the filter its `where`,
    /// the selector its `select`, the merge key its `upsert by (...)`, the
    /// row bound its `limit` and the row skip its `offset`. This is what an
    /// options value spells as one expression, and what
    /// [`with_plan`](Self::with_plan) reads back.
    fn plan(&self) -> Plan {
        let mut plan = match self.field() {
            Some(field) => Plan::from_field(&field),
            None => Plan::new(),
        };
        plan.set_filter(self.filter().clone());
        plan.set_selector(self.select().clone());
        plan.set_merge_by(self.merge_by().clone());
        plan.limit(self.max_row_size()).offset(self.row_offset())
    }

    /// Set every property from the sections of one plan.
    ///
    /// A plan's `create` section declares the field, its `where` clause is
    /// the filter, its `select` clause the selector, its upsert keys the
    /// merge key; a section the plan does not spell clears the property. A
    /// `limit` is the row bound and an `offset` the row skip. The plan's
    /// targets and source are not read:
    /// the handle these options are given to is both.
    ///
    /// # Errors
    ///
    /// Returns an error when the `create` section declares a column that
    /// cannot be typed without rows, the merge key names a column twice, or
    /// the plan carries a section these options have no property for - an
    /// `order by` - which is refused by name rather than dropped: a plan that
    /// orders rows is applied to the rows themselves, through the expression
    /// layer.
    fn set_plan(&mut self, plan: Plan) -> Result<()> {
        // Everything that can refuse does so before the first write, so a
        // refused plan leaves every section as it was.
        let declared = plan.field()?;
        distinct_merge_key(plan.merge_by())?;
        if !plan.ordering().is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.order_by"),
                reason: SmolStr::new_static(
                    "record options hold no `order by` section; apply the plan to the rows instead",
                ),
            });
        }
        self.set_declared(declared);
        self.set_filter(plan.filter_section().clone());
        self.set_select(plan.selector().clone());
        self.set_merge_by(plan.merge_by().clone());
        if let Some(limit) = plan.row_limit() {
            self.set_max_row_size(Some(limit));
        }
        if let Some(offset) = plan.row_offset() {
            self.set_row_offset(Some(offset));
        }
        Ok(())
    }

    /// The partition equalities the `where` section spells.
    ///
    /// Every conjunct comparing one column to one constant for equality,
    /// the constant spelled as
    /// [`partition_text`](crate::media::partition::partition_text) spells it,
    /// and `column is null` as the null partition. A folder read skips every
    /// leaf whose path names a different value - nothing under it is listed
    /// or decoded - and an Iceberg scan prunes by the same pairs, so
    /// path-partitioned and data-partitioned layouts answer the same clause
    /// the same way. The rest of the clause runs over the rows.
    fn partition_pairs(&self) -> Vec<(String, String)> {
        partition_pairs(self.filter())
    }

    /// The stored columns the plan reads, when it narrows them.
    ///
    /// The `select` clause and the clauses beside it name the columns a read
    /// has to decode; `None` - no selector, or one that keeps every column -
    /// is the read that already happens. This is projection pushdown without
    /// a declared field.
    fn apply_columns(&self) -> Option<Vec<String>> {
        if self.select().has_star() {
            return None;
        }
        let mut columns = self.filter().columns();
        for column in self.select().columns() {
            if !columns
                .iter()
                .any(|held| held.eq_ignore_ascii_case(&column))
            {
                columns.push(column);
            }
        }
        Some(columns)
    }

    /// Run the filter, then the selector, over a reader.
    ///
    /// Each clause binds once against the schema the clause before it
    /// produced, so a stream pays for its plan once; a clause that keeps or
    /// publishes everything costs nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when a clause does not bind against the schema it
    /// meets.
    fn apply_arrow_expressions(
        &self,
        reader: crate::arrow::BatchReader,
    ) -> Result<crate::arrow::BatchReader> {
        use arrow_array::RecordBatchReader as _;
        let schema = reader.schema();
        let late = crate::expression::filter_after_select(
            self.filter(),
            self.select(),
            schema.fields().iter().map(|field| field.name().as_str()),
        );
        if late {
            return self
                .filter()
                .apply_arrow_reader(self.select().apply_arrow_reader(reader)?);
        }
        self.select()
            .apply_arrow_reader(self.filter().apply_arrow_reader(reader)?)
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

    /// Return these options with a declared root datatype, under the root
    /// name they carry.
    #[must_use]
    fn with_dtype(mut self, dtype: DataType) -> Self {
        let name = SmolStr::new(self.name());
        self.set_field(dtype.required_field(name));
        self
    }

    /// Return these options with every property set from one plan.
    ///
    /// The plan is text - `"create (id int64) where id > 0"`, `"select id"`,
    /// `"upsert by (id)"` - a [`Field`] to declare, a selector, a filter, or
    /// a [`Plan`] built by hand; [`set_plan`](Self::set_plan) says how its
    /// sections land.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the plan is text that does not parse, or
    /// the error [`set_plan`](Self::set_plan) raises.
    fn with_plan(mut self, plan: impl IntoPlan) -> Result<Self> {
        self.set_plan(plan.into_plan()?)?;
        Ok(self)
    }

    /// Return these options keeping the rows a filter answers true for.
    ///
    /// The filter is a [`Filter`], a term, or the text of one with or
    /// without `where` in front.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the filter is text that does not parse.
    fn with_filter(mut self, filter: impl IntoFilter) -> Result<Self> {
        self.set_filter(filter.into_filter()?);
        Ok(self)
    }

    /// Set the `where` section from the scalar that spells it, as
    /// [`Filter::from_scalar`] reads one.
    ///
    /// This is the one setter every binding's `filter` property crosses
    /// through: a value is read as a scalar there and typed here.
    ///
    /// # Errors
    ///
    /// Returns the error [`Filter::from_scalar`] does.
    fn set_filter_scalar(&mut self, filter: &Scalar) -> Result<()> {
        self.set_filter(Filter::from_scalar(filter)?);
        Ok(())
    }

    /// Return these options with the `where` section a scalar spells.
    ///
    /// # Errors
    ///
    /// Returns the error [`Filter::from_scalar`] does.
    fn with_filter_scalar(mut self, filter: &Scalar) -> Result<Self> {
        self.set_filter_scalar(filter)?;
        Ok(self)
    }

    /// Set the `select` section from the scalar that spells it, as
    /// [`Selector::from_scalar`] reads one: text, a sequence of projections,
    /// or a mapping of aliases to terms.
    ///
    /// # Errors
    ///
    /// Returns the error [`Selector::from_scalar`] does.
    fn set_select_scalar(&mut self, select: &Scalar) -> Result<()> {
        self.set_select(Selector::from_scalar(select)?);
        Ok(())
    }

    /// Return these options with the `select` section a scalar spells.
    ///
    /// # Errors
    ///
    /// Returns the error [`Selector::from_scalar`] does.
    fn with_select_scalar(mut self, select: &Scalar) -> Result<Self> {
        self.set_select_scalar(select)?;
        Ok(self)
    }

    /// Set the merge key from the scalar that spells it, as
    /// [`Selector::from_scalar`] reads one.
    ///
    /// # Errors
    ///
    /// Returns the error [`Selector::from_scalar`] does, or the error
    /// [`require_merge_by`](Self::require_merge_by) does.
    fn set_merge_by_scalar(&mut self, merge_by: &Scalar) -> Result<()> {
        let merge_by = Selector::from_scalar(merge_by)?;
        // Validated before it is stored, so a refused key leaves the old one.
        distinct_merge_key(&merge_by)?;
        self.set_merge_by(merge_by);
        Ok(())
    }

    /// Return these options with the merge key a scalar spells.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_merge_by_scalar`](Self::set_merge_by_scalar) does.
    fn with_merge_by_scalar(mut self, merge_by: &Scalar) -> Result<Self> {
        self.set_merge_by_scalar(merge_by)?;
        Ok(self)
    }

    /// Set every section from the plan a scalar spells, as
    /// [`Plan::from_scalar`] reads one.
    ///
    /// # Errors
    ///
    /// Returns the error [`Plan::from_scalar`] or [`set_plan`](Self::set_plan) does.
    fn set_plan_scalar(&mut self, plan: &Scalar) -> Result<()> {
        self.set_plan(Plan::from_scalar(plan)?)
    }

    /// Return these options with every section the plan a scalar spells.
    ///
    /// # Errors
    ///
    /// Returns the error [`set_plan_scalar`](Self::set_plan_scalar) does.
    fn with_plan_scalar(mut self, plan: &Scalar) -> Result<Self> {
        self.set_plan_scalar(plan)?;
        Ok(self)
    }

    /// Return these options publishing the columns a selector names.
    ///
    /// The selector is a [`Selector`], a list of column names, or the text
    /// of one with or without `select` in front.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the selector is text that does not parse.
    fn with_select(mut self, select: impl IntoSelector) -> Result<Self> {
        self.set_select(select.into_selector()?);
        Ok(self)
    }

    /// Return these options with a different cast strictness.
    #[must_use]
    fn with_safe(mut self, safe: bool) -> Self {
        self.set_safe(safe);
        self
    }

    /// Return these options with a byte-per-batch bound.
    #[must_use]
    fn with_batch_byte_size(mut self, batch_byte_size: u64) -> Self {
        self.set_batch_byte_size(Some(batch_byte_size));
        self
    }

    /// Return these options with a row-per-batch bound.
    #[must_use]
    fn with_batch_row_size(mut self, batch_row_size: usize) -> Self {
        self.set_batch_row_size(Some(batch_row_size));
        self
    }

    /// Return these options skipping the given leading result rows.
    #[must_use]
    fn with_row_offset(mut self, row_offset: u64) -> Self {
        self.set_row_offset(Some(row_offset));
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

    /// Return these options with a publication every `commit_batch_num`
    /// batches.
    ///
    /// A zero value is retained so the write can return a typed error before
    /// touching a one-shot input. Use `None` through
    /// [`set_commit_batch_num`](Self::set_commit_batch_num) for the
    /// destination's own cadence.
    #[must_use]
    fn with_commit_batch_num(mut self, commit_batch_num: usize) -> Self {
        self.set_commit_batch_num(Some(commit_batch_num));
        self
    }

    /// Return these options with a different compression level.
    #[must_use]
    fn with_level(mut self, level: Level) -> Self {
        self.set_level(level);
        self
    }

    /// Return these options with a match key for an explicit merge.
    ///
    /// Text parses through the selector grammar and a list of names is a list
    /// of columns, so `with_merge_by("id, ts")` and `with_merge_by(["id",
    /// "ts"])` are one key.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the key is text that is not a selector.
    fn with_merge_by(mut self, merge_by: impl IntoSelector) -> Result<Self> {
        self.set_merge_by(merge_by.into_selector()?);
        self.require_merge_by().map(|()| self)
    }

    /// Refuse a match key that names a column twice, or that unnests: a key
    /// is one value per row, where an `unnest` is one row per element.
    ///
    /// # Errors
    ///
    /// Returns an error naming the repeated column, or the `unnest`.
    fn require_merge_by(&self) -> Result<()> {
        distinct_merge_key(self.merge_by())
    }

    /// Shape one batch the way these options say, completed by what is stored.
    ///
    /// This is the one definition of option-driven shaping, in three layers
    /// applied in order: the declared [`field`](Self::field) says what the
    /// rows are meant to be, the plan's `where` and `select` clauses keep and
    /// publish what they say, and `existing` - a holder's stored shape - is
    /// what the batch is finally completed onto. The declared and the stored
    /// layer are both declarations and cast by the one declared-column rule:
    /// a nullable column takes a value it cannot convert as null when
    /// [`safe`](Self::safe), and a not-null column refuses that value, a
    /// null and a missing column by name, so a write never quietly redefines
    /// a stored column for every reader of the resource. Each absent layer
    /// costs nothing.
    ///
    /// A field shapes rows by the cast alone ([`Field::apply_arrow_batch`]):
    /// a `TRANSFORM:`, `PARTITION:` or `DIGEST:` declaration it carries is
    /// metadata the rows travel under, never a column the shaping fills.
    ///
    /// Every layer answers from the schemas alone, so the shaping is compiled
    /// against the batch's schema and then applied; a caller shaping many
    /// batches of one layout holds the compiled shaping instead.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned, a declaration cannot be
    /// satisfied, or an expression does not bind against the rows.
    fn apply_arrow_batch(
        &self,
        batch: arrow_array::RecordBatch,
        existing: Option<&Field>,
    ) -> Result<arrow_array::RecordBatch> {
        Shaping::compile(self, batch.schema(), existing)?.apply(batch)
    }

    /// Shape a whole reader as [`apply_arrow_batch`](Self::apply_arrow_batch)
    /// shapes one batch, streaming - nothing is collected, each batch is
    /// shaped as it is pulled, and a layer whose target already matches costs
    /// nothing at all.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned, a declaration cannot be
    /// satisfied, or an expression does not bind against the reader.
    fn apply_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
        existing: Option<&Field>,
    ) -> Result<crate::arrow::BatchReader> {
        let options = ArrowCastOptions::new().with_safe(self.safe());
        let reader = match self.field() {
            Some(declared) => declared.apply_arrow_reader(reader, options)?,
            None => reader,
        };
        let reader = self.apply_arrow_expressions(reader)?;
        match existing {
            Some(stored) => Ok(stored.apply_arrow_reader(reader, options)?),
            None => Ok(reader),
        }
    }

    /// Skip [`row_offset`](Self::row_offset) rows of a reader, then bound it
    /// by [`max_row_size`](Self::max_row_size) and
    /// [`max_byte_size`](Self::max_byte_size).
    ///
    /// This is one more transform of the same option-driven shaping seam as
    /// [`apply_arrow_reader`](Self::apply_arrow_reader), applied *last*: the
    /// order is declared schema, then the applied expressions, then completion
    /// cast, then partition filter, then the limit, so the limit counts result rows and
    /// never rows an earlier layer dropped or reshaped. No media implements a
    /// limit - the record methods wrap the shaped reader here, exactly once
    /// per call. A media may read a row bound as a fetch plan (Parquet
    /// fetches only the leading row groups that cover it), but the trim to
    /// the exact count happens only here.
    ///
    /// The wrapper holds at most one batch and stops pulling the moment it is
    /// satisfied, so the rest of the source is never decoded. With no skip
    /// and neither bound set the reader is returned as it stands.
    ///
    /// # Errors
    ///
    /// Returns an error naming both settings when a limit or a skip is
    /// combined with a non-empty [`merge_by`](Self::merge_by): a truncated
    /// merge would update the matched keys it kept and silently drop the
    /// rest, which corrupts the resource rather than shortening the write.
    fn limit_arrow_reader(
        &self,
        reader: crate::arrow::BatchReader,
    ) -> Result<crate::arrow::BatchReader> {
        let max_rows = self.max_row_size();
        let max_bytes = self.max_byte_size();
        let skip = self.row_offset().unwrap_or(0);
        if max_rows.is_none() && max_bytes.is_none() && skip == 0 {
            return Ok(reader);
        }
        self.require_write_limits()?;
        use arrow_array::RecordBatchReader as _;
        let schema = reader.schema();
        Ok(Box::new(Limited {
            inner: reader,
            schema,
            state: WriteLimitState::new(skip, max_rows, max_bytes),
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
        let skip = self.row_offset().filter(|rows| *rows != 0);
        if max_rows.is_none() && max_bytes.is_none() && skip.is_none() {
            return Ok(());
        }
        if !self.merge_by().is_empty() {
            let mut limits = Vec::new();
            if let Some(rows) = max_rows {
                limits.push(format!("max_row_size = {rows}"));
            }
            if let Some(bytes) = max_bytes {
                limits.push(format!("max_byte_size = {bytes}"));
            }
            if let Some(rows) = skip {
                limits.push(format!("row_offset = {rows}"));
            }
            let limits = limits.join(" and ");
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    "max_row_size, max_byte_size and row_offset without merge_by - a truncated \
                     merge updates the matched keys it kept and silently drops the rest, \
                     corrupting rather than shortening",
                    format!("{limits} with merge_by `{}`", self.merge_by()),
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

/// [`IORecordOptions::apply_arrow_batch`] compiled against one schema.
///
/// The declared field, the `where` and `select` clauses and the stored field
/// are each planned or bound once against the schema the layer before hands
/// it, so a batch of that schema moves only rows.
pub(crate) struct Shaping {
    declared: Option<ArrowCastPlan>,
    /// Whether the `select` runs first, because the `where` reads a column
    /// only the selector builds.
    late: bool,
    filter: Option<Bound>,
    select: Option<BoundSelector>,
    /// The cast completing the rows onto the destination's stored field.
    existing: Option<ArrowCastPlan>,
}

impl Shaping {
    /// Compile how `options`, completed onto `existing`, shape a batch of
    /// `source`.
    ///
    /// # Errors
    ///
    /// Returns an error when a cast cannot be planned, a declaration cannot be
    /// satisfied, or an expression does not bind against the schema it meets.
    pub(crate) fn compile(
        options: &impl IORecordOptions,
        source: SchemaRef,
        existing: Option<&Field>,
    ) -> Result<Self> {
        let cast = ArrowCastOptions::new().with_safe(options.safe());
        let mut schema = source;
        let declared = match options.declared() {
            Some(declared) => {
                let plan =
                    ArrowCastPlan::compile_schema(&schema, declared, cast, Deferred::default())?;
                schema = Arc::clone(plan.target_schema()?);
                Some(plan)
            }
            None => None,
        };
        let late = crate::expression::filter_after_select(
            options.filter(),
            options.select(),
            schema.fields().iter().map(|field| field.name().as_str()),
        );
        let (filter, select) = if late {
            let select = Self::bind_select(options.select(), &mut schema)?;
            (Self::bind_filter(options.filter(), &schema)?, select)
        } else {
            let filter = Self::bind_filter(options.filter(), &schema)?;
            (filter, Self::bind_select(options.select(), &mut schema)?)
        };
        let existing = match existing {
            Some(stored) => Some(ArrowCastPlan::compile_schema(
                &schema,
                stored,
                cast,
                Deferred::default(),
            )?),
            None => None,
        };
        Ok(Self {
            declared,
            late,
            filter,
            select,
            existing,
        })
    }

    fn bind_filter(filter: &Filter, schema: &Schema) -> Result<Option<Bound>> {
        if filter.is_always_true() {
            return Ok(None);
        }
        let root = field_from_arrow_schema(super::DEFAULT_ROOT_NAME, schema)?;
        Ok(Some(filter.bind(&root)?))
    }

    /// Bind the selector and move `schema` to the one it publishes.
    fn bind_select(select: &Selector, schema: &mut SchemaRef) -> Result<Option<BoundSelector>> {
        if select.is_all() {
            return Ok(None);
        }
        let bound = select.bind(&field_from_arrow_schema(super::DEFAULT_ROOT_NAME, schema)?)?;
        *schema = bound
            .apply_arrow_batch(&RecordBatch::new_empty(Arc::clone(schema)))?
            .schema();
        Ok(Some(bound))
    }

    /// Shape one batch of the schema this was compiled against.
    ///
    /// # Errors
    ///
    /// Returns an error when a value does not fit its declared column, a
    /// declaration cannot be satisfied, or a term fails over the rows.
    pub(crate) fn apply(&self, batch: RecordBatch) -> Result<RecordBatch> {
        let mut batch = match &self.declared {
            Some(plan) => plan.reconcile_batch(batch)?,
            None => batch,
        };
        if self.late {
            batch = self.select(batch)?;
            batch = self.filter(batch)?;
        } else {
            batch = self.filter(batch)?;
            batch = self.select(batch)?;
        }
        match &self.existing {
            Some(plan) => Ok(plan.reconcile_batch(batch)?),
            None => Ok(batch),
        }
    }

    fn filter(&self, batch: RecordBatch) -> Result<RecordBatch> {
        match &self.filter {
            Some(bound) => Ok(bound.filter(&batch)?),
            None => Ok(batch),
        }
    }

    fn select(&self, batch: RecordBatch) -> Result<RecordBatch> {
        match &self.select {
            Some(bound) => bound.apply_arrow_batch(&batch),
            None => Ok(batch),
        }
    }
}
/// Refuse a match key that unnests, or names one column twice in any case:
/// a key is one value per row, where an `unnest` is one row per element.
fn distinct_merge_key(merge_by: &Selector) -> Result<()> {
    merge_by.refuse_unnest("in a key")?;
    let names = merge_by.names();
    for (index, name) in names.iter().enumerate() {
        if names[..index]
            .iter()
            .any(|held| held.eq_ignore_ascii_case(name))
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.merge_by"),
                reason: smol_str::format_smolstr!(
                    "expected each match key column once, got {name:?} twice"
                ),
            });
        }
    }
    Ok(())
}

/// The `(column, value)` equalities one filter spells, in conjunct order.
///
/// A conjunct comparing a column to a constant for equality is one pair, the
/// constant spelled as a partition directory spells it; `column is null` is
/// the null partition. Anything else is left to the rows.
pub(crate) fn partition_pairs(filter: &Filter) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for conjunct in filter.simplify().conjuncts() {
        match conjunct.term() {
            Term::Compare(left, crate::expression::Comparison::Eq, right) => {
                let (column, literal) = match (left.as_column(), right.as_literal()) {
                    (Some(column), Some(literal)) => (column, literal),
                    _ => match (right.as_column(), left.as_literal()) {
                        (Some(column), Some(literal)) => (column, literal),
                        _ => continue,
                    },
                };
                if let Ok(text) = crate::media::partition::partition_text(literal.value()) {
                    pairs.push((column.to_owned(), text.to_string()));
                }
            }
            Term::IsNull(inner) => {
                if let Some(column) = inner.as_column() {
                    pairs.push((column.to_owned(), crate::media::NULL_PARTITION.to_owned()));
                }
            }
            _ => {}
        }
    }
    pairs
}

/// Implement [`IORecordOptions`] over one struct's own fields.
///
/// Every encoding stores the same shared settings under the same names, so the
/// accessors are mechanical; what differs is the fields an encoding adds.
#[macro_export]
macro_rules! record_options_fields {
    () => {
        fn declared(&self) -> Option<&$crate::Field> {
            self.field.as_ref()
        }

        fn set_declared(&mut self, field: Option<$crate::Field>) {
            if let Some(field) = &field {
                self.name = smol_str::SmolStr::new(field.name());
            }
            self.field = field.map(|field| field.with_nullable(false));
        }

        fn name(&self) -> &str {
            self.name.as_str()
        }

        fn set_name(&mut self, name: smol_str::SmolStr) {
            if let Some(field) = self.field.take() {
                self.field = Some(field.with_name(name.clone()));
            }
            self.name = name;
        }

        fn merge_by(&self) -> &$crate::Selector {
            &self.merge_by
        }

        fn set_merge_by(&mut self, merge_by: $crate::Selector) {
            self.merge_by = merge_by;
        }

        fn filter(&self) -> &$crate::Filter {
            &self.filter
        }

        fn set_filter(&mut self, filter: $crate::Filter) {
            self.filter = filter;
        }

        fn select(&self) -> &$crate::Selector {
            &self.select
        }

        fn set_select(&mut self, select: $crate::Selector) {
            self.select = select;
        }

        fn safe(&self) -> bool {
            self.safe
        }

        fn set_safe(&mut self, safe: bool) {
            self.safe = safe;
        }

        fn batch_byte_size(&self) -> Option<u64> {
            self.batch_byte_size
        }

        fn set_batch_byte_size(&mut self, batch_byte_size: Option<u64>) {
            self.batch_byte_size = batch_byte_size;
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

        fn row_offset(&self) -> Option<u64> {
            self.row_offset
        }

        fn set_row_offset(&mut self, row_offset: Option<u64>) {
            self.row_offset = row_offset;
        }

        fn max_byte_size(&self) -> Option<u64> {
            self.max_byte_size
        }

        fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
            self.max_byte_size = max_byte_size;
        }

        fn commit_batch_num(&self) -> Option<usize> {
            self.commit_batch_num
        }

        fn set_commit_batch_num(&mut self, commit_batch_num: Option<usize>) {
            self.commit_batch_num = commit_batch_num;
        }

        fn level(&self) -> $crate::Level {
            self.level
        }

        fn set_level(&mut self, level: $crate::Level) {
            self.level = level;
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
    /// XML for Analysis rowset document options.
    Xmla(crate::xmla::XmlaOptions),
    /// CSV and TSV document options.
    Csv(crate::csv::CsvOptions),
    /// Office Open XML workbook options.
    Excel(crate::excel::ExcelOptions),
}

impl RecordOptions {
    /// Return a deterministic hash of the encoding and its complete options.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        match self {
            Self::Ipc(options) => crate::hashing::stable_hash_of(&("ipc", options)),
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => crate::hashing::stable_hash_of(&("parquet", options)),
            Self::Avro(options) => crate::hashing::stable_hash_of(&("avro", options)),
            Self::Text(options) => crate::hashing::stable_hash_of(&("text", options)),
            Self::Xmla(options) => crate::hashing::stable_hash_of(&("xmla", options)),
            Self::Csv(options) => crate::hashing::stable_hash_of(&("csv", options)),
            Self::Excel(options) => crate::hashing::stable_hash_of(&("excel", options)),
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
            Self::Ipc(_) | Self::Avro(_) | Self::Xmla(_) | Self::Csv(_) | Self::Excel(_) => {
                Err(Error::InvalidRecord {
                    path: SmolStr::new_static(path),
                    reason: smol_str::format_smolstr!(
                        "expected text options to set {setting}, got {media_type} options"
                    ),
                })
            }
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
            Self::Ipc(_) | Self::Avro(_) | Self::Xmla(_) | Self::Csv(_) | Self::Excel(_) => None,
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
            Self::Ipc(_) | Self::Text(_) | Self::Xmla(_) | Self::Csv(_) | Self::Excel(_) => {
                Err(Error::InvalidRecord {
                    path: SmolStr::new_static(path),
                    reason: smol_str::format_smolstr!(
                        "expected Avro options to set {setting}, got {media_type} options"
                    ),
                })
            }
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
            Self::Ipc(_) | Self::Text(_) | Self::Xmla(_) | Self::Csv(_) | Self::Excel(_) => None,
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
            Self::Ipc(_) | Self::Text(_) | Self::Xmla(_) | Self::Csv(_) | Self::Excel(_) => None,
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

    fn csv_mut(
        &mut self,
        path: &'static str,
        setting: &'static str,
    ) -> Result<&mut crate::csv::CsvOptions> {
        let media_type = self.mime_type();
        match self {
            Self::Csv(options) => Ok(options),
            Self::Ipc(_) | Self::Avro(_) | Self::Text(_) | Self::Xmla(_) | Self::Excel(_) => {
                Err(Error::InvalidRecord {
                    path: SmolStr::new_static(path),
                    reason: smol_str::format_smolstr!(
                        "expected CSV options to set {setting}, got {media_type} options"
                    ),
                })
            }
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected CSV options to set {setting}, got {media_type} options"
                ),
            }),
        }
    }

    /// Borrow the CSV options, or `None` for another encoding.
    const fn csv(&self) -> Option<&crate::csv::CsvOptions> {
        match self {
            Self::Csv(options) => Some(options),
            Self::Ipc(_) | Self::Avro(_) | Self::Text(_) | Self::Xmla(_) | Self::Excel(_) => None,
            #[cfg(feature = "parquet")]
            Self::Parquet(_) => None,
        }
    }

    /// Return the CSV separator byte, or `None` for another encoding.
    pub const fn csv_separator(&self) -> Option<u8> {
        match self.csv() {
            Some(options) => Some(options.separator()),
            None => None,
        }
    }

    /// Set the CSV separator byte.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant, or the error
    /// [`CsvOptions::set_separator`](crate::csv::CsvOptions::set_separator) does.
    pub fn set_csv_separator(&mut self, separator: u8) -> Result<()> {
        self.csv_mut("$.separator", "a separator")?
            .set_separator(separator)
    }

    /// Return the CSV quote byte - `Some(None)` where the dialect quotes
    /// nothing - or `None` for another encoding.
    pub const fn csv_quote(&self) -> Option<Option<u8>> {
        match self.csv() {
            Some(options) => Some(options.quote()),
            None => None,
        }
    }

    /// Set or clear the CSV quote byte.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant, or the error
    /// [`CsvOptions::set_quote`](crate::csv::CsvOptions::set_quote) does.
    pub fn set_csv_quote(&mut self, quote: Option<u8>) -> Result<()> {
        self.csv_mut("$.quote", "a quote")?.set_quote(quote)
    }

    /// Return the CSV escape byte, or `None` for another encoding.
    pub const fn csv_escape(&self) -> Option<Option<u8>> {
        match self.csv() {
            Some(options) => Some(options.escape()),
            None => None,
        }
    }

    /// Set or clear the CSV escape byte.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant, or the error
    /// [`CsvOptions::set_escape`](crate::csv::CsvOptions::set_escape) does.
    pub fn set_csv_escape(&mut self, escape: Option<u8>) -> Result<()> {
        self.csv_mut("$.escape", "an escape")?.set_escape(escape)
    }

    /// Return the CSV comment byte, or `None` for another encoding.
    pub const fn csv_comment(&self) -> Option<Option<u8>> {
        match self.csv() {
            Some(options) => Some(options.comment()),
            None => None,
        }
    }

    /// Set or clear the CSV comment byte.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant, or the error
    /// [`CsvOptions::set_comment`](crate::csv::CsvOptions::set_comment) does.
    pub fn set_csv_comment(&mut self, comment: Option<u8>) -> Result<()> {
        self.csv_mut("$.comment", "a comment byte")?
            .set_comment(comment)
    }

    /// Return whether the first record names the columns - a CSV's first
    /// record, a workbook's first row - or `None` for an encoding whose
    /// columns are named by its own schema.
    pub const fn header(&self) -> Option<bool> {
        match self {
            Self::Csv(options) => Some(options.header()),
            Self::Excel(options) => Some(options.header),
            _ => None,
        }
    }

    /// Set whether the first record names the columns: a CSV's first record,
    /// a workbook's first row.
    ///
    /// # Errors
    ///
    /// Returns an error for an encoding that is neither.
    pub fn set_header(&mut self, header: bool) -> Result<()> {
        let media_type = self.mime_type();
        match self {
            Self::Csv(options) => options.set_header(header),
            Self::Excel(options) => options.header = header,
            _ => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.header"),
                    reason: smol_str::format_smolstr!(
                        "expected CSV or Excel options to set a header, got {media_type} options"
                    ),
                });
            }
        }
        Ok(())
    }

    /// Borrow the CSV spellings of an absent value, or `None` for another
    /// encoding.
    pub fn csv_null_values(&self) -> Option<&[SmolStr]> {
        self.csv().map(crate::csv::CsvOptions::null_values)
    }

    /// Set the CSV spellings of an absent value.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant, or the error
    /// [`CsvOptions::set_null_values`](crate::csv::CsvOptions::set_null_values) does.
    pub fn set_csv_null_values<I, S>(&mut self, null_values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<SmolStr>,
    {
        self.csv_mut("$.null_values", "null spellings")?
            .set_null_values(null_values)
    }

    /// Return whether the CSV trims the blanks around an unquoted cell, or
    /// `None` for another encoding.
    pub const fn csv_trim(&self) -> Option<bool> {
        match self.csv() {
            Some(options) => Some(options.trim()),
            None => None,
        }
    }

    /// Set whether the CSV trims the blanks around an unquoted cell.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant.
    pub fn set_csv_trim(&mut self, trim: bool) -> Result<()> {
        self.csv_mut("$.trim", "trimming")?.set_trim(trim);
        Ok(())
    }

    /// Return the records a CSV samples to infer a column's datatype, or
    /// `None` for another encoding.
    pub const fn csv_infer_row_size(&self) -> Option<usize> {
        match self.csv() {
            Some(options) => Some(options.infer_row_size()),
            None => None,
        }
    }

    /// Set the records a CSV samples to infer a column's datatype.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-CSV variant, or the error
    /// [`CsvOptions::set_infer_row_size`](crate::csv::CsvOptions::set_infer_row_size) does.
    pub fn set_csv_infer_row_size(&mut self, infer_row_size: usize) -> Result<()> {
        self.csv_mut("$.infer_row_size", "a sample size")?
            .set_infer_row_size(infer_row_size)
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
            Self::Ipc(_)
            | Self::Avro(_)
            | Self::Text(_)
            | Self::Xmla(_)
            | Self::Csv(_)
            | Self::Excel(_) => Err(Error::InvalidRecord {
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
            Self::Ipc(_)
            | Self::Avro(_)
            | Self::Text(_)
            | Self::Xmla(_)
            | Self::Csv(_)
            | Self::Excel(_) => None,
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

    /// Bound the threads one file decodes or encodes on.
    ///
    /// A table that already reads or writes several files at once hands each
    /// file its share this way, so the two levels of parallelism never
    /// multiply past what it resolved. Parquet splits its row groups and
    /// columns across the share and Avro its blocks; Arrow IPC, text and an
    /// XMLA document already decode on one thread.
    pub(crate) fn set_file_threads(&mut self, threads: usize) {
        match self {
            #[cfg(feature = "parquet")]
            Self::Parquet(options) => options.threads = Some(threads.max(1)),
            Self::Avro(options) => options.threads = FileThreads(Some(threads.max(1))),
            Self::Ipc(_) | Self::Text(_) | Self::Xmla(_) | Self::Csv(_) | Self::Excel(_) => {}
        }
    }

    /// Return the Parquet row-group bound, or `None` for another encoding.
    #[cfg(feature = "parquet")]
    pub const fn parquet_max_row_group_size(&self) -> Option<usize> {
        match self {
            Self::Parquet(options) => Some(options.max_row_group_size),
            Self::Ipc(_)
            | Self::Avro(_)
            | Self::Text(_)
            | Self::Xmla(_)
            | Self::Csv(_)
            | Self::Excel(_) => None,
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
            Self::Ipc(_)
            | Self::Avro(_)
            | Self::Text(_)
            | Self::Xmla(_)
            | Self::Csv(_)
            | Self::Excel(_) => None,
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
        let keys = self.merge_by();
        if keyed != keys.is_empty() {
            return Ok(());
        }
        let reason = if keyed {
            format!("write mode {mode} requires at least one merge_by column")
        } else {
            format!("write mode {mode} does not accept merge_by; use merge mode for keyed writes")
        };
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.merge_by"),
            reason: SmolStr::new(reason),
        })
    }

    /// Validate the optional streamed-write publication cadence.
    ///
    /// This is public only for the workspace bindings, which must reject a
    /// zero cadence before converting or pulling a runtime iterator.
    #[doc(hidden)]
    pub fn require_commit_batch_num(&self) -> Result<Option<usize>> {
        match self.commit_batch_num() {
            Some(0) => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.commit_batch_num"),
                reason: SmolStr::new_static(
                    "expected commit_batch_num to be a non-zero batch count, got 0",
                ),
            }),
            commit_batch_num => Ok(commit_batch_num),
        }
    }

    /// The cadence a write publishes by: the batch count these options
    /// state, or `default`, the destination's own, where they state none.
    ///
    /// # Errors
    ///
    /// Returns the [`require_commit_batch_num`](Self::require_commit_batch_num)
    /// refusal of a zero count.
    pub(crate) fn commit_cadence(&self, default: Cadence) -> Result<Cadence> {
        Ok(match self.require_commit_batch_num()? {
            Some(batches) => Cadence::Batches(
                std::num::NonZeroUsize::new(batches).expect("a zero count was refused"),
            ),
            None => default,
        })
    }

    /// Split one already-shaped stream into bounded publication readers.
    ///
    /// The caller validates write intent and shapes the stream before entering
    /// here. Every yielded reader contains exactly one complete cadence, or
    /// the final remainder after end-of-stream; `default` is the cadence
    /// where the options state no batch count, and [`Cadence::Once`] yields
    /// the stream itself once. A source error discards only the incomplete
    /// cadence it interrupted.
    pub(crate) fn commit_arrow_readers(
        &self,
        batches: crate::arrow::BatchReader,
        default: Cadence,
    ) -> Result<CommitReaders> {
        Ok(CommitReaders {
            schema: batches.schema(),
            batches: Some(batches),
            cadence: self.commit_cadence(default)?,
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
        if base == &MimeType::XMLA {
            return Ok(Self::Xmla(crate::xmla::XmlaOptions::new()));
        }
        if base == &MimeType::CSV {
            return Ok(Self::Csv(crate::csv::CsvOptions::new()));
        }
        if base == &MimeType::TSV {
            return Ok(Self::Csv(crate::csv::CsvOptions::tsv()));
        }
        if base == &MimeType::XLSX {
            return Ok(Self::Excel(crate::excel::ExcelOptions::new()));
        }
        let encodings = if cfg!(feature = "parquet") {
            "a record encoding this build implements (application/vnd.apache.arrow.stream, application/vnd.apache.parquet, application/avro, text/plain, application/xmla+xml, text/csv, text/tab-separated-values, application/vnd.openxmlformats-officedocument.spreadsheetml.sheet)"
        } else {
            "a record encoding this build implements (application/vnd.apache.arrow.stream, application/avro, text/plain, application/xmla+xml, text/csv, text/tab-separated-values, application/vnd.openxmlformats-officedocument.spreadsheetml.sheet; the `parquet` feature is not enabled)"
        };
        // A structured text document is one value around its rows, not a
        // stream of batches: it has doors of its own, and the refusal names
        // them rather than leaving a caller to guess.
        let reason = match crate::text::Format::from_mime_type(base) {
            Ok(format) => crate::text::expected_got(
                format_args!(
                    "{encodings}; a {} document is one value, read with read_arrow or \
                     read_scalar and written with write_arrow (overwrite) or write_scalar",
                    format.as_str()
                ),
                base,
            ),
            Err(_) => crate::text::expected_got(encodings, base),
        };
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason,
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
            Self::Xmla(_) => MimeType::XMLA,
            Self::Csv(options) => {
                if options.separator() == b'\t' {
                    MimeType::TSV
                } else {
                    MimeType::CSV
                }
            }
            Self::Excel(_) => MimeType::XLSX,
        }
    }

    /// The worksheet a workbook read or write addresses, or `None` for
    /// another encoding or when none is named.
    pub fn excel_sheet(&self) -> Option<&str> {
        match self {
            Self::Excel(options) => options.sheet.as_deref(),
            _ => None,
        }
    }

    /// Set or clear the worksheet a workbook read or write addresses.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-workbook variant, or a name Excel refuses
    /// ([`validate_sheet_name`](crate::excel::validate_sheet_name)).
    pub fn set_excel_sheet(&mut self, sheet: Option<&str>) -> Result<()> {
        let options = self.excel_mut("$.sheet", "a worksheet")?;
        if let Some(sheet) = sheet {
            crate::excel::validate_sheet_name(sheet)?;
        }
        options.sheet = sheet.map(SmolStr::new);
        Ok(())
    }

    /// The cells a workbook read or write addresses, or `None` for another
    /// encoding or the whole sheet.
    pub fn excel_range(&self) -> Option<crate::excel::CellRange> {
        match self {
            Self::Excel(options) => options.range,
            _ => None,
        }
    }

    /// Set or clear the cells a workbook read or write addresses.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-workbook variant.
    pub fn set_excel_range(&mut self, range: Option<crate::excel::CellRange>) -> Result<()> {
        self.excel_mut("$.range", "a cell range")?.range = range;
        Ok(())
    }

    fn excel_mut(
        &mut self,
        path: &'static str,
        setting: &'static str,
    ) -> Result<&mut crate::excel::ExcelOptions> {
        let media_type = self.mime_type();
        match self {
            Self::Excel(options) => Ok(options),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static(path),
                reason: smol_str::format_smolstr!(
                    "expected Excel options to set {setting}, got {media_type} options"
                ),
            }),
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/media/options.rs` pins and a caller cannot reach.
    //!
    //! `commit_arrow_readers` is the crate-private splitter every bounded
    //! write pulls through: it cuts a stream at the declared batch cadence,
    //! or at the byte target a destination defaults to, without reading
    //! ahead, which is a claim only a counted reader handed straight to it
    //! can make. The readers it yields come back as an opaque iterator, so
    //! the type carrying them stays as private as it was.

    use super::RecordOptions;
    use crate::Result;
    use crate::arrow::BatchReader;

    /// Split one already-shaped stream into bounded publication readers,
    /// once after the source ends where the options state no cadence.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where the declared cadence is zero.
    pub fn commit_arrow_readers(
        options: &RecordOptions,
        batches: BatchReader,
    ) -> Result<impl Iterator<Item = Result<BatchReader>>> {
        options.commit_arrow_readers(batches, super::Cadence::Once)
    }

    /// Split one already-shaped stream into bounded publication readers,
    /// by `target` bytes of held batches where the options state no
    /// cadence - what an Iceberg table and a write session default to.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where the declared cadence is zero.
    pub fn commit_arrow_readers_by_bytes(
        options: &RecordOptions,
        batches: BatchReader,
        target: u64,
    ) -> Result<impl Iterator<Item = Result<BatchReader>>> {
        options.commit_arrow_readers(batches, super::Cadence::Bytes(target))
    }

    /// Hand one file its share of a table's threads, as a table scan or
    /// commit does before it reads or writes the file.
    pub fn set_file_threads(options: &mut RecordOptions, threads: usize) {
        options.set_file_threads(threads);
    }

    /// The thread share a file's options carry, when one was handed down;
    /// `None` for an encoding that decodes on one thread already.
    #[must_use]
    pub fn file_threads(options: &RecordOptions) -> Option<usize> {
        match options {
            #[cfg(feature = "parquet")]
            RecordOptions::Parquet(options) => options.threads,
            RecordOptions::Avro(options) => options.threads.0,
            RecordOptions::Ipc(_)
            | RecordOptions::Text(_)
            | RecordOptions::Xmla(_)
            | RecordOptions::Csv(_)
            | RecordOptions::Excel(_) => None,
        }
    }
}
