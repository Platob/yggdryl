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
//! A filter is a [`Filter`], the same one that filters a Parquet file's row
//! groups and a batch through [`Bound::filter`](crate::expression::Bound::filter).
//! Each level of the chain answers it from the statistics it carries,
//! expressed as the [`Bounds`] every other container in this crate expresses
//! them as: a partition tuple is a minimum equal to its maximum, so a conjunct
//! it proves is dropped rather than re-tested. What no level settles is
//! filtered row by row after the file is read, because a statistic bounds a
//! *file* and does not select a row.

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{Schema as ArrowSchema, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use super::manifest::{DataFile, EntryStatus, ManifestContent, ManifestEntry, ManifestFile};
use super::partition::{PartitionSpec, Transform};
use super::value::single_to_value;
use crate::arrow::BatchReader;
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred, PlanCache};
use crate::expression::{Bound, Bounds};
use crate::holder::Holder;
use crate::{DataType, Error, Field, Filter, Result, Scalar, StructType};

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
        crate::hashing::stable_hash_of(self)
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
        crate::hashing::stable_hash_of(self)
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
pub(super) fn conjuncts(schema: &Field, filter: &Filter) -> Result<Vec<Bound>> {
    // The simplified form is what is split: a negated conjunction has already
    // become the conjuncts it hid, and a run of equalities one membership
    // test, so every level of the metadata prunes on the smallest shape.
    filter
        .simplify()
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
pub(super) fn identity_column<'schema>(
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
        let (minimum, maximum) = if nan_free(dtype, summary.contains_nan.map(u64::from)) {
            (decode(&summary.lower_bound), decode(&summary.upper_bound))
        } else {
            (None, None)
        };
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
        bounds = bounds.with_column(column.name(), Some(value.clone()), Some(value), Some(0));
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
        let nans = lookup(&file.nan_value_counts, id).and_then(|count| u64::try_from(count).ok());
        let (minimum, maximum) = if nan_free(dtype, nans) {
            (
                decode(bound(&file.lower_bounds, id)),
                decode(bound(&file.upper_bounds, id)),
            )
        } else {
            (None, None)
        };
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

/// Whether a column's recorded bounds cover every value the filter reads.
///
/// Iceberg leaves NaN out of a float column's bounds, while this crate orders
/// NaN past every number, by its sign - so a float column's bounds count only
/// where a NaN count of zero proves the file holds no NaN.
fn nan_free(dtype: &DataType, nans: Option<u64>) -> bool {
    !dtype.id().is_floating() || nans == Some(0)
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
    /// Whether a file may store a column under a name the read root does
    /// not use, so its footer has to be read before its projection is made.
    renamed: bool,
    /// Each read-root column carrying a v3 `initial-default`, and that value:
    /// what a data file written before the column existed reads it as.
    defaults: Vec<(Field, Scalar)>,
    /// The threads one file's columns decode on: the whole read parallelism
    /// when files are read one at a time, a share of it when several are.
    threads: usize,
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

        let mut options = part.handle.record_options()?;
        options.set_file_threads(self.threads);
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
                // The footer is opened for the names only when a rename ever
                // happened, or when a column carries an initial default a file
                // may predate; otherwise the read root's names are the file's,
                // and the read that follows is the file's one open.
                let projected = if self.renamed || !self.defaults.is_empty() {
                    file_projection(&part.handle, &options, &stored, &self.defaults)
                } else {
                    stored
                };
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
        plans: &mut Plans,
        batch: &RecordBatch,
        partition: &[(Field, Scalar)],
        residual: &[usize],
    ) -> std::result::Result<RecordBatch, arrow_schema::ArrowError> {
        restore_partitions(batch, partition)
            .and_then(|batch| restore_partitions(&batch, &self.defaults))
            .and_then(|batch| align_by_field_id(batch, &self.read_root))
            .and_then(|batch| Ok(Self::cast(&mut plans.read, batch, &self.read_root)?))
            .and_then(|batch| apply_predicates(batch, &self.predicates, residual))
            .and_then(|batch| {
                if self.project {
                    return Ok(Self::cast(&mut plans.project, batch, &self.root)?);
                }
                Ok(batch)
            })
            .map_err(scan_error)
    }

    /// Reconcile one batch to `root` through the plan its layout compiled.
    fn cast(
        plans: &mut PlanCache<ArrowCastPlan>,
        batch: RecordBatch,
        root: &Field,
    ) -> crate::arrow::Result<RecordBatch> {
        plans
            .get_or_compile(batch.schema_ref().fields(), || {
                ArrowCastPlan::compile_schema(
                    batch.schema_ref(),
                    root,
                    ArrowCastOptions::declared(true),
                    Deferred::default(),
                )
            })?
            .reconcile_batch(batch)
    }
}

/// The two casts one stream of decoded batches holds, each compiled once per
/// layout: into the read root, and from it into the scan root.
#[derive(Default)]
struct Plans {
    /// Decoded batches into the read root.
    read: PlanCache<ArrowCastPlan>,
    /// Filtered batches of the read root into the scan root.
    project: PlanCache<ArrowCastPlan>,
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
    /// The casts every file's batches go through.
    plans: Plans,
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
/// `renamed` says whether a file may store a column under a name the read
/// root does not use - a table that ever renamed one - so the projection has
/// to read the file's footer first; a table that never did projects by the
/// read root's names and opens each file once.
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
    renamed: bool,
) -> Result<BatchReader> {
    let schema = crate::arrow::arrow_schema_from_field(&root)?;
    let qualifying = parts
        .iter()
        .filter(|part| {
            u64::try_from(part.size).is_ok_and(|size| size >= parallel.min_file_size_bytes)
        })
        .count();
    let fan_out = parallel.parallelism >= 2 && parts.len() > 1 && qualifying >= parallel.min_files;
    // Files in flight at once share the parallelism between them; one file
    // at a time has all of it for its columns.
    let in_flight = if fan_out {
        parallel.parallelism.min(parts.len())
    } else {
        1
    };
    let defaults = initial_defaults(&read_root)?;
    let refine = Arc::new(Refine {
        project: read_root.field_len() != root.field_len(),
        defaults,
        read_root,
        root,
        target,
        predicates,
        renamed,
        threads: (parallel.parallelism / in_flight).max(1),
    });
    if fan_out {
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
        plans: Plans::default(),
    }))
}

impl Iterator for Scan {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(open) = self.current.as_mut() {
                match open.reader.next() {
                    Some(Ok(batch)) => {
                        return Some(self.refine.batch(
                            &mut self.plans,
                            &batch,
                            &open.partition,
                            &open.residual,
                        ));
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
        let mut plans = Plans::default();
        for batch in reader {
            let produced = match batch {
                Ok(batch) => refine.batch(&mut plans, &batch, &part.partition, &part.residual),
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
    defaults: &[(Field, Scalar)],
) -> Field {
    let Ok(file_root) = crate::IOMedia::read_arrow_field(handle, options) else {
        return wanted.clone();
    };
    let mut children: Vec<Field> = Vec::with_capacity(wanted.field_len());
    let mut changed = false;
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
                changed = true;
                children.push(child.clone().with_name(name));
            }
            // A file written before a column with an initial default existed
            // is not asked for it: the default is restored after the read.
            None if defaults
                .iter()
                .any(|(field, _)| field.name() == child.name()) =>
            {
                changed = true;
            }
            _ => children.push(child.clone()),
        }
    }
    if !changed {
        return wanted.clone();
    }
    StructType::from_fields(children)
        .map(DataType::from)
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

/// Each read-root column carrying a v3 `initial-default`, with that value
/// canonical under the column.
fn initial_defaults(read_root: &Field) -> Result<Vec<(Field, Scalar)>> {
    let mut defaults = Vec::new();
    for child in read_root.fields() {
        if let Some(value) = child.as_iceberg().initial_default()? {
            defaults.push((child.clone(), child.scalar(value)?));
        }
    }
    Ok(defaults)
}

/// Add the columns a data file left out - identity partition columns, and
/// columns with an initial default the file predates - as their constant
/// values, typed as declared.
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
        // The value lays out once under its field and repeats by index.
        let column = crate::Serie::from_scalars(field.clone(), [value.clone()])
            .map_err(crate::arrow::Error::from)
            .and_then(|row| row.repeat(0, batch.num_rows()))
            .and_then(|column| Ok(column.require_arrow_array()?))
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        columns.push(column);
        fields.push(field.clone().into_arrow_field_ref()?);
    }
    let schema =
        Arc::new(ArrowSchema::new(fields).with_metadata(batch.schema().metadata().clone()));
    RecordBatch::try_new(schema, columns).map_err(Error::Arrow)
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
pub(super) fn read_root(root: &Field, schema: &Field, filter: &Filter) -> Result<Field> {
    let mut children: Vec<Field> = root.fields().to_vec();
    for name in filter.columns() {
        if children
            .iter()
            .any(|child| child.name().eq_ignore_ascii_case(&name))
        {
            continue;
        }
        let Some(column) = schema.dtype().get_field_by_name(&name) else {
            continue;
        };
        children.push(column.clone());
    }
    if children.len() == root.field_len() {
        return Ok(root.clone());
    }
    Field::from_parts(
        root.name(),
        DataType::from(StructType::from_fields(children)?),
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/iceberg/scan.rs` pins and a caller cannot reach.
    //!
    //! Planning is what a scan does before it reads a byte, and a caller only
    //! ever sees the plan that came out. The steps below are each a refusal or
    //! a pruning decision that is worth pinning on its own, so they are
    //! forwarded here rather than reconstructed from a whole table.

    use crate::expression::{Bound, Bounds};
    use crate::holder::Holder;
    use crate::iceberg::{DataFile, ManifestEntry, ManifestFile, PartitionSpec, ScanPlan};
    use crate::{Field, Filter, Result};

    /// Split a filter into the conjuncts every level of metadata prunes on.
    pub fn conjuncts(schema: &Field, filter: &Filter) -> Result<Vec<Bound>> {
        super::conjuncts(schema, filter)
    }

    /// The statistics a manifest entry states about one data file.
    pub fn file_bounds(file: &DataFile, spec: &PartitionSpec, schema: &Field) -> Bounds {
        super::file_bounds(file, spec, schema)
    }

    /// Which conjuncts a file's statistics leave for its rows to answer.
    pub fn file_residual(bounds: &Bounds, conjuncts: &[Bound]) -> Option<Vec<usize>> {
        super::file_residual(bounds, conjuncts)
    }

    /// The column a partition field is the identity of, with its datatype.
    pub fn identity_column<'schema>(
        spec: &PartitionSpec,
        position: usize,
        schema: &'schema Field,
    ) -> Option<&'schema Field> {
        super::identity_column(spec, position, schema)
    }

    /// Fill one null data-file row id from its v3 manifest range.
    pub fn inherit_first_row_id(
        entry: &mut ManifestEntry,
        next_row_id: &mut Option<i64>,
    ) -> Result<()> {
        super::inherit_first_row_id(entry, next_row_id)
    }

    /// Plan a scan over manifests without a table to hold them.
    pub fn plan(
        manifests: &[ManifestFile],
        spec_of: &dyn Fn(i32) -> Result<PartitionSpec>,
        manifest_at: &dyn Fn(&str) -> Result<Holder>,
        conjuncts: &[Bound],
        schema: &Field,
        for_read: bool,
    ) -> Result<ScanPlan> {
        super::plan(manifests, spec_of, manifest_at, conjuncts, schema, for_read)
    }

    /// Reject a manifest row a data scan cannot interpret.
    pub fn validate_data_entry(
        entry: &ManifestEntry,
        manifest: &ManifestFile,
        spec: &PartitionSpec,
    ) -> Result<()> {
        super::validate_data_entry(entry, manifest, spec)
    }

    /// The schema columns a file's partition tuple restores, identity only.
    pub fn partition_columns(
        spec: &PartitionSpec,
        schema: &Field,
        file: &DataFile,
    ) -> Result<Vec<(Field, crate::Scalar)>> {
        super::partition_columns(spec, schema, file)
    }
}
