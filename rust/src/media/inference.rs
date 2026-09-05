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
//! Physical identity is part of an exact scalar: `LargeUtf8`, `BinaryView`,
//! `Date64`, and every other leaf name themselves rather than collapsing to a
//! related layout. Only a newly inferred nested collection needs a layout
//! choice: a [`crate::types::Nested::Sequence`] names the ordinary `List`
//! layout because the values carry no offset width, and an enum names `Utf8`
//! because its generic identity is not an Arrow datatype.
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

use crate::types::{AsciiFamily, Bytes, Decimal, Geospatial, Integer, Nested, Temporal, Text};
use crate::{DataType, Error, Field, I256, Result, Scalar, TimeUnit};

/// Arrow's widest exact decimal, and so the widest integer a decimal can hold.
const MAX_DECIMAL_PRECISION: usize = 76;

impl Scalar {
    /// Return the datatype this value materializes into.
    ///
    /// The name is read off the variant, so a datetime keeps its unit and
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
        if rows
            .iter()
            .any(|row| !matches!(row, Self::Nested(Nested::Record(_))))
        {
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
            Self::Boolean(_) => Ok(DataType::Boolean),
            Self::Integer(Integer::I8(_)) => Ok(DataType::Int8),
            Self::Integer(Integer::I16(_)) => Ok(DataType::Int16),
            Self::Integer(Integer::I32(_)) => Ok(DataType::Int32),
            Self::Integer(Integer::I64(_)) => Ok(DataType::Int64),
            Self::Integer(Integer::U8(_)) => Ok(DataType::UInt8),
            Self::Integer(Integer::U16(_)) => Ok(DataType::UInt16),
            Self::Integer(Integer::U32(_)) => Ok(DataType::UInt32),
            Self::Integer(Integer::U64(_)) => Ok(DataType::UInt64),
            // Arrow has no 128-bit integer, and an exact decimal with scale
            // zero is an integer, so that is what a wide integer becomes.
            Self::Integer(Integer::I128(value)) => {
                integer_decimal(digits(value.get().unsigned_abs()))
            }
            Self::Integer(Integer::U128(value)) => integer_decimal(digits(value.get())),
            Self::Floating(value) => Ok(match value {
                crate::types::Floating::F16(_) => DataType::Float16,
                crate::types::Floating::F32(_) => DataType::Float32,
                crate::types::Floating::F64(_) => DataType::Float64,
            }),
            Self::Decimal(Decimal::D32(value)) => decimal_dtype(
                I256::from_i128(i128::from(value.coefficient())),
                value.scale(),
                DecimalWidth::D32,
            ),
            Self::Decimal(Decimal::D64(value)) => decimal_dtype(
                I256::from_i128(i128::from(value.coefficient())),
                value.scale(),
                DecimalWidth::D64,
            ),
            Self::Decimal(Decimal::D128(value)) => decimal_dtype(
                I256::from_i128(value.coefficient()),
                value.scale(),
                DecimalWidth::D128,
            ),
            Self::Decimal(Decimal::D256(value)) => {
                decimal_dtype(value.coefficient(), value.scale(), DecimalWidth::D256)
            }
            Self::Text(Text::Utf8(_)) => Ok(DataType::Utf8),
            Self::Text(Text::LargeUtf8(_)) => Ok(DataType::LargeUtf8),
            Self::Text(Text::Utf8View(_)) => Ok(DataType::Utf8View),
            Self::Ascii(AsciiFamily::Ascii(_)) => Ok(DataType::Ascii),
            Self::Ascii(AsciiFamily::FixedAscii(value)) => DataType::ascii(value.width()),
            Self::Ascii(AsciiFamily::Country(_)) => Ok(DataType::Country),
            Self::Ascii(AsciiFamily::Currency(_)) => Ok(DataType::Currency),
            Self::Ascii(AsciiFamily::Mic(_)) => Ok(DataType::Mic),
            Self::Ascii(AsciiFamily::Cfi(_)) => Ok(DataType::Cfi),
            Self::Version(_) => Ok(DataType::Version),
            Self::Uuid(_) => Ok(DataType::Uuid),
            Self::Enum(_) => Ok(DataType::Utf8),
            Self::Bytes(Bytes::Binary(_)) => Ok(DataType::Binary),
            Self::Bytes(Bytes::FixedSizeBinary(value)) => DataType::fixed_size_binary(
                i32::try_from(value.as_bytes().len())
                    .map_err(|_| unnameable("binary width exceeds i32".into()))?,
            ),
            Self::Bytes(Bytes::LargeBinary(_)) => Ok(DataType::LargeBinary),
            Self::Bytes(Bytes::BinaryView(_)) => Ok(DataType::BinaryView),
            Self::Geospatial(Geospatial::Geometry(_)) => DataType::geometry(None),
            Self::Geospatial(Geospatial::Geography(_)) => DataType::geography(None, None),
            Self::Temporal(Temporal::Date32(_)) => Ok(DataType::Date32),
            Self::Temporal(Temporal::Date64(_)) => Ok(DataType::Date64),
            Self::Temporal(Temporal::Time32(value)) => DataType::time32(value.unit()),
            Self::Temporal(Temporal::Time64(value)) => DataType::time64(value.unit()),
            Self::Temporal(Temporal::DateTime64(value)) => {
                resolution(value.unit(), "datetime64")?;
                Ok(DataType::DateTime64 {
                    unit: value.unit(),
                    timezone: value.timezone(),
                })
            }
            Self::Temporal(Temporal::Duration32(value)) => Ok(DataType::Duration32(value.unit())),
            Self::Temporal(Temporal::Duration64(value)) => Ok(DataType::Duration64(value.unit())),
            Self::Temporal(Temporal::Interval(value)) => Ok(DataType::Interval(value.unit())),
            Self::Nested(Nested::Sequence(values)) => {
                let (dtype, nullable) = agreed(values.as_slice().iter(), "sequence item", depth)?;
                Ok(DataType::list(Field::new("item", dtype, nullable)))
            }
            // A mapping's keys are values, not names, so its datatype is a map
            // and not a struct; a struct in this project is described by a
            // sequence, one value per declared field.
            Self::Nested(Nested::Mapping(entries)) => {
                let keys = entries.as_slice().iter().map(|(key, _)| key);
                let (key, _) = agreed(keys, "mapping key", depth)?;
                // Arrow fixes the entry nullability itself - a key is required
                // and a value is not - so only the two datatypes are inferred.
                let values = entries.as_slice().iter().map(|(_, value)| value);
                let (value, _) = agreed(values, "mapping value", depth)?;
                DataType::map_of(key, value, false)
            }
            Self::Nested(Nested::Record(entries)) => DataType::from_fields(
                entries
                    .as_map()
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
    D32,
    D64,
    D128,
    D256,
}

fn decimal_dtype(unscaled: I256, scale: i8, width: DecimalWidth) -> Result<DataType> {
    let precision = decimal_precision(unscaled, scale)?;
    match width {
        DecimalWidth::D32 => DataType::decimal32(precision, scale),
        DecimalWidth::D64 => DataType::decimal64(precision, scale),
        DecimalWidth::D128 => DataType::decimal128(precision, scale),
        DecimalWidth::D256 => DataType::decimal256(precision, scale),
    }
}

fn decimal_precision(unscaled: I256, scale: i8) -> Result<u8> {
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
    Ok(u8::try_from(precision.max(1)).unwrap_or(76))
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
