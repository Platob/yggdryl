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
//!   so a manifest whose summary excludes a value is skipped without being
//!   read, and the manifests it keeps are read side by side on the read
//!   parallelism, their entries taken in plan order;
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
use super::metadata::SortOrder;
use super::partition::{PartitionSpec, Transform};
use super::value::single_to_value;
use crate::arrow::BatchReader;
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred, PlanCache};
use crate::expression::eval::{EpochPeriod, TimeBucket};
use crate::expression::{Bound, Bounds, Function, Term};
use crate::holder::Holder;
use crate::integer::integer_from_text_as;
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
/// holds it holds it. A time transform stores the period the value falls in,
/// which bounds the column as a range instead ([`period_column`]); a bucket
/// or a truncation stores something no range of the source describes, so a
/// predicate on its source column falls through to the file's own statistics
/// and then to the rows themselves.
pub(super) fn identity_column<'schema>(
    spec: &PartitionSpec,
    position: usize,
    schema: &'schema Field,
) -> Option<&'schema Field> {
    let source = spec.fields.get(position)?;
    if source.transform != Transform::Identity {
        return None;
    }
    source_column(spec, position, schema)
}

/// The column a time partition field floors, with the period it floors to.
///
/// Every source value of a file whose tuple holds period `k` lies in
/// `[start(k), start(k + 1))`, so the tuple bounds the source column as a
/// range the way an identity tuple bounds it as one value - a coarser
/// statistic, but one a predicate on the source column prunes by through
/// the one rule every column statistic prunes by, with no second reading
/// of the predicate onto the partition value. `year` through `hour` and
/// `minutes[n]`, `week` and `quarter` all answer.
pub(super) fn period_column<'schema>(
    spec: &PartitionSpec,
    position: usize,
    schema: &'schema Field,
) -> Option<(&'schema Field, EpochPeriod)> {
    let period = spec.fields.get(position)?.transform.epoch_period()?;
    Some((source_column(spec, position, schema)?, period))
}

/// The column a derived partition column floors, with the bucket it floors
/// to.
///
/// A table computes the columns its schema derives for every row written to
/// it, so a column declaring `time_bucket(width, x)` holds the bucket `x`
/// falls in: its partition value bounds `x` as the range
/// `[start, start + width)`, exactly as a time transform's period bounds its
/// source ([`period_column`]). That is what lets a window on `x` rule out a
/// manifest by its summary of the bucket, without the predicate naming the
/// bucket at all. Only a declaration over one top-level column of the
/// bucket's own datatype answers.
pub(super) fn bucket_column<'schema>(
    column: &Field,
    schema: &'schema Field,
) -> Option<(&'schema Field, TimeBucket)> {
    let term = column.as_transform().term().ok()??;
    let Term::Function(Function::TimeBucket, arguments) = &term else {
        return None;
    };
    let [width, source] = &arguments[..] else {
        return None;
    };
    let source = source.as_column()?;
    let source = schema.fields().iter().find(|held| held.name() == source)?;
    if source.dtype() != column.dtype() {
        return None;
    }
    let bucket = TimeBucket::new(width.as_literal()?.value(), source.dtype()).ok()?;
    Some((source, bucket))
}

/// The inclusive range of source values the buckets from `lower` to `upper`
/// hold: from the first bucket's start to the last instant of the last one.
fn bucket_range(
    bucket: TimeBucket,
    source: &DataType,
    lower: Option<&Scalar>,
    upper: Option<&Scalar>,
) -> (Option<Scalar>, Option<Scalar>) {
    let at = |count: i64| match source {
        DataType::Date32 => i32::try_from(count).ok().map(Scalar::date32),
        DataType::DateTime64 { unit, timezone } => Scalar::datetime64(count, *unit, *timezone).ok(),
        _ => None,
    };
    let count = |value: &Scalar| match value {
        Scalar::Date32(date) => Some(i64::from(date.count())),
        other => other.temporal_count(),
    };
    let minimum = lower.and_then(count).and_then(at);
    let maximum = upper
        .and_then(count)
        .and_then(|start| start.checked_add(bucket.step() - 1))
        .and_then(at);
    (minimum, maximum)
}

/// The top-level schema column a partition field reads.
fn source_column<'schema>(
    spec: &PartitionSpec,
    position: usize,
    schema: &'schema Field,
) -> Option<&'schema Field> {
    let source = spec.fields.get(position)?;
    schema
        .fields()
        .iter()
        .find(|column| column.parquet_field_id().ok().flatten() == Some(source.source_id))
}

/// The inclusive range of source values a span of periods holds.
///
/// `lower` and `upper` are partition values - period numbers, a `day`
/// transform's as the `date32` it stores - and an end that is absent, or
/// whose first instant the source's own count cannot hold, bounds nothing on
/// its side.
///
/// A writer that truncated a count before the epoch toward zero rather than
/// flooring it filed instants of period `k - 1` under period `k`, for every
/// `k` up to zero: Apache Iceberg's Java writers once did for every such
/// instant, and iceberg-rust 0.10 still does for the last second of a day
/// before 1970. Java's reader keeps those files by widening its projection
/// of a negative period to the period after it
/// (`ProjectionUtil.fixInclusiveTimeProjection`), and the range here is the
/// same widening seen from the file: a lowest period at or below zero bounds
/// the source from the first instant of the period before it. Only the
/// specification's transforms are widened, and exactly where Java widens
/// them - `year`, `month`, `day` and `hour` of a timestamp, `year` and
/// `month` of a date, whose `day` is the date itself - because this crate's
/// own are written by no other writer and this one floors.
fn period_range(
    period: EpochPeriod,
    source: &DataType,
    lower: Option<&Scalar>,
    upper: Option<&Scalar>,
) -> (Option<Scalar>, Option<Scalar>) {
    let number = |value: &Scalar| match value {
        Scalar::Date32(date) => Some(i64::from(date.count())),
        other => other.as_i64(),
    };
    let start = |number: i64| match source {
        DataType::Date32 => period.start_days(number),
        DataType::DateTime64 { unit, .. } => period.start_count(number, *unit),
        _ => None,
    };
    let at = |count: i64| match source {
        DataType::Date32 => i32::try_from(count).ok().map(Scalar::date32),
        DataType::DateTime64 { unit, timezone } => Scalar::datetime64(count, *unit, *timezone).ok(),
        _ => None,
    };
    let truncated_by_writers = match source {
        DataType::Date32 => matches!(period, EpochPeriod::Year | EpochPeriod::Month),
        _ => matches!(
            period,
            EpochPeriod::Year | EpochPeriod::Month | EpochPeriod::Day | EpochPeriod::Hour
        ),
    };
    let minimum = lower
        .and_then(number)
        .and_then(|lowest| {
            if truncated_by_writers && lowest <= 0 {
                lowest.checked_sub(1)
            } else {
                Some(lowest)
            }
        })
        .and_then(start)
        .and_then(at);
    let maximum = upper
        .and_then(number)
        .and_then(|number| number.checked_add(1))
        .and_then(start)
        .and_then(|next| next.checked_sub(1))
        .and_then(at);
    (minimum, maximum)
}

/// One column's bounds gathered from every statistic that states one.
///
/// Each statistic is sound on its own - every value of the column lies
/// within it - so their intersection is too: the larger minimum, the
/// smaller maximum, whatever order the statistics arrive in. A column two
/// partition fields read, an identity beside a period or two periods, is
/// bounded by the tighter of them, never by whichever the spec lists first.
struct ColumnRange<'schema> {
    column: &'schema Field,
    minimum: Option<Scalar>,
    maximum: Option<Scalar>,
    nulls: Option<u64>,
}

impl<'schema> ColumnRange<'schema> {
    /// Narrow to one more statistic of the same column.
    fn narrow(&mut self, minimum: Option<Scalar>, maximum: Option<Scalar>) {
        let dtype = self.column.dtype();
        let keep =
            |held: &mut Option<Scalar>, other: Option<Scalar>, wanted: std::cmp::Ordering| {
                let Some(other) = other else {
                    return;
                };
                match held {
                    Some(current) => {
                        if crate::expression::eval::order(dtype, &other, current) == Some(wanted) {
                            *current = other;
                        }
                    }
                    None => *held = Some(other),
                }
            };
        keep(&mut self.minimum, minimum, std::cmp::Ordering::Greater);
        keep(&mut self.maximum, maximum, std::cmp::Ordering::Less);
    }
}

/// Fold one more statistic of `column` into the ranges gathered so far.
fn gather<'schema>(
    ranges: &mut Vec<ColumnRange<'schema>>,
    column: &'schema Field,
    minimum: Option<Scalar>,
    maximum: Option<Scalar>,
    nulls: Option<u64>,
) {
    match ranges
        .iter_mut()
        .find(|range| range.column.name() == column.name())
    {
        Some(range) => {
            range.narrow(minimum, maximum);
            range.nulls = range.nulls.or(nulls);
        }
        None => ranges.push(ColumnRange {
            column,
            minimum,
            maximum,
            nulls,
        }),
    }
}

/// The statistics a manifest-list row states about the files it names.
///
/// This is the cheapest level there is: a summary is one row of the manifest
/// list, so a manifest ruled out here is never opened at all. An identity
/// partition field bounds its schema column by the summary's own ends, and
/// a time partition field by the first instant of its lowest period and the
/// last of its highest; a bucket or a truncation bounds nothing. A column
/// several fields read is bounded by the tightest of them.
pub(super) fn manifest_bounds(
    manifest: &ManifestFile,
    spec: &PartitionSpec,
    schema: &Field,
) -> Bounds {
    let mut ranges: Vec<ColumnRange<'_>> = Vec::new();
    for (position, summary) in manifest.partitions.iter().enumerate() {
        // A summary says whether a null is present, never how many, so the
        // only count it can state is zero. A time transform of a null is
        // null and of a value a value, so its summary says the same of the
        // source column.
        let nulls = (!summary.contains_null).then_some(0);
        if let Some(column) = identity_column(spec, position, schema) {
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
            // The bucket of a null is null and of a value a value, so the
            // summary says the same of the column it floors.
            if let Some((source, bucket)) = bucket_column(column, schema) {
                let (lowest, highest) =
                    bucket_range(bucket, source.dtype(), minimum.as_ref(), maximum.as_ref());
                gather(&mut ranges, source, lowest, highest, nulls);
            }
            gather(&mut ranges, column, minimum, maximum, nulls);
            continue;
        }
        let Some((column, period)) = period_column(spec, position, schema) else {
            continue;
        };
        let Ok(dtype) = spec.fields[position].transform.result_type(column.dtype()) else {
            continue;
        };
        let decode = |held: &Option<Vec<u8>>| {
            held.as_deref()
                .and_then(|bytes| single_to_value(bytes, &dtype))
        };
        let (minimum, maximum) = period_range(
            period,
            column.dtype(),
            decode(&summary.lower_bound).as_ref(),
            decode(&summary.upper_bound).as_ref(),
        );
        gather(&mut ranges, column, minimum, maximum, nulls);
    }
    ranges.into_iter().fold(Bounds::new(None), |bounds, range| {
        bounds.with_column(
            range.column.name(),
            range.minimum,
            range.maximum,
            range.nulls,
        )
    })
}

/// The statistics a manifest entry states about one data file.
///
/// An identity partition value is the tighter of the two sources and wins: a
/// value every row of the file shares is a minimum equal to its maximum,
/// which is what lets a partition predicate settle a file outright rather
/// than merely fail to rule it out. A time partition value is the period
/// every row's source falls in, which narrows the column's own ends to the
/// tightest of them all and stands in for a null count the file left
/// unstated.
pub(super) fn file_bounds(file: &DataFile, spec: &PartitionSpec, schema: &Field) -> Bounds {
    let rows = u64::try_from(file.record_count).ok();
    let mut bounds = Bounds::new(rows);
    let mut settled: Vec<&str> = Vec::new();
    let mut periods: Vec<ColumnRange<'_>> = Vec::new();
    for position in 0..spec.fields.len() {
        let value = file
            .partition
            .get(position)
            .cloned()
            .unwrap_or(Scalar::Null);
        if let Some(column) = identity_column(spec, position, schema) {
            settled.push(column.name());
            // A bucket bounds the column it floors as a period does: every
            // row's source lies in it, and a null bucket is a null source.
            let floored = bucket_column(column, schema);
            if value.is_null() {
                if let Some((source, _)) = floored {
                    gather(&mut periods, source, None, None, rows);
                }
                bounds = bounds.with_column(column.name(), None, None, rows);
                continue;
            }
            if let Some((source, bucket)) = floored {
                let (minimum, maximum) =
                    bucket_range(bucket, source.dtype(), Some(&value), Some(&value));
                gather(&mut periods, source, minimum, maximum, Some(0));
            }
            bounds = bounds.with_column(column.name(), Some(value.clone()), Some(value), Some(0));
            continue;
        }
        let Some((column, period)) = period_column(spec, position, schema) else {
            continue;
        };
        // A period of null is the period of a null source, so every row of
        // the file holds null there; a period states no null at all.
        if value.is_null() {
            gather(&mut periods, column, None, None, rows);
            continue;
        }
        let (minimum, maximum) = period_range(period, column.dtype(), Some(&value), Some(&value));
        gather(&mut periods, column, minimum, maximum, Some(0));
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
        let (mut minimum, mut maximum) = if nan_free(dtype, nans) {
            (
                decode(bound(&file.lower_bounds, id)),
                decode(bound(&file.upper_bounds, id)),
            )
        } else {
            (None, None)
        };
        let mut nulls = nulls;
        if let Some(period) = periods
            .iter_mut()
            .find(|period| period.column.name() == column.name())
        {
            period.narrow(minimum, maximum);
            minimum = period.minimum.take();
            maximum = period.maximum.take();
            nulls = nulls.or(period.nulls);
        }
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
/// through what that closure returns. `ordered` names the column whose
/// bounds a read-only plan keeps even when no filter consults them: the
/// leading sort key an ordered record read opens a partition's files by
/// ([`partition_groups`]).
///
/// The plan runs in three steps, and its answer is the sequential one byte
/// for byte. The summaries prune first, on the calling thread with no read,
/// each kept manifest's handle resolved there. The kept manifests are then
/// read side by side - each one round trip on a store - on up to
/// `parallelism` threads, at most that many in flight, their entries
/// answered in plan order ([`crate::parallel::ordered`]); one thread, or one
/// kept manifest, reads them one after another and spawns nothing. Last, on
/// the calling thread, every entry is validated, inherits what its manifest
/// supplies and is pruned on its own statistics, manifest by manifest in
/// plan order. A read is a read whatever thread sends it, so a plan costs
/// the same calls at every parallelism.
///
/// # Errors
///
/// Returns an error when a manifest that had to be read cannot be reached or
/// decoded - the first in plan order whose step failed, as a sequential
/// plan would report it.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan(
    manifests: &[ManifestFile],
    spec_of: &dyn Fn(i32) -> Result<PartitionSpec>,
    manifest_at: &dyn Fn(&str) -> Result<Holder>,
    conjuncts: &[Bound],
    schema: &Field,
    for_read: bool,
    ordered: Option<i32>,
    parallelism: usize,
) -> Result<ScanPlan> {
    let mut plan = ScanPlan::default();
    // Step one: prune on the summaries and resolve each kept manifest's
    // handle, reading nothing. A failure here stops the walk where a
    // sequential plan would have stopped, and is reported only once every
    // manifest before it has had its own chance to fail first.
    let mut kept: Vec<(&ManifestFile, PartitionSpec)> = Vec::new();
    let mut handles: Vec<Holder> = Vec::new();
    let mut stopped = None;
    for manifest in manifests {
        if manifest.content == ManifestContent::Deletes {
            if manifest.added_files_count == Some(0) && manifest.existing_files_count == Some(0) {
                continue;
            }
            stopped = Some(unsupported_deletes(
                &manifest.manifest_path,
                "delete manifest",
            ));
            break;
        }
        let spec = match spec_of(manifest.partition_spec_id) {
            Ok(spec) => spec,
            Err(error) => {
                stopped = Some(error);
                break;
            }
        };
        let summary = manifest_bounds(manifest, &spec, schema);
        if !conjuncts
            .iter()
            .all(|conjunct| conjunct.statistics_prune(&summary))
        {
            plan.skipped.push(manifest.clone());
            continue;
        }
        match manifest_at(&manifest.manifest_path) {
            Ok(handle) => {
                plan.manifests_read += 1;
                kept.push((manifest, spec));
                handles.push(handle);
            }
            Err(error) => {
                stopped = Some(error);
                break;
            }
        }
    }

    // Step two: read the kept manifests side by side, answered in plan
    // order. A read-only plan decodes just the columns pruning consults; a
    // plan whose entries may be carried into a rewritten manifest decodes
    // everything, because a carried entry must keep its statistics.
    let with_stats = !conjuncts.is_empty();
    let threads = parallelism.min(handles.len()).max(1);
    let read = move |handle: Holder| {
        if for_read {
            super::manifest::read_planned_manifest(&handle, with_stats, ordered)
        } else {
            super::manifest::read_manifest(&handle)
        }
    };
    let entries = crate::parallel::ordered(handles, threads, 1, read).with_lane_depth(1);

    // Step three: every entry, in plan order, on the calling thread.
    for ((manifest, spec), entries) in kept.into_iter().zip(entries) {
        let mut next_row_id = manifest.first_row_id;
        for mut entry in entries? {
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
    match stopped {
        Some(error) => Err(error),
        None => Ok(plan),
    }
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
                    ArrowCastOptions::new(),
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

/// What a worker hands the consumer: one refined batch, or the failure
/// that ended its file. The file's end is the worker dropping its sender.
type Decoded = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

/// A reader that decodes several planned files at once, releasing their
/// batches strictly in plan order.
///
/// Memory is bounded per worker: at most `window` files - the resolved read
/// parallelism - are in flight, because a new worker is spawned only when
/// the release cursor finishes a file, and each worker runs at most
/// [`crate::parquet::READ_AHEAD_BATCHES`] refined batches ahead of the
/// cursor before it waits on its own channel, so what the scan holds is the
/// window's fetched column chunks plus that many batches per file, never a
/// file ahead of the cursor decoded whole.
///
/// **Dropping the reader detaches the workers rather than joining them.** A
/// worker owns everything it touches - its handle, its `Arc` of the shared
/// pipeline, its sender - so nothing borrowed outlives the drop; the dropped
/// receivers disconnect the channels, and each worker exits at its next send
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
    /// The files in flight, in plan order, each the receiving end of its
    /// worker's bounded channel; the front is the file being released.
    in_flight: std::collections::VecDeque<std::sync::mpsc::Receiver<Decoded>>,
    /// How many files may be in flight at once.
    window: usize,
}

impl ParallelScan {
    /// Start the first window of workers over the planned files.
    fn new(parts: Vec<ScanPart>, schema: SchemaRef, refine: Arc<Refine>, window: usize) -> Self {
        let window = window.max(1);
        let mut scan = Self {
            schema,
            refine,
            jobs: parts.into_iter(),
            in_flight: std::collections::VecDeque::with_capacity(window),
            window,
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
        let refine = Arc::clone(&self.refine);
        let (sender, receiver) = std::sync::mpsc::sync_channel(crate::parquet::READ_AHEAD_BATCHES);
        self.in_flight.push_back(receiver);
        // Deliberately detached: see the type docs for why drop does not join.
        let _ = std::thread::spawn(move || read_part(&part, &refine, &sender));
    }
}

impl Iterator for ParallelScan {
    type Item = Decoded;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let front = self.in_flight.front()?;
            match front.recv() {
                Ok(result) => return Some(result),
                // The worker dropped its sender: the cursor file is drained,
                // so the window has room for one more worker.
                Err(_) => {
                    self.in_flight.pop_front();
                    self.spawn_next();
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
/// cast-and-filter work and not only the decode; each is sent down the
/// worker's bounded channel, which holds the worker when it runs
/// [`crate::parquet::READ_AHEAD_BATCHES`] ahead of the cursor, and the file
/// is finished when the sender drops. Every send doubles as the liveness
/// check: a dropped reader disconnects the channel and the worker returns at
/// its next send. The body is unwind-guarded so a panicking decode reports an
/// error instead of ending the file silently.
fn read_part(part: &ScanPart, refine: &Refine, sender: &std::sync::mpsc::SyncSender<Decoded>) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = match refine.open(part) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = sender.send(Err(scan_error(error)));
                return;
            }
        };
        let mut plans = Plans::default();
        for batch in reader {
            let produced = match batch {
                Ok(batch) => refine.batch(&mut plans, &batch, &part.partition, &part.residual),
                Err(error) => Err(error),
            };
            if sender.send(produced).is_err() {
                return;
            }
        }
    }));
    if outcome.is_err() {
        let _ = sender.send(Err(scan_error(invalid(SmolStr::new_static(
            "expected the file to decode, got a panicking reader thread",
        )))));
    }
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
            .and_then(|text| integer_from_text_as::<i32>(text.as_str()));
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
        // The value is one constant column, laid out as it is exported.
        let column = crate::Serie::lit(field.clone(), value.clone(), batch.num_rows())
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

/// Whether every row of one partition group holds the same value of the
/// column `source_id` names: a column the spec takes by identity, unless it
/// is floating - one group can hold both zeros, which an order tells apart.
/// The rule the writer drops a sort key by, so a group is sorted on the
/// keys its files were written in.
pub(super) fn constant_in_group(spec: &PartitionSpec, schema: &Field, source_id: i32) -> bool {
    spec.fields
        .iter()
        .any(|part| part.source_id == source_id && part.transform == Transform::Identity)
        && super::partition::source_path(schema, source_id)
            .is_ok_and(|(_, source)| !source.dtype().id().is_floating())
}

/// The first key a partition group's rows are sorted by - the first of the
/// table's order no group holds constant - with the datatype its file
/// bounds decode under. `None` where the order has no such key, or where
/// it reads a nested column, for which a manifest keeps no bounds.
struct LeadingKey {
    /// The source column's field identifier, which keys the bounds.
    id: i32,
    /// The source column's datatype, which the bounds decode under.
    dtype: DataType,
    /// Whether the key descends.
    descending: bool,
}

impl LeadingKey {
    /// The leading key of `order` over `spec`, as [`Self`] states it.
    fn of(order: &SortOrder, spec: &PartitionSpec, schema: &Field) -> Option<Self> {
        let key = order
            .fields
            .iter()
            .find(|key| !constant_in_group(spec, schema, key.source_id))?;
        let (path, source) = super::partition::source_path(schema, key.source_id).ok()?;
        (path.len() == 1).then(|| Self {
            id: key.source_id,
            dtype: source.dtype().clone(),
            descending: key.direction == "desc",
        })
    }

    /// Where a file's rows start in this key's direction: its lower bound
    /// ascending, its upper bound descending; `None` where it records none.
    fn start(&self, file: &DataFile) -> Option<Scalar> {
        let bounds = if self.descending {
            &file.upper_bounds
        } else {
            &file.lower_bounds
        };
        bound(bounds, self.id).and_then(|bytes| single_to_value(bytes, &self.dtype))
    }

    /// Order two starts in this key's direction, a file stating none last.
    fn compare(&self, left: Option<&Scalar>, right: Option<&Scalar>) -> std::cmp::Ordering {
        match (left, right) {
            (Some(left), Some(right)) if self.descending => right.cmp(left),
            (Some(left), Some(right)) => left.cmp(right),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }
}

/// The column whose bounds an ordered record read opens a partition's files
/// by, and so the one a read-only [`plan`] keeps them for: its `ordered`.
pub(super) fn leading_key_id(
    order: &SortOrder,
    spec: &PartitionSpec,
    schema: &Field,
) -> Option<i32> {
    LeadingKey::of(order, spec, schema).map(|key| key.id)
}

/// A read's planned files grouped by partition, in the order an ordered
/// record read opens them - or handed back as they were planned when one
/// belongs to a spec other than `spec`, whose tuples do not compare.
///
/// The groups follow their partition tuples ascending, position by
/// position: a null where the sort key naming that identity column places
/// it, else last; a transform's value orders as the value it is. Inside a
/// group the files open by where their leading sort key starts
/// ([`LeadingKey::start`]), a file stating no bound last, then by data
/// sequence number and path - so a partition whose commits each cover a
/// later stretch of the key arrives already in order, and the group is read
/// once and never merged. Every field read is one the plan already holds;
/// nothing is opened.
pub(super) fn partition_groups(
    tasks: Vec<ScanTask>,
    spec: &PartitionSpec,
    order: &SortOrder,
    schema: &Field,
) -> std::result::Result<Vec<Vec<ScanTask>>, Vec<ScanTask>> {
    if tasks.iter().any(|task| task.spec.spec_id != spec.spec_id) {
        return Err(tasks);
    }
    let nulls_first: Vec<bool> = spec
        .fields
        .iter()
        .map(|part| {
            part.transform == Transform::Identity
                && order
                    .fields
                    .iter()
                    .find(|key| {
                        key.source_id == part.source_id && key.transform == Transform::Identity
                    })
                    .is_some_and(|key| key.null_order == "nulls-first")
        })
        .collect();
    let lead = LeadingKey::of(order, spec, schema);
    let mut keyed: Vec<(Option<Scalar>, ScanTask)> = tasks
        .into_iter()
        .map(|task| {
            let start = lead
                .as_ref()
                .and_then(|lead| lead.start(&task.entry.data_file));
            (start, task)
        })
        .collect();
    keyed.sort_by(|(left_start, left), (right_start, right)| {
        compare_tuples(
            &left.entry.data_file.partition,
            &right.entry.data_file.partition,
            &nulls_first,
        )
        .then_with(|| {
            lead.as_ref().map_or(std::cmp::Ordering::Equal, |lead| {
                lead.compare(left_start.as_ref(), right_start.as_ref())
            })
        })
        .then_with(|| left.entry.sequence_number.cmp(&right.entry.sequence_number))
        .then_with(|| {
            left.entry
                .data_file
                .file_path
                .cmp(&right.entry.data_file.file_path)
        })
    });
    let mut groups: Vec<Vec<ScanTask>> = Vec::new();
    for (_, task) in keyed {
        match groups.last_mut() {
            Some(group)
                if group.first().is_some_and(|first| {
                    compare_tuples(
                        &first.entry.data_file.partition,
                        &task.entry.data_file.partition,
                        &nulls_first,
                    )
                    .is_eq()
                }) =>
            {
                group.push(task);
            }
            _ => groups.push(vec![task]),
        }
    }
    Ok(groups)
}

/// Order two partition tuples position by position, a null at a position
/// first where `nulls_first` says so and last otherwise.
fn compare_tuples(left: &[Scalar], right: &[Scalar], nulls_first: &[bool]) -> std::cmp::Ordering {
    for (position, (left, right)) in left.iter().zip(right).enumerate() {
        let first = nulls_first.get(position).copied().unwrap_or(false);
        let ordering = match (left.is_null(), right.is_null()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) if first => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) if first => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => left.cmp(right),
        };
        if ordering.is_ne() {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

/// The order an ordered record read proves over `landing` - the root its
/// rows land under - once `select` has run: the table's order, its keys
/// taken while `landing` binds them, `select` publishes the columns they
/// read unchanged, and the groups were sorted on them (`sorted` keys of the
/// writer's own, [`constant_in_group`] dropping the rest).
///
/// Only where partitions arriving in tuple order are the order's own
/// leading keys: every spec field the identity of a column that is not
/// floating, the order opening with those columns in spec order,
/// ascending - an unpartitioned table vacuously. Anywhere else each
/// partition is sorted on its own, and the stream across them proves no
/// order at all.
///
/// Two keys are read off what the table stores rather than off the landed
/// rows, so each holds only where `landing` reads its column as the table
/// stores it: a column a group holds constant, ordered by the stored tuples
/// the groups arrive in, and a transform, whose group was sorted on the
/// column it reads. A transform is the last key declared: the image of a
/// sorted column keeps its order, but ties within one image are in the
/// column's order, not the next key's.
pub(super) fn proven_order(
    spec: &PartitionSpec,
    order: &SortOrder,
    schema: &Field,
    landing: &Field,
    select: &crate::Selector,
    sorted: usize,
) -> Vec<crate::expression::Ordering> {
    let leading = spec.fields.iter().enumerate().all(|(position, part)| {
        part.transform == Transform::Identity
            && order.fields.get(position).is_some_and(|key| {
                key.source_id == part.source_id
                    && key.transform == Transform::Identity
                    && key.direction == "asc"
            })
            && identity_column(spec, position, schema)
                .is_some_and(|column| !column.dtype().id().is_floating())
    });
    if !leading {
        return Vec::new();
    }
    let Ok(keys) = order.into_orderings(schema) else {
        return Vec::new();
    };
    let mut proven = Vec::with_capacity(keys.len());
    let mut sorting = 0;
    for (field, key) in order.fields.iter().zip(keys) {
        let constant = constant_in_group(spec, schema, field.source_id);
        if !constant {
            if sorting == sorted {
                break;
            }
            sorting += 1;
        }
        if key.term().bind(landing).is_err() || !publishes_unchanged(select, key.term()) {
            break;
        }
        let transformed = field.transform != Transform::Identity;
        if (constant || transformed) && !lands_as_stored(landing, schema, field.source_id) {
            break;
        }
        proven.push(key);
        if transformed && !constant {
            break;
        }
    }
    proven
}

/// Whether `landing` reads the column `source_id` names as `schema` stores
/// it: the same path, the same datatype.
fn lands_as_stored(landing: &Field, schema: &Field, source_id: i32) -> bool {
    let Ok((path, source)) = super::partition::source_path(schema, source_id) else {
        return false;
    };
    let mut landed = Some(landing);
    for name in &path {
        landed = landed.and_then(|field| field.dtype().get_field_by_name(name));
    }
    landed.is_some_and(|landed| landed.dtype() == source.dtype())
}

/// Whether `select` publishes every column `term` reads as it is stored,
/// under its own name, one row per row: so a key the rows are ordered by
/// binds, and orders them, on the far side of it too.
fn publishes_unchanged(select: &crate::Selector, term: &crate::Term) -> bool {
    if select.is_all() {
        return true;
    }
    if select.unnests() {
        return false;
    }
    term.columns().iter().all(|column| {
        let starred = select.has_star()
            && !select
                .excluded()
                .iter()
                .any(|name| name.eq_ignore_ascii_case(column))
            && !select
                .projections()
                .iter()
                .any(|projection| projection.name().eq_ignore_ascii_case(column));
        starred
            || select.projections().iter().any(|projection| {
                projection.is_column() && projection.term().as_column() == Some(column.as_str())
            })
    })
}

/// `root` declaring `keys` as the order its rows keep; as it is for none.
///
/// # Errors
///
/// Returns an error when the declaration cannot be written.
pub(super) fn declaring(mut root: Field, keys: Vec<crate::expression::Ordering>) -> Result<Field> {
    if !keys.is_empty() {
        root.as_sort_mut().set_by(keys)?;
    }
    Ok(root)
}

/// What every partition group of an ordered read is decoded under: the
/// arguments [`reader`] takes, kept so each group opens its own.
pub(super) struct GroupScan {
    /// The root the rows land under, declaring no order.
    pub(super) root: Field,
    /// That root plus the columns the filters read.
    pub(super) read_root: Field,
    /// The read root of the root that declares what the read proves: what
    /// the transport face casts each file's batches into, so they cross
    /// under that root's schema.
    pub(super) declared_read_root: Field,
    /// The column pushdown handed to each file, when the caller gave one.
    pub(super) target: Option<Field>,
    /// The conjuncts, indexed by every part's residual list.
    pub(super) predicates: Vec<Bound>,
    /// The parallel-read decision every group's reader makes.
    pub(super) parallel: super::options::ReadSettings,
    /// Whether a file may store a column under another name.
    pub(super) renamed: bool,
}

/// The records an ordered read yields: partition after partition, each
/// group's rows in the order `sorting` states.
///
/// A group is opened only when the one before it has yielded its last
/// record, so a satisfied consumer - a row limit - never decodes the next.
/// A group that needs no sort streams its scan's batches as they land; one
/// that does lands them into one [`ChunkedSerie`](crate::ChunkedSerie) - spilled under the
/// process bound as they arrive - and yields its chunks: as they landed
/// where they already keep the order, chunk by chunk and edge by edge,
/// else sorted out of core and merged. At most one group is held at once.
/// Every record is relabelled under `root`, which declares what the read
/// proves.
///
/// Two faces: [`Self::into_serie_reader`], the records, and
/// [`Self::into_arrow_reader`], the transport - which, where no group needs
/// a sort, is the scan itself and lands nothing.
pub(super) struct Partitions {
    /// The groups not yet opened, in the order they are yielded.
    groups: std::vec::IntoIter<Vec<ScanPart>>,
    /// How every group is decoded.
    scan: GroupScan,
    /// The keys each group is sorted by; none to stream it.
    sorting: Vec<crate::expression::Ordering>,
    /// The root every yielded record is typed by.
    root: Arc<Field>,
    /// The group being yielded.
    open: Option<Group>,
}

/// One opened partition group.
enum Group {
    /// A group needing no sort: its scan's batches as they land.
    Streamed(crate::SerieReader),
    /// A sorted group's chunks.
    Held(std::vec::IntoIter<crate::Serie>),
}

impl Partitions {
    /// The ordered read of `groups`, each decoded under `scan`, sorted by
    /// `sorting`, and yielded under `root`.
    ///
    /// Groups needing no sort are never held, so their files stream as one
    /// scan - the groups' files in group order - decoded side by side
    /// across partitions where the plan is worth it, as any scan is.
    pub(super) fn new(
        groups: Vec<Vec<ScanPart>>,
        scan: GroupScan,
        sorting: Vec<crate::expression::Ordering>,
        root: Arc<Field>,
    ) -> Self {
        let groups = if sorting.is_empty() {
            vec![groups.into_iter().flatten().collect()]
        } else {
            groups
        };
        Self {
            groups: groups.into_iter(),
            scan,
            sorting,
            root,
            open: None,
        }
    }

    /// Decode one group's files, and hold and sort them where it needs it.
    fn open_group(&self, parts: Vec<ScanPart>) -> crate::arrow::Result<Group> {
        let scan = &self.scan;
        let batches = reader(
            parts,
            scan.root.clone(),
            scan.read_root.clone(),
            scan.target.clone(),
            scan.predicates.clone(),
            &scan.parallel,
            scan.renamed,
        )?;
        let landed = crate::SerieReader::from_arrow_reader(
            Some(&scan.root),
            batches,
            ArrowCastOptions::new(),
        )?;
        if self.sorting.is_empty() {
            return Ok(Group::Streamed(landed));
        }
        // The same two calls a partition group is written through: the
        // chunks as they landed where they already keep the order, read
        // once chunk by chunk and edge by edge, else each sorted on its
        // own and merged, the output settled.
        let hold = crate::ChunkedSerie::from_serie_reader(landed)?;
        let hold = if hold.keeps_order(&self.sorting)? {
            hold
        } else {
            hold.into_sort_by(self.sorting.as_slice())?
        };
        Ok(Group::Held(hold.chunks().to_vec().into_iter()))
    }

    /// Stop: nothing more is opened or yielded.
    fn fuse(&mut self) {
        self.groups = Vec::new().into_iter();
        self.open = None;
    }

    /// The read's records, under `root`.
    ///
    /// Lazy, so no record is verified again on its way out: every group
    /// was proven in its order where it was held - `keeps_order` read it
    /// chunk by chunk and edge by edge, or `into_sort_by` laid it out - and
    /// the groups arrive in tuple order, which is the order's own leading
    /// keys wherever the root declares one ([`proven_order`]).
    ///
    /// # Errors
    ///
    /// Returns an error when the root does not project into Arrow.
    pub(super) fn into_serie_reader(self) -> crate::arrow::Result<crate::SerieReader> {
        crate::SerieReader::from_landed_iter(Arc::clone(&self.root), self)
    }

    /// The read as transport, under `root`'s schema.
    ///
    /// Where no group needs a sort the batches are the scan's own - the
    /// groups' files in group order, each batch reconciled to `root` and
    /// never landed, so nothing is proven or copied that a scan does not
    /// prove or copy. Otherwise the held groups' records, as batches.
    ///
    /// # Errors
    ///
    /// Returns an error when the root does not project into Arrow.
    pub(super) fn into_arrow_reader(self) -> crate::arrow::Result<BatchReader> {
        if !self.sorting.is_empty() {
            return Ok(self.into_serie_reader()?.into_arrow_reader());
        }
        let Self {
            groups, scan, root, ..
        } = self;
        Ok(reader(
            groups.flatten().collect(),
            Arc::unwrap_or_clone(root),
            scan.declared_read_root,
            scan.target,
            scan.predicates,
            &scan.parallel,
            scan.renamed,
        )?)
    }
}

impl Iterator for Partitions {
    type Item = crate::arrow::Result<crate::Serie>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let pulled = match self.open.as_mut() {
                Some(Group::Streamed(records)) => records.next(),
                // A sorted group's empty chunk - a file the residual
                // emptied - has nothing to yield.
                Some(Group::Held(chunks)) => chunks.find(|chunk| !chunk.is_empty()).map(Ok),
                None => None,
            };
            match pulled {
                Some(Ok(record)) => {
                    return Some(Ok(record.into_relabeled(Arc::clone(&self.root))));
                }
                Some(Err(error)) => {
                    self.fuse();
                    return Some(Err(error));
                }
                None => self.open = None,
            }
            let parts = self.groups.next()?;
            match self.open_group(parts) {
                Ok(group) => self.open = Some(group),
                Err(error) => {
                    self.fuse();
                    return Some(Err(error));
                }
            }
        }
    }
}

impl std::iter::FusedIterator for Partitions {}

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

    /// The statistics a manifest-list row states about the files it names.
    pub fn manifest_bounds(
        manifest: &ManifestFile,
        spec: &PartitionSpec,
        schema: &Field,
    ) -> Bounds {
        super::manifest_bounds(manifest, spec, schema)
    }

    /// The source column a time partition field bounds, by name.
    pub fn period_column_name(
        spec: &PartitionSpec,
        position: usize,
        schema: &Field,
    ) -> Option<String> {
        super::period_column(spec, position, schema).map(|(column, _)| column.name().to_owned())
    }

    /// Fill one null data-file row id from its v3 manifest range.
    pub fn inherit_first_row_id(
        entry: &mut ManifestEntry,
        next_row_id: &mut Option<i64>,
    ) -> Result<()> {
        super::inherit_first_row_id(entry, next_row_id)
    }

    /// Plan a scan over manifests without a table to hold them, reading
    /// the kept manifests on up to `parallelism` threads.
    pub fn plan(
        manifests: &[ManifestFile],
        spec_of: &dyn Fn(i32) -> Result<PartitionSpec>,
        manifest_at: &dyn Fn(&str) -> Result<Holder>,
        conjuncts: &[Bound],
        schema: &Field,
        for_read: bool,
        parallelism: usize,
    ) -> Result<ScanPlan> {
        super::plan(
            manifests,
            spec_of,
            manifest_at,
            conjuncts,
            schema,
            for_read,
            None,
            parallelism,
        )
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
