//! Parquet footer statistics projected into a manifest's `data_file` columns.
//!
//! Iceberg keeps per-column counts and bounds in the manifest so a planner can
//! skip a file without opening it, and everything it wants is already in the
//! Parquet footer this crate just wrote. The projection is deliberately
//! conservative about *bounds*: a bound travels as an encoded value, and only
//! the types [`super::value::is_portable`] names - the ones whose Parquet
//! statistic bytes are byte-for-byte the Iceberg single-value encoding - are
//! emitted, so one encoding table serves both the writer here and the scan
//! planner that tests a filter against what it wrote.

use smol_str::{SmolStr, format_smolstr};

use super::manifest::DataFile;
use super::value::{compare_single, is_portable};
use crate::parquet::{ColumnStatistics, FileStatistics};
use crate::{DataType, Error, Field, Result};

/// Project one Parquet file's footer statistics into a data file description.
///
/// # Errors
///
/// Returns an error when a schema column carries no field identifier, because
/// a manifest keys every statistic by one.
pub(super) fn data_file(schema: &Field, statistics: &FileStatistics) -> Result<DataFile> {
    let columns = leaf_columns(schema)?;

    let mut file = DataFile {
        record_count: statistics.num_rows,
        split_offsets: statistics.split_offsets(),
        ..DataFile::default()
    };

    for (name, id, data_type) in &columns {
        let mut size = 0_i64;
        let mut nulls = 0_u64;
        let mut has_nulls = false;
        let mut lower: Option<Vec<u8>> = None;
        let mut upper: Option<Vec<u8>> = None;

        for group in &statistics.row_groups {
            for column in group.columns.iter().filter(|column| &column.path == name) {
                size = size.saturating_add(column.compressed_size);
                if let Some(count) = column.null_count {
                    has_nulls = true;
                    nulls = nulls.saturating_add(count);
                }
                if is_portable(data_type) {
                    fold_bound(&mut lower, column, data_type, true);
                    fold_bound(&mut upper, column, data_type, false);
                }
            }
        }

        if size > 0 {
            file.column_sizes.push((*id, size));
        }
        // Every value of a top-level column is one row, which is what makes a
        // value count meaningful without reading the file.
        file.value_counts.push((*id, statistics.num_rows));
        if has_nulls {
            file.null_value_counts
                .push((*id, i64::try_from(nulls).unwrap_or(i64::MAX)));
        }
        if let Some(bytes) = lower {
            file.lower_bounds.push((*id, bytes));
        }
        if let Some(bytes) = upper {
            file.upper_bounds.push((*id, bytes));
        }
    }

    Ok(file)
}

/// Measure one file's statistics from its batches, before they are encoded.
///
/// This is the projection [`data_file`] performs, taken from the rows
/// themselves rather than from a Parquet footer, for the formats whose file
/// carries no footer this crate reads back. The same portability rule guards
/// the bounds, so a bound written here compares exactly as a Parquet one.
/// Column byte sizes and split offsets are left unset: both belong to the
/// encoded layout, which the batches do not know.
///
/// # Errors
///
/// Returns an error when a schema column carries no field identifier, or when
/// a bound cannot be measured.
pub(super) fn data_file_from_batches(
    schema: &Field,
    batches: &[arrow_array::RecordBatch],
) -> Result<DataFile> {
    use arrow_array::Array as _;

    let columns = leaf_columns(schema)?;
    let num_rows: i64 = batches
        .iter()
        .map(|batch| i64::try_from(batch.num_rows()).unwrap_or(i64::MAX))
        .sum();

    let mut file = DataFile {
        record_count: num_rows,
        ..DataFile::default()
    };

    for (name, id, data_type) in &columns {
        let field = schema.get_field_by_name(name).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected the measured column {name:?} in the schema, got none"
            ))
        })?;
        let mut nulls = 0_i64;
        let mut lower: Option<Vec<u8>> = None;
        let mut upper: Option<Vec<u8>> = None;
        for batch in batches {
            let Some(column) = batch.column_by_name(name) else {
                continue;
            };
            nulls = nulls.saturating_add(i64::try_from(column.null_count()).unwrap_or(i64::MAX));
            if !is_portable(data_type) {
                continue;
            }
            if let Some(encoded) = super::table::extreme(column, field, false)? {
                fold_encoded(&mut lower, &encoded, data_type, true);
            }
            if let Some(encoded) = super::table::extreme(column, field, true)? {
                fold_encoded(&mut upper, &encoded, data_type, false);
            }
        }
        file.value_counts.push((*id, num_rows));
        file.null_value_counts.push((*id, nulls));
        if let Some(bytes) = lower {
            file.lower_bounds.push((*id, bytes));
        }
        if let Some(bytes) = upper {
            file.upper_bounds.push((*id, bytes));
        }
    }
    Ok(file)
}

/// Keep the smaller or larger of a running bound and one encoded candidate.
fn fold_encoded(
    current: &mut Option<Vec<u8>>,
    candidate: &[u8],
    data_type: &DataType,
    minimum: bool,
) {
    match current {
        None => *current = Some(candidate.to_vec()),
        Some(held) => {
            let ordering = compare_single(candidate, held, data_type);
            if (minimum && ordering.is_lt()) || (!minimum && ordering.is_gt()) {
                *current = Some(candidate.to_vec());
            }
        }
    }
}

/// Return the top-level primitive columns, with their ids and datatypes.
///
/// Nested columns are skipped: a Parquet leaf below a struct has a dotted path
/// and its own field id, and matching the two up is a mapping this module does
/// not need in order to be correct. A missing statistic costs a planner a file
/// read; a wrong one costs correctness.
fn leaf_columns(schema: &Field) -> Result<Vec<(String, i32, DataType)>> {
    let mut columns = Vec::with_capacity(schema.field_len());
    for field in schema.fields() {
        let id = field.parquet_field_id()?.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a PARQUET:field_id on {:?}; call assign_field_ids first",
                field.name()
            ))
        })?;
        if field.data_type().as_fields().is_some() {
            continue;
        }
        columns.push((field.name().to_owned(), id, field.data_type().clone()));
    }
    Ok(columns)
}

/// Keep the smaller or larger of a running bound and one column chunk's.
fn fold_bound(
    current: &mut Option<Vec<u8>>,
    column: &ColumnStatistics,
    data_type: &DataType,
    minimum: bool,
) {
    let candidate = if minimum {
        column.min_bytes.as_ref()
    } else {
        column.max_bytes.as_ref()
    };
    let Some(candidate) = candidate else {
        return;
    };
    match current {
        None => *current = Some(candidate.clone()),
        Some(held) => {
            let ordering = compare_single(held, candidate, data_type);
            let replace = if minimum {
                ordering == std::cmp::Ordering::Greater
            } else {
                ordering == std::cmp::Ordering::Less
            };
            if replace {
                *current = Some(candidate.clone());
            }
        }
    }
}

/// Report a schema a manifest cannot key its statistics by.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
