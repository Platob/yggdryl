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
//! [`IOBase::children_matching`](crate::io::IOBase::children_matching) and a
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

use arrow_array::BooleanArray;
use arrow_buffer::BooleanBuffer;

use super::manifest::{
    DataFile, DataFileContent, EntryStatus, ManifestContent, ManifestEntry, ManifestFile,
};
use super::partition::{PartitionSpec, Transform};
use super::value::single_to_value;
use crate::arrow::BatchReader;
use crate::expression::{Bound, Bounds, Selector};
use crate::field::cast::ArrowCast;
use crate::generic::Holder;
use crate::{DataType, Error, Expression, Field, Result, Value};

/// One data file a scan reads, with everything a rewrite of it would need.
#[derive(Clone, Debug)]
pub struct ScanTask {
    /// The manifest entry, with the numbers its manifest supplied filled in.
    pub entry: ManifestEntry,
    /// The spec the entry's partition tuple is written against.
    pub spec: PartitionSpec,
    /// The filters this file's partition tuple did not already settle.
    pub residual: Vec<usize>,
    /// The delete files the spec's scope rules apply to this file, in
    /// manifest-list order: position deletes at a sequence number at or above
    /// the file's own, equality deletes strictly above it, both scoped to the
    /// file's partition unless the equality delete's spec is unpartitioned.
    pub deletes: Vec<ManifestEntry>,
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
    /// Every delete manifest of the snapshot, read or not: a rewrite carries
    /// these into its new manifest list so the deletes keep applying to the
    /// files it did not touch.
    pub delete_manifests: Vec<ManifestFile>,
    /// Delete manifests whose entries had to be read for the scope rules.
    pub delete_manifests_read: usize,
    /// Delete manifests excluded on their manifest-list summary alone.
    pub delete_manifests_skipped: usize,
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

    /// Return how many distinct delete files apply to the planned reads.
    ///
    /// A delete file naming only rows of files the plan already excluded is
    /// not counted, because nothing will read it.
    pub fn deletes_applied(&self) -> usize {
        let mut paths: Vec<&str> = self
            .tasks
            .iter()
            .flat_map(|task| task.deletes.iter())
            .map(|delete| delete.data_file.file_path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
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
        let data_type = column.data_type();
        let decode = |held: &Option<Vec<u8>>| {
            held.as_deref()
                .and_then(|bytes| single_to_value(bytes, data_type))
        };
        let (minimum, maximum) = (decode(&summary.lower_bound), decode(&summary.upper_bound));
        // A partition column is spelled in the path too, so the summary bounds
        // it as an attribute as well - but only when the two ends meet. A range
        // of values does not bound the *text* of those values, because text
        // does not order the way a number does.
        if let (Some(low), Some(high)) = (&minimum, &maximum) {
            if low == high {
                let text = Value::from(super::value::scalar_text(low).as_str());
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
        let value = file.partition.get(position).cloned().unwrap_or(Value::Null);
        settled.push(column.name());
        if value.is_null() {
            bounds = bounds.with_column(column.name(), None, None, rows);
            continue;
        }
        // The manifest is the authority on the value, and a path spells the
        // same one, so both spellings are recorded from the same source.
        let text = Value::from(super::value::scalar_text(&value).as_str());
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
        let data_type = column.data_type();
        let decode =
            |bytes: Option<&[u8]>| bytes.and_then(|bytes| single_to_value(bytes, data_type));
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
    spec_of: &dyn Fn(i32) -> PartitionSpec,
    manifest_at: &dyn Fn(&str) -> Result<Holder>,
    conjuncts: &[Bound],
    schema: &Field,
    for_read: bool,
) -> Result<ScanPlan> {
    let mut plan = ScanPlan::default();
    // The delete side is collected first, because a manifest list is free to
    // order its delete manifests before or after its data manifests and every
    // data file has to be tested against every delete that could scope to it.
    // What is held here is bounded and metadata-sized: one decoded entry per
    // live delete *file* of the manifests whose summaries kept their
    // partitions in play - never the delete rows themselves, which are read
    // per surviving data file when its mask is built.
    let mut deletes: Vec<DeleteCandidate> = Vec::new();
    for manifest in manifests {
        if manifest.content != ManifestContent::Deletes {
            continue;
        }
        plan.delete_manifests.push(manifest.clone());
        let spec = spec_of(manifest.partition_spec_id);
        // The same partition-summary pruning a data manifest gets: a delete
        // scoped to partitions the conjuncts exclude cannot subtract from a
        // file the plan keeps. An unpartitioned summary excludes nothing, so
        // a global equality delete is never pruned here.
        let summary = manifest_bounds(manifest, &spec, schema);
        if !conjuncts
            .iter()
            .all(|conjunct| conjunct.statistics_prune(&summary))
        {
            plan.delete_manifests_skipped += 1;
            continue;
        }
        plan.delete_manifests_read += 1;
        let handle = manifest_at(&manifest.manifest_path)?;
        // Delete manifests decode whole rather than through the planning fast
        // path: the equality ids, the referenced data file, and the path
        // bounds all steer which files each delete applies to.
        for mut entry in super::manifest::read_manifest(&handle)? {
            if entry.status == EntryStatus::Deleted {
                continue;
            }
            entry.inherit(manifest);
            if let Some(candidate) = DeleteCandidate::of(entry, &spec)? {
                deletes.push(candidate);
            }
        }
    }
    for manifest in manifests {
        if manifest.content != ManifestContent::Data {
            continue;
        }
        let spec = spec_of(manifest.partition_spec_id);
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
            let matched = file_residual(&file_bounds(&entry.data_file, &spec, schema), conjuncts);
            let applies = if matched.is_some() {
                deletes
                    .iter()
                    .filter(|delete| delete.applies_to(&entry, spec.spec_id))
                    .map(|delete| delete.entry.clone())
                    .collect()
            } else {
                // An excluded file is never read, so nothing subtracts from it.
                Vec::new()
            };
            let task = ScanTask {
                entry,
                spec: spec.clone(),
                residual: matched.clone().unwrap_or_default(),
                deletes: applies,
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

/// One live delete file, with the scope the specification applies it by.
struct DeleteCandidate {
    /// The delete file's manifest entry, its sequence numbers filled in.
    entry: ManifestEntry,
    /// Whether the file removes positions or matching values.
    content: DataFileContent,
    /// The spec its partition tuple was written under.
    spec_id: i32,
    /// Whether that spec has no fields: such an equality delete is global.
    unpartitioned: bool,
    /// The one data file the delete references, when the metadata names one -
    /// `referenced_data_file`, or `file_path` bounds that meet on one value.
    referenced: Option<SmolStr>,
}

/// The reserved field id of a position delete file's `file_path` column.
const POSITION_DELETE_PATH_ID: i32 = 2_147_483_546;

impl DeleteCandidate {
    /// Describe one live delete entry, or refuse what this build cannot read.
    ///
    /// A deletion vector - a position-delete entry whose `content_offset`
    /// points into a Puffin file - is refused by name rather than misread as
    /// an empty subtraction, so a v3 table never quietly yields deleted rows.
    ///
    /// # Errors
    ///
    /// Returns an error for a deletion vector, for a `content` outside the
    /// specification's three values, and for a data file listed in a delete
    /// manifest.
    fn of(entry: ManifestEntry, spec: &PartitionSpec) -> Result<Option<Self>> {
        let file = &entry.data_file;
        let content = file.content_kind()?;
        if content == DataFileContent::Data {
            return Err(invalid(format_smolstr!(
                "expected a delete file in a delete manifest, got a data file at {:?}",
                file.file_path
            )));
        }
        if file.content_offset.is_some() || file.content_size_in_bytes.is_some() {
            // The seam the Puffin lane fills: a deletion-vector mask source
            // plugs in here, replacing this refusal.
            return Err(invalid(format_smolstr!(
                "expected a position or equality delete file, got a deletion vector at {:?} \
                 (content_offset {}); reading deletion vectors from Puffin files is not \
                 implemented yet",
                file.file_path,
                file.content_offset.unwrap_or_default()
            )));
        }
        // A delete file with no rows subtracts nothing.
        if file.record_count == 0 {
            return Ok(None);
        }
        let referenced = file.referenced_data_file.clone().or_else(|| {
            let low = bound(&file.lower_bounds, POSITION_DELETE_PATH_ID)?;
            let high = bound(&file.upper_bounds, POSITION_DELETE_PATH_ID)?;
            // Bounds that meet name the one file every row references; a
            // truncated upper bound cannot equal the lower one, so equality
            // here is exact.
            (low == high)
                .then(|| std::str::from_utf8(low).ok().map(SmolStr::new))
                .flatten()
        });
        Ok(Some(Self {
            content,
            spec_id: spec.spec_id,
            unpartitioned: spec.fields.is_empty(),
            referenced,
            entry,
        }))
    }

    /// Say whether this delete subtracts from one data file.
    ///
    /// These are the specification's scan-planning scope rules: a position
    /// delete applies at a sequence number *at or above* the data file's own
    /// (same-commit deletes count) and only to the same partition of the same
    /// spec; an equality delete applies *strictly above* and also globally
    /// when its spec is unpartitioned. A position delete naming one referenced
    /// file applies to that file alone.
    fn applies_to(&self, data: &ManifestEntry, data_spec_id: i32) -> bool {
        let data_sequence = data.sequence_number.unwrap_or(0);
        let delete_sequence = self.entry.sequence_number.unwrap_or(0);
        let same_partition = self.spec_id == data_spec_id
            && self.entry.data_file.partition == data.data_file.partition;
        match self.content {
            DataFileContent::PositionDeletes => {
                delete_sequence >= data_sequence
                    && same_partition
                    && self
                        .referenced
                        .as_ref()
                        .is_none_or(|path| *path == data.data_file.file_path)
            }
            DataFileContent::EqualityDeletes => {
                delete_sequence > data_sequence && (self.unpartitioned || same_partition)
            }
            DataFileContent::Data => false,
        }
    }
}

/// One data file a scan still has to open.
pub(super) struct ScanPart {
    /// The handle addressing the data file.
    pub(super) handle: Holder,
    /// The file size the manifest recorded, which sizes the parallel decision.
    pub(super) size: i64,
    /// Partition columns to restore, when the file does not store them.
    pub(super) partition: Vec<(Field, Value)>,
    /// The filters this file's rows still have to be tested against.
    pub(super) residual: Vec<usize>,
    /// The subtraction the file's delete files make, resolved before opening.
    pub(super) deletes: FileDeletes,
}

/// The rows one data file's delete files remove, resolved once per file.
///
/// Both halves are built before the file is opened and never per batch: the
/// keep-mask is one bit per row of the file, and the equality predicate is
/// bound once against the read root.
#[derive(Default)]
pub(super) struct FileDeletes {
    /// One bit per row position of the file, false where a position delete
    /// removed the row. `None` keeps every row.
    pub(super) keep: Option<BooleanArray>,
    /// The negated equality-delete predicate - `not (row₁ or row₂ or ...)` -
    /// so filtering by it keeps what the deletes did not match.
    pub(super) equality: Option<Bound>,
}

/// Resolve every planned file's subtraction before any data file is opened.
///
/// One call per scan, one [`FileDeletes`] per task, in task order. Each
/// distinct delete file is read exactly once through the ordinary record
/// surface, however many data files it applies to. What is held while
/// building is bounded by the delete files themselves - the position rows
/// and equality rows they store, further narrowed to the data files the plan
/// kept - never by the table's data.
///
/// # Errors
///
/// Returns an error when a delete file cannot be read, when a position
/// delete file does not hold `(file_path, pos)` rows, or when an equality
/// delete names a field id the schema does not declare.
pub(super) fn file_deletes(
    tasks: &[ScanTask],
    delete_at: &dyn Fn(&DataFile) -> Result<Holder>,
    schema: &Field,
    read_root: &Field,
) -> Result<Vec<FileDeletes>> {
    if tasks.iter().all(|task| task.deletes.is_empty()) {
        return Ok(tasks.iter().map(|_| FileDeletes::default()).collect());
    }
    // The data files a position delete row can still matter to; positions
    // naming any other path are dropped as they stream.
    let wanted: std::collections::HashSet<&str> = tasks
        .iter()
        .map(|task| task.entry.data_file.file_path.as_str())
        .collect();
    let mut positions: std::collections::HashMap<
        SmolStr,
        std::collections::HashMap<SmolStr, Vec<i64>>,
    > = std::collections::HashMap::new();
    let mut rows: std::collections::HashMap<SmolStr, Vec<Expression>> =
        std::collections::HashMap::new();
    let mut resolved = Vec::with_capacity(tasks.len());
    for task in tasks {
        let path = task.entry.data_file.file_path.as_str();
        let mut deleted: Vec<i64> = Vec::new();
        let mut matches: Vec<Expression> = Vec::new();
        for delete in &task.deletes {
            let file = &delete.data_file;
            match file.content_kind()? {
                DataFileContent::PositionDeletes => {
                    if !positions.contains_key(&file.file_path) {
                        let handle = delete_at(file)?;
                        let index = position_delete_index(&handle, &wanted, &file.file_path)?;
                        positions.insert(file.file_path.clone(), index);
                    }
                    if let Some(found) = positions[&file.file_path].get(path) {
                        deleted.extend_from_slice(found);
                    }
                }
                DataFileContent::EqualityDeletes => {
                    if !rows.contains_key(&file.file_path) {
                        let handle = delete_at(file)?;
                        rows.insert(
                            file.file_path.clone(),
                            equality_delete_rows(&handle, file, schema)?,
                        );
                    }
                    matches.extend(rows[&file.file_path].iter().cloned());
                }
                // The planner attaches only delete contents; nothing else
                // reaches here.
                DataFileContent::Data => {}
            }
        }
        let keep =
            (!deleted.is_empty()).then(|| keep_mask(&deleted, task.entry.data_file.record_count));
        let equality = match matches.is_empty() {
            true => None,
            // Any matched row is a deleted row, so the kept rows are the
            // negation - handed to the same bind-and-vectorize layer every
            // other predicate goes through, once per file.
            false => Some(Expression::any(matches).not().bind(read_root)?),
        };
        resolved.push(FileDeletes { keep, equality });
    }
    Ok(resolved)
}

/// The two columns of a position delete file, as the specification fixes them.
fn position_delete_root() -> Result<Field> {
    Ok(DataType::from_fields([
        DataType::Utf8.required_field("file_path"),
        DataType::Int64.required_field("pos"),
    ])?
    .required_field("file_position_delete"))
}

/// Read one position delete file into per-data-file deleted positions.
///
/// The read goes through the existing record surface with the two spec
/// columns pushed down, so a delete file that also stored the deleted rows
/// never decodes them.
fn position_delete_index(
    handle: &Holder,
    wanted: &std::collections::HashSet<&str>,
    path: &SmolStr,
) -> Result<std::collections::HashMap<SmolStr, Vec<i64>>> {
    use arrow_array::{Array, Int64Array, StringArray};

    use crate::generic::IORecordOptions;
    use crate::io::IOBase;

    let root = position_delete_root()?;
    let options = handle.record_options()?.with_schema(root.clone());
    let mut index: std::collections::HashMap<SmolStr, Vec<i64>> = std::collections::HashMap::new();
    for batch in IOBase::read_arrow_batch_reader(handle, &options)? {
        let batch = root.cast_arrow_batch(batch.map_err(Error::Arrow)?, false)?;
        let (Some(files), Some(rows)) = (
            batch.column(0).as_any().downcast_ref::<StringArray>(),
            batch.column(1).as_any().downcast_ref::<Int64Array>(),
        ) else {
            return Err(invalid(format_smolstr!(
                "expected (file_path: string, pos: long) rows in the position delete file \
                 {path:?}, got ({}, {})",
                batch.column(0).data_type(),
                batch.column(1).data_type()
            )));
        };
        for row in 0..batch.num_rows() {
            let file = files.value(row);
            if wanted.contains(file) {
                match index.get_mut(file) {
                    Some(found) => found.push(rows.value(row)),
                    None => {
                        index.insert(SmolStr::new(file), vec![rows.value(row)]);
                    }
                }
            }
        }
    }
    Ok(index)
}

/// Read one equality delete file as the row predicates the spec defines.
///
/// Each stored row becomes one conjunction over the file's `equality_ids`
/// columns - `column = value`, or `column is null` where the stored value is
/// null, exactly the `IS NULL` semantics the specification gives a null
/// delete value. The caller ors the rows together and negates the whole; no
/// evaluation happens here.
fn equality_delete_rows(
    handle: &Holder,
    file: &DataFile,
    schema: &Field,
) -> Result<Vec<Expression>> {
    use arrow_array::Array;

    use crate::generic::IORecordOptions;
    use crate::io::IOBase;

    if file.equality_ids.is_empty() {
        return Err(invalid(format_smolstr!(
            "expected equality_ids on the equality delete file {:?}, got none",
            file.file_path
        )));
    }
    let mut columns: Vec<Field> = Vec::with_capacity(file.equality_ids.len());
    for id in &file.equality_ids {
        let column = schema
            .fields()
            .iter()
            .find(|column| column.parquet_field_id().ok().flatten() == Some(*id))
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a schema column with field id {id} for the equality delete {:?}, \
                     got none",
                    file.file_path
                ))
            })?;
        // A delete row's value may be null - meaning `is null` - even for a
        // column the table requires.
        columns.push(column.clone().with_nullable(true));
    }
    let root = DataType::from_fields(columns)?.required_field("equality_delete");
    let options = handle.record_options()?;
    // The same id-based projection a data file gets: a delete file written
    // before a rename still pushes the right columns down and aligns back.
    let projected = file_projection(handle, &options, &root);
    let options = options.with_schema(projected);
    let mut rows = Vec::new();
    for batch in IOBase::read_arrow_batch_reader(handle, &options)? {
        let batch = align_by_field_id(batch.map_err(Error::Arrow)?, &root)?;
        let batch = root.cast_arrow_batch(batch, false)?;
        for row in 0..batch.num_rows() {
            let mut tests = Vec::with_capacity(root.field_len());
            for (position, column) in root.fields().iter().enumerate() {
                let array = batch.column(position);
                if array.is_null(row) {
                    tests.push(Expression::column(column.name()).is_null());
                } else {
                    let value = crate::arrow::value::value_from_array(
                        column.data_type(),
                        array.as_ref(),
                        row,
                    )?;
                    // `= value` alone answers *unknown* for a null row, and
                    // the negation would then drop it; conjoining `is not
                    // null` makes the match plainly false, so a null row
                    // survives a delete naming a value - equality here is the
                    // spec's, not three-valued SQL's.
                    tests.push(
                        Expression::column(column.name())
                            .eq(Expression::typed_literal(
                                column.data_type().clone(),
                                value,
                            )?)
                            .and(Expression::column(column.name()).is_not_null()),
                    );
                }
            }
            rows.push(Expression::all(tests));
        }
    }
    Ok(rows)
}

/// Build one file's keep-mask from the positions its deletes removed.
///
/// Built once per file and sliced per batch, never rebuilt. A recorded
/// position outside `0..record_count` names no row and is ignored.
fn keep_mask(deleted: &[i64], record_count: i64) -> BooleanArray {
    let rows = usize::try_from(record_count).unwrap_or(0);
    let mut builder = arrow_buffer::BooleanBufferBuilder::new(rows);
    builder.append_n(rows, true);
    for position in deleted {
        if let Ok(position) = usize::try_from(*position) {
            if position < rows {
                builder.set_bit(position, false);
            }
        }
    }
    BooleanArray::new(builder.finish(), None)
}

/// The data file a scan is currently reading, and what it still has to apply.
struct Open {
    /// The file's own reader.
    reader: BatchReader,
    /// Partition columns to restore, when the file does not store them.
    partition: Vec<(Field, Value)>,
    /// The filters this file's rows still have to be tested against.
    residual: Vec<usize>,
    /// The subtraction the file's delete files make.
    deletes: FileDeletes,
    /// The file row position the next decoded batch starts at.
    offset: usize,
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

    /// Restore, align, cast, subtract, filter, and project one decoded batch.
    ///
    /// `offset` is the file row position the batch starts at, which is what
    /// lines the batch up against the file-wide position-delete mask; the
    /// stages before the subtraction keep every row in decode order, so the
    /// positions stay the ordinals the delete file wrote.
    fn batch(
        &self,
        batch: &RecordBatch,
        partition: &[(Field, Value)],
        residual: &[usize],
        deletes: &FileDeletes,
        offset: usize,
    ) -> std::result::Result<RecordBatch, arrow_schema::ArrowError> {
        restore_partitions(batch, partition)
            .and_then(|batch| align_by_field_id(batch, &self.read_root))
            .and_then(|batch| Ok(self.read_root.cast_arrow_batch(batch, false)?))
            .and_then(|batch| {
                subtract_and_filter(batch, deletes, offset, &self.predicates, residual)
            })
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
    let schema = crate::arrow::schema_from_field(&root)?;
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
                        let offset = open.offset;
                        open.offset += batch.num_rows();
                        return Some(self.refine.batch(
                            &batch,
                            &open.partition,
                            &open.residual,
                            &open.deletes,
                            offset,
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
                        deletes: part.deletes,
                        offset: 0,
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
        // The file row position each decoded batch starts at; decode order is
        // file order, so the running total is the batch's first position.
        let mut offset = 0_usize;
        for batch in reader {
            let produced = match batch {
                Ok(batch) => {
                    let at = offset;
                    offset += batch.num_rows();
                    refine.batch(&batch, &part.partition, &part.residual, &part.deletes, at)
                }
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

/// Keep the rows the deletes did not subtract and the filters did not drop.
///
/// The delete mask and the residual conjuncts compose by AND - deletes first,
/// then the predicate residual - into one selection applied by the shared
/// filter kernel, so a row removed by either is compacted away exactly once.
/// The conjuncts and the equality-delete predicate run through the one bound
/// evaluator, so a residual test on a data file is the same comparison a
/// listing filter, a row scan, and a vectorized filter make - there is no
/// Iceberg-specific row filter.
fn subtract_and_filter(
    batch: RecordBatch,
    deletes: &FileDeletes,
    offset: usize,
    predicates: &[Bound],
    residual: &[usize],
) -> Result<RecordBatch> {
    let rows = batch.num_rows();
    let mut mask: Option<BooleanBuffer> = deletes
        .keep
        .as_ref()
        .map(|keep| keep_slice(keep, offset, rows));
    if let Some(equality) = &deletes.equality {
        and_into(&mut mask, &equality.filter_mask(&batch)?);
    }
    for position in residual {
        let Some(predicate) = predicates.get(*position) else {
            continue;
        };
        and_into(&mut mask, &predicate.filter_mask(&batch)?);
    }
    match mask {
        None => Ok(batch),
        Some(mask) if mask.count_set_bits() == rows => Ok(batch),
        Some(mask) => {
            arrow_select::filter::filter_record_batch(&batch, &BooleanArray::new(mask, None))
                .map_err(Error::Arrow)
        }
    }
}

/// Conjoin one more selection into the running mask.
///
/// `filter_mask` never yields nulls - unknown is already false - so the plain
/// value buffers conjoin.
fn and_into(mask: &mut Option<BooleanBuffer>, more: &BooleanArray) {
    *mask = Some(match mask.take() {
        Some(mask) => &mask & more.values(),
        None => more.values().clone(),
    });
}

/// Slice a file-wide keep-mask to the rows of one batch.
///
/// A position at or past the mask's end is kept: the mask is sized by the
/// manifest's `record_count`, and a row the manifest never counted has no
/// delete naming it.
fn keep_slice(keep: &BooleanArray, offset: usize, rows: usize) -> BooleanBuffer {
    if offset.saturating_add(rows) <= keep.len() {
        return keep.values().slice(offset, rows);
    }
    (0..rows)
        .map(|row| {
            let position = offset + row;
            position >= keep.len() || keep.value(position)
        })
        .collect()
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
/// final cast rather than left out of the pushdown. `also` names further
/// columns needed the same way - the ones the plan's equality deletes match
/// rows by.
pub(super) fn read_root(
    root: &Field,
    schema: &Field,
    filter: &Expression,
    also: &[String],
) -> Result<Field> {
    let mut children: Vec<Field> = root.fields().to_vec();
    for name in filter.columns().into_iter().chain(also.iter().cloned()) {
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

/// The schema columns the plan's equality deletes match rows by.
///
/// These join the read root the way a filter's columns do: read for the test,
/// dropped by the final cast when the caller did not ask for them. An id the
/// schema does not declare is left for [`file_deletes`] to refuse by name.
pub(super) fn equality_columns(tasks: &[ScanTask], schema: &Field) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for delete in tasks.iter().flat_map(|task| task.deletes.iter()) {
        for id in &delete.data_file.equality_ids {
            let Some(column) = schema
                .fields()
                .iter()
                .find(|column| column.parquet_field_id().ok().flatten() == Some(*id))
            else {
                continue;
            };
            if !names.iter().any(|name| name == column.name()) {
                names.push(column.name().to_owned());
            }
        }
    }
    names
}

/// Report a scan a table's metadata cannot describe.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
