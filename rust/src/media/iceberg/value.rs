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
/// column gets counts but no bounds. A missing statistic costs a planner one
/// file read; a wrong one costs correctness.
pub(super) const fn is_portable(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Boolean
            | DataType::Int32
            | DataType::Int64
            | DataType::Float32
            | DataType::Float64
            | DataType::Date32
            | DataType::Time64(TimeUnit::Microsecond)
            | DataType::Timestamp(TimeUnit::Microsecond | TimeUnit::Nanosecond, _)
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Ascii
            | DataType::FixedAscii(_)
            | DataType::Country
            | DataType::Currency
            | DataType::Mic
            | DataType::Cfi
            | DataType::Guid
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_)
    )
}

/// Encode one scalar as the single value a manifest bound carries.
///
/// The datatype decides the encoding rather than the value's own variant,
/// because a column declared `Int32` still arrives as a 64-bit
/// [`Scalar::I64`]. A value that does not fit the column, and every type whose
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
        DataType::Timestamp(TimeUnit::Microsecond, None) => {
            OfficialDatum::timestamp_micros(count(value)?)
        }
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) => {
            OfficialDatum::timestamptz_micros(count(value)?)
        }
        DataType::Timestamp(TimeUnit::Nanosecond, None) => {
            OfficialDatum::timestamp_nanos(count(value)?)
        }
        DataType::Timestamp(TimeUnit::Nanosecond, Some(_)) => {
            OfficialDatum::timestamptz_nanos(count(value)?)
        }
        // A bound over an ASCII column is a string bound: the value is the
        // trimmed text.
        DataType::Utf8
        | DataType::LargeUtf8
        | DataType::Utf8View
        | DataType::Ascii
        | DataType::FixedAscii(_)
        | DataType::Country
        | DataType::Currency
        | DataType::Mic
        | DataType::Cfi => OfficialDatum::string(value.as_str()?),
        // An identifier is a `uuid` datum, built from the sixteen bytes the
        // canonical spelling parses to.
        DataType::Guid => OfficialDatum::uuid(uuid::Uuid::from_bytes(
            crate::types::guid_parse(crate::types::guid_bytes(value)?).ok()?,
        )),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => {
            OfficialDatum::binary(value.as_bytes()?.iter().copied())
        }
        DataType::FixedSizeBinary(width) => {
            let bytes = value.as_bytes()?;
            if usize::try_from(*width).ok()? != bytes.len() {
                return None;
            }
            OfficialDatum::fixed(bytes.iter().copied())
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
    match (dtype, datum.literal()) {
        (DataType::Boolean, OfficialPrimitiveLiteral::Boolean(value)) => Some(Scalar::Bool(*value)),
        (DataType::Int32, OfficialPrimitiveLiteral::Int(value)) => Some(Scalar::I32(*value)),
        (DataType::Date32, OfficialPrimitiveLiteral::Int(value)) => Some(Scalar::date32(*value)),
        (DataType::Int64, OfficialPrimitiveLiteral::Long(value)) => Some(Scalar::I64(*value)),
        (DataType::Time64(unit), OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::time64(*value, *unit, crate::Timezone::NAIVE).ok()
        }
        (DataType::Timestamp(unit, Some(zone)), OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, *unit, zone.clone()).ok()
        }
        (DataType::Timestamp(unit, None), OfficialPrimitiveLiteral::Long(value)) => {
            Scalar::datetime64(*value, *unit, crate::Timezone::NAIVE).ok()
        }
        (DataType::Float32, OfficialPrimitiveLiteral::Float(value)) => {
            Some(Scalar::F32(crate::Float32::from_f32((*value).into_inner())))
        }
        (DataType::Float64, OfficialPrimitiveLiteral::Double(value)) => {
            Some(Scalar::F64(crate::Float64::from_f64((*value).into_inner())))
        }
        (
            DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Ascii
            | DataType::FixedAscii(_)
            | DataType::Country
            | DataType::Currency
            | DataType::Mic
            | DataType::Cfi,
            OfficialPrimitiveLiteral::String(value),
        ) => Some(Scalar::from(value.as_str())),
        (DataType::Guid, OfficialPrimitiveLiteral::UInt128(value)) => Some(Scalar::String(
            crate::types::guid_text(&value.to_be_bytes()),
        )),
        (
            DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_),
            OfficialPrimitiveLiteral::Binary(value),
        ) => Some(Scalar::from(value.as_slice())),
        _ => None,
    }
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
        DataType::Timestamp(TimeUnit::Microsecond, None) if bytes.len() == 8 => {
            OfficialPrimitiveType::Timestamp
        }
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) if bytes.len() == 8 => {
            OfficialPrimitiveType::Timestamptz
        }
        DataType::Timestamp(TimeUnit::Nanosecond, None) if bytes.len() == 8 => {
            OfficialPrimitiveType::TimestampNs
        }
        DataType::Timestamp(TimeUnit::Nanosecond, Some(_)) if bytes.len() == 8 => {
            OfficialPrimitiveType::TimestamptzNs
        }
        DataType::Utf8
        | DataType::LargeUtf8
        | DataType::Utf8View
        | DataType::Ascii
        | DataType::FixedAscii(_)
        | DataType::Country
        | DataType::Currency
        | DataType::Mic
        | DataType::Cfi => OfficialPrimitiveType::String,
        DataType::Guid => OfficialPrimitiveType::Uuid,
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => {
            OfficialPrimitiveType::Binary
        }
        DataType::FixedSizeBinary(width)
            if usize::try_from(*width)
                .ok()
                .is_some_and(|width| width == bytes.len()) =>
        {
            OfficialPrimitiveType::Fixed(u64::try_from(*width).ok()?)
        }
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
    match value {
        Scalar::Date32(count, _, _)
        | Scalar::Time32(count, _, _)
        | Scalar::Duration32(count, _, _) => Some(i64::from(*count)),
        Scalar::Date64(count, _, _)
        | Scalar::Time64(count, _, _)
        | Scalar::DateTime64(count, _, _)
        | Scalar::Duration64(count, _, _) => Some(*count),
        other => other.as_i64(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promoted_bounds_decode_under_the_current_type() {
        let int = 37_i32.to_le_bytes();
        assert_eq!(
            single_to_value(&int, &DataType::Int64),
            Some(Scalar::I64(37))
        );
        assert_eq!(
            compare_single(&int, &38_i64.to_le_bytes(), &DataType::Int64),
            Some(std::cmp::Ordering::Less)
        );

        let float = 1.5_f32.to_le_bytes();
        assert_eq!(
            single_to_value(&float, &DataType::Float64).and_then(|value| value.as_f64()),
            Some(1.5)
        );
        assert_eq!(
            compare_single(&float, &2.0_f64.to_le_bytes(), &DataType::Float64),
            Some(std::cmp::Ordering::Less)
        );
    }

    #[test]
    fn malformed_bounds_are_unknown_instead_of_zero() {
        for bytes in [vec![], vec![0; 3], vec![0; 5], vec![0; 7], vec![0; 9]] {
            assert!(single_to_value(&bytes, &DataType::Int32).is_none());
            assert!(single_to_value(&bytes, &DataType::Int64).is_none());
            assert!(single_to_value(&bytes, &DataType::Float32).is_none());
            assert!(single_to_value(&bytes, &DataType::Float64).is_none());
            assert!(compare_single(&bytes, &0_i64.to_le_bytes(), &DataType::Int64).is_none());
        }
        assert!(single_to_value(&[], &DataType::Boolean).is_none());
        assert!(single_to_value(&[0, 1], &DataType::Boolean).is_none());
        assert!(single_to_value(&[2], &DataType::Boolean).is_none());
        assert!(single_to_value(&[0; 3], &DataType::FixedSizeBinary(4)).is_none());
        assert!(single_to_value(&[0; 5], &DataType::FixedSizeBinary(4)).is_none());
        assert!(single_to_value(&[0xff], &DataType::Utf8).is_none());
    }

    #[test]
    fn nan_is_never_encoded_or_decoded_as_a_bound() {
        let f32_nan = f32::NAN.to_le_bytes();
        let f64_nan = f64::NAN.to_le_bytes();

        assert!(single_value(&Scalar::from(f32::NAN), &DataType::Float32).is_none());
        assert!(single_value(&Scalar::from(f64::NAN), &DataType::Float64).is_none());
        assert!(single_to_value(&f32_nan, &DataType::Float32).is_none());
        assert!(single_to_value(&f64_nan, &DataType::Float64).is_none());
        assert!(compare_single(&f64_nan, &1.5_f64.to_le_bytes(), &DataType::Float64).is_none());

        // Signed zero and infinities are valid Iceberg bounds.
        for value in [-0.0_f64, 0.0, f64::NEG_INFINITY, f64::INFINITY] {
            let scalar = Scalar::from(value);
            let encoded = single_value(&scalar, &DataType::Float64).unwrap();
            assert_eq!(single_to_value(&encoded, &DataType::Float64), Some(scalar));
        }
    }

    #[test]
    fn official_single_value_bytes_round_trip_supported_values() {
        let cases = [
            (Scalar::Bool(true), DataType::Boolean),
            (Scalar::I32(-7), DataType::Int32),
            (Scalar::I64(9), DataType::Int64),
            (Scalar::from("é"), DataType::Utf8),
            (Scalar::from([0_u8, 1, 2].as_slice()), DataType::Binary),
        ];
        for (value, dtype) in cases {
            let bytes = single_value(&value, &dtype).expect("a supported value");
            assert_eq!(single_to_value(&bytes, &dtype), Some(value));
        }
    }
}
