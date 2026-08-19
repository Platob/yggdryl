//! Planning a read from the table's metadata, then performing it.
//!
//! **A scan never lists a directory.** The current snapshot names a manifest
//! list, the manifest list names manifests, and a manifest names data files with
//! their partition tuples and their column statistics. That chain is what
//! decides which files are opened, so a table whose `data/` folder also holds a
//! stray file, an orphan left by a failed commit, or a file an overwrite
//! replaced reads as exactly the rows its snapshot says it has.
//!
//! Each level of the chain also prunes:
//!
//! - a **manifest list row** carries one [`FieldSummary`] per partition field,
//!   so a manifest whose summary excludes a value is skipped without being read;
//! - a **manifest entry** carries the file's partition tuple, so a file outside
//!   the addressed partition is skipped without being opened;
//! - a **data file** carries per-column bounds and null counts, so a file whose
//!   statistics cannot hold the value is skipped without being opened.
//!
//! A filter is an [`Expr`](crate::Expr) - `venue = \'XNAS\' AND price BETWEEN 10
//! AND 20` - bound once against the table\'s schema and then read by every level
//! of that chain through the one
//! [`evaluate_stats`](crate::expressions::BoundPredicate::evaluate_stats). The
//! `(column, value)` pair vocabulary
//! [`IOBase::children_where`](crate::io::IOBase::children_where) filters a lake
//! with is still accepted and is sugar for one equality each.
//!
//! Each level implements [`StatsSource`] over what it happens to carry - a
//! manifest summary, a partition tuple, a data file\'s column bounds - so the
//! planner never learns how any of them is encoded, and the rows a file does
//! hold are filtered afterwards by the *same* plan, because statistics bound a
//! file and do not select a row.

use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_schema::{Schema as ArrowSchema, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use super::manifest::{DataFile, EntryStatus, ManifestContent, ManifestEntry, ManifestFile};
use super::partition::{PartitionSpec, Transform};
use super::value::single_to_value;
use crate::arrow::BatchReader;
use crate::expressions::{BoundColumn, BoundPredicate, ColumnStats, StatsSource};
use crate::field::cast::ArrowCast;
use crate::generic::Holder;
use crate::{DataType, Error, Expr, Field, Result, Value};

/// One data file a scan reads, with everything a rewrite of it would need.
#[derive(Clone, Debug)]
pub struct ScanTask {
    /// The manifest entry, with the numbers its manifest supplied filled in.
    pub entry: ManifestEntry,
    /// The spec the entry's partition tuple is written against.
    pub spec: PartitionSpec,
    /// The conjuncts this file's statistics did not settle.
    ///
    /// A conjunct the metadata proved true for every row is already gone; what
    /// is here is what the rows themselves still have to answer, as plans
    /// sharing the one graph the whole scan was bound into.
    pub residual: Vec<BoundPredicate>,
}

impl ScanTask {
    /// Borrow the data file this task reads.
    pub const fn data_file(&self) -> &DataFile {
        &self.entry.data_file
    }
}

/// The files one scan reads, and what the metadata let it leave alone.
///
/// The three lists are also what a *write* needs: what it rewrites is `tasks`,
/// and what it carries into the new snapshot untouched is `excluded` plus the
/// whole of `skipped` - a manifest nobody opened does not have to be rewritten
/// to stay true.
#[derive(Clone, Debug, Default)]
pub struct ScanPlan {
    /// The data files the scan will open, in manifest order.
    pub tasks: Vec<ScanTask>,
    /// Live files a read manifest listed that the filters excluded.
    pub excluded: Vec<ScanTask>,
    /// Manifests excluded on their manifest-list summary alone, never opened.
    pub skipped: Vec<ManifestFile>,
    /// Manifests that had to be read because their summaries allowed a match.
    pub manifests_read: usize,
}

impl ScanPlan {
    /// Return the rows the planned files hold, as the manifests counted them.
    pub fn record_count(&self) -> i64 {
        self.tasks
            .iter()
            .map(|task| task.data_file().record_count)
            .sum()
    }

    /// Return how many live data files the metadata let the scan skip.
    pub fn files_skipped(&self) -> usize {
        self.excluded.len()
    }

    /// Return how many manifests the metadata let the scan skip.
    pub fn manifests_skipped(&self) -> usize {
        self.skipped.len()
    }
}

/// What a manifest-list row knows about the files one manifest names.
///
/// A manifest carries one [`FieldSummary`] per partition field, so this answers
/// only for a column an *identity* partition field is built from: that is the
/// one transform whose partition value is the column value. A bucket, a
/// truncation, or a calendar transform stores something else, so a filter on
/// its source column falls through to the file\'s own statistics.
struct ManifestStats<'plan> {
    manifest: &'plan ManifestFile,
    spec: &'plan PartitionSpec,
}

impl StatsSource for ManifestStats<'_> {
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
        let position = identity_position(self.spec, column)?;
        // A manifest list written without summaries says nothing, so nothing
        // may be skipped on it.
        let summary = self.manifest.partitions.get(position)?;
        let data_type = column.data_type();
        Some(ColumnStats {
            lower: summary
                .lower_bound
                .as_deref()
                .and_then(|bytes| single_to_value(bytes, data_type)),
            upper: summary
                .upper_bound
                .as_deref()
                .and_then(|bytes| single_to_value(bytes, data_type)),
            // "Contains a null" is not a count, so only its absence is one.
            null_count: (!summary.contains_null).then_some(0),
            value_count: None,
        })
    }
}

/// What one data file knows about its own rows.
///
/// The partition tuple settles an identity-partitioned column exactly - every
/// row of the file holds that value - which makes it a constant column, the
/// same shape a `column=value` directory answers with. Every other column is
/// answered from the file\'s recorded bounds and counts.
struct FileStats<'plan> {
    file: &'plan DataFile,
    spec: &'plan PartitionSpec,
}

impl StatsSource for FileStats<'_> {
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
        if let Some(position) = identity_position(self.spec, column) {
            let value = self
                .file
                .partition
                .get(position)
                .cloned()
                .unwrap_or(Value::Null);
            return Some(ColumnStats::constant(value));
        }
        let id = column.field_id()?;
        let data_type = column.data_type();
        let lower =
            bound(&self.file.lower_bounds, id).and_then(|bytes| single_to_value(bytes, data_type));
        let upper =
            bound(&self.file.upper_bounds, id).and_then(|bytes| single_to_value(bytes, data_type));
        let null_count =
            lookup(&self.file.null_value_counts, id).and_then(|count| u64::try_from(count).ok());
        // A file with no recorded value count still has its row count, and the
        // two agree for a top-level column - which is what lets an all-null
        // column be recognized.
        let value_count = lookup(&self.file.value_counts, id)
            .or(Some(self.file.record_count))
            .and_then(|count| u64::try_from(count).ok());
        if lower.is_none() && upper.is_none() && null_count.is_none() {
            return None;
        }
        Some(ColumnStats {
            lower,
            upper,
            null_count,
            value_count,
        })
    }
}

/// The partition-tuple position an identity transform gives one column.
fn identity_position(spec: &PartitionSpec, column: &BoundColumn) -> Option<usize> {
    // Only a top-level column has a partition field; reaching inside a value
    // is never what a spec partitions on.
    if !column.steps().is_empty() {
        return None;
    }
    let id = column.field_id()?;
    spec.fields
        .iter()
        .position(|field| field.transform == Transform::Identity && field.source_id == id)
}

/// Read one integer statistic by field id.
fn lookup(counts: &[(i32, i64)], id: i32) -> Option<i64> {
    counts
        .iter()
        .find_map(|(key, count)| (*key == id).then_some(*count))
}

/// Read one encoded bound by field id.
fn bound(bounds: &[(i32, Vec<u8>)], id: i32) -> Option<&[u8]> {
    bounds
        .iter()
        .find_map(|(key, bytes)| (*key == id).then_some(bytes.as_slice()))
}

/// Bind one filter expression against a table schema.
///
/// A column is a claim here and a value is text, which is the split a table
/// route has always had: naming a column the schema does not declare is an
/// error listing what it does declare, while a value the column\'s type cannot
/// read makes the comparison match nothing rather than failing the scan.
///
/// # Errors
///
/// Returns an error naming the columns the schema does have when the filter
/// names one it does not.
pub(super) fn filter_predicate(schema: &Field, filter: &Expr) -> Result<BoundPredicate> {
    // The column check runs here rather than being left to the binder so the
    // message stays the table's own: a caller filtering a *table* is told
    // which columns the table declares, in the table's words.
    for name in filter.columns() {
        if schema
            .fields()
            .iter()
            .any(|field| field.name().eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let columns = schema
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(invalid(format_smolstr!(
            "expected a filter column the table schema declares, got {name:?}; it has {}",
            crate::text::elide_display(&columns),
        )));
    }
    filter.bind_declared(schema)?.into_predicate()
}

/// Bind the `(column, value)` pair vocabulary as the expression it means.
///
/// # Errors
///
/// Returns exactly what [`filter_predicate`] returns.
pub(super) fn filters(schema: &Field, pairs: &[(&str, &str)]) -> Result<BoundPredicate> {
    filter_predicate(schema, &crate::io::partition::pairs_to_expr(pairs))
}

/// Plan a scan of `manifests`, opening only what the summaries allow.
///
/// `manifest_at` resolves a recorded manifest location into a handle, which is
/// the table's business rather than the planner's - everything here works
/// through what that closure returns.
///
/// # Errors
///
/// Returns an error when a manifest that had to be read cannot be reached or
/// decoded.
pub(super) fn plan(
    manifests: &[ManifestFile],
    spec_of: &dyn Fn(i32) -> PartitionSpec,
    manifest_at: &dyn Fn(&str) -> Result<Holder>,
    predicate: &BoundPredicate,
    for_read: bool,
) -> Result<ScanPlan> {
    let mut plan = ScanPlan::default();
    let selective = !predicate.is_always_true();
    for manifest in manifests {
        if manifest.content != ManifestContent::Data {
            continue;
        }
        let spec = spec_of(manifest.partition_spec_id);
        // The cheapest level: the summary is a row of the manifest list, so a
        // manifest refuted here is never opened at all.
        let summaries = ManifestStats {
            manifest,
            spec: &spec,
        };
        if !predicate.evaluate_stats(&summaries).is_possible() {
            plan.skipped.push(manifest.clone());
            continue;
        }
        plan.manifests_read += 1;

        let handle = manifest_at(&manifest.manifest_path)?;
        // A read-only plan decodes just the columns pruning consults; a plan
        // whose entries may be carried into a rewritten manifest decodes
        // everything, because a carried entry must keep its statistics.
        let entries = if for_read {
            super::manifest::read_manifest_for_plan(&handle, selective)?
        } else {
            super::manifest::read_manifest(&handle)?
        };
        for mut entry in entries {
            if entry.status == EntryStatus::Deleted {
                continue;
            }
            entry.inherit(manifest);
            // A file with no rows is neither read nor carried: reading it would
            // cost a footer for an empty batch, and keeping it would carry a
            // name that holds nothing.
            if entry.data_file.record_count == 0 {
                continue;
            }
            // The file's own statistics answer next, and what they do not
            // settle is carried forward as the residual the rows answer.
            let statistics = FileStats {
                file: &entry.data_file,
                spec: &spec,
            };
            let residual = predicate.residual(&statistics);
            let task = ScanTask {
                entry,
                spec: spec.clone(),
                residual: residual.clone().unwrap_or_default(),
            };
            match residual {
                Some(_) => plan.tasks.push(task),
                // The statistics proved a conjunct false for every row this
                // file holds, so it is never opened.
                None => plan.excluded.push(task),
            }
        }
    }
    Ok(plan)
}

/// One data file a scan still has to open.
pub(super) struct ScanPart {
    /// The handle addressing the data file.
    pub(super) handle: Holder,
    /// The file size the manifest recorded, which sizes the parallel decision.
    pub(super) size: i64,
    /// Partition columns to restore, when the file does not store them.
    pub(super) partition: Vec<(Field, Value)>,
    /// The conjuncts this file's rows still have to be tested against.
    pub(super) residual: Vec<BoundPredicate>,
}

/// The data file a scan is currently reading, and what it still has to apply.
struct Open {
    /// The file's own reader.
    reader: BatchReader,
    /// Partition columns to restore, when the file does not store them.
    partition: Vec<(Field, Value)>,
    /// The conjuncts this file's rows still have to be tested against.
    residual: Vec<BoundPredicate>,
}

/// Everything a decoded batch still goes through, shared by both read paths.
///
/// The sequential reader borrows one of these; each parallel worker holds the
/// same one behind an [`Arc`], so the two paths cannot drift apart in how a
/// batch is restored, cast, filtered, and projected.
struct Refine {
    /// The root each file is read and filtered as.
    read_root: Field,
    /// The root every batch is finally cast to.
    root: Field,
    /// Whether the read root holds columns the scan root does not.
    project: bool,
    /// The column pushdown handed to each file, when the caller gave one.
    target: Option<Field>,
}

impl Refine {
    /// Resolve the options one data file is read under.
    ///
    /// The read root is handed down as the file's declared schema, which is what
    /// makes it the encoding's own projection mask. The partition columns are
    /// removed from it first: the file need not store them, so asking for them
    /// here would fill them with nulls and hide the manifest values
    /// [`restore_partitions`] is about to put back.
    fn file_options(&self, part: &ScanPart) -> Result<crate::generic::RecordOptions> {
        use crate::generic::IORecordOptions;
        use crate::io::IOBase;

        let options = part.handle.record_options()?;
        if self.target.is_none() {
            // Nothing was asked for, so nothing is pushed down and the file's
            // own columns come back as they are.
            return Ok(options);
        }
        let columns: Vec<&str> = part
            .partition
            .iter()
            .map(|(field, _)| field.name())
            .collect();
        match self.read_root.without_fields(&columns) {
            Ok(stored) if stored.field_len() > 0 => {
                let projected = file_projection(&part.handle, &options, &stored);
                Ok(options.with_schema(projected))
            }
            // A read root that is nothing but partition columns leaves the file
            // read unprojected; there is no column left to ask it for.
            _ => Ok(options),
        }
    }

    /// Open one planned file with its resolved options.
    fn open(&self, part: &ScanPart) -> Result<BatchReader> {
        let options = self.file_options(part)?;
        crate::io::IOBase::read_arrow_batch_reader(&part.handle, &options)
    }

    /// Restore, align, cast, filter, and project one decoded batch.
    fn batch(
        &self,
        batch: &RecordBatch,
        partition: &[(Field, Value)],
        residual: &[BoundPredicate],
    ) -> std::result::Result<RecordBatch, arrow_schema::ArrowError> {
        restore_partitions(batch, partition)
            .and_then(|batch| align_by_field_id(batch, &self.read_root))
            .and_then(|batch| Ok(self.read_root.cast_arrow_batch(batch, false)?))
            .and_then(|batch| apply_filters(batch, residual))
            .and_then(|batch| {
                if self.project {
                    return Ok(self.root.cast_arrow_batch(batch, false)?);
                }
                Ok(batch)
            })
            .map_err(scan_error)
    }
}

/// A reader over every data file one plan selected, one file at a time.
struct Scan {
    /// The files not yet opened.
    parts: std::vec::IntoIter<ScanPart>,
    /// The file currently being read, with what it still has to apply.
    current: Option<Open>,
    /// The Arrow projection of the scan root.
    schema: SchemaRef,
    /// The shared per-batch pipeline.
    refine: Arc<Refine>,
}

/// Build the reader over one set of planned files.
///
/// `root` is what the reader reports and every batch is cast to; the read root
/// is that plus whatever the filters need, so a column a filter tests is read
/// and then dropped rather than left out of the pushdown.
///
/// The reader decodes files in parallel when the plan is worth it: at least
/// [`ReadSettings::min_files`] of the planned files carry a recorded
/// `file_size_in_bytes` of [`ReadSettings::min_file_size_bytes`] or more -
/// smaller files do not count toward justifying threads - and
/// [`ReadSettings::parallelism`] allows at least two. Below any of those the
/// strictly sequential single-open path answers. Either way the batches come
/// back in exactly the plan's file order.
///
/// # Errors
///
/// Returns an error when either root cannot be projected into Arrow.
pub(super) fn reader(
    parts: Vec<ScanPart>,
    root: Field,
    read_root: Field,
    target: Option<Field>,
    parallel: &super::options::ReadSettings,
) -> Result<BatchReader> {
    let schema = crate::arrow::schema_from_field(&root)?;
    let refine = Arc::new(Refine {
        project: read_root.field_len() != root.field_len(),
        read_root,
        root,
        target,
    });
    let qualifying = parts
        .iter()
        .filter(|part| {
            u64::try_from(part.size).is_ok_and(|size| size >= parallel.min_file_size_bytes)
        })
        .count();
    if parallel.parallelism >= 2 && parts.len() > 1 && qualifying >= parallel.min_files {
        return Ok(Box::new(ParallelScan::new(
            parts,
            schema,
            refine,
            parallel.parallelism,
        )));
    }
    Ok(Box::new(Scan {
        parts: parts.into_iter(),
        current: None,
        schema,
        refine,
    }))
}

impl Iterator for Scan {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(open) = self.current.as_mut() {
                match open.reader.next() {
                    Some(Ok(batch)) => {
                        return Some(self.refine.batch(&batch, &open.partition, &open.residual));
                    }
                    Some(Err(error)) => return Some(Err(error)),
                    None => self.current = None,
                }
            }
            let part = self.parts.next()?;
            match self.refine.open(&part) {
                Ok(reader) => {
                    self.current = Some(Open {
                        reader,
                        partition: part.partition,
                        residual: part.residual,
                    });
                }
                Err(error) => return Some(Err(scan_error(error))),
            }
        }
    }
}

impl arrow_array::RecordBatchReader for Scan {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// One worker-to-consumer message: a refined batch of one file, or `None` to
/// say that file has no more.
type PartMessage = (
    usize,
    Option<std::result::Result<RecordBatch, arrow_schema::ArrowError>>,
);

/// What the consumer holds of one file that is ahead of the release cursor.
#[derive(Default)]
struct PartState {
    /// Refined batches received and not yet released, in decode order.
    batches: std::collections::VecDeque<std::result::Result<RecordBatch, arrow_schema::ArrowError>>,
    /// Whether the file's worker has sent everything it will.
    done: bool,
}

/// A reader that decodes several planned files at once, releasing their
/// batches strictly in plan order.
///
/// Memory is bounded by a sliding window: at most `window` files - the
/// resolved read parallelism - are ever in flight, because a new worker is
/// spawned only when the release cursor finishes a file, so the channel and
/// the reorder buffer together hold at most that many files' batches.
///
/// **Dropping the reader detaches the workers rather than joining them.** A
/// worker owns everything it touches - its handle, its `Arc` of the shared
/// pipeline, its sender - so nothing borrowed outlives the drop; the dropped
/// receiver disconnects the channel, and each worker exits at its next send
/// instead of decoding a file nobody wants. Joining here would make dropping
/// a reader block on a decode already in progress, which is worse than
/// letting one finish its current batch quietly.
struct ParallelScan {
    /// The Arrow projection of the scan root.
    schema: SchemaRef,
    /// The shared per-batch pipeline every worker applies.
    refine: Arc<Refine>,
    /// The files not yet handed to a worker, in plan order.
    jobs: std::vec::IntoIter<ScanPart>,
    /// The plan position the next spawned worker will decode.
    spawned: usize,
    /// Per-file reorder state, indexed by plan position.
    states: Vec<PartState>,
    /// The plan position whose batches are being released.
    released: usize,
    /// How many files may be in flight at once.
    window: usize,
    /// The senders' origin, kept so late workers can be given one.
    sender: std::sync::mpsc::Sender<PartMessage>,
    /// Where every worker's batches arrive.
    receiver: std::sync::mpsc::Receiver<PartMessage>,
}

impl ParallelScan {
    /// Start the first window of workers over the planned files.
    fn new(parts: Vec<ScanPart>, schema: SchemaRef, refine: Arc<Refine>, window: usize) -> Self {
        let total = parts.len();
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut scan = Self {
            schema,
            refine,
            jobs: parts.into_iter(),
            spawned: 0,
            states: (0..total).map(|_| PartState::default()).collect(),
            released: 0,
            window: window.max(1),
            sender,
            receiver,
        };
        for _ in 0..scan.window {
            scan.spawn_next();
        }
        scan
    }

    /// Hand the next unclaimed file to a fresh worker thread, if any remains.
    fn spawn_next(&mut self) {
        let Some(part) = self.jobs.next() else {
            return;
        };
        let index = self.spawned;
        self.spawned += 1;
        let refine = Arc::clone(&self.refine);
        let sender = self.sender.clone();
        // Deliberately detached: see the type docs for why drop does not join.
        let _ = std::thread::spawn(move || read_part(index, &part, &refine, &sender));
    }
}

impl Iterator for ParallelScan {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.released >= self.states.len() {
                return None;
            }
            if let Some(result) = self.states[self.released].batches.pop_front() {
                return Some(result);
            }
            if self.states[self.released].done {
                // The cursor file is drained, so the window has room for one
                // more worker.
                self.released += 1;
                self.spawn_next();
                continue;
            }
            match self.receiver.recv() {
                Ok((index, Some(result))) => {
                    if let Some(state) = self.states.get_mut(index) {
                        state.batches.push_back(result);
                    }
                }
                Ok((index, None)) => {
                    if let Some(state) = self.states.get_mut(index) {
                        state.done = true;
                    }
                }
                // Unreachable while this reader holds a sender; kept so a bug
                // reads as an error rather than an infinite wait.
                Err(_) => {
                    return Some(Err(scan_error(invalid(SmolStr::new_static(
                        "expected a decode worker to answer, got a closed channel",
                    )))));
                }
            }
        }
    }
}

impl arrow_array::RecordBatchReader for ParallelScan {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Decode one planned file on a worker thread.
///
/// Every batch is refined here, in the worker, so the parallelism covers the
/// cast-and-filter work and not only the decode; each is sent as
/// `(file_index, batch)` and a final `(file_index, None)` says the file is
/// finished. Every send doubles as the liveness check: a dropped reader
/// disconnects the channel and the worker returns at its next send. The body
/// is unwind-guarded so a panicking decode reports an error instead of
/// leaving the consumer waiting for a marker that would never come.
fn read_part(
    index: usize,
    part: &ScanPart,
    refine: &Refine,
    sender: &std::sync::mpsc::Sender<PartMessage>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = match refine.open(part) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = sender.send((index, Some(Err(scan_error(error)))));
                return;
            }
        };
        for batch in reader {
            let produced = match batch {
                Ok(batch) => refine.batch(&batch, &part.partition, &part.residual),
                Err(error) => Err(error),
            };
            if sender.send((index, Some(produced))).is_err() {
                return;
            }
        }
    }));
    if outcome.is_err() {
        let _ = sender.send((
            index,
            Some(Err(scan_error(invalid(SmolStr::new_static(
                "expected the file to decode, got a panicking reader thread",
            ))))),
        ));
    }
    let _ = sender.send((index, None));
}

/// Keep only the rows every residual conjunct accepts.
///
/// The conjuncts are applied one after another rather than combined, because
/// two masks applied in turn select exactly the rows their conjunction would
/// and each one narrows what the next has to test. Each is the same plan the
/// statistics were evaluated against, so a row a file was kept for and a row a
/// file was skipped for can never be judged by two different rules.
fn apply_filters(batch: RecordBatch, residual: &[BoundPredicate]) -> Result<RecordBatch> {
    let mut batch = batch;
    for predicate in residual {
        batch = predicate
            .filter_batch(&batch)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
    }
    Ok(batch)
}

/// Report a scan failure through the reader's own error type.
pub(super) fn scan_error(error: Error) -> arrow_schema::ArrowError {
    arrow_schema::ArrowError::ExternalError(Box::new(error))
}

/// Translate a projection into the names one data file spells them with.
///
/// Iceberg resolves a column by field identifier, not by name, so a file
/// written before a rename stores the column under its old name. The file's
/// own root is read - a footer-only read - and every projected column whose
/// identifier the file spells under a different name is asked for by *that*
/// name, so the encoding's pushdown still skips what it should. The decoded
/// batch is renamed back by [`align_by_field_id`] before the final cast.
///
/// A file whose schema cannot be read, or that carries no identifiers, keeps
/// the name-based projection: nothing is worse than before, and the read
/// itself will say what is wrong with the file.
fn file_projection(
    handle: &Holder,
    options: &crate::generic::RecordOptions,
    wanted: &Field,
) -> Field {
    let Ok(file_root) = crate::io::IOBase::read_arrow_field(handle, options) else {
        return wanted.clone();
    };
    let mut children: Vec<Field> = Vec::with_capacity(wanted.field_len());
    let mut renamed = false;
    for child in wanted.fields() {
        let Ok(Some(id)) = child.parquet_field_id() else {
            children.push(child.clone());
            continue;
        };
        let stored_name = file_root
            .fields()
            .iter()
            .find(|candidate| matches!(candidate.parquet_field_id(), Ok(Some(candidate_id)) if candidate_id == id))
            .map(|candidate| candidate.name().to_owned());
        match stored_name {
            Some(name) if name != child.name() => {
                renamed = true;
                children.push(child.clone().with_name(name));
            }
            _ => children.push(child.clone()),
        }
    }
    if !renamed {
        return wanted.clone();
    }
    DataType::from_fields(children)
        .and_then(|data_type| {
            Field::from_parts(
                wanted.name(),
                data_type,
                wanted.is_nullable(),
                wanted.metadata_iter(),
            )
        })
        .unwrap_or_else(|_| wanted.clone())
}

/// Rename decoded columns to the read root's names, matched by field id.
///
/// This is the read half of Iceberg's id-based column resolution: a file
/// written before a rename decodes under its old column names, and each of
/// its columns whose `PARQUET:field_id` matches a read-root column is renamed
/// to what the schema calls it now, so the cast that follows sees the column
/// rather than inventing a null one. A column without an identifier - or one
/// the read root does not declare - keeps its name.
fn align_by_field_id(batch: RecordBatch, read_root: &Field) -> Result<RecordBatch> {
    let by_id: Vec<(i32, &Field)> = read_root
        .fields()
        .iter()
        .filter_map(|child| {
            child
                .parquet_field_id()
                .ok()
                .flatten()
                .map(|id| (id, child))
        })
        .collect();
    if by_id.is_empty() {
        return Ok(batch);
    }
    let schema = batch.schema();
    let mut changed = false;
    let mut fields: Vec<Arc<arrow_schema::Field>> = Vec::with_capacity(schema.fields().len());
    for field in schema.fields() {
        let id = field
            .metadata()
            .get(crate::metadata::PARQUET_FIELD_ID_KEY)
            .and_then(|text| text.trim().parse::<i32>().ok());
        let target = id
            .and_then(|id| by_id.iter().find(|(candidate, _)| *candidate == id))
            .filter(|(_, target)| target.name() != field.name());
        match target {
            Some((_, target)) => {
                changed = true;
                fields.push(Arc::new(field.as_ref().clone().with_name(target.name())));
            }
            None => fields.push(Arc::clone(field)),
        }
    }
    if !changed {
        return Ok(batch);
    }
    let schema = Arc::new(ArrowSchema::new(fields).with_metadata(schema.metadata().clone()));
    RecordBatch::try_new(schema, batch.columns().to_vec()).map_err(Error::Arrow)
}

/// Add the partition columns a data file left out, typed as declared.
fn restore_partitions(batch: &RecordBatch, partition: &[(Field, Value)]) -> Result<RecordBatch> {
    let missing: Vec<&(Field, Value)> = partition
        .iter()
        .filter(|(field, _)| batch.schema().index_of(field.name()).is_err())
        .collect();
    if missing.is_empty() {
        return Ok(batch.clone());
    }

    let mut fields: Vec<Arc<arrow_schema::Field>> =
        batch.schema().fields().iter().map(Arc::clone).collect();
    let mut columns = batch.columns().to_vec();
    for (field, value) in missing {
        let scalar = crate::arrow::scalar_array(field, value)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        columns.push(repeat(&scalar, batch.num_rows())?);
        fields.push(field.to_arrow_ref()?);
    }
    let schema =
        Arc::new(ArrowSchema::new(fields).with_metadata(batch.schema().metadata().clone()));
    RecordBatch::try_new(schema, columns).map_err(Error::Arrow)
}

/// Repeat one value into a column of `rows` rows.
fn repeat(value: &ArrayRef, rows: usize) -> Result<ArrayRef> {
    let indices = UInt32Array::from(vec![0_u32; rows]);
    arrow_select::take::take(value.as_ref(), &indices, None).map_err(Error::Arrow)
}

/// Pair each identity partition column's Field with the manifest's value.
///
/// Only [`Transform::Identity`] restores a column: its partition value *is*
/// the column value, so a data file that left the column out reads it back
/// from the manifest. A transformed field - `days(at)`, `bucket(4, id)` -
/// stores a derived value under a name that is not a schema column at all,
/// so restoring it would add a column no reader asked for; the source column
/// itself is stored in the data file, as Spark stores it.
pub(super) fn partition_columns(
    spec: &PartitionSpec,
    schema: &Field,
    file: &DataFile,
) -> Result<Vec<(Field, Value)>> {
    let partition = spec.partition_field(schema)?;
    Ok(partition
        .fields()
        .iter()
        .zip(file.partition.iter())
        .zip(spec.fields.iter())
        .filter(|(_, spec_field)| spec_field.transform == Transform::Identity)
        .map(|((field, value), _)| (field.clone(), value.clone()))
        .collect())
}

/// Return the root a file is read as: the scan's own root plus what it filters.
///
/// A filter may name a column the caller never asked for, and the rows still
/// have to be tested against it, so the column is read and then dropped by the
/// final cast rather than left out of the pushdown.
pub(super) fn read_root(root: &Field, schema: &Field, predicate: &BoundPredicate) -> Result<Field> {
    let mut children: Vec<Field> = root.fields().to_vec();
    for name in predicate.columns() {
        if children
            .iter()
            .any(|child| child.name().eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let Some(column) = schema.get_field_by_name(&name) else {
            continue;
        };
        children.push(column.clone());
    }
    if children.len() == root.field_len() {
        return Ok(root.clone());
    }
    Field::from_parts(
        root.name(),
        DataType::from_fields(children)?,
        root.is_nullable(),
        root.metadata_iter(),
    )
}

/// Report a scan a table's metadata cannot describe.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
