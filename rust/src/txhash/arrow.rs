//! Coupled-value columns over Arrow data.
//!
//! A column of coupled values is one `fixed_size_binary` of the coupled
//! width, and it is built in one pass: the instant column is read once as a
//! unix count at the declared resolution, the digest column is whatever
//! [`crate::xxhash::arrow`] answers for the same rows, and the two are laid
//! side by side per row. Nothing here hashes on its own - a row's digest half
//! is exactly [`row_digests`](crate::xxhash::arrow::row_digests) of that
//! row, so a coupled column and a plain digest column agree wherever they
//! overlap.
//!
//! A null instant names no key, so the coupled cell is null. The digest
//! functions answer no nulls, so a coupled column carries exactly the instant
//! column's nulls.

use std::sync::Arc;

use arrow_array::types::{
    Date32Type, Date64Type, Int8Type, Int16Type, Int32Type, Int64Type, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type,
};
use arrow_array::{
    Array, ArrayRef, ArrowPrimitiveType, FixedSizeBinaryArray, Int32Array, Int64Array,
    PrimitiveArray, RecordBatch, StructArray, UInt32Array, UInt64Array,
};
use arrow_buffer::{Buffer, NullBuffer, ScalarBuffer};
use arrow_schema::{DataType as ArrowDataType, TimeUnit as ArrowTimeUnit};

use crate::arrow::{Error, Result};
use crate::xxhash::arrow::{
    ArrowDigestState, collect as collect_digests, column_digests_with, downcast, row_digests_with,
};
use crate::{DataType, Digest, DigestAlgorithm, Field, TimeUnit, Timezone};

use super::time::{restate_unix, restatement_overflow, validate_unit};
use super::value::{TxHash, UNIX_WIDTH, width};

/// Return whether a datatype can be read as an instant column.
///
/// The three shapes [`unix_array`] reads: a datetime at any resolution and
/// zone, a date, and an integer that is already a unix count.
pub(crate) const fn accepts_time(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::DateTime64 { .. }
            | DataType::Date32
            | DataType::Date64
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}

/// Read an instant column as unix counts of `unit`, nulls kept.
///
/// A timestamp of any resolution and zone, a date, or an integer column:
/// [`restate_unix`] carries the rule each count follows, settled once for the
/// column so the pass is one multiplication or one division per row. A
/// column whose storage is already the answer - a timestamp at `unit`, a
/// `date64` at milliseconds, an `int64` - shares its buffer rather than
/// copying it; a narrower integer or a day count widens into a fresh one.
///
/// ```
/// use arrow_array::{Array as _, TimestampSecondArray};
/// use yggdryl::{TimeUnit, txhash};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let seconds = TimestampSecondArray::from(vec![Some(1_700_000_000), None]).with_timezone("UTC");
/// let micros = txhash::arrow::unix_array(&seconds, TimeUnit::Microsecond)?;
/// assert_eq!(micros.value(0), 1_700_000_000_000_000);
/// assert!(micros.is_null(1));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when the column is none of those shapes, when `unit` is
/// not a clock resolution, or when a count does not fit `unit`.
pub fn unix_array(array: &dyn Array, unit: TimeUnit) -> Result<Int64Array> {
    let (counts, from) = raw_counts(array)?;
    match from {
        Some(from) => Restatement::new(from, unit)?.column(&counts),
        None => {
            validate_unit(unit)?;
            Ok(counts)
        }
    }
}

/// Read an instant column as the counts it stores and the unit it stores
/// them in, `None` when the counts are already at whatever unit is asked.
///
/// An `int64`-backed layout shares its buffer; a narrower integer or a day
/// count widens into a fresh one. A `uint64` is refused where a count does
/// not fit, because a signed 64-bit count is what every instant here is.
fn raw_counts(array: &dyn Array) -> Result<(Int64Array, Option<TimeUnit>)> {
    fn shared<T: ArrowPrimitiveType<Native = i64>>(array: &dyn Array) -> Result<Int64Array> {
        let array = downcast::<PrimitiveArray<T>>(array)?;
        Ok(Int64Array::new(
            array.values().clone(),
            array.nulls().cloned(),
        ))
    }
    fn widened<T: ArrowPrimitiveType>(array: &dyn Array) -> Result<Int64Array>
    where
        i64: From<T::Native>,
    {
        Ok(downcast::<PrimitiveArray<T>>(array)?.unary(i64::from))
    }
    let read = match array.data_type() {
        ArrowDataType::Timestamp(ArrowTimeUnit::Second, _) => (
            shared::<TimestampSecondType>(array)?,
            Some(TimeUnit::Second),
        ),
        ArrowDataType::Timestamp(ArrowTimeUnit::Millisecond, _) => (
            shared::<TimestampMillisecondType>(array)?,
            Some(TimeUnit::Millisecond),
        ),
        ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, _) => (
            shared::<TimestampMicrosecondType>(array)?,
            Some(TimeUnit::Microsecond),
        ),
        ArrowDataType::Timestamp(ArrowTimeUnit::Nanosecond, _) => (
            shared::<TimestampNanosecondType>(array)?,
            Some(TimeUnit::Nanosecond),
        ),
        ArrowDataType::Date32 => (widened::<Date32Type>(array)?, Some(TimeUnit::Day)),
        ArrowDataType::Date64 => (shared::<Date64Type>(array)?, Some(TimeUnit::Millisecond)),
        // An integer is the count already; the width is the only thing read.
        ArrowDataType::Int64 => (shared::<Int64Type>(array)?, None),
        ArrowDataType::Int8 => (widened::<Int8Type>(array)?, None),
        ArrowDataType::Int16 => (widened::<Int16Type>(array)?, None),
        ArrowDataType::Int32 => (widened::<Int32Type>(array)?, None),
        ArrowDataType::UInt8 => (widened::<UInt8Type>(array)?, None),
        ArrowDataType::UInt16 => (widened::<UInt16Type>(array)?, None),
        ArrowDataType::UInt32 => (widened::<UInt32Type>(array)?, None),
        ArrowDataType::UInt64 => (
            downcast::<UInt64Array>(array)?.try_unary::<_, Int64Type, Error>(|count| {
                i64::try_from(count).map_err(|_| {
                    Error::IncompatibleSchema(format!(
                        "instant {count} does not fit a signed 64-bit unix count"
                    ))
                })
            })?,
            None,
        ),
        other => {
            return Err(Error::IncompatibleSchema(format!(
                "expected a timestamp, date, or integer instant column, got {other}"
            )));
        }
    };
    Ok(read)
}

/// One restatement between two resolutions, settled once for a column.
///
/// [`restate_unix`] is the rule, asked once per direction: a coarser source
/// scales by one factor and can overflow, a finer source floors by one
/// divisor and cannot fail. The per-row work is that one multiplication or
/// division and nothing else.
#[derive(Clone, Copy)]
enum Restatement {
    Same,
    Scale(i64),
    Floor(i64),
}

impl Restatement {
    fn new(from: TimeUnit, unit: TimeUnit) -> Result<Self> {
        if from == unit {
            validate_unit(unit)?;
            return Ok(Self::Same);
        }
        let scale = restate_unix(1, from, unit)?;
        if scale > 1 {
            return Ok(Self::Scale(scale));
        }
        Ok(Self::Floor(restate_unix(1, unit, from)?))
    }

    /// Restate one count.
    fn count(self, count: i64) -> Result<i64> {
        match self {
            Self::Same => Ok(count),
            Self::Scale(scale) => count
                .checked_mul(scale)
                .ok_or_else(|| Error::from(restatement_overflow())),
            Self::Floor(divisor) => Ok(count.div_euclid(divisor)),
        }
    }

    /// Restate a whole column, sharing it when there is nothing to do.
    fn column(self, counts: &Int64Array) -> Result<Int64Array> {
        match self {
            Self::Same => Ok(counts.clone()),
            Self::Scale(_) => counts.try_unary::<_, Int64Type, Error>(|count| self.count(count)),
            Self::Floor(divisor) => Ok(counts.unary(|count| count.div_euclid(divisor))),
        }
    }
}

/// Read a selected instant leaf under a Struct path for the rows a fill
/// recomputes, hiding what its parents hide.
///
/// The fill reads its time source through the same Struct-only descent a
/// digest source takes. Only a row `mask` selects is restated, so a hidden
/// row, or one holding a value the fill preserves, can never fail the batch
/// over an instant it does not couple; every other row is null here. A count
/// that does not fit the holder's unit is refused naming `holder`, the row,
/// and the count, so the refusal points at the cell rather than at the rule.
pub(crate) fn unix_selection(
    columns: &[ArrayRef],
    fields: &[Field],
    steps: &[usize],
    parent_nulls: Option<&NullBuffer>,
    unit: TimeUnit,
    mask: &[bool],
    holder: &str,
) -> Result<Int64Array> {
    let mut arrays = columns;
    let mut fields = fields;
    let mut hidden = parent_nulls.cloned();
    for (depth, index) in steps.iter().copied().enumerate() {
        let array = &arrays[index];
        if depth + 1 == steps.len() {
            let (counts, from) = raw_counts(array.as_ref())?;
            if mask.len() != counts.len() {
                return Err(Error::IncompatibleSchema(format!(
                    "holder {holder}: digest:time source has {} rows, the batch {}",
                    counts.len(),
                    mask.len()
                )));
            }
            let restatement = match from {
                Some(from) => Restatement::new(from, unit)?,
                None => Restatement::Same,
            };
            let mut values = Vec::with_capacity(counts.len());
            let mut valid = Vec::with_capacity(counts.len());
            for (row, selected) in mask.iter().copied().enumerate() {
                let present = selected
                    && counts.is_valid(row)
                    && hidden.as_ref().is_none_or(|nulls| nulls.is_valid(row));
                values.push(if present {
                    let count = counts.value(row);
                    restatement.count(count).map_err(|_| {
                        Error::IncompatibleSchema(format!(
                            "holder {holder} row {row}: instant {count} at {} does not fit a signed 64-bit count of {unit}",
                            from.map_or_else(|| unit.to_string(), |from| from.to_string())
                        ))
                    })?
                } else {
                    0
                });
                valid.push(present);
            }
            return Ok(Int64Array::new(
                ScalarBuffer::from(values),
                Some(NullBuffer::from(valid)),
            ));
        }
        let nested = downcast::<StructArray>(array.as_ref())?;
        hidden = NullBuffer::union(hidden.as_ref(), nested.nulls());
        arrays = nested.columns();
        fields = fields[index].fields();
    }
    Err(Error::IncompatibleSchema(
        "a digest time selection cannot have an empty path".to_owned(),
    ))
}

/// Lay unix counts and digests side by side, one coupled cell per row.
///
/// A row whose instant is null - or that `nulls` marks - is a null cell;
/// the canonical bytes of every other row are the value's own.
pub(crate) fn collect(
    unix: &Int64Array,
    digests: &[Digest],
    nulls: Option<&NullBuffer>,
    unit: TimeUnit,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    if unix.len() != digests.len() {
        return Err(Error::IncompatibleSchema(format!(
            "instant column has {} rows, the digests {}",
            unix.len(),
            digests.len()
        )));
    }
    validate_unit(unit)?;
    let width = width(algorithm);
    let nulls = NullBuffer::union(unix.nulls(), nulls);
    let mut flat = vec![0_u8; unix.len() * width];
    for (row, digest) in digests.iter().enumerate() {
        if nulls.as_ref().is_some_and(|nulls| nulls.is_null(row)) {
            continue;
        }
        if digest.algorithm() != algorithm {
            return Err(Error::IncompatibleSchema(format!(
                "row {row}: expected a {algorithm} digest, got {}",
                digest.algorithm()
            )));
        }
        let value = TxHash::couple(unit, unix.value(row), *digest);
        flat[row * width..(row + 1) * width].copy_from_slice(&value.into_bytes());
    }
    Ok(Arc::new(FixedSizeBinaryArray::new(
        fixed(width),
        Buffer::from_vec(flat),
        nulls,
    )))
}

/// The width as Arrow spells it.
fn fixed(width: usize) -> i32 {
    i32::try_from(width).unwrap_or_else(|_| unreachable!("no coupled value is wider than 24 bytes"))
}

/// Couple every row's digest with the instant beside it.
///
/// The digest half is [`row_digests`](crate::xxhash::arrow::row_digests) of
/// the batch under the same algorithm: every column but a `digest:role`
/// holder, the instant column included when it is one, framed as the row's
/// ordered sequence. The instant column is read as [`unix_array`] reads it
/// and need not be a column of the batch at all. The answer is one
/// `fixed_size_binary` of the coupled width, null where the instant is.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Array as _, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray};
/// use arrow_schema::{DataType, Field, Schema};
/// use yggdryl::{DigestAlgorithm, TimeUnit, txhash};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let batch = RecordBatch::try_new(
///     Arc::new(Schema::new(vec![
///         Field::new("symbol", DataType::Utf8, false),
///         Field::new("quantity", DataType::Int64, false),
///     ])),
///     vec![
///         Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
///         Arc::new(Int64Array::from(vec![100, 250, 100])),
///     ],
/// )?;
/// let times = TimestampMicrosecondArray::from(vec![1_700_000_000_000_000, 1_700_000_000_000_001, 1_700_000_000_000_002]);
///
/// let coupled = txhash::arrow::row_txhashes(&batch, &times, TimeUnit::Microsecond, DigestAlgorithm::Xxh3)?;
/// assert_eq!(coupled.data_type(), &DataType::FixedSizeBinary(16));
/// // The instant leads, so identical rows at different instants stay apart
/// // and sort in time order.
/// let (instants, digests) = txhash::arrow::decompose(&coupled, TimeUnit::Microsecond, DigestAlgorithm::Xxh3)?;
/// assert_eq!(instants.data_type(), &DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into())));
/// assert_eq!(&digests, &yggdryl::xxhash::arrow::row_digests(&batch, DigestAlgorithm::Xxh3)?);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when `times` is not an instant column of the batch's
/// length, when a column's schema does not project to the core datatype
/// model, or when a value cannot be represented.
pub fn row_txhashes(
    batch: &RecordBatch,
    times: &dyn Array,
    unit: TimeUnit,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    row_txhashes_with(&algorithm.digester(), batch, times, unit)
}

/// The prototype-driven form of [`row_txhashes`], seeded or not.
pub(crate) fn row_txhashes_with<S: ArrowDigestState>(
    prototype: &S,
    batch: &RecordBatch,
    times: &dyn Array,
    unit: TimeUnit,
) -> Result<ArrayRef> {
    require_length(times, batch.num_rows())?;
    let unix = unix_array(times, unit)?;
    let digests = row_digests_with(prototype, batch)?;
    collect(&unix, &digests, None, unit, prototype.algorithm())
}

/// Couple every cell's digest with the instant beside it.
///
/// The digest half is
/// [`column_digests`](crate::xxhash::arrow::column_digests) of the array
/// under `field`: the cell's own value, reconciled to the declaration first
/// and with no row framing around it.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Array as _, ArrayRef, Int32Array, Int64Array, StringArray, TimestampSecondArray};
/// use yggdryl::xxhash::arrow::column_digests;
/// use yggdryl::{DataType, DigestAlgorithm, Field, TimeUnit, txhash};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let instants = TimestampSecondArray::from(vec![1_700_000_000, 1_700_000_001]);
/// let symbols: ArrayRef = Arc::new(StringArray::from(vec!["AAPL", "MSFT"]));
/// let field = Field::new("symbol", DataType::utf8(), false);
///
/// let coupled = txhash::arrow::column_txhashes(&instants, Arc::clone(&symbols), &field, TimeUnit::Second, DigestAlgorithm::Xxh3)?;
/// let (times, digests) = txhash::arrow::decompose(coupled.as_ref(), TimeUnit::Second, DigestAlgorithm::Xxh3)?;
/// assert_eq!(&digests, &column_digests(symbols, &field, DigestAlgorithm::Xxh3)?);
/// assert_eq!(times.len(), 2);
///
/// // Reconciliation is the column digest's: an int32 column read under an
/// // int64 declaration is the same numbers.
/// let declared = Field::new("quantity", DataType::Int64, false);
/// let narrow: ArrayRef = Arc::new(Int32Array::from(vec![100, 250]));
/// let wide: ArrayRef = Arc::new(Int64Array::from(vec![100, 250]));
/// assert_eq!(
///     &txhash::arrow::column_txhashes(&instants, narrow, &declared, TimeUnit::Second, DigestAlgorithm::Xxh3)?,
///     &txhash::arrow::column_txhashes(&instants, wide, &declared, TimeUnit::Second, DigestAlgorithm::Xxh3)?,
/// );
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when `times` is not an instant column of the array's
/// length, when the array cannot be reconciled to `field`, or when a value
/// cannot be represented.
pub fn column_txhashes(
    times: &dyn Array,
    values: ArrayRef,
    field: &Field,
    unit: TimeUnit,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    column_txhashes_with(&algorithm.digester(), times, values, field, unit)
}

/// The prototype-driven form of [`column_txhashes`], seeded or not.
pub(crate) fn column_txhashes_with<S: ArrowDigestState>(
    prototype: &S,
    times: &dyn Array,
    values: ArrayRef,
    field: &Field,
    unit: TimeUnit,
) -> Result<ArrayRef> {
    require_length(times, values.len())?;
    let unix = unix_array(times, unit)?;
    let digests = column_digests_with(prototype, values, field)?;
    collect(&unix, &digests, None, unit, prototype.algorithm())
}

fn require_length(times: &dyn Array, rows: usize) -> Result<()> {
    if times.len() == rows {
        return Ok(());
    }
    Err(Error::IncompatibleSchema(format!(
        "instant column has {} rows, the values {rows}",
        times.len()
    )))
}

/// Couple an instant column with a digest column already computed.
///
/// The digest column is the width [`row_digests`](crate::xxhash::arrow::row_digests)
/// answers for the algorithm (`uint32`, `uint64`, or `fixed_size_binary(16)`),
/// or the signed same-width storage a holder may keep, read as the same bits.
/// A null on either side is a null cell.
///
/// ```
/// use arrow_array::cast::AsArray as _;
/// use arrow_array::types::{TimestampSecondType, UInt64Type};
/// use arrow_array::{Array as _, Int64Array, TimestampMillisecondArray, UInt64Array};
/// use yggdryl::{DigestAlgorithm, TimeUnit, txhash};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let instants = TimestampMillisecondArray::from(vec![Some(1_000), None]);
/// let digests = UInt64Array::from(vec![7, 8]);
/// let coupled = txhash::arrow::compose(&instants, &digests, TimeUnit::Second, DigestAlgorithm::Xxh3)?;
/// assert!(coupled.is_null(1), "a null instant is a null cell");
///
/// // A signed holder's storage is the same bits.
/// let signed = Int64Array::from(vec![7, 8]);
/// assert_eq!(&txhash::arrow::compose(&instants, &signed, TimeUnit::Second, DigestAlgorithm::Xxh3)?, &coupled);
///
/// // The halves come back at the unit and width they went in.
/// let (times, split) = txhash::arrow::decompose(coupled.as_ref(), TimeUnit::Second, DigestAlgorithm::Xxh3)?;
/// assert_eq!(times.as_primitive::<TimestampSecondType>().value(0), 1);
/// assert_eq!(split.as_primitive::<UInt64Type>().value(0), 7);
/// assert!(times.is_null(1) && split.is_null(1));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when the columns differ in length or the digest column
/// is not the algorithm's width.
pub fn compose(
    times: &dyn Array,
    digests: &dyn Array,
    unit: TimeUnit,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    require_length(times, digests.len())?;
    let unix = unix_array(times, unit)?;
    let payloads = digests_of(digests, algorithm)?;
    collect(&unix, &payloads, digests.nulls(), unit, algorithm)
}

/// Read a digest column back as the digests it holds, nulls as zero.
fn digests_of(array: &dyn Array, algorithm: DigestAlgorithm) -> Result<Vec<Digest>> {
    let payloads: Vec<u128> = match (algorithm, array.data_type()) {
        (DigestAlgorithm::Xxh32, ArrowDataType::UInt32) => downcast::<UInt32Array>(array)?
            .values()
            .iter()
            .map(|value| u128::from(*value))
            .collect(),
        (DigestAlgorithm::Xxh32, ArrowDataType::Int32) => downcast::<Int32Array>(array)?
            .values()
            .iter()
            .map(|value| u128::from(u32::from_ne_bytes(value.to_ne_bytes())))
            .collect(),
        (DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3, ArrowDataType::UInt64) => {
            downcast::<UInt64Array>(array)?
                .values()
                .iter()
                .map(|value| u128::from(*value))
                .collect()
        }
        (DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3, ArrowDataType::Int64) => {
            downcast::<Int64Array>(array)?
                .values()
                .iter()
                .map(|value| u128::from(u64::from_ne_bytes(value.to_ne_bytes())))
                .collect()
        }
        (DigestAlgorithm::Xxh128, ArrowDataType::FixedSizeBinary(16)) => {
            let array = downcast::<FixedSizeBinaryArray>(array)?;
            (0..array.len())
                .map(|row| {
                    let mut wide = [0_u8; 16];
                    wide.copy_from_slice(array.value(row));
                    u128::from_be_bytes(wide)
                })
                .collect()
        }
        (algorithm, other) => {
            return Err(Error::IncompatibleSchema(format!(
                "expected a {} column of {algorithm} digests, got {other}",
                match algorithm {
                    DigestAlgorithm::Xxh32 => "uint32 or int32",
                    DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => "uint64 or int64",
                    DigestAlgorithm::Xxh128 => "fixed_size_binary(16)",
                }
            )));
        }
    };
    Ok(payloads
        .into_iter()
        .map(|payload| Digest::new(algorithm, payload))
        .collect())
}

/// Split a coupled column into its instant column and its digest column.
///
/// The instant column is a `Timestamp` of `unit` in UTC, the digest column
/// the width [`row_digests`](crate::xxhash::arrow::row_digests) answers;
/// nulls carry to both. [`compose`] is the inverse.
///
/// # Errors
///
/// Returns an error when the column is not a `fixed_size_binary` of the
/// coupled width.
pub fn decompose(
    array: &dyn Array,
    unit: TimeUnit,
    algorithm: DigestAlgorithm,
) -> Result<(ArrayRef, ArrayRef)> {
    validate_unit(unit)?;
    let array = downcast::<FixedSizeBinaryArray>(array)?;
    let width = width(algorithm);
    if array.value_length() != fixed(width) {
        return Err(Error::IncompatibleSchema(format!(
            "expected a fixed_size_binary({width}) column of {unit} instants with {algorithm} digests, got {}",
            array.data_type()
        )));
    }
    let mut unix = Vec::with_capacity(array.len());
    let mut digests = Vec::with_capacity(array.len());
    for row in 0..array.len() {
        if array.is_null(row) {
            unix.push(0);
            digests.push(Digest::new(algorithm, 0));
            continue;
        }
        // The width was checked on the column, so a row is read as its two
        // halves with nothing left to validate.
        let bytes = array.value(row);
        let mut instant = [0_u8; UNIX_WIDTH];
        instant.copy_from_slice(&bytes[..UNIX_WIDTH]);
        unix.push(i64::from_be_bytes(instant));
        digests.push(Digest::from_bytes(algorithm, &bytes[UNIX_WIDTH..])?);
    }
    let nulls = array.nulls().cloned();
    Ok((
        timestamps(ScalarBuffer::from(unix), nulls.clone(), unit),
        collect_digests(&digests, algorithm, nulls),
    ))
}

/// Build the UTC timestamp column of one resolution.
fn timestamps(counts: ScalarBuffer<i64>, nulls: Option<NullBuffer>, unit: TimeUnit) -> ArrayRef {
    let zone = Timezone::UTC.as_str();
    match unit {
        TimeUnit::Second => {
            Arc::new(PrimitiveArray::<TimestampSecondType>::new(counts, nulls).with_timezone(zone))
        }
        TimeUnit::Millisecond => Arc::new(
            PrimitiveArray::<TimestampMillisecondType>::new(counts, nulls).with_timezone(zone),
        ),
        TimeUnit::Microsecond => Arc::new(
            PrimitiveArray::<TimestampMicrosecondType>::new(counts, nulls).with_timezone(zone),
        ),
        TimeUnit::Nanosecond => Arc::new(
            PrimitiveArray::<TimestampNanosecondType>::new(counts, nulls).with_timezone(zone),
        ),
        _ => unreachable!("the unit was validated first"),
    }
}

#[cfg(test)]
mod tests;
