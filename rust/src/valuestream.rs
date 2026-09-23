//! One byte stream for any value: the stream's version, the value's
//! [`DataTypeId`] and the payload that identifier says how to read, a
//! nested value's children inside it, and a long payload compressed.
//!
//! This is the crate's own encoding, and what it keeps is the leaf: a
//! value read back is the value that was written, width, unit, zone and
//! charset included. [`crate::Variant`] is the other byte form of a value
//! and the one media write - the Apache Parquet Variant binary encoding,
//! whose vocabulary is the standard's and therefore smaller. Pickle,
//! a FIX row and any caller moving one value as bytes take this one; a
//! `variant` column takes that one.
//!
//! The stream starts with two bytes, the version and the identifier, and
//! what follows is fixed by the identifier alone: a number is its
//! little-endian bytes and nothing else, because the identifier already
//! says how wide it is; a payload of no fixed width - text, bytes, a
//! geometry - is a compression byte, a size and the bytes, compressed with
//! zstd once it is past [`COMPRESS_FROM`]; a list, a map or a struct is a
//! count and then each child as the same encoding without the version
//! byte, which the whole stream stated once. What a datatype states
//! beside its identifier travels where the value needs it - a decimal's
//! scale, a clock's unit and zone, a fixed text's width - so the bytes
//! decode to the value that was encoded, leaf for leaf, and to nothing
//! else.
//!
//! [`Scalar::encode_value_stream_bytes`] answers the stream one chunk
//! at a time, [`Scalar::into_value_bytes`] the whole of it, and
//! [`Scalar::decode_value_bytes`] and
//! [`Scalar::decode_value_stream_bytes`] read either back. A
//! [`DataTypeValue`](crate::DataTypeValue) and a [`DataType`] answer the
//! same four doors for a value cast to the datatype first, so a column
//! encodes what its datatype holds and decodes to it. The identifiers are
//! the same bytes [`Scalar::write_bytes`] feeds a digest, laid out by
//! [family](crate::DataTypeKind::id).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::bytes::BytesType;
use crate::string::StringType;
use crate::{
    DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, Struct, TimeUnit, Timezone,
};

/// The one version of the encoding, the first byte of every stream.
pub const VALUE_STREAM_VERSION: u8 = 0;

/// The payload length from which a payload of no fixed width is compressed:
/// past four kibibytes, the bytes are a zstd frame.
pub const COMPRESS_FROM: usize = 4 * 1024;

/// The compression byte of a payload stored as it is.
const UNCOMPRESSED: u8 = 0;

/// The compression byte of a payload stored as a zstd frame.
const ZSTD: u8 = 1;

/// The units a clock states, by the byte the encoding writes for each.
const UNITS: [TimeUnit; 8] = [
    TimeUnit::Day,
    TimeUnit::Second,
    TimeUnit::Millisecond,
    TimeUnit::Microsecond,
    TimeUnit::Nanosecond,
    TimeUnit::YearMonth,
    TimeUnit::DayTime,
    TimeUnit::MonthDayNano,
];

fn unit_byte(unit: TimeUnit) -> u8 {
    UNITS
        .iter()
        .position(|held| *held == unit)
        .map_or(0, |at| at as u8)
}

/// The stream [`Scalar::encode_value_stream_bytes`] answers: one chunk
/// per value, the children of a nested value after its header, in order.
///
/// The stream holds the values it has yet to write and nothing it has
/// written, so a tree is encoded with one chunk in hand at a time; a chunk
/// is the whole of one leaf, so a long text is one chunk.
pub struct ValueStream {
    pending: Vec<Frame>,
    started: bool,
}

enum Frame {
    Value(Scalar),
    Name(SmolStr),
}

impl ValueStream {
    fn over(value: Scalar) -> Self {
        Self {
            pending: vec![Frame::Value(value)],
            started: false,
        }
    }
}

impl Iterator for ValueStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        let frame = self.pending.pop()?;
        let mut chunk = Vec::new();
        if !self.started {
            chunk.push(VALUE_STREAM_VERSION);
            self.started = true;
        }
        match frame {
            Frame::Name(name) => write_name(&mut chunk, &name),
            // An Arrow-held value is the native value it holds, made here
            // so its children are the stream's own frames.
            Frame::Value(Scalar::Arrow(_)) => match frame_value(&frame).into_native() {
                Ok(native) => self.push_children(&native, &mut chunk),
                Err(_) => chunk.push(DataTypeId::Null.as_u8()),
            },
            Frame::Value(value) => self.push_children(&value, &mut chunk),
        }
        Some(chunk)
    }
}

/// The value one frame holds.
fn frame_value(frame: &Frame) -> &Scalar {
    match frame {
        Frame::Value(value) => value,
        Frame::Name(_) => unreachable!("a name frame holds no value"),
    }
}

impl ValueStream {
    /// Writes `value`'s own bytes into `chunk` and holds its children to
    /// write after it, last first, so they come out in order.
    fn push_children(&mut self, value: &Scalar, chunk: &mut Vec<u8>) {
        let mut children = Vec::new();
        encode(value, chunk, &mut children);
        self.pending
            .extend(children.into_iter().rev().map(|child| match child {
                Child::Value(value) => Frame::Value(value.clone()),
                Child::Name(name) => Frame::Name(name.clone()),
            }));
    }
}

/// One child of a nested value, as the value holds it.
enum Child<'value> {
    Value(&'value Scalar),
    Name(&'value SmolStr),
}

/// Appends the whole encoding of `root` - its own bytes, then every child
/// in order, each the same way - to `out`, borrowing every child from the
/// root instead of cloning each child.
///
/// The same pre-order the stream writes one chunk at a time, so the bytes
/// are the stream's concatenated; iterative, so a value nested deeper than
/// a stack holds still encodes.
fn encode_whole(root: &Scalar, out: &mut Vec<u8>) {
    let mut pending: Vec<Child<'_>> = vec![Child::Value(root)];
    let mut children: Vec<Child<'_>> = Vec::new();
    while let Some(frame) = pending.pop() {
        match frame {
            Child::Name(name) => write_name(out, name),
            // An Arrow-held value is the native value it holds: made here,
            // and encoded whole while it is in hand.
            Child::Value(value @ Scalar::Arrow(_)) => match value.into_native() {
                Ok(native) => encode_whole(&native, out),
                Err(_) => out.push(DataTypeId::Null.as_u8()),
            },
            Child::Value(value) => {
                children.clear();
                encode(value, out, &mut children);
                pending.extend(children.drain(..).rev());
            }
        }
    }
}

/// Appends a struct member's name: its size, then its bytes.
fn write_name(chunk: &mut Vec<u8>, name: &str) {
    write_size(chunk, name.len());
    chunk.extend_from_slice(name.as_bytes());
}

impl std::iter::FusedIterator for ValueStream {}

/// Appends `size` as an unsigned LEB128 count.
fn write_size(chunk: &mut Vec<u8>, size: usize) {
    let mut rest = size as u64;
    loop {
        let byte = (rest & 0x7f) as u8;
        rest >>= 7;
        if rest == 0 {
            chunk.push(byte);
            return;
        }
        chunk.push(byte | 0x80);
    }
}

/// Appends one payload of no fixed width: the compression byte, the size
/// and the bytes, a zstd frame past [`COMPRESS_FROM`].
fn write_variable(chunk: &mut Vec<u8>, payload: &[u8]) {
    let compressed = (payload.len() > COMPRESS_FROM)
        .then(|| crate::zstd::dump(payload).ok())
        .flatten()
        .filter(|compressed| compressed.len() < payload.len());
    if let Some(compressed) = compressed {
        chunk.push(ZSTD);
        write_size(chunk, compressed.len());
        chunk.extend_from_slice(&compressed);
        return;
    }
    chunk.push(UNCOMPRESSED);
    write_size(chunk, payload.len());
    chunk.extend_from_slice(payload);
}

fn write_text(chunk: &mut Vec<u8>, id: DataTypeId, text: &str) {
    chunk.push(id.as_u8());
    write_variable(chunk, text.as_bytes());
}

fn write_clock(chunk: &mut Vec<u8>, id: DataTypeId, unit: TimeUnit, zone: Option<&Timezone>) {
    chunk.push(id.as_u8());
    chunk.push(unit_byte(unit));
    if let Some(zone) = zone {
        write_size(chunk, zone.as_str().len());
        chunk.extend_from_slice(zone.as_str().as_bytes());
    }
}

/// Appends one value's own bytes to `chunk`, and answers the children a
/// nested value has in order - a list's values, a map's key then value per
/// entry, a struct's name then value per entry - for the caller to write
/// after it. An Arrow-held value is the caller's to make native first.
fn encode<'value>(value: &'value Scalar, chunk: &mut Vec<u8>, children: &mut Vec<Child<'value>>) {
    match value {
        Scalar::Arrow(_) => chunk.push(DataTypeId::Null.as_u8()),
        Scalar::Null => chunk.push(DataTypeId::Null.as_u8()),
        Scalar::Boolean(held) => {
            chunk.push(DataTypeId::Boolean.as_u8());
            chunk.push(u8::from(held.get()));
        }
        Scalar::Int8(held) => fixed(chunk, DataTypeId::Int8, &held.get().to_le_bytes()),
        Scalar::Int16(held) => fixed(chunk, DataTypeId::Int16, &held.get().to_le_bytes()),
        Scalar::Int32(held) => fixed(chunk, DataTypeId::Int32, &held.get().to_le_bytes()),
        Scalar::Int64(held) => fixed(chunk, DataTypeId::Int64, &held.get().to_le_bytes()),
        Scalar::Int128(held) => fixed(chunk, DataTypeId::Int128, &held.get().to_le_bytes()),
        Scalar::UInt8(held) => fixed(chunk, DataTypeId::UInt8, &held.get().to_le_bytes()),
        Scalar::UInt16(held) => fixed(chunk, DataTypeId::UInt16, &held.get().to_le_bytes()),
        Scalar::UInt32(held) => fixed(chunk, DataTypeId::UInt32, &held.get().to_le_bytes()),
        Scalar::UInt64(held) => fixed(chunk, DataTypeId::UInt64, &held.get().to_le_bytes()),
        Scalar::UInt128(held) => fixed(chunk, DataTypeId::UInt128, &held.get().to_le_bytes()),
        Scalar::Float16(held) => fixed(
            chunk,
            DataTypeId::Float16,
            &held.as_f16().to_bits().to_le_bytes(),
        ),
        Scalar::Float32(held) => fixed(
            chunk,
            DataTypeId::Float32,
            &held.as_f32().to_bits().to_le_bytes(),
        ),
        Scalar::Float64(held) => fixed(
            chunk,
            DataTypeId::Float64,
            &held.as_f64().to_bits().to_le_bytes(),
        ),
        Scalar::Decimal32(held) => {
            chunk.push(DataTypeId::Decimal32.as_u8());
            chunk.push(held.scale() as u8);
            chunk.extend_from_slice(&held.coefficient().to_le_bytes());
        }
        Scalar::Decimal64(held) => {
            chunk.push(DataTypeId::Decimal64.as_u8());
            chunk.push(held.scale() as u8);
            chunk.extend_from_slice(&held.coefficient().to_le_bytes());
        }
        Scalar::Decimal128(held) => {
            chunk.push(DataTypeId::Decimal128.as_u8());
            chunk.push(held.scale() as u8);
            chunk.extend_from_slice(&held.coefficient().to_le_bytes());
        }
        Scalar::Decimal256(held) => {
            chunk.push(DataTypeId::Decimal256.as_u8());
            chunk.push(held.scale() as u8);
            chunk.extend_from_slice(&held.coefficient().into_le_bytes());
        }
        Scalar::Date32(held) => fixed(chunk, DataTypeId::Date32, &held.count().to_le_bytes()),
        Scalar::Date64(held) => fixed(chunk, DataTypeId::Date64, &held.count().to_le_bytes()),
        Scalar::Time32(held) => {
            write_clock(
                chunk,
                DataTypeId::Time32,
                held.unit(),
                Some(&held.timezone()),
            );
            chunk.extend_from_slice(&held.count().to_le_bytes());
        }
        Scalar::Time64(held) => {
            write_clock(
                chunk,
                DataTypeId::Time64,
                held.unit(),
                Some(&held.timezone()),
            );
            chunk.extend_from_slice(&held.count().to_le_bytes());
        }
        Scalar::DateTime64(held) => {
            write_clock(
                chunk,
                DataTypeId::DateTime64,
                held.unit(),
                Some(&held.timezone()),
            );
            chunk.extend_from_slice(&held.count().to_le_bytes());
        }
        Scalar::Duration32(held) => {
            write_clock(chunk, DataTypeId::Duration32, held.unit(), None);
            chunk.extend_from_slice(&held.count().to_le_bytes());
        }
        Scalar::Duration64(held) => {
            write_clock(chunk, DataTypeId::Duration64, held.unit(), None);
            chunk.extend_from_slice(&held.count().to_le_bytes());
        }
        Scalar::Interval(held) => {
            write_clock(chunk, DataTypeId::Interval, held.unit(), None);
            chunk.extend_from_slice(&held.months().to_le_bytes());
            chunk.extend_from_slice(&held.days().to_le_bytes());
            chunk.extend_from_slice(&held.nanoseconds().to_le_bytes());
        }
        Scalar::String(held) => {
            let parameters = held.parameters();
            chunk.push(parameters.id().as_u8());
            if let Some(width) = parameters.fixed().or(parameters.max()) {
                write_size(chunk, width as usize);
            }
            write_variable(chunk, held.as_str().as_bytes());
        }
        Scalar::Bytes(held) => {
            let parameters = held.parameters();
            chunk.push(parameters.id().as_u8());
            if let Some(width) = parameters.fixed().or(parameters.max()) {
                write_size(chunk, width as usize);
            }
            write_variable(chunk, held.as_bytes());
        }
        Scalar::Uuid(held) => fixed(chunk, DataTypeId::Uuid, &held.into_bytes()),
        Scalar::Version(held) => write_text(chunk, DataTypeId::Version, &held.to_string()),
        Scalar::Url(held) => write_text(chunk, DataTypeId::Url, &held.to_string()),
        Scalar::Urn(held) => write_text(chunk, DataTypeId::Urn, &held.to_string()),
        Scalar::Timezone(held) => write_text(chunk, DataTypeId::Timezone, held.as_str()),
        Scalar::MimeType(held) => write_text(chunk, DataTypeId::MimeType, held.as_str()),
        Scalar::MediaType(held) => write_text(chunk, DataTypeId::MediaType, &held.to_string()),
        Scalar::Geometry(held) => {
            chunk.push(DataTypeId::Geometry.as_u8());
            write_variable(chunk, held.as_bytes());
        }
        Scalar::Geography(held) => {
            chunk.push(DataTypeId::Geography.as_u8());
            write_variable(chunk, held.as_bytes());
        }
        Scalar::List(held)
        | Scalar::ListView(held)
        | Scalar::FixedSizeList(held)
        | Scalar::LargeList(held)
        | Scalar::LargeListView(held) => {
            chunk.push(DataTypeId::List.as_u8());
            write_size(chunk, held.len());
            match held.as_slice() {
                Some(rows) => children.extend(rows.iter().map(Child::Value)),
                // A column lends no row: each is built and written whole
                // here, the same pre-order the children stack writes.
                None => {
                    for row in held.rows().iter() {
                        encode_whole(row, chunk);
                    }
                }
            }
        }
        Scalar::Map(held) | Scalar::SortedMap(held) => {
            chunk.push(DataTypeId::Map.as_u8());
            write_size(chunk, held.as_slice().len());
            for (key, value) in held.as_slice() {
                children.push(Child::Value(key));
                children.push(Child::Value(value));
            }
        }
        Scalar::Struct(held) => {
            chunk.push(DataTypeId::Struct.as_u8());
            write_size(chunk, held.as_map().len());
            for (name, value) in held.as_map() {
                children.push(Child::Name(name));
                children.push(Child::Value(value));
            }
        }
        // A variant is the two buffers the Parquet Variant encoding is,
        // kept as they are: this stream restates no other encoding.
        Scalar::Variant(held) => {
            chunk.push(DataTypeId::Variant.as_u8());
            write_variable(chunk, held.metadata());
            write_variable(chunk, held.value());
        }
        // A registered code is its text under its own identifier.
        code @ crate::code_scalars!() => write_text(
            chunk,
            code.id(),
            code.as_str().expect("a code borrows its text"),
        ),
    }
}

fn fixed(chunk: &mut Vec<u8>, id: DataTypeId, bytes: &[u8]) {
    chunk.push(id.as_u8());
    chunk.extend_from_slice(bytes);
}

// ------------------------------------------------------------------------
// Decoding.
// ------------------------------------------------------------------------

/// One pass over encoded bytes, refusing at the byte it could not read.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn refuse(&self, reason: impl Into<SmolStr>) -> Error {
        Error::Codec {
            format: "value",
            position: self.at,
            reason: reason.into(),
        }
    }

    fn byte(&mut self) -> Result<u8> {
        let byte = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| self.refuse("the stream ends before its value does"))?;
        self.at += 1;
        Ok(byte)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| self.refuse(format_smolstr!("{count} bytes announced, fewer held")))?;
        let held = &self.bytes[self.at..end];
        self.at = end;
        Ok(held)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut held = [0; N];
        held.copy_from_slice(self.take(N)?);
        Ok(held)
    }

    fn size(&mut self) -> Result<usize> {
        let mut value: u64 = 0;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return usize::try_from(value)
                    .map_err(|_| self.refuse("a size wider than this platform holds"));
            }
        }
        Err(self.refuse("a size of more than ten bytes"))
    }

    /// One payload of no fixed width: the bytes as the stream holds them,
    /// borrowed, or a zstd frame's contents, owned.
    fn variable(&mut self) -> Result<Cow<'a, [u8]>> {
        let compression = self.byte()?;
        let size = self.size()?;
        let held = self.take(size)?;
        match compression {
            UNCOMPRESSED => Ok(Cow::Borrowed(held)),
            ZSTD => crate::zstd::load(held).map(Cow::Owned),
            other => Err(self.refuse(format_smolstr!("compression {other} is not one this reads"))),
        }
    }

    fn text(&mut self) -> Result<Cow<'a, str>> {
        match self.variable()? {
            Cow::Borrowed(held) => std::str::from_utf8(held)
                .map(Cow::Borrowed)
                .map_err(|_| self.refuse("text that is not UTF-8")),
            Cow::Owned(held) => String::from_utf8(held)
                .map(Cow::Owned)
                .map_err(|_| self.refuse("text that is not UTF-8")),
        }
    }

    fn unit(&mut self) -> Result<TimeUnit> {
        let byte = self.byte()?;
        UNITS
            .get(usize::from(byte))
            .copied()
            .ok_or_else(|| self.refuse(format_smolstr!("unit {byte} is not one a clock states")))
    }

    fn zone(&mut self) -> Result<Timezone> {
        let size = self.size()?;
        let held = std::str::from_utf8(self.take(size)?)
            .map_err(|_| self.refuse("a zone that is not UTF-8"))?;
        Timezone::from_str(held)
    }

    fn name(&mut self) -> Result<SmolStr> {
        let size = self.size()?;
        std::str::from_utf8(self.take(size)?)
            .map(SmolStr::new)
            .map_err(|_| self.refuse("a name that is not UTF-8"))
    }

    fn value(&mut self, depth: usize) -> Result<Scalar> {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            return Err(self.refuse("a value nested deeper than the parse limit"));
        }
        let byte = self.byte()?;
        let Some(id) = DataTypeId::from_u8(byte) else {
            return Err(self.refuse(match DataTypeKind::of_u8(byte) {
                Some(kind) => {
                    format_smolstr!("byte {byte:#04x} is a placeholder of the {kind} family")
                }
                None => format_smolstr!("byte {byte:#04x} names no datatype"),
            }));
        };
        Ok(match id {
            DataTypeId::Null => Scalar::Null,
            DataTypeId::Boolean => Scalar::from(self.byte()? != 0),
            DataTypeId::Int8 => Scalar::from(i8::from_le_bytes(self.array()?)),
            DataTypeId::Int16 => Scalar::from(i16::from_le_bytes(self.array()?)),
            DataTypeId::Int32 => Scalar::from(i32::from_le_bytes(self.array()?)),
            DataTypeId::Int64 => Scalar::from(i64::from_le_bytes(self.array()?)),
            DataTypeId::Int128 => Scalar::from(i128::from_le_bytes(self.array()?)),
            DataTypeId::UInt8 => Scalar::from(u8::from_le_bytes(self.array()?)),
            DataTypeId::UInt16 => Scalar::from(u16::from_le_bytes(self.array()?)),
            DataTypeId::UInt32 => Scalar::from(u32::from_le_bytes(self.array()?)),
            DataTypeId::UInt64 => Scalar::from(u64::from_le_bytes(self.array()?)),
            DataTypeId::UInt128 => Scalar::from(u128::from_le_bytes(self.array()?)),
            DataTypeId::Float16 => {
                Scalar::from(half::f16::from_bits(u16::from_le_bytes(self.array()?)))
            }
            DataTypeId::Float32 => Scalar::from(f32::from_bits(u32::from_le_bytes(self.array()?))),
            DataTypeId::Float64 => Scalar::from(f64::from_bits(u64::from_le_bytes(self.array()?))),
            DataTypeId::Decimal32 => {
                let scale = self.byte()? as i8;
                Scalar::Decimal32(crate::Decimal32::new(
                    i32::from_le_bytes(self.array()?),
                    scale,
                ))
            }
            DataTypeId::Decimal64 => {
                let scale = self.byte()? as i8;
                Scalar::Decimal64(crate::Decimal64::new(
                    i64::from_le_bytes(self.array()?),
                    scale,
                ))
            }
            DataTypeId::Decimal128 => {
                let scale = self.byte()? as i8;
                Scalar::Decimal128(crate::Decimal128::new(
                    i128::from_le_bytes(self.array()?),
                    scale,
                ))
            }
            DataTypeId::Decimal256 => {
                let scale = self.byte()? as i8;
                Scalar::Decimal256(crate::Decimal256::new(
                    crate::i256::from_le_bytes(self.array()?),
                    scale,
                ))
            }
            DataTypeId::Date32 => Scalar::date32(i32::from_le_bytes(self.array()?)),
            DataTypeId::Date64 => Scalar::date64(i64::from_le_bytes(self.array()?)),
            DataTypeId::Time32 => {
                let unit = self.unit()?;
                let zone = self.zone()?;
                Scalar::time32(i32::from_le_bytes(self.array()?), unit, zone)?
            }
            DataTypeId::Time64 => {
                let unit = self.unit()?;
                let zone = self.zone()?;
                Scalar::time64(i64::from_le_bytes(self.array()?), unit, zone)?
            }
            DataTypeId::DateTime64 => {
                let unit = self.unit()?;
                let zone = self.zone()?;
                Scalar::datetime64(i64::from_le_bytes(self.array()?), unit, zone)?
            }
            DataTypeId::Duration32 => {
                let unit = self.unit()?;
                Scalar::duration32(i32::from_le_bytes(self.array()?), unit)?
            }
            DataTypeId::Duration64 => {
                let unit = self.unit()?;
                Scalar::duration64(i64::from_le_bytes(self.array()?), unit)?
            }
            DataTypeId::Interval => {
                let unit = self.unit()?;
                let months = i32::from_le_bytes(self.array()?);
                let days = i32::from_le_bytes(self.array()?);
                let nanoseconds = i64::from_le_bytes(self.array()?);
                Scalar::Interval(crate::Interval::new(months, days, nanoseconds, unit)?)
            }
            DataTypeId::Uuid => Scalar::Uuid(crate::Uuid::from_bytes(&self.array::<16>()?)?),
            DataTypeId::Geometry => {
                Scalar::Geometry(crate::Geometry::new(Arc::<[u8]>::from(&*self.variable()?))?)
            }
            DataTypeId::Geography => Scalar::Geography(crate::Geography::new(Arc::<[u8]>::from(
                &*self.variable()?,
            ))?),
            DataTypeId::List => {
                let count = self.size()?;
                let mut values = Vec::with_capacity(count.min(1 << 16));
                for _ in 0..count {
                    values.push(self.value(depth + 1)?);
                }
                Scalar::from_sequence(values)
            }
            DataTypeId::Map => {
                let count = self.size()?;
                let mut entries = Vec::with_capacity(count.min(1 << 16));
                for _ in 0..count {
                    let key = self.value(depth + 1)?;
                    let value = self.value(depth + 1)?;
                    entries.push((key, value));
                }
                Scalar::from_mapping(entries)?
            }
            DataTypeId::Variant => {
                let metadata = self.variable()?;
                let value = self.variable()?;
                Scalar::Variant(crate::Variant::new(
                    Arc::<[u8]>::from(&*metadata),
                    Arc::<[u8]>::from(&*value),
                )?)
            }
            DataTypeId::Struct => {
                let count = self.size()?;
                let mut entries = BTreeMap::new();
                for _ in 0..count {
                    let name = self.name()?;
                    let value = self.value(depth + 1)?;
                    match entries.entry(name) {
                        Entry::Vacant(slot) => {
                            slot.insert(value);
                        }
                        Entry::Occupied(held) => {
                            return Err(self
                                .refuse(format_smolstr!("a struct naming {} twice", held.key())));
                        }
                    }
                }
                Scalar::Struct(Struct::new(Arc::new(entries)))
            }
            other if other.kind() == DataTypeKind::Bytes => {
                let width = match BytesType::from_id(other, 0) {
                    Some(parameters) if parameters.fixed().or(parameters.max()).is_some() => {
                        u32::try_from(self.size()?)
                            .map_err(|_| self.refuse("a width past what a column states"))?
                    }
                    _ => 0,
                };
                let Some(parameters) = BytesType::from_id(other, width) else {
                    return Err(self.refuse(format_smolstr!("{other} is no bytes leaf")));
                };
                Scalar::Bytes(crate::Bytes::from_storage(&self.variable()?, parameters))
            }
            other if StringType::from_id(other, 1).is_some() => {
                let width = match StringType::from_id(other, 0) {
                    Some(parameters) if parameters.fixed().or(parameters.max()).is_some() => {
                        u32::try_from(self.size()?)
                            .map_err(|_| self.refuse("a width past what a column states"))?
                    }
                    _ => 0,
                };
                let Some(parameters) = StringType::from_id(other, width) else {
                    return Err(self.refuse(format_smolstr!("{other} is no string leaf")));
                };
                Scalar::String(crate::Str::from_storage(&self.text()?, parameters))
            }
            // A code, a version, a location, a zone, a media type: the text
            // under the identifier, read through the datatype's own door.
            other if matches!(other.kind(), DataTypeKind::Code | DataTypeKind::Text) => {
                let text = self.text()?;
                // The identifier's own datatype, the process's one rather
                // than one parsed from the identifier's name per value.
                let dtype = crate::typed::prebuilt_dtype(other).ok_or_else(|| {
                    self.refuse(format_smolstr!("{other} holds no value of its own"))
                })?;
                crate::value::dtype_scalar(dtype, Scalar::from(&*text))?
            }
            other => {
                return Err(self.refuse(format_smolstr!("{other} holds no value of its own")));
            }
        })
    }
}

impl Scalar {
    /// This value as the value stream, one chunk at a time: the
    /// version and the identifier first, then the payload, then a nested
    /// value's children in order.
    ///
    /// The stream holds what it has yet to write and nothing it wrote, so
    /// a tree is encoded with one leaf in hand at a time;
    /// [`Self::into_value_bytes`] is the same bytes in one buffer. An
    /// Arrow-held value encodes as the native value it holds, and as a
    /// null where it holds none the native reading can spell.
    #[must_use]
    pub fn encode_value_stream_bytes(&self) -> ValueStream {
        ValueStream::over(self.clone())
    }

    /// This value as the value stream, whole: the stream's chunks in
    /// one buffer, written into it directly.
    #[must_use]
    pub fn into_value_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(16);
        bytes.push(VALUE_STREAM_VERSION);
        encode_whole(self, &mut bytes);
        bytes
    }

    /// The value one value stream holds, whole: the version, the
    /// identifier, the payload, and nothing after it.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal, positioned at the byte it could not
    /// read: another version, a byte naming no datatype or a family's own
    /// placeholder, a payload cut short, a compression this does not read,
    /// text that is not UTF-8, a struct naming one child twice, a value
    /// the leaf refuses, or bytes left over after the value.
    pub fn decode_value_bytes(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader { bytes, at: 0 };
        let version = reader.byte()?;
        if version != VALUE_STREAM_VERSION {
            return Err(reader.refuse(format_smolstr!(
                "version {version} is not the {VALUE_STREAM_VERSION} this reads"
            )));
        }
        let value = reader.value(0)?;
        if reader.at != bytes.len() {
            return Err(reader.refuse(format_smolstr!(
                "{} bytes left after the value",
                bytes.len() - reader.at
            )));
        }
        Ok(value)
    }

    /// The value a value stream split into chunks holds - what
    /// [`Self::encode_value_stream_bytes`] answered, or any other cut of
    /// the same bytes - gathered and read as
    /// [`Self::decode_value_bytes`] reads it.
    ///
    /// # Errors
    ///
    /// [`Self::decode_value_bytes`] carries the rule.
    pub fn decode_value_stream_bytes<I>(chunks: I) -> Result<Self>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        let mut bytes = Vec::new();
        for chunk in chunks {
            bytes.extend_from_slice(chunk.as_ref());
        }
        Self::decode_value_bytes(&bytes)
    }
}

impl DataType {
    /// `value` cast to this datatype and encoded, one chunk at a time.
    ///
    /// # Errors
    ///
    /// Returns the cast's refusal where the value is not one this datatype
    /// holds.
    pub fn encode_value_stream_bytes(&self, value: &Scalar) -> Result<ValueStream> {
        Ok(self.cast_scalar(value)?.encode_value_stream_bytes())
    }

    /// `value` cast to this datatype and encoded, whole.
    ///
    /// # Errors
    ///
    /// Returns the cast's refusal where the value is not one this datatype
    /// holds.
    pub fn encode_value_bytes(&self, value: &Scalar) -> Result<Vec<u8>> {
        Ok(self.cast_scalar(value)?.into_value_bytes())
    }

    /// The value one value stream holds, cast to this datatype.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal, or the cast's where the bytes hold a
    /// value this datatype does not.
    pub fn decode_value_bytes(&self, bytes: &[u8]) -> Result<Scalar> {
        self.cast_scalar(&Scalar::decode_value_bytes(bytes)?)
    }

    /// The value a value stream split into chunks holds, cast to this
    /// datatype.
    ///
    /// # Errors
    ///
    /// [`Self::decode_value_bytes`] carries the rule.
    pub fn decode_value_stream_bytes<I>(&self, chunks: I) -> Result<Scalar>
    where
        I: IntoIterator,
        I::Item: AsRef<[u8]>,
    {
        self.cast_scalar(&Scalar::decode_value_stream_bytes(chunks)?)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/valuestream.rs` pins and a caller cannot reach.
    //!
    //! The compression flag a payload carries is private: a caller sees the
    //! bytes and the value they read back, never the byte that says whether
    //! zstd was used. The pins are on the wire itself, so they name the two
    //! spellings, restated here rather than made public in the encoder.

    /// The flag byte a payload written as-is carries.
    pub const UNCOMPRESSED: u8 = super::UNCOMPRESSED;
    /// The flag byte a payload written as a zstd frame carries.
    pub const ZSTD: u8 = super::ZSTD;
}
