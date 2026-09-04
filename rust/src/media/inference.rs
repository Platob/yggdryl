//! The datatype a value already is.
//!
//! Every [`Scalar`] variant carries the parts its Arrow datatype needs - the
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
//! use yggdryl::{DataType, TimeUnit, Timezone, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! assert_eq!(Scalar::from(u64::MAX).dtype()?, DataType::UInt64);
//! assert_eq!(Scalar::d128(1_050, 2).dtype()?, DataType::decimal128(4, 2)?);
//! assert_eq!(
//!     Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::NAIVE)?.dtype()?,
//!     DataType::DateTime64 {
//!         unit: TimeUnit::Microsecond,
//!         timezone: Timezone::NAIVE,
//!     },
//! );
//! assert_eq!(
//!     Scalar::from_sequence([Scalar::from("AAPL"), Scalar::Null]).dtype()?,
//!     DataType::list(yggdryl::Field::new("item", DataType::Utf8, true)),
//! );
//! # Ok(())
//! # }
//! ```

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, I256, Result, Scalar, TimeUnit};

/// Arrow's widest exact decimal, and so the widest integer a decimal can hold.
const MAX_DECIMAL_PRECISION: usize = 76;

impl Scalar {
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
    pub fn dtype(&self) -> Result<DataType> {
        self.dtype_at(0)
    }

    /// Infer the exact Field for one scalar value.
    ///
    /// The stable name is `value`; a null value names a nullable Null field.
    pub fn inferred_scalar_field(&self) -> Result<Field> {
        Ok(Field::new("value", self.dtype()?, self.is_null()))
    }

    /// Infer the exact item Field for one outer Sequence.
    ///
    /// The stable name is `item`, matching the child name [`Self::dtype`]
    /// gives an inferred List. Empty sequences are ambiguous and require a
    /// declared Field.
    pub fn inferred_array_field(&self) -> Result<Field> {
        let Some(values) = self.as_sequence() else {
            return Err(unnameable(format_smolstr!(
                "expected an outer Sequence to infer an array item Field, got {}",
                self.kind()
            )));
        };
        if values.is_empty() {
            return Err(unnameable(SmolStr::new_static(
                "cannot infer an array item Field from an empty Sequence; pass a Field",
            )));
        }
        self.dtype()?
            .get_field(0)
            .cloned()
            .map(|field| field.with_name("item"))
            .ok_or_else(|| {
                unnameable(SmolStr::new_static(
                    "an outer Sequence did not infer one List item Field",
                ))
            })
    }

    /// Infer one non-null Struct root from named Record rows.
    ///
    /// Positional Sequence rows carry no column names and empty rows carry no
    /// datatype, so both require a declared Field. The stable root name is
    /// `row` in Rust, Python, and JavaScript.
    pub fn inferred_struct_field(&self) -> Result<Field> {
        let Some(rows) = self.as_sequence() else {
            return Err(unnameable(format_smolstr!(
                "expected a non-empty Sequence of named Record rows to infer a Struct Field, got {}",
                self.kind()
            )));
        };
        if rows.is_empty() {
            return Err(unnameable(SmolStr::new_static(
                "cannot infer a Struct Field from empty rows; pass a Struct Field",
            )));
        }
        if rows.iter().any(|row| !matches!(row, Self::Record(_))) {
            return Err(unnameable(SmolStr::new_static(
                "positional Sequence rows cannot infer field names; pass a Struct Field",
            )));
        }
        let item = self.dtype()?.get_field(0).cloned().ok_or_else(|| {
            unnameable(SmolStr::new_static(
                "named Record rows did not infer one List item Field",
            ))
        })?;
        let root = item.with_name("row").with_nullable(false);
        root.validate_struct_root()?;
        Ok(root)
    }

    /// Name this value's datatype, refusing to recurse past the shared limit.
    ///
    /// A value can nest as deeply as whoever built it chose, and a datatype is
    /// walked recursively, so the walk is bounded the way every other recursive
    /// descent in the project is rather than by the size of the native stack.
    fn dtype_at(&self, depth: usize) -> Result<DataType> {
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
            Self::F16(_) => Ok(DataType::Float16),
            Self::F32(_) => Ok(DataType::Float32),
            Self::F64(_) => Ok(DataType::Float64),
            Self::D128(unscaled, scale) => {
                decimal_dtype(I256::from_i128(*unscaled), *scale, DecimalWidth::D128)
            }
            Self::D256(unscaled, scale) => decimal_dtype(*unscaled, *scale, DecimalWidth::D256),
            Self::String(_) => Ok(DataType::Utf8),
            Self::Enum(_) => Ok(DataType::Utf8),
            // The datatype model has no geospatial family yet, so a geometry
            // names what it stores: the WKB bytes.
            Self::Bytes(_) | Self::Geospatial(_) => Ok(DataType::Binary),
            Self::Date32(_, unit, zone) => {
                require(
                    *unit == TimeUnit::Day && zone.is_naive(),
                    "invalid Date32 parts",
                )?;
                Ok(DataType::Date32)
            }
            Self::Date64(_, unit, zone) => {
                require(
                    *unit == TimeUnit::Millisecond && zone.is_naive(),
                    "invalid Date64 parts",
                )?;
                Ok(DataType::Date64)
            }
            Self::Time32(_, unit, zone) => {
                require(zone.is_naive(), "Time32 cannot carry a timezone")?;
                DataType::time32(*unit)
            }
            Self::Time64(_, unit, zone) => {
                require(zone.is_naive(), "Time64 cannot carry a timezone")?;
                DataType::time64(*unit)
            }
            Self::DateTime64(_, unit, zone) => {
                resolution(*unit, "datetime64")?;
                Ok(DataType::DateTime64 {
                    unit: *unit,
                    timezone: *zone,
                })
            }
            Self::Duration32(_, unit, zone) => {
                require(
                    unit.is_arrow_time() && zone.is_naive(),
                    "invalid Duration32 parts",
                )?;
                Ok(DataType::Duration32(*unit))
            }
            Self::Duration64(_, unit, zone) => {
                require(
                    unit.is_arrow_time() && zone.is_naive(),
                    "invalid Duration64 parts",
                )?;
                Ok(DataType::Duration64(*unit))
            }
            Self::Sequence(values) => {
                let (dtype, nullable) = agreed(values.iter(), "sequence item", depth)?;
                Ok(DataType::list(Field::new("item", dtype, nullable)))
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
            Self::Record(entries) => DataType::from_fields(
                entries
                    .iter()
                    .map(|(name, value)| {
                        let nullable = value.is_null();
                        value
                            .dtype_at(depth + 1)
                            .map(|dtype| Field::new(name.as_str(), dtype, nullable))
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        }
    }
}

/// Return the one datatype every non-null value names, and whether any was null.
fn agreed<'a>(
    values: impl IntoIterator<Item = &'a Scalar>,
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
        let dtype = value.dtype_at(depth + 1)?;
        match agreed.take() {
            Some(existing) => match merge_inferred(&existing, &dtype) {
                Some(merged) => agreed = Some(merged),
                None => {
                    return Err(unnameable(format_smolstr!(
                        "every {role} must name one datatype, got {} and {}",
                        crate::text::elide_display(&existing),
                        crate::text::elide_display(&dtype),
                    )));
                }
            },
            None => agreed = Some(dtype),
        }
    }
    // Nothing but nulls names the null type, which is a real Arrow column.
    Ok(agreed.map_or((DataType::Null, true), |dtype| (dtype, nullable)))
}

/// The one datatype two sampled values share, or `None`.
fn merge_inferred(left: &DataType, right: &DataType) -> Option<DataType> {
    // Inference widens: two sampled rows meet at the type that holds both.
    // The rule table is [`DataType::merge_with`]'s, so an inferred schema and
    // a declared one are reconciled by exactly the same rules.
    left.merge_exact(right, crate::types::Widening::Up).ok()
}

/// Return the exact decimal a coefficient and scale name.
enum DecimalWidth {
    D128,
    D256,
}

fn decimal_dtype(unscaled: I256, scale: i8, width: DecimalWidth) -> Result<DataType> {
    // Arrow requires a positive scale to fit inside the precision, so a
    // coefficient of 5 at scale 3 is `0.005` and needs three digits, not one.
    let precision = unscaled
        .to_string()
        .trim_start_matches('-')
        .len()
        .max(usize::try_from(scale.max(0)).unwrap_or(0));
    if precision > MAX_DECIMAL_PRECISION {
        return Err(unnameable(format_smolstr!(
            "a decimal of {precision} digits exceeds Arrow's maximum precision of {MAX_DECIMAL_PRECISION}"
        )));
    }
    let precision = u8::try_from(precision.max(1)).unwrap_or(76);
    match width {
        DecimalWidth::D128 => DataType::decimal128(precision, scale),
        DecimalWidth::D256 => DataType::decimal256(precision, scale),
    }
}

/// Return the exact decimal a 128-bit integer of `digits` digits needs.
fn integer_decimal(digits: u32) -> Result<DataType> {
    DataType::decimal(u8::try_from(digits.max(1)).unwrap_or(1), 0)
}

/// Reject a calendar interval layout where a temporal resolution is required.
fn resolution(unit: TimeUnit, kind: &'static str) -> Result<()> {
    if unit.is_arrow_time() {
        Ok(())
    } else {
        Err(unnameable(format_smolstr!(
            "a {kind} unit must be a temporal resolution, got {unit}"
        )))
    }
}

fn require(valid: bool, reason: &'static str) -> Result<()> {
    valid.then_some(()).ok_or_else(|| unnameable(reason.into()))
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
