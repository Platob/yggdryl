//! The Avro object container Iceberg keeps its manifests in.
//!
//! Iceberg writes table metadata as JSON but manifest lists and manifests as
//! Avro, so reaching a data file means decoding Avro. That is one small binary
//! format - a header naming a writer schema, then blocks of records separated
//! by a synchronization marker - and implementing it here is what keeps the
//! module's promise that no dependency is added for the table format itself.
//!
//! Rows cross this boundary as the same [`Value`] the JSON parser produces, so
//! a manifest row and a metadata document are read with one vocabulary: a
//! record is a mapping, an array is a sequence, and a union carries the branch
//! value directly rather than a wrapper, because that is how Iceberg's optional
//! fields are meant to read.
//!
//! A container is read whole. A manifest describes files, not rows, so it is
//! small by construction; the streaming that matters happens one level up, in
//! the data files themselves.

use std::collections::HashMap;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::io::IOBase;
use crate::{Codec, Error, Level, Result, Value};

/// The four bytes that open every Avro object container.
const MAGIC: [u8; 4] = [b'O', b'b', b'j', 1];

/// The header key naming the writer schema.
const SCHEMA_KEY: &str = "avro.schema";

/// The header key naming the block compression codec.
const CODEC_KEY: &str = "avro.codec";

/// Length of the synchronization marker that closes every block.
const SYNC_LEN: usize = 16;

/// One Avro schema node.
///
/// Only the branches Iceberg's manifest schemas actually use are modeled. A
/// logical type annotation is deliberately not modeled: it never changes the
/// physical encoding, and the manifest layer above knows what a field means.
#[derive(Clone, Debug)]
pub(super) enum Schema {
    /// The empty type, encoded as zero bytes.
    Null,
    /// A single byte holding zero or one.
    Boolean,
    /// A zig-zag variable-length 32-bit integer.
    Int,
    /// A zig-zag variable-length 64-bit integer.
    Long,
    /// Four little-endian IEEE 754 bytes.
    Float,
    /// Eight little-endian IEEE 754 bytes.
    Double,
    /// A length-prefixed byte string.
    Bytes,
    /// A length-prefixed UTF-8 string.
    String,
    /// Named ordered fields, encoded back to back.
    Record(Arc<Record>),
    /// A symbol chosen by index.
    Enum(Arc<[SmolStr]>),
    /// Length-prefixed blocks of one item type.
    Array(Arc<Schema>),
    /// Length-prefixed blocks of string-keyed values.
    Map(Arc<Schema>),
    /// A branch index followed by that branch's value.
    Union(Arc<[Schema]>),
    /// Exactly `size` raw bytes.
    Fixed(usize),
}

/// The fields of one Avro record type, in declaration order.
#[derive(Clone, Debug)]
pub(super) struct Record {
    /// The record's declared name, which later references resolve against.
    pub(super) name: SmolStr,
    /// Field names paired with their schemas, in encoding order.
    pub(super) fields: Vec<(SmolStr, Schema)>,
}

impl Schema {
    /// Read an Avro schema from its JSON representation.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a schema this implementation
    /// covers, naming the construct that was found.
    pub(super) fn from_json(document: &Value) -> Result<Self> {
        let mut named = HashMap::new();
        parse_schema(document, &mut named)
    }

    /// Return the name a caller would use to refer to this schema.
    fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Int => "int",
            Self::Long => "long",
            Self::Float => "float",
            Self::Double => "double",
            Self::Bytes => "bytes",
            Self::String => "string",
            Self::Record(_) => "record",
            Self::Enum(_) => "enum",
            Self::Array(_) => "array",
            Self::Map(_) => "map",
            Self::Union(_) => "union",
            Self::Fixed(_) => "fixed",
        }
    }
}

/// Parse one schema node, registering any named type it declares.
fn parse_schema(document: &Value, named: &mut HashMap<SmolStr, Schema>) -> Result<Schema> {
    if let Some(name) = document.as_str() {
        return resolve_name(name, named);
    }
    if let Some(branches) = document.as_sequence() {
        let mut parsed = Vec::with_capacity(branches.len());
        for branch in branches {
            parsed.push(parse_schema(branch, named)?);
        }
        return Ok(Schema::Union(parsed.into()));
    }
    if document.as_mapping().is_none() {
        return Err(invalid(format_smolstr!(
            "expected an Avro schema name, union, or object, got {}",
            document.kind()
        )));
    }

    let type_name = document
        .get_key_str("type")
        .ok_or_else(|| invalid(SmolStr::new_static("expected an Avro schema \"type\"")))?;
    // A `type` may itself be a nested schema, which is how Iceberg spells a
    // logical annotation over an array or a union.
    let Some(type_name) = type_name.as_str() else {
        return parse_schema(type_name, named);
    };

    match type_name {
        "record" | "error" => parse_record(document, named),
        "enum" => {
            let symbols = document
                .get_key_str("symbols")
                .and_then(Value::as_sequence)
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected an Avro enum \"symbols\" array",
                    ))
                })?;
            let mut names = Vec::with_capacity(symbols.len());
            for symbol in symbols {
                names.push(SmolStr::new(symbol.as_str().ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected an Avro enum symbol string, got {}",
                        symbol.kind()
                    ))
                })?));
            }
            let schema = Schema::Enum(names.into());
            register(document, &schema, named);
            Ok(schema)
        }
        "array" => {
            let items = document.get_key_str("items").ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected an Avro array \"items\" schema",
                ))
            })?;
            Ok(Schema::Array(Arc::new(parse_schema(items, named)?)))
        }
        "map" => {
            let values = document.get_key_str("values").ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected an Avro map \"values\" schema",
                ))
            })?;
            Ok(Schema::Map(Arc::new(parse_schema(values, named)?)))
        }
        "fixed" => {
            let size = document
                .get_key_str("size")
                .and_then(Value::as_i64)
                .and_then(|size| usize::try_from(size).ok())
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a non-negative Avro fixed \"size\"",
                    ))
                })?;
            let schema = Schema::Fixed(size);
            register(document, &schema, named);
            Ok(schema)
        }
        other => resolve_name(other, named),
    }
}

/// Parse a record schema and register it before its own fields are read.
fn parse_record(document: &Value, named: &mut HashMap<SmolStr, Schema>) -> Result<Schema> {
    let name = document
        .get_key_str("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(SmolStr::new_static("expected an Avro record \"name\"")))?;
    let entries = document
        .get_key_str("fields")
        .and_then(Value::as_sequence)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro record \"fields\" array on {name:?}"
            ))
        })?;

    let mut fields = Vec::with_capacity(entries.len());
    for entry in entries {
        let field_name = entry
            .get_key_str("name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an Avro field \"name\" inside {name:?}"
                ))
            })?;
        let field_type = entry.get_key_str("type").ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro field \"type\" on {name:?}.{field_name}"
            ))
        })?;
        fields.push((SmolStr::new(field_name), parse_schema(field_type, named)?));
    }

    let schema = Schema::Record(Arc::new(Record {
        name: SmolStr::new(name),
        fields,
    }));
    named.insert(SmolStr::new(name), schema.clone());
    Ok(schema)
}

/// Remember a named schema so a later reference by name resolves.
fn register(document: &Value, schema: &Schema, named: &mut HashMap<SmolStr, Schema>) {
    if let Some(name) = document.get_key_str("name").and_then(Value::as_str) {
        named.insert(SmolStr::new(name), schema.clone());
    }
}

/// Resolve a bare schema name: a primitive, or a type declared earlier.
fn resolve_name(name: &str, named: &HashMap<SmolStr, Schema>) -> Result<Schema> {
    Ok(match name {
        "null" => Schema::Null,
        "boolean" => Schema::Boolean,
        "int" => Schema::Int,
        "long" => Schema::Long,
        "float" => Schema::Float,
        "double" => Schema::Double,
        "bytes" => Schema::Bytes,
        "string" => Schema::String,
        other => named.get(other).cloned().ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro primitive name or a type declared earlier, got {other:?}"
            ))
        })?,
    })
}

/// Append a zig-zag variable-length integer.
fn put_long(target: &mut Vec<u8>, value: i64) {
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
fn put_bytes(target: &mut Vec<u8>, bytes: &[u8]) {
    put_long(target, bytes.len() as i64);
    target.extend_from_slice(bytes);
}

/// A borrowed position inside an encoded container.
struct Cursor<'bytes> {
    /// The bytes being decoded.
    bytes: &'bytes [u8],
    /// The next byte to read.
    position: usize,
}

impl<'bytes> Cursor<'bytes> {
    /// Start at the beginning of `bytes`.
    const fn new(bytes: &'bytes [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Take exactly `count` bytes.
    fn take(&mut self, count: usize) -> Result<&'bytes [u8]> {
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
    fn long(&mut self) -> Result<i64> {
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
    fn bytes(&mut self) -> Result<&'bytes [u8]> {
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
    const fn is_exhausted(&self) -> bool {
        self.position >= self.bytes.len()
    }
}

/// Encode one value against a schema.
///
/// # Errors
///
/// Returns an error when the value does not fit the schema, naming both.
pub(super) fn encode(schema: &Schema, value: &Value, target: &mut Vec<u8>) -> Result<()> {
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
fn decode(schema: &Schema, cursor: &mut Cursor<'_>) -> Result<Value> {
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
fn block_count(cursor: &mut Cursor<'_>) -> Result<u64> {
    let count = cursor.long()?;
    if count < 0 {
        // A negative count is followed by the block's byte size, which a
        // reader that decodes every item does not need.
        cursor.long()?;
        return Ok(count.unsigned_abs());
    }
    Ok(count.unsigned_abs())
}

/// One decoded Avro object container.
#[derive(Debug)]
pub(super) struct Container {
    /// The header's key/value metadata, minus the reserved schema and codec.
    pub(super) metadata: Vec<(SmolStr, SmolStr)>,
    /// Every decoded row, in file order.
    pub(super) rows: Vec<Value>,
}

impl Container {
    /// Return one metadata value by key.
    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }
}

/// Read every row of the Avro object container a handle holds.
///
/// A manifest describes files rather than rows, so the whole container is read
/// at once; the streaming that matters is one level up, over the data files a
/// manifest points at.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro object container, when the
/// codec is one this build does not implement, or when a row does not decode.
pub(super) fn read_container<H: IOBase + ?Sized>(handle: &H) -> Result<Container> {
    let bytes = handle.read_all()?;
    let mut cursor = Cursor::new(&bytes);

    let magic = cursor.take(MAGIC.len())?;
    if magic != MAGIC {
        return Err(invalid(format_smolstr!(
            "expected an Avro object container starting with {MAGIC:?}, got {magic:?}"
        )));
    }

    let mut header = Vec::new();
    loop {
        let count = block_count(&mut cursor)?;
        if count == 0 {
            break;
        }
        for _ in 0..count {
            let key = SmolStr::new(std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                codec(
                    cursor.position,
                    format_smolstr!("expected UTF-8 in an Avro header key, got {error}"),
                )
            })?);
            let value = cursor.bytes()?.to_vec();
            header.push((key, value));
        }
    }
    let sync: [u8; SYNC_LEN] = cursor.take(SYNC_LEN)?.try_into().map_err(|_| {
        codec(
            cursor.position,
            SmolStr::new_static("expected a sixteen-byte Avro synchronization marker"),
        )
    })?;

    let lookup = |key: &str| -> Option<&[u8]> {
        header
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value.as_slice()))
    };
    let schema_bytes = lookup(SCHEMA_KEY).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Avro header carrying {SCHEMA_KEY:?}"
        ))
    })?;
    let schema_json = crate::json::from_slice(schema_bytes)?;
    let schema = Schema::from_json(&schema_json)?;
    let codec_name = lookup(CODEC_KEY)
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_else(|| "null".to_owned());
    let block_codec = block_codec(&codec_name)?;

    let mut rows = Vec::new();
    while !cursor.is_exhausted() {
        let count = cursor.long()?;
        let count = u64::try_from(count).map_err(|_| {
            codec(
                cursor.position,
                format_smolstr!("expected a non-negative Avro block count, got {count}"),
            )
        })?;
        let payload = cursor.bytes()?;
        let marker = cursor.take(SYNC_LEN)?;
        if marker != sync {
            return Err(codec(
                cursor.position,
                SmolStr::new_static(
                    "expected the header's synchronization marker after an Avro block",
                ),
            ));
        }
        let decoded = block_codec.load(payload)?;
        let mut block = Cursor::new(&decoded);
        for _ in 0..count {
            rows.push(decode(&schema, &mut block)?);
        }
    }

    let metadata = header
        .into_iter()
        .filter(|(key, _)| key != SCHEMA_KEY && key != CODEC_KEY)
        .map(|(key, value)| (key, SmolStr::new(String::from_utf8_lossy(&value))))
        .collect();

    Ok(Container { metadata, rows })
}

/// Replace a handle's bytes with an Avro object container holding `rows`.
///
/// Every row is written as one block, compressed with raw deflate, which is
/// what the `deflate` codec name means and what the reference implementations
/// write by default.
///
/// # Errors
///
/// Returns an error when the schema JSON is not a schema, when a row does not
/// fit it, or when the write fails.
pub(super) fn write_container<H: IOBase + ?Sized>(
    handle: &mut H,
    schema_json: &Value,
    metadata: &[(&str, String)],
    rows: &[Value],
) -> Result<()> {
    let schema = Schema::from_json(schema_json)?;
    let encoded_schema = crate::json::to_vec(schema_json)?;
    let sync = sync_marker();

    let mut payload = Vec::new();
    for row in rows {
        encode(&schema, row, &mut payload)?;
    }
    let compressed = Codec::Deflate.dump_with_level(&payload, Level::DEFAULT)?;

    let mut output = Vec::with_capacity(compressed.len() + 512);
    output.extend_from_slice(&MAGIC);
    put_long(&mut output, metadata.len() as i64 + 2);
    put_bytes(&mut output, SCHEMA_KEY.as_bytes());
    put_bytes(&mut output, &encoded_schema);
    put_bytes(&mut output, CODEC_KEY.as_bytes());
    put_bytes(&mut output, b"deflate");
    for (key, value) in metadata {
        put_bytes(&mut output, key.as_bytes());
        put_bytes(&mut output, value.as_bytes());
    }
    put_long(&mut output, 0);
    output.extend_from_slice(&sync);

    if !rows.is_empty() {
        put_long(&mut output, rows.len() as i64);
        put_bytes(&mut output, &compressed);
        output.extend_from_slice(&sync);
    }

    handle.write_all_bytes(&output)
}

/// Return the content coding one Avro codec name selects.
fn block_codec(name: &str) -> Result<Codec> {
    match name {
        "null" => Ok(Codec::Identity),
        // Avro's "deflate" is the raw stream, with no zlib wrapper.
        "deflate" => Ok(Codec::Deflate),
        "zstandard" => Ok(Codec::Zstd),
        other => Err(invalid(format_smolstr!(
            "expected an Avro block codec this build implements (null, deflate, zstandard), got \
             {other:?}"
        ))),
    }
}

/// Produce a synchronization marker unlikely to occur inside a block.
///
/// The marker only has to be constant within one file and improbable in its
/// data, so hashing process-seeded state is enough and avoids a dependency
/// whose only job would be sixteen bytes.
fn sync_marker() -> [u8; SYNC_LEN] {
    use std::hash::{BuildHasher, Hasher};

    let state = std::collections::hash_map::RandomState::new();
    let mut marker = [0_u8; SYNC_LEN];
    for (half, chunk) in marker.chunks_mut(8).enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_usize(half);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
    }
    marker
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
fn codec(position: usize, reason: SmolStr) -> Error {
    Error::Codec {
        format: "avro",
        position,
        reason,
    }
}

/// Report a malformed Avro document whose position is the document itself.
fn invalid(reason: SmolStr) -> Error {
    codec(0, reason)
}
