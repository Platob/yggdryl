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
//! A filter is an [`Expression`], the same one that filters a lake through
//! [`IOBase::children_matching`](crate::IOBase::children_matching) and a
//! batch through [`Bound::filter`](crate::expression::Bound::filter). Each
//! level of the chain answers it from the statistics it carries, expressed as
//! the [`Bounds`] every other container in this crate expresses them as: a
//! partition tuple is a minimum equal to its maximum, so a conjunct it proves
//! is dropped rather than re-tested, and a file's own path answers every free
//! `&holder.*` attribute. What no level settles is filtered row by row after
//! the file is read, because a statistic bounds a *file* and does not select a
//! row.

use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_schema::{Schema as ArrowSchema, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use super::manifest::{DataFile, EntryStatus, ManifestContent, ManifestEntry, ManifestFile};
use super::partition::{PartitionSpec, Transform};
use super::value::single_to_value;
use crate::arrow::BatchReader;
use crate::expression::{Bound, Bounds, Selector};
use crate::holder::Holder;
use crate::types::cast::ArrowCast;
use crate::{DataType, Error, Expression, Field, Result, Scalar};

/// One data file a scan reads, with everything a rewrite of it would need.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScanTask {
    /// The manifest entry, with the numbers its manifest supplied filled in.
    pub entry: ManifestEntry,
    /// The spec the entry's partition tuple is written against.
    pub spec: PartitionSpec,
    /// The filters this file's partition tuple did not already settle.
    pub residual: Vec<usize>,
}

impl ScanTask {
    /// Return a deterministic hash of this complete executable task.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

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
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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
    /// Return a deterministic hash of the ordered tasks and pruning report.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Return the rows the planned files hold, as the manifests counted them.
    ///
    /// # Errors
    ///
    /// Returns an error when a manifest carries a negative count or the total
    /// does not fit in a signed 64-bit integer.
    pub fn record_count(&self) -> Result<i64> {
        self.tasks
            .iter()
            .enumerate()
            .try_fold(0_i64, |total, (index, task)| {
                let count = task.data_file().record_count;
                let path = format_smolstr!("$.tasks[{index}].data_file.record_count");
                if count < 0 {
                    return Err(Error::InvalidRecord {
                        path,
                        reason: SmolStr::new_static(
                            "expected an Iceberg manifest record count to be non-negative",
                        ),
                    });
                }
                total
                    .checked_add(count)
                    .ok_or_else(|| Error::InvalidRecord {
                        path,
                        reason: SmolStr::new_static(
                            "planned Iceberg record count does not fit in i64",
                        ),
                    })
            })
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

/// Resolve one predicate into the conjuncts a scan prunes and filters with.
///
/// A scan tests conjuncts rather than one expression because pruning is
/// per-conjunct: a file whose partition tuple settles the first conjunct still
/// has to test the second against its rows, and the list is what carries that
/// distinction from the planner to the reader.
///
/// # Errors
///
/// Returns an error when the predicate names a column the schema does not
/// declare, or two operands that share no type.
pub(super) fn conjuncts(schema: &Field, filter: &Expression) -> Result<Vec<Bound>> {
    filter
        .conjuncts()
        .iter()
        .map(|conjunct| conjunct.bind(schema))
        .collect()
}

/// The column a partition field is the identity of, with its own datatype.
///
/// Only [`Transform::Identity`] answers: it is the one transform whose
/// partition value *is* the column value, so every row of a file whose tuple
/// holds it holds it. A bucket, a truncation, or a calendar transform stores
/// something else, so a predicate on its source column falls through to the
/// file's own statistics and then to the rows themselves.
fn identity_column<'schema>(
    spec: &PartitionSpec,
    position: usize,
    schema: &'schema Field,
) -> Option<&'schema Field> {
    let source = spec.fields.get(position)?;
    if source.transform != Transform::Identity {
        return None;
    }
    schema
        .fields()
        .iter()
        .find(|column| column.parquet_field_id().ok().flatten() == Some(source.source_id))
}

/// The statistics a manifest-list row states about the files it names.
///
/// This is the cheapest level there is: a summary is one row of the manifest
/// list, so a manifest ruled out here is never opened at all. Only identity
/// partition fields contribute, because only they bound a schema column.
pub(super) fn manifest_bounds(
    manifest: &ManifestFile,
    spec: &PartitionSpec,
    schema: &Field,
) -> Bounds {
    let mut bounds = Bounds::new(None);
    for (position, summary) in manifest.partitions.iter().enumerate() {
        let Some(column) = identity_column(spec, position, schema) else {
            continue;
        };
        let dtype = column.dtype();
        let decode = |held: &Option<Vec<u8>>| {
            held.as_deref()
                .and_then(|bytes| single_to_value(bytes, dtype))
        };
        let (minimum, maximum) = (decode(&summary.lower_bound), decode(&summary.upper_bound));
        // A partition column is spelled in the path too, so the summary bounds
        // it as an attribute as well - but only when the two ends meet. A range
        // of values does not bound the *text* of those values, because text
        // does not order the way a number does.
        if let (Some(low), Some(high)) = (&minimum, &maximum) {
            if low == high {
                let text = Scalar::from(super::value::scalar_text(low).as_str());
                bounds = bounds.with_attribute(
                    Selector::Partition(column.name().into()),
                    Some(text.clone()),
                    Some(text),
                    Some(0),
                );
            }
        }
        bounds = bounds.with_column(
            column.name(),
            minimum,
            maximum,
            // A summary says whether a null is present, never how many, so the
            // only count it can state is zero.
            (!summary.contains_null).then_some(0),
        );
    }
    bounds
}

/// The statistics a manifest entry states about one data file.
///
/// The partition tuple is the tighter of the two sources and wins: a value
/// every row of the file shares is a minimum equal to its maximum, which is
/// what lets a partition predicate settle a file outright rather than merely
/// fail to rule it out.
pub(super) fn file_bounds(file: &DataFile, spec: &PartitionSpec, schema: &Field) -> Bounds {
    let rows = u64::try_from(file.record_count).ok();
    let mut bounds = Bounds::new(rows);
    // The file's own path answers every free holder attribute exactly, so a
    // predicate about the file - its name, its extension, its partition
    // directories - is settled here without opening it.
    if let Ok(url) = crate::Url::from_str(&file.file_path) {
        bounds = bounds.with(Bounds::from_url(&url));
    }
    let mut settled: Vec<&str> = Vec::new();
    for position in 0..spec.fields.len() {
        let Some(column) = identity_column(spec, position, schema) else {
            continue;
        };
        let value = file
            .partition
            .get(position)
            .cloned()
            .unwrap_or(Scalar::Null);
        settled.push(column.name());
        if value.is_null() {
            bounds = bounds.with_column(column.name(), None, None, rows);
            continue;
        }
        // The manifest is the authority on the value, and a path spells the
        // same one, so both spellings are recorded from the same source.
        let text = Scalar::from(super::value::scalar_text(&value).as_str());
        bounds = bounds
            .with_attribute(
                Selector::Partition(column.name().into()),
                Some(text.clone()),
                Some(text),
                Some(0),
            )
            .with_column(column.name(), Some(value.clone()), Some(value), Some(0));
    }
    for column in schema.fields() {
        if settled.contains(&column.name()) {
            continue;
        }
        let Ok(Some(id)) = column.parquet_field_id() else {
            continue;
        };
        let dtype = column.dtype();
        let decode = |bytes: Option<&[u8]>| bytes.and_then(|bytes| single_to_value(bytes, dtype));
        let nulls = lookup(&file.null_value_counts, id).and_then(|count| u64::try_from(count).ok());
        let minimum = decode(bound(&file.lower_bounds, id));
        let maximum = decode(bound(&file.upper_bounds, id));
        if minimum.is_none() && maximum.is_none() && nulls.is_none() {
            continue;
        }
        bounds = bounds.with_column(column.name(), minimum, maximum, nulls);
    }
    bounds
}

/// Return which conjuncts a file's statistics leave for its rows to answer.
///
/// `None` means the file cannot hold a matching row at all. Otherwise the
/// answer is the conjuncts the statistics did not settle outright - a conjunct
/// every row provably satisfies is dropped here rather than re-tested per row.
fn file_residual(bounds: &Bounds, conjuncts: &[Bound]) -> Option<Vec<usize>> {
    let mut residual = Vec::new();
    for (position, conjunct) in conjuncts.iter().enumerate() {
        match conjunct.statistics_certainty(bounds) {
            Some(false) => return None,
            Some(true) => {}
            // A conjunct the rows cannot answer - one that asks only about the
            // file - has had its only chance here. Unsettled, it keeps the file
            // rather than being handed to a row filter that would answer
            // unknown for every row and drop them all.
            None if !conjunct.reads_rows() => {}
            None => residual.push(position),
        }
    }
    Some(residual)
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
    spec_of: &dyn Fn(i32) -> Result<PartitionSpec>,
    manifest_at: &dyn Fn(&str) -> Result<Holder>,
    conjuncts: &[Bound],
    schema: &Field,
    for_read: bool,
) -> Result<ScanPlan> {
    let mut plan = ScanPlan::default();
    for manifest in manifests {
        if manifest.content == ManifestContent::Deletes {
            if manifest.added_files_count == Some(0) && manifest.existing_files_count == Some(0) {
                continue;
            }
            return Err(unsupported_deletes(
                &manifest.manifest_path,
                "delete manifest",
            ));
        }
        let spec = spec_of(manifest.partition_spec_id)?;
        let summary = manifest_bounds(manifest, &spec, schema);
        if !conjuncts
            .iter()
            .all(|conjunct| conjunct.statistics_prune(&summary))
        {
            plan.skipped.push(manifest.clone());
            continue;
        }
        plan.manifests_read += 1;

        let handle = manifest_at(&manifest.manifest_path)?;
        // A read-only plan decodes just the columns pruning consults; a plan
        // whose entries may be carried into a rewritten manifest decodes
        // everything, because a carried entry must keep its statistics.
        let entries = if for_read {
            super::manifest::read_manifest_for_plan(&handle, !conjuncts.is_empty())?
        } else {
            super::manifest::read_manifest(&handle)?
        };
        let mut next_row_id = manifest.first_row_id;
        for mut entry in entries {
            validate_data_entry(&entry, manifest, &spec)?;
            if entry.status == EntryStatus::Deleted {
                continue;
            }
            entry.inherit(manifest)?;
            inherit_first_row_id(&mut entry, &mut next_row_id)?;
            // A file with no rows is neither read nor carried: reading it would
            // cost a footer for an empty batch, and keeping it would carry a
            // name that holds nothing.
            if entry.data_file.record_count == 0 {
                continue;
            }
            let matched = file_residual(&file_bounds(&entry.data_file, &spec, schema), conjuncts);
            let task = ScanTask {
                entry,
                spec: spec.clone(),
                residual: matched.clone().unwrap_or_default(),
            };
            if matched.is_some() {
                plan.tasks.push(task);
            } else {
                plan.excluded.push(task);
            }
        }
    }
    Ok(plan)
}

/// Fill one null data-file row id from its v3 manifest range.
fn inherit_first_row_id(entry: &mut ManifestEntry, next_row_id: &mut Option<i64>) -> Result<()> {
    if entry.data_file.first_row_id.is_some() {
        return Ok(());
    }
    let Some(first_row_id) = *next_row_id else {
        return Ok(());
    };
    let record_count = entry.data_file.record_count;
    if first_row_id < 0 || record_count < 0 {
        return Err(invalid(format_smolstr!(
            "expected non-negative row lineage for {:?}, got first_row_id {first_row_id} and record_count {record_count}",
            entry.data_file.file_path
        )));
    }
    let next = first_row_id.checked_add(record_count).ok_or_else(|| {
        invalid(format_smolstr!(
            "row id overflow for {:?}: {first_row_id} + {record_count}",
            entry.data_file.file_path
        ))
    })?;
    entry.data_file.first_row_id = Some(first_row_id);
    *next_row_id = Some(next);
    Ok(())
}

/// Reject manifest rows a data scan cannot interpret without changing rows.
fn validate_data_entry(
    entry: &ManifestEntry,
    manifest: &ManifestFile,
    spec: &PartitionSpec,
) -> Result<()> {
    match entry.data_file.content {
        0 => {}
        1 => {
            return Err(unsupported_deletes(
                &manifest.manifest_path,
                "position-delete data file",
            ));
        }
        2 => {
            return Err(unsupported_deletes(
                &manifest.manifest_path,
                "equality-delete data file",
            ));
        }
        content => {
            return Err(invalid(format_smolstr!(
                "expected data-file content 0 in data manifest {:?}, got {content}",
                manifest.manifest_path
            )));
        }
    }
    if entry.data_file.partition.len() != spec.fields.len() {
        return Err(invalid(format_smolstr!(
            "expected {} partition values for spec {}, got {} in {:?}",
            spec.fields.len(),
            spec.spec_id,
            entry.data_file.partition.len(),
            entry.data_file.file_path
        )));
    }
    Ok(())
}

/// Report row-delete semantics this scan does not yet apply.
fn unsupported_deletes(path: &str, kind: &'static str) -> Error {
    Error::iceberg(format_smolstr!(
        "{kind} row application is not supported by yggdryl scans (manifest: {path})"
    ))
}

/// One data file a scan still has to open.
pub(super) struct ScanPart {
    /// The handle addressing the data file.
    pub(super) handle: Holder,
    /// The file size the manifest recorded, which sizes the parallel decision.
    pub(super) size: i64,
    /// Partition columns to restore, when the file does not store them.
    pub(super) partition: Vec<(Field, Scalar)>,
    /// The filters this file's rows still have to be tested against.
    pub(super) residual: Vec<usize>,
}

/// The data file a scan is currently reading, and what it still has to apply.
struct Open {
    /// The file's own reader.
    reader: BatchReader,
    /// Partition columns to restore, when the file does not store them.
    partition: Vec<(Field, Scalar)>,
    /// The filters this file's rows still have to be tested against.
    residual: Vec<usize>,
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
    /// The conjuncts, indexed by every part's residual list.
    predicates: Vec<Bound>,
}

impl Refine {
    /// Resolve the options one data file is read under.
    ///
    /// The read root is handed down as the file's declared schema, which is what
    /// makes it the encoding's own projection mask. The partition columns are
    /// removed from it first: the file need not store them, so asking for them
    /// here would fill them with nulls and hide the manifest values
    /// [`restore_partitions`] is about to put back.
    fn file_options(&self, part: &ScanPart) -> Result<crate::media::RecordOptions> {
        use crate::IOMedia;
        use crate::media::IORecordOptions;

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
                Ok(options.with_field(projected))
            }
            // A read root that is nothing but partition columns leaves the file
            // read unprojected; there is no column left to ask it for.
            _ => Ok(options),
        }
    }

    /// Open one planned file with its resolved options.
    fn open(&self, part: &ScanPart) -> Result<BatchReader> {
        let options = self.file_options(part)?;
        crate::IOMedia::read_arrow_reader(&part.handle, &options)
    }

    /// Restore, align, cast, filter, and project one decoded batch.
    fn batch(
        &self,
        batch: &RecordBatch,
        partition: &[(Field, Scalar)],
        residual: &[usize],
    ) -> std::result::Result<RecordBatch, arrow_schema::ArrowError> {
        restore_partitions(batch, partition)
            .and_then(|batch| align_by_field_id(batch, &self.read_root))
            .and_then(|batch| Ok(self.read_root.cast_arrow_batch(batch, false)?))
            .and_then(|batch| apply_predicates(batch, &self.predicates, residual))
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
    predicates: Vec<Bound>,
    parallel: &super::options::ReadSettings,
) -> Result<BatchReader> {
    let schema = crate::arrow::arrow_schema_from_field(&root)?;
    let refine = Arc::new(Refine {
        project: read_root.field_len() != root.field_len(),
        read_root,
        root,
        target,
        predicates,
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

/// Keep the rows of one batch its file's statistics did not already settle.
///
/// The conjuncts run through the one bound evaluator, so a residual test on a
/// data file is the same comparison a listing filter, a row scan, and a
/// vectorized filter make - there is no Iceberg-specific row filter.
fn apply_predicates(
    batch: RecordBatch,
    predicates: &[Bound],
    residual: &[usize],
) -> Result<RecordBatch> {
    let mut batch = batch;
    for position in residual {
        let Some(predicate) = predicates.get(*position) else {
            continue;
        };
        batch = predicate.filter(&batch)?;
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
/// written before a rename stores the column under its pre-rename name. The file's
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
    options: &crate::media::RecordOptions,
    wanted: &Field,
) -> Field {
    let Ok(file_root) = crate::IOMedia::read_arrow_field(handle, options) else {
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
        .and_then(|dtype| {
            Field::from_parts(
                wanted.name(),
                dtype,
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
fn restore_partitions(batch: &RecordBatch, partition: &[(Field, Scalar)]) -> Result<RecordBatch> {
    let missing: Vec<&(Field, Scalar)> = partition
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
        fields.push(field.clone().into_arrow_ref()?);
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
) -> Result<Vec<(Field, Scalar)>> {
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
pub(super) fn read_root(root: &Field, schema: &Field, filter: &Expression) -> Result<Field> {
    let mut children: Vec<Field> = root.fields().to_vec();
    for name in filter.columns() {
        if children
            .iter()
            .any(|child| child.name().eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let Some(column) = schema.get_field_by_path(&name) else {
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

#[cfg(test)]
mod delete_tests {
    use super::*;
    use crate::media::iceberg::PartitionField;

    fn manifest(content: ManifestContent) -> ManifestFile {
        ManifestFile {
            manifest_path: "metadata/delete-manifest.avro".into(),
            manifest_length: 128,
            partition_spec_id: 0,
            content,
            sequence_number: 2,
            min_sequence_number: 1,
            added_snapshot_id: 7,
            added_files_count: Some(1),
            existing_files_count: Some(0),
            deleted_files_count: Some(0),
            added_rows_count: Some(1),
            existing_rows_count: Some(0),
            deleted_rows_count: Some(0),
            partitions: Vec::new(),
            key_metadata: None,
            first_row_id: None,
        }
    }

    fn schema() -> Field {
        DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row")
    }

    fn entry(content: i32) -> ManifestEntry {
        ManifestEntry::added(
            7,
            DataFile {
                content,
                file_path: "data/part.parquet".into(),
                mime_type: crate::MimeType::PARQUET,
                record_count: 1,
                file_size_in_bytes: 128,
                ..DataFile::default()
            },
        )
    }

    fn assert_unsupported(error: Error, kind: &str) {
        let Error::Iceberg { reason, source } = error else {
            panic!("expected a typed Iceberg unsupported error, got {error}");
        };
        assert!(source.is_none());
        assert!(reason.contains(kind), "{reason}");
    }

    #[test]
    fn live_delete_manifests_fail_before_any_file_is_read() {
        let error = plan(
            &[manifest(ManifestContent::Deletes)],
            &|_| Ok(PartitionSpec::unpartitioned()),
            &|_| panic!("delete manifest must not be opened as row data"),
            &[],
            &schema(),
            true,
        )
        .unwrap_err();
        assert_unsupported(error, "delete manifest");
    }

    #[test]
    fn delete_manifests_with_unknown_counts_are_not_assumed_empty() {
        let mut unknown = manifest(ManifestContent::Deletes);
        unknown.added_files_count = None;
        unknown.existing_files_count = None;
        let error = plan(
            &[unknown],
            &|_| Ok(PartitionSpec::unpartitioned()),
            &|_| panic!("unknown delete counts must fail before the manifest is opened"),
            &[],
            &schema(),
            true,
        )
        .unwrap_err();
        assert_unsupported(error, "delete manifest");
    }

    #[test]
    fn delete_manifests_without_live_files_are_inert() {
        let mut deleted = manifest(ManifestContent::Deletes);
        deleted.added_files_count = Some(0);
        deleted.deleted_files_count = Some(1);
        deleted.added_rows_count = Some(0);
        deleted.deleted_rows_count = Some(1);
        let planned = plan(
            &[deleted],
            &|_| panic!("an inert delete manifest has no spec to resolve"),
            &|_| panic!("an inert delete manifest has no entries to read"),
            &[],
            &schema(),
            true,
        )
        .unwrap();
        assert_eq!(planned, ScanPlan::default());
    }

    #[test]
    fn missing_partition_specs_fail_before_pruning() {
        let error = plan(
            &[manifest(ManifestContent::Data)],
            &|spec_id| {
                Err(invalid(format_smolstr!(
                    "expected partition spec id {spec_id}, got none"
                )))
            },
            &|_| panic!("a manifest with an unresolved spec must not be opened"),
            &[],
            &schema(),
            true,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("expected partition spec id 0, got none"),
            "{error}"
        );
    }

    #[test]
    fn delete_files_hidden_in_data_manifests_are_rejected() {
        let manifest = manifest(ManifestContent::Data);
        for (content, kind) in [(1, "position-delete"), (2, "equality-delete")] {
            let error =
                validate_data_entry(&entry(content), &manifest, &PartitionSpec::unpartitioned())
                    .unwrap_err();
            assert_unsupported(error, kind);
        }
    }

    #[test]
    fn partition_tuples_must_match_the_referenced_spec() {
        let spec = PartitionSpec {
            spec_id: 3,
            fields: vec![PartitionField::identity(1, 1_000, "id")],
        };
        let error = validate_data_entry(&entry(0), &manifest(ManifestContent::Data), &spec)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected 1 partition values for spec 3, got 0"),
            "{error}"
        );
    }

    #[test]
    fn unknown_transforms_never_supply_pruning_bounds() {
        let mut schema = schema();
        crate::media::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec {
            spec_id: 3,
            fields: vec![PartitionField {
                source_id: 1,
                field_id: 1_000,
                name: "id_opaque".into(),
                transform: Transform::Unknown,
            }],
        };
        assert!(identity_column(&spec, 0, &schema).is_none());
    }

    #[test]
    fn data_files_inherit_contiguous_row_ids_without_moving_explicit_ids() {
        let mut cursor = Some(10);
        let mut first = entry(0);
        first.data_file.record_count = 3;
        inherit_first_row_id(&mut first, &mut cursor).unwrap();
        assert_eq!(first.data_file.first_row_id, Some(10));
        assert_eq!(cursor, Some(13));

        let mut explicit = entry(0);
        explicit.data_file.first_row_id = Some(100);
        explicit.data_file.record_count = 7;
        inherit_first_row_id(&mut explicit, &mut cursor).unwrap();
        assert_eq!(explicit.data_file.first_row_id, Some(100));
        assert_eq!(cursor, Some(13));

        let mut last = entry(0);
        last.data_file.record_count = 2;
        inherit_first_row_id(&mut last, &mut cursor).unwrap();
        assert_eq!(last.data_file.first_row_id, Some(13));
        assert_eq!(cursor, Some(15));
    }

    #[test]
    fn data_files_keep_null_row_ids_without_a_manifest_range() {
        let mut entry = entry(0);
        let mut cursor = None;
        inherit_first_row_id(&mut entry, &mut cursor).unwrap();
        assert_eq!(entry.data_file.first_row_id, None);
        assert_eq!(cursor, None);
    }

    #[test]
    fn row_id_overflow_fails_without_mutating_the_file_or_cursor() {
        let mut entry = entry(0);
        entry.data_file.record_count = 2;
        let mut cursor = Some(i64::MAX);
        let error = inherit_first_row_id(&mut entry, &mut cursor)
            .unwrap_err()
            .to_string();
        assert!(error.contains("row id overflow"), "{error}");
        assert_eq!(entry.data_file.first_row_id, None);
        assert_eq!(cursor, Some(i64::MAX));
    }
}

#[cfg(test)]
mod bound_tests {
    use super::*;

    fn schema(dtype: DataType) -> Field {
        let mut schema = DataType::from_fields([dtype.required_field("value")])
            .unwrap()
            .required_field("row");
        crate::media::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    fn residual(
        dtype: DataType,
        lower: Vec<u8>,
        upper: Vec<u8>,
        value: Scalar,
    ) -> Option<Vec<usize>> {
        let schema = schema(dtype);
        let file = DataFile {
            record_count: 1,
            lower_bounds: vec![(1, lower)],
            upper_bounds: vec![(1, upper)],
            ..DataFile::default()
        };
        let filter = Expression::column("value").eq(Expression::literal(value));
        let conjuncts = conjuncts(&schema, &filter).unwrap();
        file_residual(
            &file_bounds(&file, &PartitionSpec::unpartitioned(), &schema),
            &conjuncts,
        )
    }

    #[test]
    fn promoted_bounds_prune_under_the_current_schema_type() {
        let int = 37_i32.to_le_bytes().to_vec();
        assert_eq!(
            residual(DataType::Int64, int.clone(), int, Scalar::I64(37)),
            Some(Vec::new()),
            "an Int bound evolved to Long proves the matching value"
        );

        let float = 1.5_f32.to_le_bytes().to_vec();
        assert_eq!(
            residual(
                DataType::Float64,
                float.clone(),
                float,
                Scalar::from(1.5_f64)
            ),
            Some(Vec::new()),
            "a Float bound evolved to Double proves the matching value"
        );
    }

    #[test]
    fn malformed_bounds_leave_the_filter_for_rows() {
        assert_eq!(
            residual(DataType::Int64, vec![0; 3], vec![0; 9], Scalar::I64(37)),
            Some(vec![0]),
            "malformed statistics cannot exclude or settle a file"
        );
    }
}
