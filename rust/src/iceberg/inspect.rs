//! The table's own history, rendered as ordinary record batches.
//!
//! A table records what happened to it - when each snapshot became current,
//! what each commit did, which files the current snapshot holds - and the most
//! useful shape for that record is the one every other read in the project
//! already produces: a [`BatchReader`]. These builders take the metadata a
//! [`super::Table`] holds and render it as columns, so "show me the history"
//! is one read call and not a walk over structs.
//!
//! The columns follow the names PyIceberg's inspection tables use, so a reader
//! moving between the two sees one vocabulary. Column-statistic columns are
//! deliberately absent from `files` for now: the bounds a manifest carries are
//! encoded per datatype, and rendering them faithfully is its own piece of
//! work rather than a column of text.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::builder::{MapBuilder, StringBuilder};
use arrow_array::{
    ArrayRef, BooleanArray, Int32Array, Int64Array, RecordBatch, StringArray,
    TimestampMillisecondArray,
};

use crate::arrow::BatchReader;
use crate::{Error, Result};

use super::manifest::{DataFile, file_format_name};
use super::metadata::TableMetadata;
use super::partition::PartitionSpec;

/// Render when each snapshot became current, oldest first.
///
/// `is_current_ancestor` says whether the row is on the current snapshot's
/// parent chain, which is what separates the table's live lineage from a
/// branch that was abandoned by a rollback.
pub(super) fn history(metadata: &TableMetadata) -> Result<BatchReader> {
    let ancestors = current_ancestors(metadata);
    let mut made_current_at = Vec::with_capacity(metadata.snapshot_log.len());
    let mut snapshot_ids = Vec::with_capacity(metadata.snapshot_log.len());
    let mut parent_ids = Vec::with_capacity(metadata.snapshot_log.len());
    let mut is_ancestor = Vec::with_capacity(metadata.snapshot_log.len());
    for (timestamp_ms, snapshot_id) in &metadata.snapshot_log {
        made_current_at.push(*timestamp_ms);
        snapshot_ids.push(*snapshot_id);
        parent_ids.push(
            metadata
                .snapshot_by_id(*snapshot_id)
                .and_then(|snapshot| snapshot.parent_snapshot_id),
        );
        is_ancestor.push(ancestors.contains(snapshot_id));
    }

    let batch = RecordBatch::try_from_iter_with_nullable([
        ("made_current_at", timestamp_column(made_current_at), false),
        (
            "snapshot_id",
            Arc::new(Int64Array::from(snapshot_ids)) as ArrayRef,
            false,
        ),
        (
            "parent_id",
            Arc::new(Int64Array::from(parent_ids)) as ArrayRef,
            true,
        ),
        (
            "is_current_ancestor",
            Arc::new(BooleanArray::from(is_ancestor)) as ArrayRef,
            false,
        ),
    ])
    .map_err(Error::Arrow)?;
    Ok(crate::arrow::batch_reader(batch.schema(), [batch]))
}

/// Render every retained snapshot, in the order the metadata stores them.
pub(super) fn snapshots(metadata: &TableMetadata) -> Result<BatchReader> {
    let count = metadata.snapshots.len();
    let mut committed_at = Vec::with_capacity(count);
    let mut snapshot_ids = Vec::with_capacity(count);
    let mut parent_ids = Vec::with_capacity(count);
    let mut operations = Vec::with_capacity(count);
    let mut manifest_lists = Vec::with_capacity(count);
    let mut summaries = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
    for snapshot in &metadata.snapshots {
        committed_at.push(snapshot.timestamp_ms);
        snapshot_ids.push(snapshot.snapshot_id);
        parent_ids.push(snapshot.parent_snapshot_id);
        operations.push(snapshot.operation().to_owned());
        manifest_lists.push(snapshot.manifest_list.to_string());
        for (key, value) in &snapshot.summary {
            // The `operation` key is reported by its own column, the way
            // PyIceberg separates it from the free-form summary counters.
            if key != "operation" {
                summaries.keys().append_value(key);
                summaries.values().append_value(value);
            }
        }
        summaries.append(true).map_err(Error::Arrow)?;
    }

    let batch = RecordBatch::try_from_iter_with_nullable([
        ("committed_at", timestamp_column(committed_at), false),
        (
            "snapshot_id",
            Arc::new(Int64Array::from(snapshot_ids)) as ArrayRef,
            false,
        ),
        (
            "parent_id",
            Arc::new(Int64Array::from(parent_ids)) as ArrayRef,
            true,
        ),
        (
            "operation",
            Arc::new(StringArray::from(operations)) as ArrayRef,
            false,
        ),
        (
            "manifest_list",
            Arc::new(StringArray::from(manifest_lists)) as ArrayRef,
            false,
        ),
        ("summary", Arc::new(summaries.finish()) as ArrayRef, false),
    ])
    .map_err(Error::Arrow)?;
    Ok(crate::arrow::batch_reader(batch.schema(), [batch]))
}

/// Render the live data files of one planned snapshot.
///
/// The partition column is the `column=value` chain the tuple names - the
/// same text the layout spells - because the tuple's own struct shape differs
/// per spec and a table can hold files under more than one.
pub(super) fn files(entries: &[(DataFile, PartitionSpec)]) -> Result<BatchReader> {
    let mut file_paths = Vec::with_capacity(entries.len());
    let mut file_formats = Vec::with_capacity(entries.len());
    let mut spec_ids = Vec::with_capacity(entries.len());
    let mut partitions = Vec::with_capacity(entries.len());
    let mut record_counts = Vec::with_capacity(entries.len());
    let mut file_sizes = Vec::with_capacity(entries.len());
    for (file, spec) in entries {
        file_paths.push(file.file_path.to_string());
        file_formats.push(file_format_name(&file.mime_type)?);
        spec_ids.push(spec.spec_id);
        partitions.push(spec.partition_path(&file.partition)?);
        record_counts.push(file.record_count);
        file_sizes.push(file.file_size_in_bytes);
    }

    let batch = RecordBatch::try_from_iter_with_nullable([
        (
            "file_path",
            Arc::new(StringArray::from(file_paths)) as ArrayRef,
            false,
        ),
        (
            "file_format",
            Arc::new(StringArray::from(file_formats)) as ArrayRef,
            false,
        ),
        (
            "spec_id",
            Arc::new(Int32Array::from(spec_ids)) as ArrayRef,
            false,
        ),
        (
            "partition",
            Arc::new(StringArray::from(partitions)) as ArrayRef,
            false,
        ),
        (
            "record_count",
            Arc::new(Int64Array::from(record_counts)) as ArrayRef,
            false,
        ),
        (
            "file_size_in_bytes",
            Arc::new(Int64Array::from(file_sizes)) as ArrayRef,
            false,
        ),
    ])
    .map_err(Error::Arrow)?;
    Ok(crate::arrow::batch_reader(batch.schema(), [batch]))
}

/// Collect the snapshot ids on the current snapshot's parent chain.
fn current_ancestors(metadata: &TableMetadata) -> HashSet<i64> {
    let mut ancestors = HashSet::new();
    let mut cursor = metadata.current_snapshot_id;
    while let Some(id) = cursor {
        if !ancestors.insert(id) {
            // A parent cycle is corrupt metadata; stopping is the safe answer.
            break;
        }
        cursor = metadata
            .snapshot_by_id(id)
            .and_then(|snapshot| snapshot.parent_snapshot_id);
    }
    ancestors
}

/// Build a UTC millisecond timestamp column from raw counts.
fn timestamp_column(counts: Vec<i64>) -> ArrayRef {
    Arc::new(TimestampMillisecondArray::from(counts).with_timezone("UTC"))
}
