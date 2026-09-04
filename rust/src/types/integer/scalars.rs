//! Integer scalar canonicalization and validation.

use smol_str::{SmolStr, format_smolstr};

use crate::types::value::{PathSegment, ValidationFailure, canonical_error, expected};
use crate::{DataType, Error, Result, Scalar, TimeUnit};

pub(crate) fn canonical_signed(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(integer) = value.as_i128() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "validated signed value could not be canonicalized from {}",
                value.kind()
            ),
        });
    };
    let canonical = match dtype {
        DataType::Int8 => Scalar::I8(i8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int16 => Scalar::I16(i16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int32 => Scalar::I32(i32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::Int64 | DataType::Interval(TimeUnit::YearMonth) => {
            Scalar::I64(i64::try_from(integer).map_err(canonical_integer_error)?)
        }
        _ => unreachable!("signed canonicalization requires a signed datatype"),
    };
    let changed = match &canonical {
        Scalar::I8(expected) => !matches!(value, Scalar::I8(current) if current == expected),
        Scalar::I16(expected) => !matches!(value, Scalar::I16(current) if current == expected),
        Scalar::I32(expected) => !matches!(value, Scalar::I32(current) if current == expected),
        Scalar::I64(expected) => !matches!(value, Scalar::I64(current) if current == expected),
        _ => unreachable!("signed canonical value has a signed kind"),
    };
    Ok((canonical, changed))
}

pub(crate) fn canonical_unsigned(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(integer) = value.as_u128() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated unsigned value could not be canonicalized"),
        });
    };
    let canonical = match dtype {
        DataType::UInt8 => Scalar::U8(u8::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt16 => Scalar::U16(u16::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt32 => Scalar::U32(u32::try_from(integer).map_err(canonical_integer_error)?),
        DataType::UInt64 => Scalar::U64(u64::try_from(integer).map_err(canonical_integer_error)?),
        _ => unreachable!("unsigned canonicalization requires an unsigned datatype"),
    };
    let changed = match &canonical {
        Scalar::U8(expected) => !matches!(value, Scalar::U8(current) if current == expected),
        Scalar::U16(expected) => !matches!(value, Scalar::U16(current) if current == expected),
        Scalar::U32(expected) => !matches!(value, Scalar::U32(current) if current == expected),
        Scalar::U64(expected) => !matches!(value, Scalar::U64(current) if current == expected),
        _ => unreachable!("unsigned canonical value has an unsigned kind"),
    };
    Ok((canonical, changed))
}

fn canonical_integer_error(_error: impl std::fmt::Display) -> Error {
    canonical_error("integer does not fit declared width")
}

pub(crate) fn validate_signed(
    value: &Scalar,
    minimum: i128,
    maximum: i128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_i128() {
        Some(value) if (minimum..=maximum).contains(&value) => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

pub(crate) fn validate_unsigned(
    value: &Scalar,
    maximum: u128,
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    match value.as_u128() {
        Some(value) if value <= maximum => Ok(()),
        _ => Err(expected(expected_name, value)),
    }
}

pub(crate) fn validate_integer_tuple(
    value: &Scalar,
    widths: &[u8],
    expected_name: &str,
) -> std::result::Result<(), ValidationFailure> {
    let values = value
        .as_sequence()
        .ok_or_else(|| expected(expected_name, value))?;
    if values.len() != widths.len() {
        return Err(ValidationFailure::new(format_smolstr!(
            "{expected_name} requires {} integer components, got {}",
            widths.len(),
            values.len()
        )));
    }
    for (index, (value, width)) in values.iter().zip(widths).enumerate() {
        let (minimum, maximum) = if *width == 32 {
            (i128::from(i32::MIN), i128::from(i32::MAX))
        } else {
            (i128::from(i64::MIN), i128::from(i64::MAX))
        };
        validate_signed(value, minimum, maximum, expected_name)
            .map_err(|failure| failure.prepend(PathSegment::Index(index)))?;
    }
    Ok(())
}
