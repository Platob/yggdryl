use std::str::FromStr;

use base64::Engine as _;
use smol_str::{SmolStr, format_smolstr};

use crate::types::Nested;
use crate::{DataType, Error, Field, I256, Result, Scalar};

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
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => integer(value, field),
        DataType::Float16 | DataType::Float32 | DataType::Float64 => floating(value, field),
        DataType::Boolean => boolean(value, field),
        DataType::Geometry(_) | DataType::Geography(_) => geospatial(value, field),
        DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::DateTime64 { .. }
        | DataType::Duration32(_)
        | DataType::Duration64(_) => temporal(value, field),
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
    let Some(values) = value.as_sequence() else {
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
        Scalar::Nested(Nested::Record(entries)) => {
            let prepared = entries
                .as_map()
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
        Scalar::Nested(Nested::Sequence(values)) => {
            if values.as_slice().len() != fields.len() {
                return Err(invalid(field, "struct array has the wrong length"));
            }
            values
                .as_slice()
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
    let Some(values) = value.as_sequence() else {
        return Err(invalid(field, "expected [type_id, value] for a union"));
    };
    let [type_id, payload] = values else {
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
        Scalar::Nested(Nested::Mapping(entries)) => entries.as_slice().to_vec(),
        Scalar::Nested(Nested::Record(entries)) => entries
            .as_map()
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

/// Read one integer a document spells rather than types.
///
/// A format with integer literals hands one over already typed and it passes
/// through here untouched. A format that carries only text - XML, and every
/// column a delimited file holds - spells it, and the declared field is what
/// says to read those digits as a number. The accepted spelling is the one
/// every codec writes: an optional sign and decimal digits, nothing else. The
/// exact width is applied by the canonical value contract afterwards.
fn integer(value: Scalar, field: &Field) -> Result<Scalar> {
    let Some(text) = value.as_str() else {
        return Ok(value);
    };
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(expected_text(field));
    }
    let magnitude = digits.parse::<u128>().map_err(|_| expected_text(field))?;
    if !negative {
        return Ok(Scalar::from(magnitude));
    }
    // The most negative integer has no positive counterpart, so it is named
    // rather than reached by negating one.
    i128::try_from(magnitude)
        .ok()
        .and_then(i128::checked_neg)
        .or_else(|| (magnitude == 1 << 127).then_some(i128::MIN))
        .map(Scalar::from)
        .ok_or_else(|| expected_text(field))
}

/// Read one floating-point value a document spells rather than types.
///
/// The accepted spellings are the ones the codecs write: a decimal or
/// exponential literal, and `inf`, `infinity` or `nan` in any case, with an
/// optional sign. A literal whose value does not fit - `1e400`, which parses
/// as infinity, or `1e-400`, which parses as zero - is refused rather than
/// answered with a number nobody wrote.
fn floating(value: Scalar, field: &Field) -> Result<Scalar> {
    let Some(text) = value.as_str() else {
        return Ok(value);
    };
    let text = text.trim();
    let parsed = text.parse::<f64>().map_err(|_| expected_text(field))?;
    let magnitude = text.trim_start_matches(['-', '+']);
    if !parsed.is_finite() && !is_non_finite_text(magnitude) {
        return Err(overflowed(field, text));
    }
    if parsed == 0.0
        && magnitude
            .split(['e', 'E'])
            .next()
            .is_some_and(has_significant_digit)
    {
        return Err(overflowed(field, text));
    }
    Ok(Scalar::from(parsed))
}

/// Return whether text spells a value that is not a finite number.
fn is_non_finite_text(text: &str) -> bool {
    text.eq_ignore_ascii_case("inf")
        || text.eq_ignore_ascii_case("infinity")
        || text.eq_ignore_ascii_case("nan")
}

/// Return whether a mantissa holds a digit other than zero.
fn has_significant_digit(mantissa: &str) -> bool {
    mantissa
        .bytes()
        .any(|byte| byte.is_ascii_digit() && byte != b'0')
}

/// Read one boolean a document spells rather than types.
///
/// `true` and `false` in any case, plus the `1` and `0` an XML schema's
/// boolean lexical space also admits.
fn boolean(value: Scalar, field: &Field) -> Result<Scalar> {
    let Some(text) = value.as_str() else {
        return Ok(value);
    };
    let text = text.trim();
    if text.eq_ignore_ascii_case("true") || text == "1" {
        return Ok(Scalar::from(true));
    }
    if text.eq_ignore_ascii_case("false") || text == "0" {
        return Ok(Scalar::from(false));
    }
    Err(expected_text(field))
}

/// Name a literal the declared width cannot hold.
fn overflowed(field: &Field, text: &str) -> Error {
    invalid(
        field,
        format_smolstr!(
            "expected a value {} can hold, got {}",
            field.dtype(),
            crate::text::elide_display(&text)
        ),
    )
}

/// Name the datatype whose text spelling was expected.
fn expected_text(field: &Field) -> Error {
    invalid(field, format_smolstr!("expected {} text", field.dtype()))
}

fn binary(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::Text(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded.as_str().as_bytes())
            .map(Scalar::from)
            .map_err(|_| invalid(field, "expected base64 text")),
        value => Ok(value),
    }
}

fn geospatial(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::Text(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded.as_str().as_bytes())
            .map(Scalar::from)
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
        Scalar::Text(value) => Some(value.to_string()),
        Scalar::Integer(value) => Some(value.to_string()),
        Scalar::Floating(value) if value.as_f64().is_finite() => Some(value.as_f64().to_string()),
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

/// Read a temporal from its text spelling under the field's declared type.
///
/// The spelling, the exact restatement in the declared unit and the zone rule
/// are [`Scalar::from_temporal_text`]; the field only names where the value
/// sat, so the reason the reading gives survives into the record error.
fn temporal(value: Scalar, field: &Field) -> Result<Scalar> {
    let Scalar::Text(text) = value else {
        return Ok(value);
    };
    Scalar::from_temporal_text(field.dtype(), text.as_str())
        .map_err(|error| invalid(field, reason_of(&error)))
}

/// The reason one error carries, as a record error restates it.
fn reason_of(error: &Error) -> SmolStr {
    match error {
        Error::Parse {
            target,
            position,
            reason,
        } => format_smolstr!("expected an ISO {target}: {reason} at byte {position}"),
        Error::InvalidRecord { reason, .. } => reason.clone(),
        other => SmolStr::new(other.to_string()),
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
