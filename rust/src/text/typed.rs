use std::str::FromStr;

use base64::Engine as _;
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, I256, Result, Scalar, TimeUnit, Timezone};

/// Interpret a natural text value under one field, then validate it.
pub(crate) fn with_field(value: Scalar, field: &Field) -> Result<Scalar> {
    let value = prepare(value, field)?;
    // Row canonicalization is the single schema conversion implementation.
    // A one-child root gives scalar and struct parses that same path.
    let root = Field::new("$", DataType::from_fields([field.clone()])?, false);
    let row = root.canonicalize_value(Scalar::from_sequence([value]))?;
    root.validate_value(&row)?;
    row.as_sequence()
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| invalid(field, "canonical row is empty"))
}

fn prepare(value: Scalar, field: &Field) -> Result<Scalar> {
    if value.is_null() {
        return Ok(value);
    }
    match field.dtype() {
        DataType::Decimal32 { scale, .. }
        | DataType::Decimal64 { scale, .. }
        | DataType::Decimal128 { scale, .. } => decimal(value, *scale, false, field),
        DataType::Decimal256 { scale, .. } => decimal(value, *scale, true, field),
        DataType::Binary
        | DataType::FixedSizeBinary(_)
        | DataType::LargeBinary
        | DataType::BinaryView => binary(value, field),
        DataType::Geometry(_) | DataType::Geography(_) => geospatial(value, field),
        DataType::Date32 => date32(value, field),
        DataType::Date64 => date64(value, field),
        DataType::Time32(unit) => time32(value, *unit, field),
        DataType::Time64(unit) => time64(value, *unit, field),
        DataType::Timestamp(unit, zone) => datetime64(value, *unit, zone.as_ref(), field),
        DataType::Duration32(unit) => duration32(value, *unit, field),
        DataType::Duration64(unit) => duration64(value, *unit, field),
        DataType::List(child)
        | DataType::ListView(child)
        | DataType::FixedSizeList(child, _)
        | DataType::LargeList(child)
        | DataType::LargeListView(child) => sequence(value, |value| prepare(value, child), field),
        DataType::Struct(fields) => structure(value, fields, field),
        DataType::Union(fields, _) => union(value, fields, field),
        DataType::Dictionary(dictionary) => prepare_for_type(value, dictionary.value(), field),
        DataType::Map(map) => mapping(value, map, field),
        DataType::RunEndEncoded(encoded) => prepare(value, encoded.values()),
        _ => Ok(value),
    }
}

fn prepare_for_type(value: Scalar, dtype: &DataType, context: &Field) -> Result<Scalar> {
    prepare(
        value,
        &Field::new(context.name(), dtype.clone(), context.is_nullable()),
    )
}

fn sequence(
    value: Scalar,
    mut prepare_value: impl FnMut(Scalar) -> Result<Scalar>,
    field: &Field,
) -> Result<Scalar> {
    let Scalar::Sequence(values) = value else {
        return Err(invalid(field, "expected an array"));
    };
    values
        .iter()
        .cloned()
        .map(&mut prepare_value)
        .collect::<Result<Vec<_>>>()
        .map(Scalar::from_sequence)
}

fn structure(value: Scalar, fields: &crate::Fields, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::Record(entries) => {
            let prepared = entries
                .iter()
                .map(|(name, value)| {
                    let child = fields
                        .get_by_name(name)
                        .ok_or_else(|| invalid(field, format_smolstr!("unknown field {name:?}")))?;
                    Ok((name.clone(), prepare(value.clone(), child)?))
                })
                .collect::<Result<Vec<_>>>()?;
            Scalar::from_record(prepared)
        }
        Scalar::Sequence(values) => {
            if values.len() != fields.len() {
                return Err(invalid(field, "struct array has the wrong length"));
            }
            values
                .iter()
                .cloned()
                .zip(fields.iter())
                .map(|(value, child)| prepare(value, child))
                .collect::<Result<Vec<_>>>()
                .map(Scalar::from_sequence)
        }
        _ => Err(invalid(field, "expected an object or ordered struct array")),
    }
}

fn union(value: Scalar, fields: &crate::UnionFields, field: &Field) -> Result<Scalar> {
    let Scalar::Sequence(values) = value else {
        return Err(invalid(field, "expected [type_id, value] for a union"));
    };
    let [type_id, payload] = values.as_ref() else {
        return Err(invalid(field, "expected [type_id, value] for a union"));
    };
    let id = type_id
        .as_i128()
        .and_then(|id| i8::try_from(id).ok())
        .ok_or_else(|| invalid(field, "union type id must fit i8"))?;
    let branch = fields
        .iter()
        .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
        .ok_or_else(|| invalid(field, "union type id is not declared"))?;
    Ok(Scalar::from_sequence([
        type_id.clone(),
        prepare(payload.clone(), branch)?,
    ]))
}

fn mapping(value: Scalar, map: &crate::MapType, field: &Field) -> Result<Scalar> {
    let fields = map.entries().fields();
    let [key_field, value_field] = fields else {
        return Err(invalid(
            field,
            "map entries do not contain key and value fields",
        ));
    };
    let entries = match value {
        Scalar::Mapping(entries) => entries.iter().cloned().collect::<Vec<_>>(),
        Scalar::Record(entries) => entries
            .iter()
            .map(|(name, value)| (Scalar::from(name.as_str()), value.clone()))
            .collect(),
        _ => return Err(invalid(field, "expected an object or mapping")),
    };
    Scalar::from_mapping(
        entries
            .into_iter()
            .map(|(key, value)| Ok((prepare(key, key_field)?, prepare(value, value_field)?)))
            .collect::<Result<Vec<_>>>()?,
    )
}

fn binary(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::String(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map(Scalar::from)
            .map_err(|_| invalid(field, "expected base64 text")),
        value => Ok(value),
    }
}

fn geospatial(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::String(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map(|bytes| Scalar::Geospatial(bytes.into()))
            .map_err(|_| invalid(field, "expected base64 WKB text")),
        value => Ok(value),
    }
}

fn decimal(value: Scalar, scale: i8, wide: bool, field: &Field) -> Result<Scalar> {
    if value.is_decimal() {
        return Ok(value);
    }
    let text = scalar_number_text(&value)
        .ok_or_else(|| invalid(field, "expected decimal text or a number"))?;
    let coefficient = decimal_coefficient(&text, scale).map_err(|reason| invalid(field, reason))?;
    if wide {
        Ok(Scalar::d256(coefficient, scale))
    } else {
        coefficient
            .as_i128()
            .map(|coefficient| Scalar::d128(coefficient, scale))
            .ok_or_else(|| invalid(field, "decimal coefficient exceeds 128 bits"))
    }
}

fn scalar_number_text(value: &Scalar) -> Option<String> {
    match value {
        Scalar::String(value) => Some(value.to_string()),
        Scalar::I8(value) => Some(value.to_string()),
        Scalar::I16(value) => Some(value.to_string()),
        Scalar::I32(value) => Some(value.to_string()),
        Scalar::I64(value) => Some(value.to_string()),
        Scalar::I128(value) => Some(value.to_string()),
        Scalar::U8(value) => Some(value.to_string()),
        Scalar::U16(value) => Some(value.to_string()),
        Scalar::U32(value) => Some(value.to_string()),
        Scalar::U64(value) => Some(value.to_string()),
        Scalar::U128(value) => Some(value.to_string()),
        Scalar::F16(value) if value.as_f64().is_finite() => Some(value.as_f64().to_string()),
        Scalar::F32(value) if value.as_f64().is_finite() => Some(value.as_f64().to_string()),
        Scalar::F64(value) if value.as_f64().is_finite() => Some(value.as_f64().to_string()),
        _ => None,
    }
}

fn decimal_coefficient(text: &str, target_scale: i8) -> std::result::Result<I256, &'static str> {
    let text = text.trim();
    let exponent_at = text.find(['e', 'E']);
    let (mantissa, exponent) = exponent_at.map_or((text, 0_i32), |position| {
        let exponent = text[position + 1..].parse::<i32>().unwrap_or(i32::MIN);
        (&text[..position], exponent)
    });
    if exponent == i32::MIN
        || exponent_at.is_some_and(|position| text[position + 1..].contains(['e', 'E']))
    {
        return Err("invalid decimal exponent");
    }
    let (sign, mantissa) = match mantissa.as_bytes().first() {
        Some(b'-') => ("-", &mantissa[1..]),
        Some(b'+') => ("", &mantissa[1..]),
        _ => ("", mantissa),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.contains('.')
        || fraction.contains('.')
        || (whole.is_empty() && fraction.is_empty())
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
    {
        return Err("invalid decimal digits");
    }
    let digits = format!("{sign}{whole}{fraction}");
    let mut coefficient =
        I256::from_str(&digits).map_err(|_| "decimal coefficient exceeds 256 bits")?;
    if coefficient == I256::ZERO {
        return Ok(coefficient);
    }
    let source_scale = i32::try_from(fraction.len())
        .map_err(|_| "decimal scale is too large")?
        .checked_sub(exponent)
        .ok_or("decimal scale is too large")?;
    let shift = i32::from(target_scale)
        .checked_sub(source_scale)
        .ok_or("decimal scale is too large")?;
    if shift >= 0 {
        for _ in 0..shift {
            coefficient = coefficient
                .checked_mul_ten()
                .ok_or("decimal coefficient exceeds 256 bits")?;
        }
    } else {
        for _ in 0..-shift {
            coefficient = coefficient
                .divided_by_ten()
                .ok_or("decimal has more fractional digits than the field allows")?;
        }
    }
    Ok(coefficient)
}

fn date32(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::String(text) => crate::generic::iso::parse_date(&text)
            .map(Scalar::date32)
            .map_err(|_| invalid(field, "expected an ISO date")),
        value => Ok(value),
    }
}

fn date64(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::String(text) => crate::generic::iso::parse_date(&text)
            .ok()
            .and_then(|days| i64::from(days).checked_mul(86_400_000))
            .map(Scalar::date64)
            .ok_or_else(|| invalid(field, "expected an ISO date")),
        value => Ok(value),
    }
}

fn time32(value: Scalar, unit: TimeUnit, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::String(text) => {
            let (count, source, zone) =
                parse_time_with_zone(&text).map_err(|_| invalid(field, "expected an ISO time"))?;
            if !zone.is_naive() {
                return Err(invalid(
                    field,
                    "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
                ));
            }
            let count = rescale(count, source, unit)
                .and_then(|count| i32::try_from(count).ok())
                .ok_or_else(|| invalid(field, "time does not fit its declared unit"))?;
            Scalar::time32(count, unit, zone)
        }
        value => Ok(value),
    }
}

fn time64(value: Scalar, unit: TimeUnit, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::String(text) => {
            let (count, source, zone) =
                parse_time_with_zone(&text).map_err(|_| invalid(field, "expected an ISO time"))?;
            if !zone.is_naive() {
                return Err(invalid(
                    field,
                    "time-of-day cannot carry a timezone; use DateTime64 for a zoned instant",
                ));
            }
            let count = rescale(count, source, unit)
                .ok_or_else(|| invalid(field, "time does not fit its declared unit"))?;
            Scalar::time64(count, unit, zone)
        }
        value => Ok(value),
    }
}

fn datetime64(
    value: Scalar,
    unit: TimeUnit,
    declared_zone: Option<&Timezone>,
    field: &Field,
) -> Result<Scalar> {
    let Scalar::String(text) = value else {
        return Ok(value);
    };
    let (count, source, parsed_zone) = if declared_zone.is_some() {
        let (count, source, zone) = crate::generic::iso::parse_timestamp(&text)
            .map_err(|_| invalid(field, "expected an ISO timestamp with timezone"))?;
        (count, source, zone)
    } else {
        let (count, source) = crate::generic::iso::parse_datetime(&text)
            .map_err(|_| invalid(field, "expected an ISO timezone-naive datetime"))?;
        (count, source, Timezone::NAIVE)
    };
    let count = rescale(count, source, unit)
        .ok_or_else(|| invalid(field, "datetime does not fit its declared unit"))?;
    Scalar::datetime64(count, unit, declared_zone.cloned().unwrap_or(parsed_zone))
}

fn duration32(value: Scalar, unit: TimeUnit, field: &Field) -> Result<Scalar> {
    let Scalar::String(text) = value else {
        return Ok(value);
    };
    let (count, source) = crate::generic::iso::parse_duration(&text)
        .map_err(|_| invalid(field, "expected an ISO duration or a plain clock"))?;
    let count = rescale(count, source, unit)
        .ok_or_else(|| invalid(field, "duration does not fit its declared unit"))?;
    Scalar::duration32(
        i32::try_from(count).map_err(|_| invalid(field, "duration exceeds 32 bits"))?,
        unit,
    )
}

fn duration64(value: Scalar, unit: TimeUnit, field: &Field) -> Result<Scalar> {
    let Scalar::String(text) = value else {
        return Ok(value);
    };
    let (count, source) = crate::generic::iso::parse_duration(&text)
        .map_err(|_| invalid(field, "expected an ISO duration or a plain clock"))?;
    let count = rescale(count, source, unit)
        .ok_or_else(|| invalid(field, "duration does not fit its declared unit"))?;
    Scalar::duration64(count, unit)
}

fn parse_time_with_zone(text: &str) -> Result<(i64, TimeUnit, Timezone)> {
    if let Some(clock) = text.strip_suffix('Z') {
        let (count, unit) = crate::generic::iso::parse_time(clock)?;
        return Ok((count, unit, Timezone::UTC));
    }
    if text.len() >= 6 {
        let suffix = &text[text.len() - 6..];
        if matches!(suffix.as_bytes().first(), Some(b'+' | b'-')) && suffix.as_bytes()[3] == b':' {
            let zone = Timezone::from_str(suffix)?;
            let (count, unit) = crate::generic::iso::parse_time(&text[..text.len() - 6])?;
            return Ok((count, unit, zone));
        }
    }
    let (count, unit) = crate::generic::iso::parse_time(text)?;
    Ok((count, unit, Timezone::NAIVE))
}

fn rescale(count: i64, source: TimeUnit, target: TimeUnit) -> Option<i64> {
    let source = nanoseconds(source)?;
    let target = nanoseconds(target)?;
    let nanoseconds = i128::from(count).checked_mul(source)?;
    (nanoseconds % target == 0)
        .then(|| i64::try_from(nanoseconds / target).ok())
        .flatten()
}

const fn nanoseconds(unit: TimeUnit) -> Option<i128> {
    match unit {
        TimeUnit::Day => Some(86_400_000_000_000),
        TimeUnit::Second => Some(1_000_000_000),
        TimeUnit::Millisecond => Some(1_000_000),
        TimeUnit::Microsecond => Some(1_000),
        TimeUnit::Nanosecond => Some(1),
        TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => None,
    }
}

fn invalid(field: &Field, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{}", field.name()),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::decimal_coefficient;
    use crate::I256;

    #[test]
    fn decimals_are_restated_exactly_at_the_field_scale() {
        assert_eq!(decimal_coefficient("10.50", 2).unwrap(), I256::from(1_050));
        assert_eq!(decimal_coefficient("1.05e1", 2).unwrap(), I256::from(1_050));
        assert!(decimal_coefficient("1.005", 2).is_err());
    }
}
