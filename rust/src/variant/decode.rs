//! The decode walk: Variant metadata and value bytes back into one [`Value`].
//!
//! Every read goes through a position-carrying cursor, so a truncation, an
//! unknown type id, or an out-of-range offset reports the exact byte it
//! failed at. Containers bound every count a buffer promises against the
//! bytes actually present before allocating for it, and the shared
//! [`Limits`] cap depth, node count, and input size the way the other
//! codecs' readers do.

use std::collections::HashSet;

use smol_str::{SmolStr, format_smolstr};

use crate::{Float, Float32, Limits, Result, TimeUnit, Timezone, Value};

use super::metadata::MetadataView;
use super::{
    BASIC_OBJECT, BASIC_PRIMITIVE, BASIC_SHORT_STRING, PRIMITIVE_BINARY, PRIMITIVE_DATE,
    PRIMITIVE_DECIMAL4, PRIMITIVE_DECIMAL8, PRIMITIVE_DECIMAL16, PRIMITIVE_DOUBLE, PRIMITIVE_FALSE,
    PRIMITIVE_FLOAT, PRIMITIVE_INT8, PRIMITIVE_INT16, PRIMITIVE_INT32, PRIMITIVE_INT64,
    PRIMITIVE_MAX, PRIMITIVE_NULL, PRIMITIVE_STRING, PRIMITIVE_TIME_MICROS,
    PRIMITIVE_TIMESTAMP_MICROS, PRIMITIVE_TIMESTAMP_NANOS, PRIMITIVE_TIMESTAMP_NTZ_MICROS,
    PRIMITIVE_TIMESTAMP_NTZ_NANOS, PRIMITIVE_TRUE, PRIMITIVE_UUID, byte_word, codec,
};

/// Decode one Variant `(metadata, value)` byte pair under the default
/// [`Limits`].
///
/// # Errors
///
/// See [`decode_value_with_limits`].
pub fn decode_value(metadata: &[u8], value: &[u8]) -> Result<Value> {
    decode_value_with_limits(metadata, value, Limits::default())
}

/// Decode one Variant `(metadata, value)` byte pair under explicit
/// [`Limits`].
///
/// The metadata dictionary may be unsorted - older writers did not set the
/// `sorted_strings` bit - and containers may use wider offset, id, and count
/// spellings than they need, as the specification permits. The whole value
/// slice must be one value: trailing bytes are refused, because a caller
/// holding a column cell wants to know the cell is exactly what it decoded.
///
/// # Errors
///
/// Returns an error naming the byte position when either buffer is truncated,
/// the metadata version is not 1, an offset or field id is out of range, a
/// primitive type id is unknown, an object repeats a field name, a decimal
/// scale is beyond 38, string bytes are not UTF-8, bytes trail the value, or
/// the input exceeds the given limits.
pub fn decode_value_with_limits(metadata: &[u8], value: &[u8], limits: Limits) -> Result<Value> {
    let input = metadata.len().saturating_add(value.len());
    if input > limits.max_input_bytes() {
        return Err(codec(
            0,
            format_smolstr!(
                "expected at most {} input bytes, got {input}",
                limits.max_input_bytes()
            ),
        ));
    }
    let dictionary = MetadataView::parse(metadata, limits)?;
    let mut cursor = Cursor::new(value, 0);
    let mut budget = limits.max_nodes();
    let decoded = read_value(&mut cursor, &dictionary, 1, &mut budget, limits)?;
    cursor.finish()?;
    Ok(decoded)
}

/// A byte position over one value slice. Children decode from sub-slices, so
/// the cursor carries the slice's offset within the whole value buffer and
/// every error reports an absolute position.
struct Cursor<'bytes> {
    bytes: &'bytes [u8],
    position: usize,
    base: usize,
}

impl<'bytes> Cursor<'bytes> {
    const fn new(bytes: &'bytes [u8], base: usize) -> Self {
        Self {
            bytes,
            position: 0,
            base,
        }
    }

    /// The absolute position of the next byte in the value buffer.
    const fn at(&self) -> usize {
        self.base + self.position
    }

    /// The bytes not yet consumed.
    const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Consume exactly `len` bytes, or report what was wanted and what is left.
    fn take(&mut self, len: usize, what: &'static str) -> Result<&'bytes [u8]> {
        let Some(taken) = self.bytes.get(self.position..self.position + len) else {
            return Err(codec(
                self.at(),
                format_smolstr!(
                    "expected {len} {word} of {what}, got {remaining}",
                    word = byte_word(len),
                    remaining = self.remaining()
                ),
            ));
        };
        self.position += len;
        Ok(taken)
    }

    /// Consume one byte.
    fn take_u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    /// Consume one unsigned little-endian value of `width` bytes.
    fn take_unsigned(&mut self, width: usize, what: &'static str) -> Result<u64> {
        let taken = self.take(width, what)?;
        let mut value = 0_u64;
        for (index, byte) in taken.iter().enumerate() {
            value |= u64::from(*byte) << (8 * index);
        }
        Ok(value)
    }

    /// Consume a fixed-width little-endian array.
    fn take_array<const WIDTH: usize>(&mut self, what: &'static str) -> Result<[u8; WIDTH]> {
        Ok(self
            .take(WIDTH, what)?
            .try_into()
            .expect("exactly WIDTH bytes were just taken"))
    }

    /// Refuse bytes trailing the value: a cell is exactly one value.
    fn finish(&self) -> Result<()> {
        if self.position == self.bytes.len() {
            return Ok(());
        }
        Err(codec(
            self.at(),
            format_smolstr!(
                "expected the end of the buffer, got {remaining} trailing {word}",
                remaining = self.remaining(),
                word = byte_word(self.remaining())
            ),
        ))
    }
}

/// Spend one node from the decode budget.
fn spend(budget: &mut usize, limits: Limits, position: usize) -> Result<()> {
    if *budget == 0 {
        return Err(codec(
            position,
            format_smolstr!("expected a value of at most {} nodes", limits.max_nodes()),
        ));
    }
    *budget -= 1;
    Ok(())
}

/// Read one complete value from the cursor, leaving it just past the value.
///
/// `depth` is the container nesting level this value sits at - the root is
/// level 1 - and only a container checks it, because only containers nest.
fn read_value(
    cursor: &mut Cursor<'_>,
    dictionary: &MetadataView<'_>,
    depth: usize,
    budget: &mut usize,
    limits: Limits,
) -> Result<Value> {
    spend(budget, limits, cursor.at())?;
    let header_position = cursor.at();
    let header = cursor.take_u8("value header")?;
    let basic_type = header & 0b11;
    let value_header = header >> 2;
    match basic_type {
        BASIC_PRIMITIVE => read_primitive(cursor, value_header, header_position),
        BASIC_SHORT_STRING => {
            let bytes = cursor.take(usize::from(value_header), "short string")?;
            let text = utf8(bytes, header_position + 1)?;
            Ok(Value::String(SmolStr::new(text)))
        }
        BASIC_OBJECT => {
            check_depth(depth, limits, header_position)?;
            read_object(cursor, value_header, dictionary, depth, budget, limits)
        }
        _ => {
            check_depth(depth, limits, header_position)?;
            read_array(cursor, value_header, dictionary, depth, budget, limits)
        }
    }
}

/// Refuse a container nested past the depth limit, at its header byte.
fn check_depth(depth: usize, limits: Limits, position: usize) -> Result<()> {
    if depth > limits.max_depth() {
        return Err(codec(
            position,
            format_smolstr!(
                "expected a value at most {} levels deep",
                limits.max_depth()
            ),
        ));
    }
    Ok(())
}

/// Read one primitive's value data after its header byte.
#[allow(clippy::too_many_lines)]
fn read_primitive(
    cursor: &mut Cursor<'_>,
    primitive_id: u8,
    header_position: usize,
) -> Result<Value> {
    Ok(match primitive_id {
        PRIMITIVE_NULL => Value::Null,
        PRIMITIVE_TRUE => Value::Bool(true),
        PRIMITIVE_FALSE => Value::Bool(false),
        PRIMITIVE_INT8 => Value::I8(i8::from_le_bytes(cursor.take_array("an int8 value")?)),
        PRIMITIVE_INT16 => Value::I16(i16::from_le_bytes(cursor.take_array("an int16 value")?)),
        PRIMITIVE_INT32 => Value::I32(i32::from_le_bytes(cursor.take_array("an int32 value")?)),
        PRIMITIVE_INT64 => Value::I64(i64::from_le_bytes(cursor.take_array("an int64 value")?)),
        PRIMITIVE_DOUBLE => Value::F64(Float::from_f64(f64::from_le_bytes(
            cursor.take_array("a double value")?,
        ))),
        PRIMITIVE_DECIMAL4 => {
            let scale = read_scale(cursor)?;
            let unscaled = i32::from_le_bytes(cursor.take_array("a decimal4 unscaled value")?);
            Value::Decimal(i128::from(unscaled), scale)
        }
        PRIMITIVE_DECIMAL8 => {
            let scale = read_scale(cursor)?;
            let unscaled = i64::from_le_bytes(cursor.take_array("a decimal8 unscaled value")?);
            Value::Decimal(i128::from(unscaled), scale)
        }
        PRIMITIVE_DECIMAL16 => {
            let scale = read_scale(cursor)?;
            let unscaled = i128::from_le_bytes(cursor.take_array("a decimal16 unscaled value")?);
            Value::Decimal(unscaled, scale)
        }
        PRIMITIVE_DATE => Value::Date(i32::from_le_bytes(cursor.take_array("a date value")?)),
        PRIMITIVE_TIMESTAMP_MICROS => Value::Timestamp(
            i64::from_le_bytes(cursor.take_array("a timestamp value")?),
            TimeUnit::Microsecond,
            Timezone::UTC,
        ),
        PRIMITIVE_TIMESTAMP_NTZ_MICROS => Value::DateTime(
            i64::from_le_bytes(cursor.take_array("a timestamp value")?),
            TimeUnit::Microsecond,
        ),
        PRIMITIVE_FLOAT => Value::F32(Float32::from_f32(f32::from_le_bytes(
            cursor.take_array("a float value")?,
        ))),
        PRIMITIVE_BINARY => {
            let length = cursor.take_array::<4>("a binary size")?;
            let length = u32::from_le_bytes(length) as usize;
            Value::Bytes(cursor.take(length, "binary data")?.into())
        }
        PRIMITIVE_STRING => {
            let length = cursor.take_array::<4>("a string size")?;
            let length = u32::from_le_bytes(length) as usize;
            let start = cursor.at();
            let bytes = cursor.take(length, "string data")?;
            Value::String(SmolStr::new(utf8(bytes, start)?))
        }
        PRIMITIVE_TIME_MICROS => Value::Time(
            i64::from_le_bytes(cursor.take_array("a time value")?),
            TimeUnit::Microsecond,
        ),
        PRIMITIVE_TIMESTAMP_NANOS => Value::Timestamp(
            i64::from_le_bytes(cursor.take_array("a timestamp value")?),
            TimeUnit::Nanosecond,
            Timezone::UTC,
        ),
        PRIMITIVE_TIMESTAMP_NTZ_NANOS => Value::DateTime(
            i64::from_le_bytes(cursor.take_array("a timestamp value")?),
            TimeUnit::Nanosecond,
        ),
        // The tree has no UUID kind, so the sixteen big-endian bytes come
        // back as bytes - the same spelling the workspace's `fixed[16]` UUID
        // columns use.
        PRIMITIVE_UUID => Value::Bytes(cursor.take(16, "a uuid value")?.into()),
        other => {
            return Err(codec(
                header_position,
                format_smolstr!(
                    "expected a primitive type id between 0 and {PRIMITIVE_MAX}, got {other}"
                ),
            ));
        }
    })
}

/// Read one decimal scale byte, bounded to the specification's 0..=38.
fn read_scale(cursor: &mut Cursor<'_>) -> Result<i8> {
    let position = cursor.at();
    let scale = cursor.take_u8("a decimal scale")?;
    if scale > 38 {
        return Err(codec(
            position,
            format_smolstr!("expected a decimal scale between 0 and 38, got {scale}"),
        ));
    }
    Ok(i8::try_from(scale).expect("the scale was bounded to 0..=38"))
}

/// Read one object: count, field ids, offsets, then each field value from
/// its own offset, in the name-sorted order the ids are spelled in.
fn read_object(
    cursor: &mut Cursor<'_>,
    value_header: u8,
    dictionary: &MetadataView<'_>,
    depth: usize,
    budget: &mut usize,
    limits: Limits,
) -> Result<Value> {
    let field_offset_size = usize::from(value_header & 0b11) + 1;
    let field_id_size = usize::from((value_header >> 2) & 0b11) + 1;
    let is_large = value_header & 0b1_0000 != 0;
    let count = read_count(cursor, is_large)?;
    // Bound the promised lists against the bytes present before allocating:
    // `count` ids, `count + 1` offsets, and at least one byte per value.
    let promised = count * field_id_size + (count + 1) * field_offset_size + count;
    if promised > cursor.remaining() {
        return Err(codec(
            cursor.at(),
            format_smolstr!(
                "expected {promised} {} of object fields, got {}",
                byte_word(promised),
                cursor.remaining()
            ),
        ));
    }
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        let position = cursor.at();
        let id = cursor.take_unsigned(field_id_size, "a field id")?;
        let id = usize::try_from(id).unwrap_or(usize::MAX);
        let Some(name) = dictionary.get(id) else {
            return Err(codec(
                position,
                format_smolstr!(
                    "expected a field id below the dictionary size {}, got {id}",
                    dictionary.len()
                ),
            ));
        };
        names.push(name);
    }
    let mut seen = HashSet::with_capacity(names.len());
    for name in &names {
        if !seen.insert(*name) {
            return Err(codec(
                cursor.at(),
                format_smolstr!("expected unique object field names, got {name:?} twice"),
            ));
        }
    }
    let values = read_fields(
        cursor,
        count,
        field_offset_size,
        dictionary,
        depth,
        budget,
        limits,
    )?;
    let entries: Vec<(Value, Value)> = names
        .into_iter()
        .map(|name| Value::String(SmolStr::new(name)))
        .zip(values)
        .collect();
    Ok(Value::Mapping(entries.into()))
}

/// Read one array: count, offsets, then each element from its own offset.
fn read_array(
    cursor: &mut Cursor<'_>,
    value_header: u8,
    dictionary: &MetadataView<'_>,
    depth: usize,
    budget: &mut usize,
    limits: Limits,
) -> Result<Value> {
    let field_offset_size = usize::from(value_header & 0b11) + 1;
    let is_large = value_header & 0b100 != 0;
    let count = read_count(cursor, is_large)?;
    // Bound the promised offsets and one byte per element before allocating.
    let promised = (count + 1) * field_offset_size + count;
    if promised > cursor.remaining() {
        return Err(codec(
            cursor.at(),
            format_smolstr!(
                "expected {promised} {} of array elements, got {}",
                byte_word(promised),
                cursor.remaining()
            ),
        ));
    }
    let values = read_fields(
        cursor,
        count,
        field_offset_size,
        dictionary,
        depth,
        budget,
        limits,
    )?;
    Ok(Value::from_sequence(values))
}

/// Read a container's element count at the width `is_large` names.
fn read_count(cursor: &mut Cursor<'_>, is_large: bool) -> Result<usize> {
    let count = if is_large {
        u64::from(u32::from_le_bytes(
            cursor.take_array::<4>("an element count")?,
        ))
    } else {
        u64::from(cursor.take_u8("an element count")?)
    };
    Ok(usize::try_from(count).expect("a four-byte count fits usize"))
}

/// Read a container's `count + 1` offsets and then its `count` values, each
/// from its own offset within the shared field-bytes region.
fn read_fields(
    cursor: &mut Cursor<'_>,
    count: usize,
    field_offset_size: usize,
    dictionary: &MetadataView<'_>,
    depth: usize,
    budget: &mut usize,
    limits: Limits,
) -> Result<Vec<Value>> {
    let mut offsets = Vec::with_capacity(count + 1);
    let mut offset_positions = Vec::with_capacity(count + 1);
    for _ in 0..=count {
        offset_positions.push(cursor.at());
        let offset = cursor.take_unsigned(field_offset_size, "a field offset")?;
        offsets.push(usize::try_from(offset).unwrap_or(usize::MAX));
    }
    let total = *offsets
        .last()
        .expect("one more offset than fields was read");
    let region_base = cursor.at();
    let region = cursor.take(total, "container field data")?;
    let mut values = Vec::with_capacity(count);
    for (index, offset) in offsets[..count].iter().enumerate() {
        if *offset > region.len() {
            return Err(codec(
                offset_positions[index],
                format_smolstr!("expected a field offset of at most {total}, got {offset}"),
            ));
        }
        let mut field_cursor = Cursor::new(&region[*offset..], region_base + *offset);
        values.push(read_value(
            &mut field_cursor,
            dictionary,
            depth + 1,
            budget,
            limits,
        )?);
    }
    Ok(values)
}

/// Validate UTF-8 string bytes, reporting the first invalid byte's position.
fn utf8(bytes: &[u8], start: usize) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|error| {
        codec(
            start + error.valid_up_to(),
            SmolStr::new_static("expected UTF-8 string bytes, got an invalid sequence"),
        )
    })
}
