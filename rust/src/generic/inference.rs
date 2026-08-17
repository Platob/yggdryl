//! The datatype a value already is.
//!
//! Every [`Value`] variant carries the parts its Arrow datatype needs - the
//! width of an integer, the unit and zone of a timestamp, the scale of a
//! decimal - so naming that datatype is a read, not a guess. This module is
//! where the read lives, because a caller holding rows and no schema has
//! nowhere else to get one.
//!
//! Inference is exact rather than accommodating. A sequence whose children do
//! not agree is an error, not a widened common type, because the widening a
//! caller wants depends on what the caller is going to do with it: promoting
//! `1` and `1.5` to a double loses exactness, promoting them to a decimal picks
//! a scale nobody asked for, and refusing lets the caller say which it wanted.
//! A null child agrees with anything and makes the child field nullable.
//!
//! Several Arrow layouts hold the same logical thing, so inference names the
//! narrow one every time: `Utf8` not `LargeUtf8`, `Binary` not `LargeBinary`,
//! `List` not `LargeList`, `Date32` not `Date64`. A caller who wants the wide
//! layout is asking for a physical decision, which is a schema's job and not a
//! value's.
//!
//! ```
//! use yggdryl::{DataType, TimeUnit, Value};
//!
//! # fn main() -> yggdryl::Result<()> {
//! assert_eq!(Value::from(u64::MAX).data_type()?, DataType::UInt64);
//! assert_eq!(Value::decimal(1_050, 2).data_type()?, DataType::decimal(4, 2)?);
//! assert_eq!(
//!     Value::timestamp(0, TimeUnit::Microsecond, None)?.data_type()?,
//!     DataType::Timestamp(TimeUnit::Microsecond, None),
//! );
//! assert_eq!(
//!     Value::from_sequence([Value::from("AAPL"), Value::Null]).data_type()?,
//!     DataType::list(yggdryl::Field::new("item", DataType::Utf8, true)),
//! );
//! # Ok(())
//! # }
//! ```

use smol_str::{SmolStr, format_smolstr};

use super::value::Value;
use crate::{DataType, Error, Field, Result, TimeUnit};

/// Arrow's widest exact decimal, and so the widest integer a decimal can hold.
const MAX_DECIMAL_PRECISION: u32 = 76;

impl Value {
    /// Return the datatype this value materializes into.
    ///
    /// The name is read off the variant, so a timestamp keeps its unit and
    /// zone, a decimal keeps its scale, and an unsigned integer stays unsigned
    /// instead of being narrowed into a signed column it may not fit.
    ///
    /// # Errors
    ///
    /// Returns an error when a sequence's children or a mapping's keys or
    /// values do not all name one datatype, when a 128-bit integer needs more
    /// digits than Arrow's widest decimal holds, when a temporal carries a
    /// calendar interval layout instead of a resolution, or when the value
    /// nests past the shared recursion limit.
    pub fn data_type(&self) -> Result<DataType> {
        self.data_type_at(0)
    }

    /// Name this value's datatype, refusing to recurse past the shared limit.
    ///
    /// A value can nest as deeply as whoever built it chose, and a datatype is
    /// walked recursively, so the walk is bounded the way every other recursive
    /// descent in the project is rather than by the size of the native stack.
    fn data_type_at(&self, depth: usize) -> Result<DataType> {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            return Err(unnameable(format_smolstr!(
                "value nesting exceeds the hard limit of {}",
                DataType::PARSE_RECURSION_LIMIT
            )));
        }
        match self {
            Self::Null => Ok(DataType::Null),
            Self::Bool(_) => Ok(DataType::Boolean),
            Self::I8(_) => Ok(DataType::Int8),
            Self::I16(_) => Ok(DataType::Int16),
            Self::I32(_) => Ok(DataType::Int32),
            Self::I64(_) => Ok(DataType::Int64),
            Self::U8(_) => Ok(DataType::UInt8),
            Self::U16(_) => Ok(DataType::UInt16),
            Self::U32(_) => Ok(DataType::UInt32),
            Self::U64(_) => Ok(DataType::UInt64),
            // Arrow has no 128-bit integer, and an exact decimal with scale
            // zero is an integer, so that is what a wide integer becomes.
            Self::I128(value) => integer_decimal(digits(value.unsigned_abs())),
            Self::U128(value) => integer_decimal(digits(*value)),
            Self::F32(_) => Ok(DataType::Float32),
            Self::F64(_) => Ok(DataType::Float64),
            Self::Decimal(unscaled, scale) => decimal_data_type(*unscaled, *scale),
            Self::String(_) => Ok(DataType::Utf8),
            Self::Bytes(_) => Ok(DataType::Binary),
            Self::Date(_) => Ok(DataType::Date32),
            Self::Time(_, unit) => DataType::time(*unit),
            Self::Timestamp(_, unit, zone) => {
                resolution(*unit, "timestamp")?;
                Ok(DataType::Timestamp(*unit, Some(zone.clone())))
            }
            Self::DateTime(_, unit) => {
                resolution(*unit, "timestamp")?;
                Ok(DataType::Timestamp(*unit, None))
            }
            Self::Duration(_, unit) => {
                resolution(*unit, "duration")?;
                Ok(DataType::Duration(*unit))
            }
            // A record carries its type; inference is what it already knows.
            Self::Record(data_type, _) => Ok((**data_type).clone()),
            Self::Sequence(values) => {
                let (data_type, nullable) = agreed(values.iter(), "sequence item", depth)?;
                Ok(DataType::list(Field::new("item", data_type, nullable)))
            }
            // A mapping's keys are values, not names, so its datatype is a map
            // and not a struct; a struct in this project is described by a
            // sequence, one value per declared field.
            Self::Mapping(entries) => {
                let keys = entries.iter().map(|(key, _)| key);
                let (key, _) = agreed(keys, "mapping key", depth)?;
                // Arrow fixes the entry nullability itself - a key is required
                // and a value is not - so only the two datatypes are inferred.
                let values = entries.iter().map(|(_, value)| value);
                let (value, _) = agreed(values, "mapping value", depth)?;
                DataType::map_of(key, value, false)
            }
        }
    }
}

/// Return the one datatype every non-null value names, and whether any was null.
fn agreed<'a>(
    values: impl IntoIterator<Item = &'a Value>,
    role: &'static str,
    depth: usize,
) -> Result<(DataType, bool)> {
    let mut agreed: Option<DataType> = None;
    let mut nullable = false;
    for value in values {
        if value.is_null() {
            nullable = true;
            continue;
        }
        let data_type = value.data_type_at(depth + 1)?;
        match &agreed {
            Some(existing) if existing != &data_type => {
                return Err(unnameable(format_smolstr!(
                    "every {role} must name one datatype, got {} and {}",
                    crate::text::elide_display(existing),
                    crate::text::elide_display(&data_type),
                )));
            }
            Some(_) => {}
            None => agreed = Some(data_type),
        }
    }
    // Nothing but nulls names the null type, which is a real Arrow column.
    Ok(agreed.map_or((DataType::Null, true), |data_type| (data_type, nullable)))
}

/// Return the exact decimal a coefficient and scale name.
fn decimal_data_type(unscaled: i128, scale: i8) -> Result<DataType> {
    // Arrow requires a positive scale to fit inside the precision, so a
    // coefficient of 5 at scale 3 is `0.005` and needs three digits, not one.
    let precision = digits(unscaled.unsigned_abs()).max(u32::try_from(scale.max(0)).unwrap_or(0));
    if precision > MAX_DECIMAL_PRECISION {
        return Err(unnameable(format_smolstr!(
            "a decimal of {precision} digits exceeds Arrow's maximum precision of {MAX_DECIMAL_PRECISION}"
        )));
    }
    DataType::decimal(u8::try_from(precision.max(1)).unwrap_or(1), scale)
}

/// Return the exact decimal a 128-bit integer of `digits` digits needs.
fn integer_decimal(digits: u32) -> Result<DataType> {
    DataType::decimal(u8::try_from(digits.max(1)).unwrap_or(1), 0)
}

/// Reject a calendar interval layout where a temporal resolution is required.
fn resolution(unit: TimeUnit, kind: &'static str) -> Result<()> {
    if unit.is_temporal() {
        Ok(())
    } else {
        Err(unnameable(format_smolstr!(
            "a {kind} unit must be a temporal resolution, got {unit}"
        )))
    }
}

/// Return how many decimal digits a magnitude is written with.
fn digits(magnitude: u128) -> u32 {
    let mut digits = 1;
    let mut remaining = magnitude / 10;
    while remaining != 0 {
        digits += 1;
        remaining /= 10;
    }
    digits
}

/// Build the failure raised when a value names no single datatype.
fn unnameable(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason,
    }
}

#[cfg(test)]
mod tests;
