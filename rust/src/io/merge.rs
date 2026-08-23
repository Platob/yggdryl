//! Key-matched merging of an incoming reader into what a resource stores.
//!
//! An explicit merge updates a row whose key is already stored and appends a
//! row whose key is new. The encodings underneath are whole-value containers -
//! an Arrow IPC stream and a Parquet file each carry one schema and one footer -
//! so "update" means producing the merged contents and rewriting, which is
//! what [`merged`] returns.
//!
//! The match key is encoded through Arrow's own row format, so two rows compare
//! equal exactly when every key column holds the same value, including nulls and
//! nested values, without rendering anything as text.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_ipc::reader::FileReader;
use arrow_ipc::writer::FileWriter;
use arrow_row::{RowConverter, SortField};
use arrow_schema::{ArrowError, SchemaRef};

use crate::arrow::{BatchReader, arrow_schema_from_field, from_reader_error};
use crate::field::cast::ArrowCast;
use crate::{Error, Field, Result};

/// One key's positions in the held result, as `(batch, row)` pairs.
type Positions = Vec<(usize, usize)>;

/// The latest spooled row for one key that was not present in stored data.
#[derive(Clone, Copy)]
struct SpooledPosition {
    /// First-appearance order among new keys.
    ordinal: usize,
    /// IPC record-batch position in the spill file.
    batch: usize,
    /// Row position within that record batch.
    row: u32,
}

/// Mutable state accumulated while incoming rows are folded into a merge.
#[derive(Default)]
struct MergeState {
    held: Vec<RecordBatch>,
    index: HashMap<Box<[u8]>, Positions>,
    spill: Option<FileWriter<TemporaryFile>>,
    spill_batches: usize,
    appended: HashMap<Box<[u8]>, SpooledPosition>,
}

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
    let schema = arrow_schema_from_field(field)?;
    let keys = key_indices(&schema, merge_by_names)?;
    let converter = RowConverter::new(
        keys.iter()
            .map(|index| SortField::new(schema.field(*index).data_type().clone()))
            .collect(),
    )
    .map_err(Error::Arrow)?;

    // The stored side is what has to be held. Updating a row means finding it
    // by key, and a reader cannot be rewound to a row it has already yielded,
    // so the rows a later key might land on stay in memory. Incoming payloads
    // never accumulate in memory: rows with new keys spill to an Arrow IPC
    // file one batch at a time. Only their key-to-position index is retained,
    // because a later duplicate must replace the first arrival without adding
    // another row. The temporary file is unlinked when the returned reader is
    // dropped, including on an encoding failure.
    let mut state = MergeState::default();
    for batch in stored {
        let batch = batch.map_err(from_reader_error)?;
        if batch.num_rows() == 0 {
            continue;
        }
        index_batch(
            &converter,
            &batch,
            &keys,
            state.held.len(),
            &mut state.index,
        )?;
        state.held.push(batch);
    }

    for batch in incoming {
        let batch = field.cast_arrow_batch(batch.map_err(from_reader_error)?, safe)?;
        if batch.num_rows() == 0 {
            continue;
        }
        state.fold_in(&converter, &batch, &keys)?;
    }

    let MergeState {
        held,
        spill,
        appended,
        ..
    } = state;
    let Some(spill) = spill else {
        return Ok(crate::arrow::batch_reader(schema, held));
    };
    let file = spill.into_inner().map_err(Error::Arrow)?;
    let spilled = FileReader::try_new(file, None).map_err(Error::Arrow)?;
    let mut positions: Vec<SpooledPosition> = appended.into_values().collect();
    positions.sort_unstable_by_key(|position| position.ordinal);
    Ok(Box::new(SpilledMergeReader {
        schema,
        stored: held.into_iter(),
        spilled,
        positions: positions.into_iter().peekable(),
        current_batch: None,
    }))
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

impl MergeState {
    /// Apply one incoming batch to stored rows and spill its new-key rows.
    fn fold_in(
        &mut self,
        converter: &RowConverter,
        batch: &RecordBatch,
        keys: &[usize],
    ) -> Result<()> {
        let rows = converter
            .convert_columns(&key_columns(batch, keys))
            .map_err(Error::Arrow)?;

        let mut updates: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        let mut appends: Vec<(Box<[u8]>, u32)> = Vec::new();
        for row in 0..batch.num_rows() {
            let key = Box::<[u8]>::from(rows.row(row).as_ref());
            let position = u32::try_from(row).map_err(oversized_row)?;
            if let Some(positions) = self.index.get(&key) {
                for (batch_position, held_row) in positions {
                    updates
                        .entry(*batch_position)
                        .or_default()
                        .push((*held_row, row));
                }
                continue;
            }
            appends.push((key, position));
        }

        for (position, rows) in updates {
            self.held[position] = replace_rows(&self.held[position], batch, &rows)?;
        }
        if !appends.is_empty() {
            let rows = UInt32Array::from(
                appends
                    .iter()
                    .map(|(_, position)| *position)
                    .collect::<Vec<_>>(),
            );
            let taken =
                arrow_select::take::take_record_batch(batch, &rows).map_err(Error::Arrow)?;
            if self.spill.is_none() {
                let file = TemporaryFile::new()?;
                self.spill =
                    Some(FileWriter::try_new(file, taken.schema().as_ref()).map_err(Error::Arrow)?);
            }
            self.spill
                .as_mut()
                .expect("the spill writer was initialized")
                .write(&taken)
                .map_err(Error::Arrow)?;
            let batch_position = self.spill_batches;
            self.spill_batches += 1;
            for (row, (key, _)) in appends.into_iter().enumerate() {
                let row = u32::try_from(row).map_err(oversized_row)?;
                let ordinal = self
                    .appended
                    .get(&key)
                    .map_or_else(|| self.appended.len(), |position| position.ordinal);
                self.appended.insert(
                    key,
                    SpooledPosition {
                        ordinal,
                        batch: batch_position,
                        row,
                    },
                );
            }
        }
        Ok(())
    }
}

/// Yield held stored batches, then the latest spooled row for each new key.
///
/// Consecutive rows from one spill batch are gathered into one Arrow take so
/// common unique-key input keeps its original batch granularity. A key updated
/// by a later batch can require a seek, but the reader still retains at most
/// one spilled payload batch at a time.
struct SpilledMergeReader {
    schema: SchemaRef,
    stored: std::vec::IntoIter<RecordBatch>,
    spilled: FileReader<TemporaryFile>,
    positions: std::iter::Peekable<std::vec::IntoIter<SpooledPosition>>,
    current_batch: Option<(usize, RecordBatch)>,
}

impl Iterator for SpilledMergeReader {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(batch) = self.stored.next() {
            return Some(Ok(batch));
        }
        let first = self.positions.next()?;
        let mut rows = vec![first.row];
        while let Some(next) = self.positions.peek() {
            if next.batch != first.batch {
                break;
            }
            rows.push(next.row);
            self.positions.next();
        }
        if self.current_batch.as_ref().map(|(position, _)| *position) != Some(first.batch) {
            if let Err(error) = self.spilled.set_index(first.batch) {
                return Some(Err(error));
            }
            let Some(batch) = self.spilled.next() else {
                return Some(Err(ArrowError::IpcError(format!(
                    "merge spill ended before record batch {}",
                    first.batch
                ))));
            };
            match batch {
                Ok(batch) => self.current_batch = Some((first.batch, batch)),
                Err(error) => return Some(Err(error)),
            }
        }
        let batch = &self
            .current_batch
            .as_ref()
            .expect("the requested spill batch was loaded")
            .1;
        Some(arrow_select::take::take_record_batch(
            batch,
            &UInt32Array::from(rows),
        ))
    }
}

impl arrow_array::RecordBatchReader for SpilledMergeReader {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// A process-private spill file removed as soon as its merge reader is gone.
struct TemporaryFile {
    file: Option<File>,
    path: PathBuf,
}

impl TemporaryFile {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let directory = std::env::temp_dir();
        for _ in 0..100 {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(
                "yggdryl-merge-{}-{sequence}.arrow",
                std::process::id()
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        file: Some(file),
                        path,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(Error::Io(error)),
            }
        }
        Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not reserve a unique merge spill file after 100 attempts",
        )))
    }

    fn file(&mut self) -> &mut File {
        self.file.as_mut().expect("the spill file is open")
    }
}

impl Read for TemporaryFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.file().read(buffer)
    }
}

impl Write for TemporaryFile {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.file().write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file().flush()
    }
}

impl Seek for TemporaryFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file().seek(position)
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        // Windows refuses to unlink an open file, so close it first.
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
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
