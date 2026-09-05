//! Binary encoding, decoding, and skipping of one Avro datum.
//!
//! Decoding is budgeted: every decoded node spends from a per-datum budget and
//! every nesting level checks a depth bound, so a hostile byte sequence - a
//! block-count loop over zero-byte items, a recursive schema driven arbitrarily
//! deep by data - is a typed error, never an allocation the process dies of.

use smol_str::{SmolStr, format_smolstr};
use std::collections::HashMap;

use crate::TimeUnit;
use crate::types::{Nested, Temporal};
use crate::{Error, Limits, Result, Scalar, Timezone};

use super::schema::{Node, RecordType};

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

    /// Read a zig-zag variable-length integer that must fit 32 bits.
    pub(crate) fn int(&mut self) -> Result<i32> {
        let value = self.long()?;
        i32::try_from(value).map_err(|_| {
            codec(
                self.position,
                format_smolstr!("expected an Avro int within 32 bits, got {value}"),
            )
        })
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

    /// Read a length-prefixed UTF-8 string.
    pub(crate) fn string(&mut self) -> Result<&'bytes str> {
        std::str::from_utf8(self.bytes()?).map_err(|error| {
            codec(
                self.position,
                format_smolstr!("expected UTF-8 in an Avro string, got {error}"),
            )
        })
    }

    /// Read four little-endian bytes as a float.
    pub(crate) fn float(&mut self) -> Result<f32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| {
            codec(
                self.position,
                SmolStr::new_static("expected four bytes of an Avro float"),
            )
        })?;
        Ok(f32::from_le_bytes(bytes))
    }

    /// Read eight little-endian bytes as a double.
    pub(crate) fn double(&mut self) -> Result<f64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| {
            codec(
                self.position,
                SmolStr::new_static("expected eight bytes of an Avro double"),
            )
        })?;
        Ok(f64::from_le_bytes(bytes))
    }

    /// Return whether every byte has been consumed.
    pub(crate) const fn is_exhausted(&self) -> bool {
        self.position >= self.bytes.len()
    }
}

/// Read one array or map block count, resolving the byte-sized form.
///
/// A negative count is the size-carrying form: the count's magnitude is
/// followed by the block's encoded byte size, which lets a skipping reader
/// jump the whole block without decoding items.
pub(crate) fn block_count(cursor: &mut Cursor<'_>) -> Result<(u64, Option<usize>)> {
    let count = cursor.long()?;
    if count < 0 {
        let size = cursor.long()?;
        let size = usize::try_from(size).map_err(|_| {
            codec(
                cursor.position,
                format_smolstr!("expected a non-negative block byte size, got {size}"),
            )
        })?;
        return Ok((count.unsigned_abs(), Some(size)));
    }
    Ok((count.unsigned_abs(), None))
}

/// Shared state for decoding, skipping, and encoding datums against a schema.
pub(crate) struct DatumCodec<'schema> {
    /// The schema's named types, for resolving references.
    pub(crate) names: &'schema HashMap<SmolStr, Node>,
    /// The nesting and allocation bounds.
    pub(crate) limits: Limits,
}

impl DatumCodec<'_> {
    /// Spend one node from a decode budget.
    pub(crate) fn spend(&self, budget: &mut usize) -> Result<()> {
        if *budget == 0 {
            return Err(invalid(format_smolstr!(
                "expected an Avro datum of at most {} nodes",
                self.limits.max_nodes()
            )));
        }
        *budget -= 1;
        Ok(())
    }

    /// Check one nesting level against the depth bound.
    pub(crate) fn descend(&self, depth: usize) -> Result<usize> {
        if depth >= self.limits.max_depth() {
            return Err(invalid(format_smolstr!(
                "expected an Avro datum at most {} levels deep",
                self.limits.max_depth()
            )));
        }
        Ok(depth + 1)
    }

    /// Resolve a reference to the named type it points at.
    pub(crate) fn resolve<'n>(&'n self, name: &str) -> Result<&'n Node> {
        self.names.get(name).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a declared Avro type named {name:?}"
            ))
        })
    }

    /// Decode one value against a schema node.
    pub(crate) fn decode<'node>(
        &'node self,
        mut node: &'node Node,
        cursor: &mut Cursor<'_>,
        mut depth: usize,
        budget: &mut usize,
    ) -> Result<Scalar> {
        loop {
            self.spend(budget)?;
            let value = match node {
                Node::Null => Scalar::Null,
                Node::Boolean => {
                    Scalar::from(cursor.take(1)?.first().is_some_and(|byte| *byte != 0))
                }
                Node::Int => Scalar::from(cursor.int()?),
                Node::Long => Scalar::from(cursor.long()?),
                Node::Float => Scalar::from(cursor.float()?),
                Node::Double => Scalar::from(cursor.double()?),
                Node::Bytes => Scalar::from(cursor.bytes()?),
                Node::String | Node::Uuid => Scalar::from(SmolStr::new(cursor.string()?)),
                Node::Date => Scalar::date32(cursor.int()?),
                Node::TimeMillis => {
                    Scalar::time32(cursor.int()?, TimeUnit::Millisecond, Timezone::NAIVE)?
                }
                Node::TimeMicros => {
                    Scalar::time64(cursor.long()?, TimeUnit::Microsecond, Timezone::NAIVE)?
                }
                Node::TimestampMillis => {
                    Scalar::datetime64(cursor.long()?, TimeUnit::Millisecond, Timezone::UTC)?
                }
                Node::TimestampMicros => {
                    Scalar::datetime64(cursor.long()?, TimeUnit::Microsecond, Timezone::UTC)?
                }
                Node::TimestampNanos => {
                    Scalar::datetime64(cursor.long()?, TimeUnit::Nanosecond, Timezone::UTC)?
                }
                Node::LocalTimestampMillis => {
                    Scalar::datetime64(cursor.long()?, TimeUnit::Millisecond, Timezone::NAIVE)?
                }
                Node::LocalTimestampMicros => {
                    Scalar::datetime64(cursor.long()?, TimeUnit::Microsecond, Timezone::NAIVE)?
                }
                Node::LocalTimestampNanos => {
                    Scalar::datetime64(cursor.long()?, TimeUnit::Nanosecond, Timezone::NAIVE)?
                }
                Node::Decimal(decimal) => {
                    let bytes = match &decimal.fixed {
                        Some(fixed) => cursor.take(fixed.size)?,
                        None => cursor.bytes()?,
                    };
                    let unscaled = decimal_from_bytes(bytes).ok_or_else(|| {
                        codec(
                            cursor.position,
                            format_smolstr!(
                                "expected a decimal of at most 38 digits, got {} bytes",
                                bytes.len()
                            ),
                        )
                    })?;
                    Scalar::d128(unscaled, decimal.scale as i8)
                }
                // DESIGN: the value model has no three-part month/day/millisecond
                // interval, so a duration keeps its twelve raw bytes; the Arrow
                // bridge is where they become a typed interval.
                Node::Duration(fixed) | Node::UuidFixed(fixed) | Node::Fixed(fixed) => {
                    Scalar::from(cursor.take(fixed.size)?)
                }
                Node::Enum(symbols) => {
                    let index = cursor.long()?;
                    let symbol = usize::try_from(index)
                        .ok()
                        .and_then(|index| symbols.symbols.get(index))
                        .ok_or_else(|| {
                            codec(
                                cursor.position,
                                format_smolstr!(
                                    "expected an Avro enum index below {}, got {index}",
                                    symbols.symbols.len()
                                ),
                            )
                        })?;
                    Scalar::from(symbol.clone())
                }
                Node::Record(record) => {
                    let depth = self.descend(depth)?;
                    let mut entries = Vec::with_capacity(record.fields.len());
                    for field in &record.fields {
                        entries.push((
                            field.name.clone(),
                            self.decode(&field.schema, cursor, depth, budget)?,
                        ));
                    }
                    Scalar::from_record(entries)?
                }
                Node::Array(items) => {
                    let depth = self.descend(depth)?;
                    let mut values = Vec::new();
                    loop {
                        let (count, _) = block_count(cursor)?;
                        if count == 0 {
                            break;
                        }
                        for _ in 0..count {
                            values.push(self.decode(items, cursor, depth, budget)?);
                        }
                    }
                    Scalar::from_sequence(values)
                }
                Node::Map(values) => {
                    let depth = self.descend(depth)?;
                    let mut entries = Vec::new();
                    loop {
                        let (count, _) = block_count(cursor)?;
                        if count == 0 {
                            break;
                        }
                        for _ in 0..count {
                            self.spend(budget)?;
                            let key = std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                                codec(
                                    cursor.position,
                                    format_smolstr!(
                                        "expected UTF-8 in an Avro map key, got {error}"
                                    ),
                                )
                            })?;
                            entries.push((
                                Scalar::from(key),
                                self.decode(values, cursor, depth, budget)?,
                            ));
                        }
                    }
                    Scalar::from_mapping(entries)?
                }
                Node::Union(branches) => {
                    depth = self.descend(depth)?;
                    let index = cursor.long()?;
                    node = usize::try_from(index)
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
                    continue;
                }
                Node::Ref(name) => {
                    node = self.resolve(name)?;
                    depth = self.descend(depth)?;
                    continue;
                }
            };
            return Ok(value);
        }
    }

    /// Skip one value without decoding it.
    ///
    /// Length-prefixed values jump by their prefix, fixed-width values by
    /// their width, and an array or map block written in the size-carrying
    /// form jumps by its byte size without visiting a single item - which is
    /// what makes a projection cheap.
    pub(crate) fn skip<'node>(
        &'node self,
        mut node: &'node Node,
        cursor: &mut Cursor<'_>,
        mut depth: usize,
        budget: &mut usize,
    ) -> Result<()> {
        loop {
            self.spend(budget)?;
            match node {
                Node::Null => {}
                Node::Boolean => {
                    cursor.take(1)?;
                }
                Node::Int
                | Node::Long
                | Node::Date
                | Node::TimeMillis
                | Node::TimeMicros
                | Node::TimestampMillis
                | Node::TimestampMicros
                | Node::TimestampNanos
                | Node::LocalTimestampMillis
                | Node::LocalTimestampMicros
                | Node::LocalTimestampNanos
                | Node::Enum(_) => {
                    cursor.long()?;
                }
                Node::Float => {
                    cursor.take(4)?;
                }
                Node::Double => {
                    cursor.take(8)?;
                }
                Node::Bytes | Node::String | Node::Uuid => {
                    cursor.bytes()?;
                }
                Node::Decimal(decimal) => match &decimal.fixed {
                    Some(fixed) => {
                        cursor.take(fixed.size)?;
                    }
                    None => {
                        cursor.bytes()?;
                    }
                },
                Node::Duration(fixed) | Node::UuidFixed(fixed) | Node::Fixed(fixed) => {
                    cursor.take(fixed.size)?;
                }
                Node::Record(record) => {
                    let depth = self.descend(depth)?;
                    for field in &record.fields {
                        self.skip(&field.schema, cursor, depth, budget)?;
                    }
                }
                Node::Array(items) => {
                    let depth = self.descend(depth)?;
                    loop {
                        let (count, size) = block_count(cursor)?;
                        if count == 0 {
                            break;
                        }
                        if let Some(size) = size {
                            cursor.take(size)?;
                            continue;
                        }
                        for _ in 0..count {
                            self.skip(items, cursor, depth, budget)?;
                        }
                    }
                }
                Node::Map(values) => {
                    let depth = self.descend(depth)?;
                    loop {
                        let (count, size) = block_count(cursor)?;
                        if count == 0 {
                            break;
                        }
                        if let Some(size) = size {
                            cursor.take(size)?;
                            continue;
                        }
                        for _ in 0..count {
                            self.spend(budget)?;
                            cursor.bytes()?;
                            self.skip(values, cursor, depth, budget)?;
                        }
                    }
                }
                Node::Union(branches) => {
                    depth = self.descend(depth)?;
                    let index = cursor.long()?;
                    node = usize::try_from(index)
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
                    continue;
                }
                Node::Ref(name) => {
                    node = self.resolve(name)?;
                    depth = self.descend(depth)?;
                    continue;
                }
            }
            return Ok(());
        }
    }

    /// Encode one value against a schema node.
    ///
    /// # Errors
    ///
    /// Returns an error when the value does not fit the schema, naming both.
    pub(crate) fn encode<'node>(
        &'node self,
        mut node: &'node Node,
        value: &Scalar,
        target: &mut Vec<u8>,
        mut depth: usize,
    ) -> Result<()> {
        loop {
            match node {
                Node::Null => {
                    if !value.is_null() {
                        return Err(mismatch("null", value));
                    }
                }
                Node::Boolean => target.push(u8::from(
                    value.as_bool().ok_or_else(|| mismatch("boolean", value))?,
                )),
                Node::Int => put_long(target, i64::from(int_value(value, "int")?)),
                Node::Long => put_long(
                    target,
                    value.as_i64().ok_or_else(|| mismatch("long", value))?,
                ),
                Node::Float => {
                    let number = value.as_f64().ok_or_else(|| mismatch("float", value))?;
                    target.extend_from_slice(&(number as f32).to_le_bytes());
                }
                Node::Double => {
                    let number = value.as_f64().ok_or_else(|| mismatch("double", value))?;
                    target.extend_from_slice(&number.to_le_bytes());
                }
                Node::Bytes => put_bytes(
                    target,
                    value.as_bytes().ok_or_else(|| mismatch("bytes", value))?,
                ),
                Node::String | Node::Uuid => put_bytes(
                    target,
                    value
                        .as_str()
                        .ok_or_else(|| mismatch(node.kind(), value))?
                        .as_bytes(),
                ),
                Node::Date => {
                    let days = match value {
                        Scalar::Temporal(Temporal::Date32(date)) => date.count(),
                        other => int_value(other, "date")?,
                    };
                    put_long(target, i64::from(days));
                }
                Node::TimeMillis => {
                    let count =
                        temporal_count(value, "time-millis", TimeUnit::Millisecond, time_parts)?;
                    let count = i32::try_from(count).map_err(|_| {
                        invalid(format_smolstr!(
                            "expected an Avro time-millis within 32 bits, got {count}"
                        ))
                    })?;
                    put_long(target, i64::from(count));
                }
                Node::TimeMicros => put_long(
                    target,
                    temporal_count(value, "time-micros", TimeUnit::Microsecond, time_parts)?,
                ),
                Node::TimestampMillis => put_long(
                    target,
                    temporal_count(
                        value,
                        "timestamp-millis",
                        TimeUnit::Millisecond,
                        instant_parts,
                    )?,
                ),
                Node::TimestampMicros => put_long(
                    target,
                    temporal_count(
                        value,
                        "timestamp-micros",
                        TimeUnit::Microsecond,
                        instant_parts,
                    )?,
                ),
                Node::TimestampNanos => put_long(
                    target,
                    temporal_count(
                        value,
                        "timestamp-nanos",
                        TimeUnit::Nanosecond,
                        instant_parts,
                    )?,
                ),
                Node::LocalTimestampMillis => put_long(
                    target,
                    temporal_count(
                        value,
                        "local-timestamp-millis",
                        TimeUnit::Millisecond,
                        datetime_parts,
                    )?,
                ),
                Node::LocalTimestampMicros => put_long(
                    target,
                    temporal_count(
                        value,
                        "local-timestamp-micros",
                        TimeUnit::Microsecond,
                        datetime_parts,
                    )?,
                ),
                Node::LocalTimestampNanos => put_long(
                    target,
                    temporal_count(
                        value,
                        "local-timestamp-nanos",
                        TimeUnit::Nanosecond,
                        datetime_parts,
                    )?,
                ),
                Node::Decimal(decimal) => {
                    let unscaled = decimal_unscaled(value, decimal.scale)?;
                    let digits = unscaled
                        .unsigned_abs()
                        .checked_ilog10()
                        .map_or(1, |log| log + 1);
                    if digits > decimal.precision {
                        return Err(invalid(format_smolstr!(
                            "expected a decimal of at most {} digits, got {unscaled}",
                            decimal.precision
                        )));
                    }
                    match &decimal.fixed {
                        Some(fixed) => {
                            let bytes =
                                decimal_to_fixed(unscaled, fixed.size).ok_or_else(|| {
                                    invalid(format_smolstr!(
                                        "expected a decimal fitting {} fixed bytes, got {unscaled}",
                                        fixed.size
                                    ))
                                })?;
                            target.extend_from_slice(&bytes);
                        }
                        None => put_bytes(target, &decimal_to_bytes(unscaled)),
                    }
                }
                Node::Duration(fixed) => {
                    let bytes = value
                        .as_bytes()
                        .ok_or_else(|| mismatch("duration", value))?;
                    if bytes.len() != fixed.size {
                        return Err(invalid(format_smolstr!(
                            "expected {} bytes for an Avro duration, got {}",
                            fixed.size,
                            bytes.len()
                        )));
                    }
                    target.extend_from_slice(bytes);
                }
                Node::UuidFixed(fixed) => {
                    if let Some(text) = value.as_str() {
                        let bytes = uuid_bytes(text).ok_or_else(|| {
                            invalid(format_smolstr!(
                                "expected an RFC 4122 uuid string, got {text:?}"
                            ))
                        })?;
                        target.extend_from_slice(&bytes);
                    } else {
                        let bytes = value.as_bytes().ok_or_else(|| mismatch("uuid", value))?;
                        if bytes.len() != fixed.size {
                            return Err(invalid(format_smolstr!(
                                "expected {} bytes for an Avro uuid, got {}",
                                fixed.size,
                                bytes.len()
                            )));
                        }
                        target.extend_from_slice(bytes);
                    }
                }
                Node::Fixed(fixed) => {
                    let bytes = value.as_bytes().ok_or_else(|| mismatch("fixed", value))?;
                    if bytes.len() != fixed.size {
                        return Err(invalid(format_smolstr!(
                            "expected {} bytes for an Avro fixed value, got {}",
                            fixed.size,
                            bytes.len()
                        )));
                    }
                    target.extend_from_slice(bytes);
                }
                Node::Enum(symbols) => {
                    let symbol = value.as_str().ok_or_else(|| mismatch("enum", value))?;
                    let index = symbols
                        .symbols
                        .iter()
                        .position(|candidate| candidate == symbol)
                        .ok_or_else(|| {
                            invalid(format_smolstr!(
                                "expected one of the Avro enum symbols {:?}, got {symbol:?}",
                                symbols.symbols
                            ))
                        })?;
                    put_long(target, index as i64);
                }
                Node::Record(record) => {
                    let depth = self.descend(depth)?;
                    self.encode_record(record, value, target, depth)?;
                }
                Node::Array(items) => {
                    let depth = self.descend(depth)?;
                    let values = value
                        .as_sequence()
                        .ok_or_else(|| mismatch("array", value))?;
                    if !values.is_empty() {
                        put_long(target, values.len() as i64);
                        for item in values {
                            self.encode(items, item, target, depth)?;
                        }
                    }
                    // A zero count closes the last block, so an empty array is one byte.
                    put_long(target, 0);
                }
                Node::Map(values) => {
                    let depth = self.descend(depth)?;
                    match value {
                        Scalar::Nested(Nested::Record(entries)) => {
                            if !entries.as_map().is_empty() {
                                put_long(target, entries.as_map().len() as i64);
                                for (key, item) in entries.as_map() {
                                    put_bytes(target, key.as_bytes());
                                    self.encode(values, item, target, depth)?;
                                }
                            }
                        }
                        Scalar::Nested(Nested::Mapping(entries)) => {
                            if !entries.as_slice().is_empty() {
                                put_long(target, entries.as_slice().len() as i64);
                                for (key, item) in entries.as_slice() {
                                    let key = key
                                        .as_str()
                                        .ok_or_else(|| mismatch("map key string", key))?;
                                    put_bytes(target, key.as_bytes());
                                    self.encode(values, item, target, depth)?;
                                }
                            }
                        }
                        _ => return Err(mismatch("map", value)),
                    }
                    put_long(target, 0);
                }
                Node::Union(branches) => {
                    depth = self.descend(depth)?;
                    let index = self.union_branch(branches, value).ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected a value matching one Avro union branch, got {}",
                            value.kind()
                        ))
                    })?;
                    put_long(target, index as i64);
                    node = &branches[index];
                    continue;
                }
                Node::Ref(name) => {
                    node = self.resolve(name)?;
                    depth = self.descend(depth)?;
                    continue;
                }
            }
            return Ok(());
        }
    }

    /// Encode a record's fields in declaration order.
    fn encode_record(
        &self,
        record: &RecordType,
        value: &Scalar,
        target: &mut Vec<u8>,
        depth: usize,
    ) -> Result<()> {
        if value.as_record().is_none() && value.as_mapping().is_none() {
            return Err(mismatch(&format_smolstr!("record {}", record.name), value));
        }
        for field in &record.fields {
            // A field a caller left out is null, which every optional Iceberg
            // manifest field is; a required one then fails here by name.
            let field_value = value.get_key_str(&field.name).unwrap_or(&Scalar::Null);
            self.encode(&field.schema, field_value, target, depth)
                .map_err(|error| match error {
                    Error::Codec {
                        format,
                        position,
                        reason,
                    } => Error::Codec {
                        format,
                        position,
                        reason: format_smolstr!("{}.{}: {reason}", record.name, field.name),
                    },
                    other => other,
                })?;
        }
        Ok(())
    }

    /// Choose the union branch a value belongs to.
    fn union_branch(&self, branches: &[Node], value: &Scalar) -> Option<usize> {
        branches
            .iter()
            .position(|branch| self.accepts(branch, value))
    }

    /// Return whether a value can encode against a node.
    fn accepts(&self, node: &Node, value: &Scalar) -> bool {
        match node {
            Node::Null => value.is_null(),
            Node::Boolean => value.as_bool().is_some(),
            Node::Int => int_value(value, "int").is_ok(),
            Node::Long => value.as_i64().is_some(),
            Node::Float | Node::Double => value.as_f64().is_some(),
            Node::Bytes => value.as_bytes().is_some(),
            Node::String | Node::Uuid | Node::Enum(_) => value.as_str().is_some(),
            Node::Date => {
                matches!(value, Scalar::Temporal(Temporal::Date32(_))) || value.as_i64().is_some()
            }
            Node::TimeMillis | Node::TimeMicros => {
                time_parts(value).is_some() || value.as_i64().is_some()
            }
            Node::TimestampMillis | Node::TimestampMicros | Node::TimestampNanos => {
                instant_parts(value).is_some() || value.as_i64().is_some()
            }
            Node::LocalTimestampMillis | Node::LocalTimestampMicros | Node::LocalTimestampNanos => {
                datetime_parts(value).is_some() || value.as_i64().is_some()
            }
            Node::Decimal(_) => {
                value.is_decimal() || value.as_i64().is_some() || value.as_bytes().is_some()
            }
            Node::Duration(fixed) | Node::Fixed(fixed) => value
                .as_bytes()
                .is_some_and(|bytes| bytes.len() == fixed.size),
            Node::UuidFixed(fixed) => {
                value
                    .as_bytes()
                    .is_some_and(|bytes| bytes.len() == fixed.size)
                    || value
                        .as_str()
                        .is_some_and(|text| uuid_bytes(text).is_some())
            }
            Node::Record(_) => value.as_record().is_some() || value.as_mapping().is_some(),
            Node::Map(_) => value.as_record().is_some() || value.as_mapping().is_some(),
            Node::Array(_) => value.as_sequence().is_some(),
            Node::Union(_) => false,
            Node::Ref(name) => self
                .names
                .get(name.as_str())
                .is_some_and(|target| self.accepts(target, value)),
        }
    }

    /// Return the decode budget one datum starts with.
    pub(crate) fn budget(&self) -> usize {
        self.limits.max_nodes()
    }
}

/// Read an integer value that must fit 32 bits.
fn int_value(value: &Scalar, expected: &str) -> Result<i32> {
    let wide = value.as_i64().ok_or_else(|| mismatch(expected, value))?;
    i32::try_from(wide).map_err(|_| {
        invalid(format_smolstr!(
            "expected an Avro {expected} within 32 bits, got {wide}"
        ))
    })
}

/// Split a time-of-day value into its count and unit.
fn time_parts(value: &Scalar) -> Option<(i64, TimeUnit)> {
    match value {
        Scalar::Temporal(Temporal::Time32(time)) => Some((i64::from(time.count()), time.unit())),
        Scalar::Temporal(Temporal::Time64(time)) => Some((time.count(), time.unit())),
        _ => None,
    }
}

/// Split an instant value into its count and unit.
fn instant_parts(value: &Scalar) -> Option<(i64, TimeUnit)> {
    match value {
        Scalar::Temporal(Temporal::DateTime64(datetime)) if !datetime.timezone().is_naive() => {
            Some((datetime.count(), datetime.unit()))
        }
        _ => None,
    }
}

/// Split a naive wall-clock value into its count and unit.
fn datetime_parts(value: &Scalar) -> Option<(i64, TimeUnit)> {
    match value {
        Scalar::Temporal(Temporal::DateTime64(datetime)) if datetime.timezone().is_naive() => {
            Some((datetime.count(), datetime.unit()))
        }
        _ => None,
    }
}

/// Read a temporal count in the schema's unit, converting only losslessly.
fn temporal_count(
    value: &Scalar,
    expected: &str,
    unit: TimeUnit,
    parts: fn(&Scalar) -> Option<(i64, TimeUnit)>,
) -> Result<i64> {
    if let Some((count, from)) = parts(value) {
        return convert_count(count, from, unit).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro {expected} count convertible from {from} to {unit}, got {count}"
            ))
        });
    }
    value.as_i64().ok_or_else(|| mismatch(expected, value))
}

/// Convert a temporal count between units without losing precision.
fn convert_count(count: i64, from: TimeUnit, to: TimeUnit) -> Option<i64> {
    let rank = |unit: TimeUnit| match unit {
        TimeUnit::Second => Some(0_u32),
        TimeUnit::Millisecond => Some(3),
        TimeUnit::Microsecond => Some(6),
        TimeUnit::Nanosecond => Some(9),
        _ => None,
    };
    let from = rank(from)?;
    let to = rank(to)?;
    if from == to {
        return Some(count);
    }
    if from < to {
        return count.checked_mul(10_i64.checked_pow(to - from)?);
    }
    let divisor = 10_i64.checked_pow(from - to)?;
    (count % divisor == 0).then(|| count / divisor)
}

/// Read a decimal's unscaled integer at the schema's scale.
fn decimal_unscaled(value: &Scalar, scale: u32) -> Result<i128> {
    match value {
        Scalar::Decimal(_) => value.decimal_unscaled_at(scale as i8).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a decimal exactly representable at scale {scale}"
            ))
        }),
        other => {
            if let Some(bytes) = other.as_bytes() {
                return decimal_from_bytes(bytes).ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a decimal of at most 38 digits, got {} bytes",
                        bytes.len()
                    ))
                });
            }
            let whole = other.as_i64().ok_or_else(|| mismatch("decimal", other))?;
            i128::from(whole)
                .checked_mul(10_i128.checked_pow(scale).ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a decimal scale below 39, got {scale}"
                    ))
                })?)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a decimal fitting 38 digits, got {whole} at scale {scale}"
                    ))
                })
        }
    }
}

/// Read a big-endian two's-complement unscaled integer.
pub(crate) fn decimal_from_bytes(bytes: &[u8]) -> Option<i128> {
    if bytes.is_empty() {
        return Some(0);
    }
    let negative = bytes[0] & 0x80 != 0;
    let filler = if negative { 0xFF } else { 0x00 };
    let mut buffer = [filler; 16];
    if bytes.len() > 16 {
        // Longer runs are legal when the leading bytes are pure sign filler.
        let (extra, tail) = bytes.split_at(bytes.len() - 16);
        if extra.iter().any(|byte| *byte != filler) || (tail[0] & 0x80 != 0) != negative {
            return None;
        }
        buffer.copy_from_slice(tail);
    } else {
        buffer[16 - bytes.len()..].copy_from_slice(bytes);
    }
    Some(i128::from_be_bytes(buffer))
}

/// Render an unscaled integer as its minimal big-endian two's complement.
pub(crate) fn decimal_to_bytes(unscaled: i128) -> Vec<u8> {
    let bytes = unscaled.to_be_bytes();
    let filler = if unscaled < 0 { 0xFF } else { 0x00 };
    let mut start = 0;
    while start < 15 && bytes[start] == filler && (bytes[start + 1] & 0x80 != 0) == (unscaled < 0) {
        start += 1;
    }
    bytes[start..].to_vec()
}

/// Render an unscaled integer sign-extended to a fixed size.
pub(crate) fn decimal_to_fixed(unscaled: i128, size: usize) -> Option<Vec<u8>> {
    let minimal = decimal_to_bytes(unscaled);
    if minimal.len() > size {
        return None;
    }
    let filler = if unscaled < 0 { 0xFF } else { 0x00 };
    let mut bytes = vec![filler; size];
    bytes[size - minimal.len()..].copy_from_slice(&minimal);
    Some(bytes)
}

/// Parse the canonical 8-4-4-4-12 uuid text into its sixteen bytes.
fn uuid_bytes(text: &str) -> Option<[u8; 16]> {
    let mut bytes = [0_u8; 16];
    let mut characters = text.chars();
    let mut index = 0;
    let groups = [8, 4, 4, 4, 12];
    for (position, group) in groups.iter().enumerate() {
        if position > 0 && characters.next() != Some('-') {
            return None;
        }
        for _ in 0..group / 2 {
            let high = characters.next()?.to_digit(16)?;
            let low = characters.next()?.to_digit(16)?;
            bytes[index] = ((high << 4) | low) as u8;
            index += 1;
        }
    }
    characters.next().is_none().then_some(bytes)
}

/// Report a value that does not fit the schema it is being written against.
fn mismatch(expected: &str, value: &Scalar) -> Error {
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
