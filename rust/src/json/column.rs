//! A landed column's rows written as the JSON [`JsonField`](super::wire::JsonField) spells each one,
//! read off the column's leaves where they lie rather than built into a
//! value per row.
//!
//! [`JsonColumn::bind`] narrows every leaf of the column once - a record's
//! children, a sequence's cut and items, a map's entries - so writing a row
//! is a walk over buffers: a struct is the keys its field names, spelled
//! once, around its children's rows; a sequence the items its cut
//! states; text the bytes it holds. Every other leaf writes the row's value
//! through [`JsonField`](super::wire::JsonField) itself, so the bytes are the ones the value path
//! writes, by construction rather than by a second spelling.

use std::ops::Range;

use serde::Serializer as _;

use crate::serie::{
    BooleanSerie, Decimal128Serie, FixedSizeSerieSerie, Float32Serie, Float64Serie, Int8Serie,
    Int16Serie, Int32Serie, Int64Serie, LargeSerieSerie, LargeSerieViewSerie, LargeUtf8StringSerie,
    MapSerie, SerieSerie, SerieViewSerie, StructSerie, UInt8Serie, UInt16Serie, UInt32Serie,
    UInt64Serie, Utf8StringSerie, Utf8ViewStringSerie,
};
use crate::{DataType, Result, Serie};

/// One landed column bound for writing as JSON: its storage narrowed once,
/// so a row is written from its buffers and never built as a value.
pub(crate) enum JsonColumn<'a> {
    /// A record column: each child behind the key its field names, spelled
    /// once with its colon.
    Struct {
        column: &'a StructSerie,
        keys: Vec<Vec<u8>>,
        children: Vec<JsonColumn<'a>>,
    },
    /// A sequence column of any layout: each row the items its cut states.
    Sequence {
        cut: Cut<'a>,
        items: Box<JsonColumn<'a>>,
    },
    /// A map column keyed by text with no absent key: each entry the key's
    /// text, then its value.
    Map {
        column: &'a MapSerie,
        keys: Text<'a>,
        values: Box<JsonColumn<'a>>,
    },
    /// UTF-8 text of a string datatype, written as it lies.
    Text(Text<'a>),
    /// A leaf whose rows are native values of its own datatype, written by
    /// the serializer call the value path makes for each.
    Native(Native<'a>),
    /// Any other column: each row's value, written by [`JsonField`](super::wire::JsonField).
    Value {
        column: &'a Serie,
        dtype: &'a DataType,
    },
}

impl<'a> JsonColumn<'a> {
    /// Narrow `column` and every column below it to the leaves its rows are
    /// written from.
    pub(crate) fn bind(column: &'a Serie) -> Result<Self> {
        let Some(field) = column.field() else {
            return Ok(Self::Value {
                column,
                dtype: &DataType::Null,
            });
        };
        let dtype = field.dtype();
        Ok(match dtype {
            DataType::Struct(fields) => match column.as_struct() {
                Some(records) => Self::Struct {
                    column: records,
                    keys: fields
                        .iter()
                        .map(|child| {
                            let mut key = serde_json::to_vec(child.name())
                                .map_err(|error| super::codec_error(0, &error.to_string()))?;
                            key.push(b':');
                            Ok(key)
                        })
                        .collect::<Result<_>>()?,
                    children: records
                        .children()
                        .iter()
                        .map(Self::bind)
                        .collect::<Result<_>>()?,
                },
                None => Self::Value { column, dtype },
            },
            DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(..)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_) => match Cut::of(column) {
                Some(cut) => Self::Sequence {
                    items: Box::new(Self::bind(cut.items())?),
                    cut,
                },
                None => Self::Value { column, dtype },
            },
            // A key with no text - or no key at all, which JSON cannot
            // spell - is the value path's to write and to refuse.
            DataType::Map(_) | DataType::SortedMap(_) => match column.as_map() {
                Some(map)
                    if map.keys().null_count() == 0
                        && map.keys().field().is_some_and(|key| is_text(key.dtype())) =>
                {
                    match Text::of(map.keys()) {
                        Some(keys) => Self::Map {
                            column: map,
                            keys,
                            values: Box::new(Self::bind(map.values())?),
                        },
                        None => Self::Value { column, dtype },
                    }
                }
                _ => Self::Value { column, dtype },
            },
            text if is_text(text) => {
                Text::of(column).map_or(Self::Value { column, dtype }, Self::Text)
            }
            _ => Native::of(column, dtype).map_or(Self::Value { column, dtype }, Self::Native),
        })
    }

    /// Append row `row` to `out` as the JSON [`JsonField`](super::wire::JsonField) writes for its
    /// value: `null` where the row is absent.
    pub(crate) fn write(&self, row: usize, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Struct {
                column,
                keys,
                children,
            } => {
                if column.nulls().is_some_and(|nulls| nulls.is_null(row)) {
                    out.extend_from_slice(b"null");
                    return Ok(());
                }
                out.push(b'{');
                for (index, (key, child)) in keys.iter().zip(children).enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(key);
                    child.write(row, out)?;
                }
                out.push(b'}');
            }
            Self::Sequence { cut, items } => {
                let Some(range) = cut.range(row) else {
                    out.extend_from_slice(b"null");
                    return Ok(());
                };
                out.push(b'[');
                for (index, item) in range.enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    items.write(item, out)?;
                }
                out.push(b']');
            }
            Self::Map {
                column,
                keys,
                values,
            } => {
                let Some(range) = column.range(row) else {
                    out.extend_from_slice(b"null");
                    return Ok(());
                };
                out.push(b'{');
                for (index, entry) in range.enumerate() {
                    if index != 0 {
                        out.push(b',');
                    }
                    // Bound only over a key column with no absent key.
                    keys.write(entry, out)?;
                    out.push(b':');
                    values.write(entry, out)?;
                }
                out.push(b'}');
            }
            Self::Text(text) => text.write(row, out)?,
            Self::Native(native) => native.write(row, out)?,
            Self::Value { column, dtype } => {
                super::into_field_vec(&column.scalar(row)?, dtype, out)?;
            }
        }
        Ok(())
    }
}

/// Whether a datatype is a string leaf, whose value is the text its column
/// holds.
fn is_text(dtype: &DataType) -> bool {
    dtype.string_parameters().is_some()
}

/// The cut of a sequence column, under whichever of the five layouts it is.
pub(crate) enum Cut<'a> {
    Offsets(&'a SerieSerie),
    LargeOffsets(&'a LargeSerieSerie),
    View(&'a SerieViewSerie),
    LargeView(&'a LargeSerieViewSerie),
    Fixed(&'a FixedSizeSerieSerie),
}

impl<'a> Cut<'a> {
    fn of(column: &'a Serie) -> Option<Self> {
        column
            .as_serie()
            .map(Self::Offsets)
            .or_else(|| column.as_large_serie().map(Self::LargeOffsets))
            .or_else(|| column.as_serie_view().map(Self::View))
            .or_else(|| column.as_large_serie_view().map(Self::LargeView))
            .or_else(|| column.as_fixed_size_serie().map(Self::Fixed))
    }

    const fn items(&self) -> &'a Serie {
        match self {
            Self::Offsets(cut) => cut.items(),
            Self::LargeOffsets(cut) => cut.items(),
            Self::View(cut) => cut.items(),
            Self::LargeView(cut) => cut.items(),
            Self::Fixed(cut) => cut.items(),
        }
    }

    /// The items row `row` holds, `None` where it is absent.
    fn range(&self, row: usize) -> Option<Range<usize>> {
        match self {
            Self::Offsets(cut) => cut.range(row),
            Self::LargeOffsets(cut) => cut.range(row),
            Self::View(cut) => cut.range(row),
            Self::LargeView(cut) => cut.range(row),
            Self::Fixed(cut) => cut.range(row),
        }
    }
}

/// A leaf column read as the native values its own datatype holds: an
/// integer, a float, a boolean, a `decimal128` at its scale. Its storage is
/// paired with exactly that datatype, so a row's value is the native one
/// and nothing a reading would restate.
pub(crate) enum Native<'a> {
    Boolean(&'a BooleanSerie),
    Int8(&'a Int8Serie),
    Int16(&'a Int16Serie),
    Int32(&'a Int32Serie),
    Int64(&'a Int64Serie),
    UInt8(&'a UInt8Serie),
    UInt16(&'a UInt16Serie),
    UInt32(&'a UInt32Serie),
    UInt64(&'a UInt64Serie),
    Float32(&'a Float32Serie),
    Float64(&'a Float64Serie),
    Decimal128(&'a Decimal128Serie, i8),
}

impl<'a> Native<'a> {
    fn of(column: &'a Serie, dtype: &DataType) -> Option<Self> {
        Some(match dtype {
            DataType::Boolean => Self::Boolean(column.as_boolean()?),
            DataType::Int8 => Self::Int8(column.as_int8()?),
            DataType::Int16 => Self::Int16(column.as_int16()?),
            DataType::Int32 => Self::Int32(column.as_int32()?),
            DataType::Int64 => Self::Int64(column.as_int64()?),
            DataType::UInt8 => Self::UInt8(column.as_uint8()?),
            DataType::UInt16 => Self::UInt16(column.as_uint16()?),
            DataType::UInt32 => Self::UInt32(column.as_uint32()?),
            DataType::UInt64 => Self::UInt64(column.as_uint64()?),
            DataType::Float32 => Self::Float32(column.as_float32()?),
            DataType::Float64 => Self::Float64(column.as_float64()?),
            DataType::Decimal128 { scale, .. } => Self::Decimal128(column.as_decimal128()?, *scale),
            _ => return None,
        })
    }

    /// Append row `row` as the serializer call [`JsonRef`](super::wire::JsonRef)
    /// makes for its value, `null` where it is absent.
    fn write(&self, row: usize, out: &mut Vec<u8>) -> Result<()> {
        let mut serializer = serde_json::Serializer::new(&mut *out);
        let written = match self {
            Self::Boolean(column) => column
                .value(row)
                .map(|value| serializer.serialize_bool(value)),
            Self::Int8(column) => column
                .value(row)
                .map(|value| serializer.serialize_i8(value)),
            Self::Int16(column) => column
                .value(row)
                .map(|value| serializer.serialize_i16(value)),
            Self::Int32(column) => column
                .value(row)
                .map(|value| serializer.serialize_i32(value)),
            Self::Int64(column) => column
                .value(row)
                .map(|value| serializer.serialize_i64(value)),
            Self::UInt8(column) => column
                .value(row)
                .map(|value| serializer.serialize_u8(value)),
            Self::UInt16(column) => column
                .value(row)
                .map(|value| serializer.serialize_u16(value)),
            Self::UInt32(column) => column
                .value(row)
                .map(|value| serializer.serialize_u32(value)),
            Self::UInt64(column) => column
                .value(row)
                .map(|value| serializer.serialize_u64(value)),
            Self::Float32(column) => column
                .value(row)
                .map(|value| super::wire::serialize_float(&mut serializer, f64::from(value))),
            Self::Float64(column) => column
                .value(row)
                .map(|value| super::wire::serialize_float(&mut serializer, value)),
            Self::Decimal128(column, scale) => column
                .value(row)
                .map(|value| serializer.collect_str(&crate::Decimal128::new(value, *scale))),
        };
        written
            .unwrap_or_else(|| serializer.serialize_none())
            .map_err(|error| super::codec_error(0, &error.to_string()))
    }
}

/// A column whose storage is UTF-8 text, under whichever layout holds it.
pub(crate) enum Text<'a> {
    Utf8(&'a Utf8StringSerie),
    LargeUtf8(&'a LargeUtf8StringSerie),
    Utf8View(&'a Utf8ViewStringSerie),
}

impl<'a> Text<'a> {
    fn of(column: &'a Serie) -> Option<Self> {
        column
            .as_utf8()
            .map(Self::Utf8)
            .or_else(|| column.as_large_utf8().map(Self::LargeUtf8))
            .or_else(|| column.as_utf8_view().map(Self::Utf8View))
    }

    /// Append row `row` as the JSON string [`JsonField`](super::wire::JsonField) writes for the
    /// text, `null` where it is absent.
    fn write(&self, row: usize, out: &mut Vec<u8>) -> Result<()> {
        let text = match self {
            Self::Utf8(column) => column.value(row),
            Self::LargeUtf8(column) => column.value(row),
            Self::Utf8View(column) => column.value(row),
        };
        match text {
            Some(text) => serde_json::to_writer(&mut *out, text)
                .map_err(|error| super::codec_error(0, &error.to_string())),
            None => {
                out.extend_from_slice(b"null");
                Ok(())
            }
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/json/column.rs` pins and a caller cannot reach.
    use crate::{Result, Serie};

    /// Every row of a landed column as the JSON its leaves write, `None`
    /// where the row is absent.
    pub fn rows(column: &Serie) -> Result<Vec<Option<Vec<u8>>>> {
        let bound = super::JsonColumn::bind(column)?;
        (0..column.len())
            .map(|row| {
                if column.is_null(row)? {
                    return Ok(None);
                }
                let mut out = Vec::new();
                bound.write(row, &mut out)?;
                Ok(Some(out))
            })
            .collect()
    }

    /// Every row as the value path writes it: the row's value, then
    /// `JsonField`.
    pub fn rows_by_value(column: &Serie) -> Result<Vec<Option<Vec<u8>>>> {
        let dtype = column.field().map(crate::Field::dtype);
        (0..column.len())
            .map(|row| {
                let value = column.scalar(row)?;
                if value.is_null() {
                    return Ok(None);
                }
                let mut out = Vec::new();
                crate::json::into_field_vec(
                    &value,
                    dtype.unwrap_or(&crate::DataType::Null),
                    &mut out,
                )?;
                Ok(Some(out))
            })
            .collect()
    }
}
