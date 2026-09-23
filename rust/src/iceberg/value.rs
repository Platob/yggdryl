//! Iceberg's two renderings of one scalar value: its text and its bytes.
//!
//! Two places in the format store a value as text rather than as data: a
//! partition directory name and a snapshot summary entry. Both need the same
//! rendering, and neither can use the core [`Scalar`]'s serialization, because
//! `"XNAS"` must become `XNAS` and not `"XNAS"`.
//!
//! The rendering itself is not Iceberg's. A `column=value` directory is the
//! layout the whole project reads and writes, so the text comes from
//! [`crate::media::partition::partition_text`] - the same formatter, with the same
//! null spelling, that a partitioned folder write applies to a column. A table
//! this module writes is therefore a lake the rest of the crate can walk,
//! rather than one that happens to look like one.
//!
//! The textual rendering is deliberately not the inverse of anything. A
//! partition path spells a null value `null`, which is indistinguishable from
//! the string `"null"`, so a reader takes partition values from the manifest and
//! treats the path as layout only.
//!
//! The other rendering is the *single-value binary* one, which is what a
//! manifest bound and a manifest-list field summary carry. It is emitted only
//! for the types whose Parquet statistic bytes already are that encoding, which
//! is what lets [`super::statistics`] hand a footer's bytes straight to a
//! manifest and lets a scan compare a filter against them without decoding
//! either side. A type outside that set has no bound rather than a bound that
//! means something else.

use iceberg_official::spec::{
    Datum as OfficialDatum, PrimitiveLiteral as OfficialPrimitiveLiteral,
    PrimitiveType as OfficialPrimitiveType,
};
use smol_str::SmolStr;

use crate::string::is_text_storage;
use crate::{DataType, Scalar, TimeUnit};

/// The literal Iceberg writes for a null partition value.
pub(super) const NULL_TEXT: &str = crate::media::partition::NULL_PARTITION;

/// Render one scalar value the way a `column=value` directory spells it.
///
/// A value that names no datatype - a sequence whose children disagree, a
/// mapping - has no directory spelling at all, so it falls back to its JSON
/// form: lossless and readable rather than invented. A partition tuple never
/// contains one, because a partition value is a scalar.
pub(super) fn scalar_text(value: &Scalar) -> SmolStr {
    crate::media::partition::partition_text(value).unwrap_or_else(|_| {
        crate::json::into_bytes(value)
            .ok()
            .and_then(|encoded| String::from_utf8(encoded).ok())
            .map_or_else(|| SmolStr::new_static(NULL_TEXT), SmolStr::new)
    })
}

/// Return whether a Parquet statistic byte string is also the Iceberg one.
///
/// A decimal is the case that differs - Parquet stores it big-endian in a fixed
/// width, Iceberg stores the minimal two's-complement big-endian - so a decimal
/// column gets counts but no bounds. A string in a charset other than UTF-8
/// or US-ASCII is the other: its statistic bytes are not the UTF-8 an Iceberg
/// string bound holds. A missing statistic costs a planner one file read; a
/// wrong one costs correctness.
pub(super) const fn is_portable(dtype: &DataType) -> bool {
    if let Some(parameters) = dtype.string_parameters() {
        return is_text_storage(parameters);
    }
    // Iceberg has `string` and nothing that carries a code's identity, so
    // every registered code is portable as the text it is.
    if dtype.is_code() {
        return true;
    }
    matches!(
        dtype,
        DataType::Boolean
            | DataType::Int32
            | DataType::Int64
            | DataType::Float32
            | DataType::Float64
            | DataType::Date32
            | DataType::Time64(TimeUnit::Microsecond)
            | DataType::DateTime64 {
                unit: TimeUnit::Microsecond | TimeUnit::Nanosecond,
                ..
            }
            | DataType::Uuid
            | crate::bytes_dtypes!()
    )
}

/// Return whether `dtype` is a string whose bytes are the UTF-8 an Iceberg
/// string holds.
const fn is_text_string(dtype: &DataType) -> bool {
    match dtype.string_parameters() {
        Some(parameters) => is_text_storage(parameters),
        None => false,
    }
}

/// Encode one scalar as the single value a manifest bound carries.
///
/// The datatype decides the encoding rather than the value's own variant,
/// because a column declared `Int32` still arrives as a 64-bit
/// [`crate::integer::Int64`]. A value that does not fit the column, and every type whose
/// encoding is not [`is_portable`], has no bytes rather than the wrong ones.
pub(super) fn single_value(value: &Scalar, dtype: &DataType) -> Option<Vec<u8>> {
    let datum = match dtype {
        DataType::Boolean => OfficialDatum::bool(value.as_bool()?),
        DataType::Int32 => OfficialDatum::int(i32::try_from(count(value)?).ok()?),
        DataType::Date32 => OfficialDatum::date(i32::try_from(count(value)?).ok()?),
        DataType::Int64 => OfficialDatum::long(count(value)?),
        #[allow(clippy::cast_possible_truncation)]
        DataType::Float32 => OfficialDatum::float(value.as_f64()? as f32),
        DataType::Float64 => OfficialDatum::double(value.as_f64()?),
        DataType::Time64(TimeUnit::Microsecond) => {
            OfficialDatum::time_micros(count(value)?).ok()?
        }
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone,
        } if timezone.is_naive() => OfficialDatum::timestamp_micros(count(value)?),
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            ..
        } => OfficialDatum::timestamptz_micros(count(value)?),
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone,
        } if timezone.is_naive() => OfficialDatum::timestamp_nanos(count(value)?),
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            ..
        } => OfficialDatum::timestamptz_nanos(count(value)?),
        // A bound over a text-storage string or a code is a string bound: the
        // value is the trimmed text.
        text if is_text_string(text) => OfficialDatum::string(value.as_str()?),
        code if code.is_code() => OfficialDatum::string(value.as_str()?),
        // An identifier is a `uuid` datum, built from the sixteen bytes the
        // canonical spelling parses to.
        DataType::Uuid => {
            let bytes = match value {
                Scalar::Uuid(value) => value.into_bytes(),
                _ => crate::uuid_parse(crate::uuid_bytes(value)?).ok()?,
            };
            OfficialDatum::uuid(uuid::Uuid::from_bytes(bytes))
        }
        crate::bytes_dtypes!() => {
            let bytes = value.as_bytes()?;
            match dtype.bytes_parameters()?.fixed() {
                None => OfficialDatum::binary(bytes.iter().copied()),
                Some(width) if usize::try_from(width).ok()? == bytes.len() => {
                    OfficialDatum::fixed(bytes.iter().copied())
                }
                Some(_) => return None,
            }
        }
        _ => return None,
    };
    // Iceberg orders floating values for comparisons, but its metrics contract
    // explicitly forbids NaN as either bound. Keep that semantic validation at
    // the shared bound codec so data-file and manifest summaries cannot emit a
    // decodable-but-invalid value.
    if datum.is_nan() {
        return None;
    }
    datum.to_bytes().ok().map(|bytes| bytes.into_vec())
}

/// Read one scalar back out of the single value a manifest bound carries.
///
/// The inverse of [`single_value`], and the reason a manifest bound can be
/// handed to the crate's own statistics pruner: the pruner compares values,
/// not bytes, so a bound has to become a value exactly once. A type whose
/// encoding [`is_portable`] does not cover has no value rather than a wrong
/// one, and the pruner then simply declines.
pub(super) fn single_to_value(bytes: &[u8], dtype: &DataType) -> Option<Scalar> {
    let datum = official_datum(bytes, dtype)?;
    let value = match (dtype, datum.literal()) {
        (DataType::Boolean, OfficialPrimitiveLiteral::Boolean(value)) => Scalar::from(*value),
        (DataType::Int32, OfficialPrimitiveLiteral::Int(value)) => Scalar::from(*value),
        (DataType::Date32, OfficialPrimitiveLiteral::Int(value)) => Scalar::date32(*value),
        (DataType::Int64, OfficialPrimitiveLiteral::Long(value)) => Scalar::from(*value),
        (DataType::Time64(unit), OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::time64(*value, *unit, crate::Timezone::NAIVE).ok()?
        }
        (DataType::DateTime64 { unit, timezone }, OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, *unit, *timezone).ok()?
        }
        (DataType::Float32, OfficialPrimitiveLiteral::Float(value)) => {
            Scalar::from(crate::Float32::from_f32((*value).into_inner()))
        }
        (DataType::Float64, OfficialPrimitiveLiteral::Double(value)) => {
            Scalar::from(crate::Float64::from_f64((*value).into_inner()))
        }
        (text, OfficialPrimitiveLiteral::String(value)) if is_text_string(text) => {
            Scalar::from(value.as_str())
        }
        // A bound is read off a column, so it becomes the value the column
        // holds: the pruner compares a code against a code, never against
        // the bare text a string bound carries.
        (code, OfficialPrimitiveLiteral::String(value)) if code.is_code() => {
            code.scalar(value.as_str()).ok()?
        }
        (DataType::Uuid, OfficialPrimitiveLiteral::UInt128(value)) => {
            Scalar::from(crate::uuid_text(&value.to_be_bytes()))
        }
        (crate::bytes_dtypes!(), OfficialPrimitiveLiteral::Binary(value)) => {
            Scalar::from(value.as_slice())
        }
        _ => return None,
    };
    dtype.scalar(value).ok()
}

/// Decode only a well-formed single-value representation.
///
/// Apache Iceberg handles promoted Int-to-Long and Float-to-Double bounds. Its
/// boolean and fixed decoders are deliberately permissive, so exact wire
/// widths are checked here before delegating. A malformed external bound is
/// unknown rather than a synthetic value a planner could prune against.
fn official_datum(bytes: &[u8], dtype: &DataType) -> Option<OfficialDatum> {
    let primitive = match dtype {
        DataType::Boolean if matches!(bytes, [0] | [1]) => OfficialPrimitiveType::Boolean,
        DataType::Int32 if bytes.len() == 4 => OfficialPrimitiveType::Int,
        DataType::Date32 if bytes.len() == 4 => OfficialPrimitiveType::Date,
        DataType::Int64 if matches!(bytes.len(), 4 | 8) => OfficialPrimitiveType::Long,
        DataType::Float32 if bytes.len() == 4 => OfficialPrimitiveType::Float,
        DataType::Float64 if matches!(bytes.len(), 4 | 8) => OfficialPrimitiveType::Double,
        DataType::Time64(TimeUnit::Microsecond) if bytes.len() == 8 => OfficialPrimitiveType::Time,
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone,
        } if bytes.len() == 8 && timezone.is_naive() => OfficialPrimitiveType::Timestamp,
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            ..
        } if bytes.len() == 8 => OfficialPrimitiveType::Timestamptz,
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone,
        } if bytes.len() == 8 && timezone.is_naive() => OfficialPrimitiveType::TimestampNs,
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            ..
        } if bytes.len() == 8 => OfficialPrimitiveType::TimestamptzNs,
        text if is_text_string(text) => OfficialPrimitiveType::String,
        code if code.is_code() => OfficialPrimitiveType::String,
        DataType::Uuid => OfficialPrimitiveType::Uuid,
        crate::bytes_dtypes!() => match dtype.bytes_parameters()?.fixed() {
            None => OfficialPrimitiveType::Binary,
            Some(width) if usize::try_from(width).ok() == Some(bytes.len()) => {
                OfficialPrimitiveType::Fixed(u64::from(width))
            }
            Some(_) => return None,
        },
        _ => return None,
    };
    let datum = OfficialDatum::try_from_bytes(bytes, primitive).ok()?;
    // Treat invalid external NaN bounds as unknown. A planner may use an
    // unknown bound only conservatively; admitting NaN here could prove a file
    // disjoint under total float ordering and hide matching rows.
    (!datum.is_nan()).then_some(datum)
}

/// Read the integer count a value holds, whatever it counts.
///
/// A date counts days, a time counts its unit since midnight, and a timestamp
/// counts its unit since the epoch, so all three are one integer to an encoder.
fn count(value: &Scalar) -> Option<i64> {
    value.temporal_count().or_else(|| value.as_i64())
}

/// Compare two single values the way their datatype orders them.
///
/// A little-endian integer does not order as bytes do, so folding bounds across
/// row groups, and testing a filter against one, has to decode before it
/// compares. Text and bytes are the exception: they order lexicographically in
/// both encodings.
pub(super) fn compare_single(
    left: &[u8],
    right: &[u8],
    dtype: &DataType,
) -> Option<std::cmp::Ordering> {
    Some(single_to_value(left, dtype)?.cmp(&single_to_value(right, dtype)?))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/iceberg/value.rs` pins and a caller cannot reach.

    use crate::{DataType, Scalar};

    /// Encode one scalar as Iceberg's portable single-value bytes.
    pub fn single_value(value: &Scalar, dtype: &DataType) -> Option<Vec<u8>> {
        super::single_value(value, dtype)
    }

    /// Read Iceberg's portable single-value bytes back under a datatype.
    pub fn single_to_value(bytes: &[u8], dtype: &DataType) -> Option<Scalar> {
        super::single_to_value(bytes, dtype)
    }

    /// Order two encoded single values the way their datatype orders them.
    pub fn compare_single(
        left: &[u8],
        right: &[u8],
        dtype: &DataType,
    ) -> Option<std::cmp::Ordering> {
        super::compare_single(left, right, dtype)
    }

    /// Whether a datatype has a portable Iceberg bound at all.
    pub const fn is_portable(dtype: &DataType) -> bool {
        super::is_portable(dtype)
    }
}
