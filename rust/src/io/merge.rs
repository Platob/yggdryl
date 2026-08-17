//! Key-matched merging of an incoming reader into what a resource stores.
//!
//! A write whose options name a match key is not a replacement: a row whose key
//! is already stored updates that row, and a row whose key is not appends. The
//! encodings underneath are whole-value containers - an Arrow IPC stream and a
//! Parquet file each carry one schema and one footer - so "update" means
//! producing the merged contents and rewriting, which is what [`merged`]
//! returns.
//!
//! The match key is encoded through Arrow's own row format, so two rows compare
//! equal exactly when every key column holds the same value, including nulls and
//! nested values, without rendering anything as text.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_row::{RowConverter, SortField};

use crate::arrow::{BatchReader, from_reader_error, schema_from_field};
use crate::field::cast::ArrowCast;
use crate::{Error, Field, Result};

/// One key's positions in the held result, as `(batch, row)` pairs.
type Positions = Vec<(usize, usize)>;

/// Merge `incoming` into `stored`, matching rows on the `merge_by_names` columns.
///
/// Both sides are read as `field`: `stored` is expected to already be that
/// shape and every incoming batch is cast to it, so the two agree column for
/// column before a single key is compared.
///
/// Duplicates are resolved by stating the rule rather than refusing the input.
/// A key stored more than once has *every* occurrence updated, because a key is
/// a match rule here and not a declared constraint. A key arriving more than
/// once in `incoming` applies more than once, so the last arrival wins - and a
/// key that is new applies to the row its first arrival appended, so a merge
/// never introduces a duplicate the incoming side did not already store.
///
/// # Errors
///
/// Returns an error when `merge_by_names` names a column `field` does not declare,
/// when a key column's datatype has no row encoding, or on the first read or
/// cast failure from either side.
pub(crate) fn merged(
    stored: BatchReader,
    incoming: BatchReader,
    field: &Field,
    merge_by_names: &[String],
    safe: bool,
) -> Result<BatchReader> {
    let schema = schema_from_field(field)?;
    let keys = key_indices(&schema, merge_by_names)?;
    let converter = RowConverter::new(
        keys.iter()
            .map(|index| SortField::new(schema.field(*index).data_type().clone()))
            .collect(),
    )
    .map_err(Error::Arrow)?;

    // The stored side is what has to be held. Updating a row means finding it
    // by key, and a reader cannot be rewound to a row it has already yielded,
    // so the rows a later key might land on stay in memory. The incoming side
    // is bounded: one batch is pulled, matched, folded in, and dropped before
    // the next is pulled, so nothing accumulates in the incoming reader's name.
    // What grows past the stored size is the result itself, which a rewrite has
    // to know in full before an encoding can write its schema and its footer.
    let mut held: Vec<RecordBatch> = Vec::new();
    let mut index: HashMap<Box<[u8]>, Positions> = HashMap::new();
    for batch in stored {
        let batch = batch.map_err(from_reader_error)?;
        if batch.num_rows() == 0 {
            continue;
        }
        index_batch(&converter, &batch, &keys, held.len(), &mut index)?;
        held.push(batch);
    }

    for batch in incoming {
        let batch = field.cast_arrow_batch(batch.map_err(from_reader_error)?, safe)?;
        if batch.num_rows() == 0 {
            continue;
        }
        fold_in(&converter, &batch, &keys, &mut held, &mut index)?;
    }

    Ok(crate::arrow::batch_reader(schema, held))
}

/// Resolve the stored positions of the match-key columns.
fn key_indices(schema: &arrow_schema::Schema, merge_by_names: &[String]) -> Result<Vec<usize>> {
    if merge_by_names.is_empty() {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::SmolStr::new_static(
                "expected at least one column to merge on, got an empty match key",
            ),
        });
    }
    merge_by_names
        .iter()
        .map(|name| {
            schema.index_of(name).map_err(|_| {
                let stored = schema
                    .fields()
                    .iter()
                    .map(|field| field.name().as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        format_args!("merge_by_names column {name:?} among the stored columns"),
                        crate::text::elide_display(&stored),
                    ),
                }
            })
        })
        .collect()
}

/// Borrow the match-key columns of one batch.
fn key_columns(batch: &RecordBatch, keys: &[usize]) -> Vec<ArrayRef> {
    keys.iter()
        .map(|index| Arc::clone(batch.column(*index)))
        .collect()
}

/// Record where every key of `batch` lives in the held result.
fn index_batch(
    converter: &RowConverter,
    batch: &RecordBatch,
    keys: &[usize],
    position: usize,
    index: &mut HashMap<Box<[u8]>, Positions>,
) -> Result<()> {
    let rows = converter
        .convert_columns(&key_columns(batch, keys))
        .map_err(Error::Arrow)?;
    for row in 0..batch.num_rows() {
        index
            .entry(Box::from(rows.row(row).as_ref()))
            .or_default()
            .push((position, row));
    }
    Ok(())
}

/// Apply one incoming batch to the held result.
fn fold_in(
    converter: &RowConverter,
    batch: &RecordBatch,
    keys: &[usize],
    held: &mut Vec<RecordBatch>,
    index: &mut HashMap<Box<[u8]>, Positions>,
) -> Result<()> {
    let rows = converter
        .convert_columns(&key_columns(batch, keys))
        .map_err(Error::Arrow)?;

    let mut updates: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    let mut appends: Vec<u32> = Vec::new();
    // A key new to the stored side may still arrive twice in this batch; the
    // second arrival replaces the row the first one claimed rather than adding
    // a second row with the same key.
    let mut claimed: HashMap<Box<[u8]>, usize> = HashMap::new();
    for row in 0..batch.num_rows() {
        let key = Box::<[u8]>::from(rows.row(row).as_ref());
        let position = u32::try_from(row).map_err(oversized_row)?;
        if let Some(positions) = index.get(&key) {
            for (batch_position, held_row) in positions {
                updates
                    .entry(*batch_position)
                    .or_default()
                    .push((*held_row, row));
            }
            continue;
        }
        match claimed.get(&key) {
            Some(claim) => appends[*claim] = position,
            None => {
                claimed.insert(key, appends.len());
                appends.push(position);
            }
        }
    }

    for (position, rows) in updates {
        held[position] = replace_rows(&held[position], batch, &rows)?;
    }
    if !appends.is_empty() {
        let taken = arrow_select::take::take_record_batch(batch, &UInt32Array::from(appends))
            .map_err(Error::Arrow)?;
        index_batch(converter, &taken, keys, held.len(), index)?;
        held.push(taken);
    }
    Ok(())
}

/// Rebuild `held` with the named rows taken from `incoming` instead.
///
/// Both batches carry the same columns in the same order, so one interleave per
/// column is the whole rewrite: every row keeps its own values except the ones
/// the match key pointed at.
fn replace_rows(
    held: &RecordBatch,
    incoming: &RecordBatch,
    rows: &[(usize, usize)],
) -> Result<RecordBatch> {
    let mut sources: Vec<(usize, usize)> = (0..held.num_rows()).map(|row| (0, row)).collect();
    for (held_row, incoming_row) in rows {
        sources[*held_row] = (1, *incoming_row);
    }
    let columns = (0..held.num_columns())
        .map(|column| {
            arrow_select::interleave::interleave(
                &[
                    held.column(column).as_ref(),
                    incoming.column(column).as_ref(),
                ],
                &sources,
            )
            .map_err(Error::Arrow)
        })
        .collect::<Result<Vec<ArrayRef>>>()?;
    RecordBatch::try_new(held.schema(), columns).map_err(Error::Arrow)
}

/// Report a batch with more rows than Arrow's take kernel can address.
fn oversized_row(_: std::num::TryFromIntError) -> Error {
    Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$"),
        reason: smol_str::SmolStr::new_static(
            "expected a batch addressable by u32 row indices, got one with more rows than that",
        ),
    }
}

#[cfg(test)]
mod tests;
