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
//! Physical identity is part of an exact scalar: `large_utf8`, `binary_view`,
//! `Date64`, and every other leaf name themselves rather than collapsing to a
//! related layout. Only a newly inferred nested collection needs a layout
//! choice: a run names the ordinary `Serie`
//! layout because the values carry no offset width, and an enum names `utf8`
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
//!     DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)?,
//! );
//! assert_eq!(
//!     Scalar::from_sequence([Scalar::from("AAPL"), Scalar::Null]).dtype()?,
//!     DataType::serie(yggdryl::Field::new("item", DataType::utf8(), true)),
//! );
//! # Ok(())
//! # }
//! ```

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Field, Result, Scalar, StructType, i256};

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
    /// gives an inferred Serie. Empty sequences are ambiguous and require a
    /// declared Field.
    pub fn inferred_array_field(&self) -> Result<Field> {
        let Some(values) = self.as_serie() else {
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
                    "an outer Sequence did not infer one Serie item Field",
                ))
            })
    }

    /// Infer one non-null Struct root from named Record rows.
    ///
    /// Positional Sequence rows carry no column names and empty rows carry no
    /// datatype, so both require a declared Field. The stable root name is
    /// `row` in Rust, Python, and JavaScript.
    pub fn inferred_struct_field(&self) -> Result<Field> {
        let Some(rows) = self.as_serie() else {
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
        // A run's rows must name their columns themselves; a column's field
        // already does, and is what the datatype below reads.
        if rows
            .as_slice()
            .is_some_and(|rows| rows.iter().any(|row| !matches!(row, Self::Struct(_))))
        {
            return Err(unnameable(SmolStr::new_static(
                "positional Sequence rows cannot infer field names; pass a Struct Field",
            )));
        }
        let item = self.dtype()?.get_field(0).cloned().ok_or_else(|| {
            unnameable(SmolStr::new_static(
                "named Record rows did not infer one Serie item Field",
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
            Self::Int8(_) => Ok(DataType::Int8),
            Self::Int16(_) => Ok(DataType::Int16),
            Self::Int32(_) => Ok(DataType::Int32),
            Self::Int64(_) => Ok(DataType::Int64),
            Self::UInt8(_) => Ok(DataType::UInt8),
            Self::UInt16(_) => Ok(DataType::UInt16),
            Self::UInt32(_) => Ok(DataType::UInt32),
            Self::UInt64(_) => Ok(DataType::UInt64),
            // Arrow has no 128-bit integer, and an exact decimal with scale
            // zero is an integer, so that is what a wide integer becomes.
            Self::Int128(value) => integer_decimal(digits(value.get().unsigned_abs())),
            Self::UInt128(value) => integer_decimal(digits(value.get())),
            Self::Float16(_) => Ok(DataType::Float16),
            Self::Float32(_) => Ok(DataType::Float32),
            Self::Float64(_) => Ok(DataType::Float64),
            Self::Decimal32(value) => decimal_dtype(
                i256::from_i128(i128::from(value.coefficient())),
                value.scale(),
                DecimalWidth::Decimal32,
            ),
            Self::Decimal64(value) => decimal_dtype(
                i256::from_i128(i128::from(value.coefficient())),
                value.scale(),
                DecimalWidth::Decimal64,
            ),
            Self::Decimal128(value) => decimal_dtype(
                i256::from_i128(value.coefficient()),
                value.scale(),
                DecimalWidth::Decimal128,
            ),
            Self::Decimal256(value) => {
                decimal_dtype(value.coefficient(), value.scale(), DecimalWidth::Decimal256)
            }
            // A string value already declares its leaf - its layout, its
            // charset and its number - so the inferred datatype is what the
            // value says it is rather than a guess over its characters; the
            // door refuses a numbered leaf stating zero. A code is its own
            // identity.
            crate::string_scalars!(_) => {
                DataType::string(self.string_parameters().expect("a string leaf"))
            }
            Self::Country(_) => Ok(DataType::Country),
            Self::Ccy(_) => Ok(DataType::Ccy),
            Self::MicCode(_) => Ok(DataType::MicCode),
            Self::CfiCode(_) => Ok(DataType::CfiCode),
            Self::Side(_) => Ok(DataType::Side),
            Self::State(_) => Ok(DataType::State),
            Self::TimeInForce(_) => Ok(DataType::TimeInForce),
            Self::IsinCode(_) => Ok(DataType::IsinCode),
            Self::CusipCode(_) => Ok(DataType::CusipCode),
            Self::SedolCode(_) => Ok(DataType::SedolCode),
            Self::BloombergCode(_) => Ok(DataType::BloombergCode),
            Self::FIGICode(_) => Ok(DataType::FIGICode),
            Self::Unit(_) => Ok(DataType::Unit),
            Self::Version(_) => Ok(DataType::Version),
            Self::Url(_) => Ok(DataType::url()),
            Self::Urn(_) => Ok(DataType::urn()),
            Self::Timezone(_) => Ok(DataType::Timezone),
            Self::MimeType(_) => Ok(DataType::MimeType),
            Self::MediaType(_) => Ok(DataType::MediaType),
            Self::Uuid(_) => Ok(DataType::Uuid),
            // A variant declares its own types per value, so the column it
            // proves is the variant itself, never what one row decodes to.
            Self::Variant(_) => Ok(DataType::Variant),
            crate::bytes_scalars!(_) => {
                DataType::bytes(self.bytes_parameters().expect("a byte leaf"))
            }
            Self::Geometry(_) => DataType::geometry(None),
            Self::Geography(_) => DataType::geography(None, None),
            Self::Date32(_) => Ok(DataType::date32()),
            Self::Date64(_) => Ok(DataType::date64()),
            Self::Time32(value) => DataType::time32(value.unit()),
            Self::Time64(value) => DataType::time64(value.unit()),
            Self::DateTime64(value) => DataType::datetime64(value.unit(), value.timezone()),
            Self::Duration32(value) => DataType::duration32(value.unit()),
            Self::Duration64(value) => DataType::duration64(value.unit()),
            Self::Interval(value) => DataType::interval(value.unit()),
            // A column carries its field and is read; a run has its item
            // agreed back out of its rows.
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => {
                let item = match values.field() {
                    Some(field) => field.clone().with_name("item"),
                    None => {
                        let rows = values.rows();
                        let (dtype, nullable) = agreed(rows.iter(), "sequence item", depth)?;
                        Field::new("item", dtype, nullable)
                    }
                };
                // The variant is the layout the value declares.
                match self {
                    Self::SerieView(_) => Ok(DataType::serie_view(item)),
                    Self::LargeSerie(_) => Ok(DataType::large_serie(item)),
                    Self::LargeSerieView(_) => Ok(DataType::large_serie_view(item)),
                    Self::FixedSizeSerie(_) => DataType::fixed_size_serie(
                        item,
                        i32::try_from(values.len()).map_err(|_| Error::InvalidDataType {
                            kind: "FixedSizeSerie",
                            reason: smol_str::format_smolstr!(
                                "{} items are past a fixed size serie's i32 width",
                                values.len()
                            ),
                        })?,
                    ),
                    _ => Ok(DataType::serie(item)),
                }
            }
            // A mapping's keys are values, not names, so its datatype is a map
            // and not a struct; a struct in this project is described by a
            // sequence, one value per declared field.
            Self::Map(entries) | Self::SortedMap(entries) => {
                let keys = entries.as_slice().iter().map(|(key, _)| key);
                let (key, _) = agreed(keys, "mapping key", depth)?;
                // Arrow fixes the entry nullability itself - a key is required
                // and a value is not - so only the two datatypes are inferred.
                let values = entries.as_slice().iter().map(|(_, value)| value);
                let (value, _) = agreed(values, "mapping value", depth)?;
                DataType::map_of(key, value, false)
            }
            Self::Struct(entries) => StructType::from_fields(
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
            )
            .map(DataType::from),
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
    left.merge_exact(right, crate::Widening::Up).ok()
}

/// Return the exact decimal a coefficient and scale name.
enum DecimalWidth {
    Decimal32,
    Decimal64,
    Decimal128,
    Decimal256,
}

fn decimal_dtype(unscaled: i256, scale: i8, width: DecimalWidth) -> Result<DataType> {
    let precision = decimal_precision(unscaled, scale)?;
    match width {
        DecimalWidth::Decimal32 => DataType::decimal32(precision, scale),
        DecimalWidth::Decimal64 => DataType::decimal64(precision, scale),
        DecimalWidth::Decimal128 => DataType::decimal128(precision, scale),
        DecimalWidth::Decimal256 => DataType::decimal256(precision, scale),
    }
}

fn decimal_precision(unscaled: i256, scale: i8) -> Result<u8> {
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
