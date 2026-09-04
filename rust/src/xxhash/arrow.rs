//! Per-row and per-column digest arrays over Arrow data.
//!
//! A digest column is the dedup key, the change-detection column, and the
//! hash-join key a table actually wants, and it is one pass over the buffers
//! rather than one materialized value per cell.
//!
//! The answer is defined by the value model, not by the layout: a row digest
//! equals feeding that row's [`Scalar`](crate::Scalar) through
//! [`Scalar::write_bytes`](crate::Scalar::write_bytes), and a column digest
//! equals feeding that cell's value. Where the layout allows it the bytes are
//! read straight from the Arrow buffer into the same encoding; everything else
//! falls back to the shared scalar boundary, so the path stays exhaustive over
//! every datatype family.

use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{
    Date32Type, Date64Type, DurationMicrosecondType, DurationMillisecondType,
    DurationNanosecondType, DurationSecondType, Float16Type, Float32Type, Float64Type, Int8Type,
    Int16Type, Int32Type, Int64Type, Time32MillisecondType, Time32SecondType,
    Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, Decimal128Array, Decimal256Array, FixedSizeBinaryArray, RecordBatch,
    UInt32Array, UInt64Array,
};

use crate::arrow::{Error, Result};
use crate::generic::TemporalFamily;
use crate::{DataType, Digest, DigestAlgorithm, Digester, Field, I256, TimeUnit, Timezone};

use super::scalar::{
    write_binary, write_bool, write_decimal, write_float, write_null, write_sequence_header,
    write_signed, write_string, write_temporal, write_unsigned,
};

/// Digest every row of a batch, in schema order.
///
/// A row is the ordered sequence of its columns, which is the canonical row
/// shape everywhere in this project, so the answer is exactly
/// `arrow::batch_to_value(batch)`'s rows fed through
/// [`Scalar::write_bytes`](crate::Scalar::write_bytes) - without building one
/// value.
///
/// The result is a `UInt32Array` for XXH32, a `UInt64Array` for the two
/// 64-bit algorithms, and a `FixedSizeBinary(16)` of canonical big-endian
/// bytes for XXH3-128, which has no native Arrow integer wide enough.
///
/// ```
/// use arrow_array::{Int64Array, RecordBatch, StringArray, UInt64Array};
/// use arrow_array::cast::AsArray as _;
/// use arrow_schema::{DataType, Field, Schema};
/// use std::sync::Arc;
///
/// use yggdryl::DigestAlgorithm;
/// use yggdryl::xxhash::arrow::row_digests;
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
///
/// let digests = row_digests(&batch, DigestAlgorithm::Xxh3_64)?;
/// let digests = digests.as_primitive::<arrow_array::types::UInt64Type>();
/// // Identical rows answer identical digests, which is what makes this a
/// // dedup key.
/// assert_eq!(digests.value(0), digests.value(2));
/// assert_ne!(digests.value(0), digests.value(1));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when a column's schema does not project to the core
/// datatype model, or a value cannot be represented.
pub fn row_digests(batch: &RecordBatch, algorithm: DigestAlgorithm) -> Result<ArrayRef> {
    let fields: Vec<Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| Field::from_arrow_ref(Arc::clone(field)).map_err(Error::from))
        .collect::<Result<_>>()?;
    let columns = batch.columns();
    let mut digests = Vec::with_capacity(batch.num_rows());
    let mut digester = algorithm.digester();
    for index in 0..batch.num_rows() {
        digester.clear();
        write_sequence_header(&mut digester, columns.len());
        for (column, field) in columns.iter().zip(fields.iter()) {
            feed_cell(&mut digester, field.dtype(), column.as_ref(), index)?;
        }
        digests.push(digester.as_digest());
    }
    Ok(collect(&digests, algorithm))
}

/// Digest every value of one column.
///
/// This is the single-column form [`row_digests`] composes: each answer is the
/// cell's own value fed through
/// [`Scalar::write_bytes`](crate::Scalar::write_bytes), with no row framing
/// around it. A null feeds the null tag, so a null and an empty string never
/// collide.
///
/// # Errors
///
/// Returns an error when `field` does not describe `array`, or a value cannot
/// be represented.
pub fn column_digests(
    array: &dyn Array,
    field: &Field,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    let mut digests = Vec::with_capacity(array.len());
    let mut digester = algorithm.digester();
    for index in 0..array.len() {
        digester.clear();
        feed_cell(&mut digester, field.dtype(), array, index)?;
        digests.push(digester.as_digest());
    }
    Ok(collect(&digests, algorithm))
}

/// Build the digest column the algorithm's width calls for.
fn collect(digests: &[Digest], algorithm: DigestAlgorithm) -> ArrayRef {
    match algorithm {
        DigestAlgorithm::Xxh32 => Arc::new(UInt32Array::from_iter_values(
            digests.iter().filter_map(|digest| digest.as_u32()),
        )),
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3_64 => Arc::new(
            UInt64Array::from_iter_values(digests.iter().filter_map(|digest| digest.as_u64())),
        ),
        DigestAlgorithm::Xxh3_128 => {
            // The canonical big-endian bytes, because no Arrow integer is 128
            // bits wide and a pair of `u64` columns would put the wire order
            // in the caller's hands.
            let bytes: Vec<[u8; 16]> = digests
                .iter()
                .map(|digest| {
                    let mut wide = [0_u8; 16];
                    wide.copy_from_slice(&digest.into_bytes());
                    wide
                })
                .collect();
            // Built from the flat buffer rather than an iterator, because an
            // empty batch carries no element for a width to be inferred from
            // and the column still has to be `FixedSizeBinary(16)`.
            let flat: Vec<u8> = bytes.concat();
            Arc::new(FixedSizeBinaryArray::new(
                16,
                arrow_buffer::Buffer::from_vec(flat),
                None,
            ))
        }
    }
}

/// Feed one cell's canonical bytes, reading the buffer where the layout allows.
///
/// The fallback is the shared scalar boundary, so every datatype family the
/// core can read is covered; the buffer arms exist only to skip materializing
/// a value whose bytes are already sitting in the column.
fn feed_cell(
    digester: &mut Digester,
    dtype: &DataType,
    array: &dyn Array,
    index: usize,
) -> Result<()> {
    // A union or a run-end encoding hides its own validity, so absence there
    // is the child's answer rather than the parent's, exactly as the scalar
    // boundary reads it.
    if array.is_null(index) && !matches!(dtype, DataType::Union(..) | DataType::RunEndEncoded(_)) {
        write_null(digester);
        return Ok(());
    }
    match dtype {
        DataType::Null => write_null(digester),
        DataType::Boolean => write_bool(digester, array.as_boolean().value(index)),
        DataType::Int8 => write_signed(
            digester,
            i128::from(array.as_primitive::<Int8Type>().value(index)),
        ),
        DataType::Int16 => {
            write_signed(
                digester,
                i128::from(array.as_primitive::<Int16Type>().value(index)),
            );
        }
        DataType::Int32 => {
            write_signed(
                digester,
                i128::from(array.as_primitive::<Int32Type>().value(index)),
            );
        }
        DataType::Int64 => {
            write_signed(
                digester,
                i128::from(array.as_primitive::<Int64Type>().value(index)),
            );
        }
        DataType::UInt8 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt8Type>().value(index)),
            );
        }
        DataType::UInt16 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt16Type>().value(index)),
            );
        }
        DataType::UInt32 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt32Type>().value(index)),
            );
        }
        DataType::UInt64 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt64Type>().value(index)),
            );
        }
        DataType::Float16 => write_float(
            digester,
            f64::from(array.as_primitive::<Float16Type>().value(index).to_f32()),
        ),
        DataType::Float32 => write_float(
            digester,
            f64::from(array.as_primitive::<Float32Type>().value(index)),
        ),
        DataType::Float64 => {
            write_float(digester, array.as_primitive::<Float64Type>().value(index))
        }
        DataType::Utf8 => write_string(digester, array.as_string::<i32>().value(index)),
        DataType::LargeUtf8 => write_string(digester, array.as_string::<i64>().value(index)),
        DataType::Utf8View => write_string(digester, array.as_string_view().value(index)),
        DataType::Binary => write_binary(digester, array.as_binary::<i32>().value(index)),
        DataType::LargeBinary => write_binary(digester, array.as_binary::<i64>().value(index)),
        DataType::BinaryView => write_binary(digester, array.as_binary_view().value(index)),
        DataType::FixedSizeBinary(_) => write_binary(
            digester,
            downcast::<FixedSizeBinaryArray>(array)?.value(index),
        ),
        DataType::Decimal128 { scale, .. } => write_decimal(
            digester,
            I256::from_i128(downcast::<Decimal128Array>(array)?.value(index)),
            *scale,
        ),
        DataType::Decimal256 { scale, .. } => write_decimal(
            digester,
            I256::from_le_bytes(
                downcast::<Decimal256Array>(array)?
                    .value(index)
                    .to_le_bytes(),
            ),
            *scale,
        ),
        DataType::Date32 => temporal(
            digester,
            TemporalFamily::Date,
            i64::from(array.as_primitive::<Date32Type>().value(index)),
            TimeUnit::Day,
            &Timezone::NAIVE,
        ),
        DataType::Date64 => temporal(
            digester,
            TemporalFamily::Date,
            array.as_primitive::<Date64Type>().value(index),
            TimeUnit::Millisecond,
            &Timezone::NAIVE,
        ),
        DataType::Time32(unit) => {
            let count = match unit {
                TimeUnit::Second => array.as_primitive::<Time32SecondType>().value(index),
                TimeUnit::Millisecond => array.as_primitive::<Time32MillisecondType>().value(index),
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::Time,
                i64::from(count),
                *unit,
                &Timezone::NAIVE,
            );
        }
        DataType::Time64(unit) => {
            let count = match unit {
                TimeUnit::Microsecond => array.as_primitive::<Time64MicrosecondType>().value(index),
                TimeUnit::Nanosecond => array.as_primitive::<Time64NanosecondType>().value(index),
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::Time,
                count,
                *unit,
                &Timezone::NAIVE,
            );
        }
        DataType::Timestamp(unit, zone) => {
            let count = match unit {
                TimeUnit::Second => array.as_primitive::<TimestampSecondType>().value(index),
                TimeUnit::Millisecond => array
                    .as_primitive::<TimestampMillisecondType>()
                    .value(index),
                TimeUnit::Microsecond => array
                    .as_primitive::<TimestampMicrosecondType>()
                    .value(index),
                TimeUnit::Nanosecond => {
                    array.as_primitive::<TimestampNanosecondType>().value(index)
                }
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::DateTime,
                count,
                *unit,
                zone.as_ref().unwrap_or(&Timezone::NAIVE),
            );
        }
        DataType::Duration64(unit) => {
            let count = match unit {
                TimeUnit::Second => array.as_primitive::<DurationSecondType>().value(index),
                TimeUnit::Millisecond => {
                    array.as_primitive::<DurationMillisecondType>().value(index)
                }
                TimeUnit::Microsecond => {
                    array.as_primitive::<DurationMicrosecondType>().value(index)
                }
                TimeUnit::Nanosecond => array.as_primitive::<DurationNanosecondType>().value(index),
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::Duration,
                count,
                *unit,
                &Timezone::NAIVE,
            );
        }
        // Ascii text is trimmed on the way out, geospatial carries its own
        // tag, and every nested, dictionary, union, run-end, and variant
        // layout composes values rather than holding one buffer, so all of
        // them read through the shared boundary.
        _ => return fallback(digester, dtype, array, index),
    }
    Ok(())
}

/// Feed a temporal read straight from a buffer.
fn temporal(
    digester: &mut Digester,
    family: TemporalFamily,
    count: i64,
    unit: TimeUnit,
    zone: &Timezone,
) {
    write_temporal(digester, family, count, unit, zone);
}

/// Feed one cell through the shared scalar boundary.
fn fallback(
    digester: &mut Digester,
    dtype: &DataType,
    array: &dyn Array,
    index: usize,
) -> Result<()> {
    let value = crate::arrow::value::value_from_array(dtype, array, index)?;
    digester.write_scalar(&value);
    Ok(())
}

/// Downcast an array to the layout its datatype names.
fn downcast<T: 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected an Arrow {} array, got {}",
            std::any::type_name::<T>(),
            array.data_type()
        ))
    })
}

#[cfg(test)]
mod tests;
