//! Binary encoding and decoding of one Avro datum against a schema.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Value};

use super::schema::{Record, Schema};

/// Append a zig-zag variable-length integer.
pub(crate) fn put_long(target: &mut Vec<u8>, value: i64) {
    // Zig-zag keeps small negatives short, which is what Avro encodes with.
    let mut encoded = ((value << 1) ^ (value >> 63)) as u64;
    loop {
        let byte = u8::try_from(encoded & 0x7f).unwrap_or_default();
        encoded >>= 7;
        if encoded == 0 {
            target.push(byte);
            return;
        }
        target.push(byte | 0x80);
    }
}

/// Append a length-prefixed byte run.
pub(crate) fn put_bytes(target: &mut Vec<u8>, bytes: &[u8]) {
    put_long(target, bytes.len() as i64);
    target.extend_from_slice(bytes);
}

/// A borrowed position inside an encoded container.
pub(crate) struct Cursor<'bytes> {
    /// The bytes being decoded.
    bytes: &'bytes [u8],
    /// The next byte to read.
    pub(crate) position: usize,
}

impl<'bytes> Cursor<'bytes> {
    /// Start at the beginning of `bytes`.
    pub(crate) const fn new(bytes: &'bytes [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Take exactly `count` bytes.
    pub(crate) fn take(&mut self, count: usize) -> Result<&'bytes [u8]> {
        let end = self.position.checked_add(count).ok_or_else(|| {
            truncated(
                self.position,
                format_smolstr!("{count} bytes"),
                "an overflow",
            )
        })?;
        if end > self.bytes.len() {
            return Err(truncated(
                self.position,
                format_smolstr!("{count} bytes"),
                &format_smolstr!("{} bytes", self.bytes.len() - self.position),
            ));
        }
        let taken = &self.bytes[self.position..end];
        self.position = end;
        Ok(taken)
    }

    /// Read a zig-zag variable-length integer.
    pub(crate) fn long(&mut self) -> Result<i64> {
        let mut shift = 0_u32;
        let mut accumulated = 0_u64;
        loop {
            let byte = *self.take(1)?.first().unwrap_or(&0);
            if shift > 63 {
                return Err(codec(
                    self.position,
                    SmolStr::new_static("expected a variable-length integer of at most 10 bytes"),
                ));
            }
            accumulated |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(((accumulated >> 1) as i64) ^ -((accumulated & 1) as i64))
    }

    /// Read a length-prefixed byte run.
    pub(crate) fn bytes(&mut self) -> Result<&'bytes [u8]> {
        let length = self.long()?;
        let length = usize::try_from(length).map_err(|_| {
            codec(
                self.position,
                format_smolstr!("expected a non-negative byte length, got {length}"),
            )
        })?;
        self.take(length)
    }

    /// Return whether every byte has been consumed.
    pub(crate) const fn is_exhausted(&self) -> bool {
        self.position >= self.bytes.len()
    }
}

/// Encode one value against a schema.
///
/// # Errors
///
/// Returns an error when the value does not fit the schema, naming both.
pub(crate) fn encode(schema: &Schema, value: &Value, target: &mut Vec<u8>) -> Result<()> {
    match schema {
        Schema::Null => {
            if !value.is_null() {
                return Err(mismatch("null", value));
            }
        }
        Schema::Boolean => target.push(u8::from(
            value.as_bool().ok_or_else(|| mismatch("boolean", value))?,
        )),
        Schema::Int | Schema::Long => {
            put_long(
                target,
                value
                    .as_i64()
                    .ok_or_else(|| mismatch(schema.kind(), value))?,
            );
        }
        Schema::Float => {
            let number = value.as_f64().ok_or_else(|| mismatch("float", value))?;
            target.extend_from_slice(&(number as f32).to_le_bytes());
        }
        Schema::Double => {
            let number = value.as_f64().ok_or_else(|| mismatch("double", value))?;
            target.extend_from_slice(&number.to_le_bytes());
        }
        Schema::Bytes => put_bytes(
            target,
            value.as_bytes().ok_or_else(|| mismatch("bytes", value))?,
        ),
        Schema::String => put_bytes(
            target,
            value
                .as_str()
                .ok_or_else(|| mismatch("string", value))?
                .as_bytes(),
        ),
        Schema::Fixed(size) => {
            let bytes = value.as_bytes().ok_or_else(|| mismatch("fixed", value))?;
            if bytes.len() != *size {
                return Err(invalid(format_smolstr!(
                    "expected {size} bytes for an Avro fixed value, got {}",
                    bytes.len()
                )));
            }
            target.extend_from_slice(bytes);
        }
        Schema::Enum(symbols) => {
            let symbol = value.as_str().ok_or_else(|| mismatch("enum", value))?;
            let index = symbols
                .iter()
                .position(|candidate| candidate == symbol)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected one of the Avro enum symbols {symbols:?}, got {symbol:?}"
                    ))
                })?;
            put_long(target, index as i64);
        }
        Schema::Record(record) => encode_record(record, value, target)?,
        Schema::Array(items) => {
            let values = value
                .as_sequence()
                .ok_or_else(|| mismatch("array", value))?;
            if !values.is_empty() {
                put_long(target, values.len() as i64);
                for item in values {
                    encode(items, item, target)?;
                }
            }
            // A zero count closes the last block, so an empty array is one byte.
            put_long(target, 0);
        }
        Schema::Map(values) => {
            let entries = value.as_mapping().ok_or_else(|| mismatch("map", value))?;
            if !entries.is_empty() {
                put_long(target, entries.len() as i64);
                for (key, item) in entries {
                    let key = key
                        .as_str()
                        .ok_or_else(|| mismatch("map key string", key))?;
                    put_bytes(target, key.as_bytes());
                    encode(values, item, target)?;
                }
            }
            put_long(target, 0);
        }
        Schema::Union(branches) => {
            let index = union_branch(branches, value).ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a value matching one Avro union branch, got {}",
                    value.kind()
                ))
            })?;
            put_long(target, index as i64);
            encode(&branches[index], value, target)?;
        }
    }
    Ok(())
}

/// Encode a record's fields in declaration order.
fn encode_record(record: &Record, value: &Value, target: &mut Vec<u8>) -> Result<()> {
    if value.as_mapping().is_none() {
        return Err(mismatch(&format_smolstr!("record {}", record.name), value));
    }
    for (name, schema) in &record.fields {
        // A field a caller left out is null, which every optional Iceberg
        // manifest field is; a required one then fails here by name.
        let field = value.get_key_str(name).unwrap_or(&Value::Null);
        encode(schema, field, target).map_err(|error| match error {
            Error::Codec {
                format,
                position,
                reason,
            } => Error::Codec {
                format,
                position,
                reason: format_smolstr!("{}.{name}: {reason}", record.name),
            },
            other => other,
        })?;
    }
    Ok(())
}

/// Choose the union branch a value belongs to.
fn union_branch(branches: &[Schema], value: &Value) -> Option<usize> {
    branches.iter().position(|branch| match branch {
        Schema::Null => value.is_null(),
        Schema::Boolean => value.as_bool().is_some(),
        Schema::Int | Schema::Long => value.as_i64().is_some(),
        Schema::Float | Schema::Double => value.as_f64().is_some(),
        Schema::Bytes | Schema::Fixed(_) => value.as_bytes().is_some(),
        Schema::String | Schema::Enum(_) => value.as_str().is_some(),
        Schema::Record(_) | Schema::Map(_) => value.as_mapping().is_some(),
        Schema::Array(_) => value.as_sequence().is_some(),
        Schema::Union(_) => false,
    })
}

/// Decode one value against a schema.
pub(crate) fn decode(schema: &Schema, cursor: &mut Cursor<'_>) -> Result<Value> {
    Ok(match schema {
        Schema::Null => Value::Null,
        Schema::Boolean => Value::Bool(cursor.take(1)?.first().is_some_and(|byte| *byte != 0)),
        Schema::Int | Schema::Long => Value::I64(cursor.long()?),
        Schema::Float => {
            let bytes: [u8; 4] = cursor.take(4)?.try_into().map_err(|_| {
                codec(
                    cursor.position,
                    SmolStr::new_static("expected four bytes of an Avro float"),
                )
            })?;
            Value::from(f64::from(f32::from_le_bytes(bytes)))
        }
        Schema::Double => {
            let bytes: [u8; 8] = cursor.take(8)?.try_into().map_err(|_| {
                codec(
                    cursor.position,
                    SmolStr::new_static("expected eight bytes of an Avro double"),
                )
            })?;
            Value::from(f64::from_le_bytes(bytes))
        }
        Schema::Bytes => Value::Bytes(Arc::from(cursor.bytes()?)),
        Schema::String => Value::String(SmolStr::new(
            std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                codec(
                    cursor.position,
                    format_smolstr!("expected UTF-8 in an Avro string, got {error}"),
                )
            })?,
        )),
        Schema::Fixed(size) => Value::Bytes(Arc::from(cursor.take(*size)?)),
        Schema::Enum(symbols) => {
            let index = cursor.long()?;
            let symbol = usize::try_from(index)
                .ok()
                .and_then(|index| symbols.get(index))
                .ok_or_else(|| {
                    codec(
                        cursor.position,
                        format_smolstr!(
                            "expected an Avro enum index below {}, got {index}",
                            symbols.len()
                        ),
                    )
                })?;
            Value::String(symbol.clone())
        }
        Schema::Record(record) => {
            let mut entries = Vec::with_capacity(record.fields.len());
            for (name, field) in &record.fields {
                entries.push((Value::from(name.clone()), decode(field, cursor)?));
            }
            Value::from_mapping(entries)?
        }
        Schema::Array(items) => {
            let mut values = Vec::new();
            loop {
                let count = block_count(cursor)?;
                if count == 0 {
                    break;
                }
                for _ in 0..count {
                    values.push(decode(items, cursor)?);
                }
            }
            Value::from_sequence(values)
        }
        Schema::Map(values) => {
            let mut entries = Vec::new();
            loop {
                let count = block_count(cursor)?;
                if count == 0 {
                    break;
                }
                for _ in 0..count {
                    let key = std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                        codec(
                            cursor.position,
                            format_smolstr!("expected UTF-8 in an Avro map key, got {error}"),
                        )
                    })?;
                    entries.push((Value::from(key), decode(values, cursor)?));
                }
            }
            Value::from_mapping(entries)?
        }
        Schema::Union(branches) => {
            let index = cursor.long()?;
            let branch = usize::try_from(index)
                .ok()
                .and_then(|index| branches.get(index))
                .ok_or_else(|| {
                    codec(
                        cursor.position,
                        format_smolstr!(
                            "expected an Avro union branch below {}, got {index}",
                            branches.len()
                        ),
                    )
                })?;
            decode(branch, cursor)?
        }
    })
}

/// Read one array or map block count, resolving the byte-sized form.
pub(crate) fn block_count(cursor: &mut Cursor<'_>) -> Result<u64> {
    let count = cursor.long()?;
    if count < 0 {
        // A negative count is followed by the block's byte size, which a
        // reader that decodes every item does not need.
        cursor.long()?;
        return Ok(count.unsigned_abs());
    }
    Ok(count.unsigned_abs())
}

/// Report a value that does not fit the schema it is being written against.
fn mismatch(expected: &str, value: &Value) -> Error {
    invalid(format_smolstr!(
        "expected an Avro {expected} value, got {}",
        value.kind()
    ))
}

/// Report a container that ends before the value it promised.
fn truncated(position: usize, expected: SmolStr, actual: &str) -> Error {
    codec(
        position,
        format_smolstr!("expected {expected}, got {actual}"),
    )
}

/// Report a malformed Avro document at a byte position.
pub(crate) fn codec(position: usize, reason: SmolStr) -> Error {
    Error::Codec {
        format: "avro",
        position,
        reason,
    }
}

/// Report a malformed Avro document whose position is the document itself.
pub(crate) fn invalid(reason: SmolStr) -> Error {
    codec(0, reason)
}
