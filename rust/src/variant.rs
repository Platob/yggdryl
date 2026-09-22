//! The Apache Parquet Variant binary encoding, version 1: one value as a
//! metadata dictionary and a value payload, the pair every medium here
//! writes for a `variant` column.
//!
//! The encoding is the specification's, byte for byte -
//! [VariantEncoding.md][spec] version `1` - so a value written here is a
//! value Spark, Iceberg, Parquet and Arrow read, and a value any of them
//! wrote reads back here. A [`Variant`] is that pair and nothing else: the
//! `metadata`, a header byte, a dictionary size, its offsets and the key
//! bytes; and the `value`, one `value_metadata` byte naming a basic type
//! and a primitive, a short string, an object or an array, then the
//! payload that byte says how to read.
//!
//! [`crate::ValueStream`] is the crate's own byte form of a value and
//! keeps every leaf it holds; this one keeps what the standard can spell,
//! which is less. Supported leaves without a standard physical type take
//! [the spelling the JSON codec gives them](crate::json). Unrepresentable
//! values are refused. Decoding uses the twenty-one physical types the
//! standard names:
//!
//! | value | variant physical type | reads back as |
//! | --- | --- | --- |
//! | `null` | `null` | `null` |
//! | `boolean` | `boolean` | `boolean` |
//! | `int8`, `int16`, `int32`, `int64` | the same width | the same width |
//! | `uint8`, `uint16`, `uint32` | the next signed width up | that width |
//! | `uint64`, `int128`, `uint128` | `int64`, else `decimal16` | that type |
//! | `float16`, `float32` | `float` | `float32` |
//! | `float64` | `double` | `float64` |
//! | `decimal32/64/128/256` | `decimal4`/`decimal8`/`decimal16` | `decimal32/64/128` |
//! | `date32`, `date64` | `date` | `date32` |
//! | `time32`, `time64` | `time` (microseconds, no zone) | `time64` |
//! | `datetime64` | `timestamp`, zoned or not, in microseconds or nanoseconds | `datetime64` |
//! | `uuid` | `uuid` | `uuid` |
//! | every string, code, version, location, zone and media type | `string` | `utf8` |
//! | every byte layout, and a geometry or geography's WKB | `binary` | `binary` |
//! | `duration32`, `duration64` | `string`, the ISO-8601 spelling | `utf8` |
//! | `interval` | the JSON codec's number or array | that shape |
//! | a list | `array` | a list |
//! | a struct, and a mapping whose keys are text | `object` | a struct |
//!
//! A decimal past thirty-eight digits, a time whose count is not a whole
//! microsecond, a zoned time, a zoned duration and a
//! mapping with a key that is not text are what no reading spells, and
//! each is refused by name.
//!
//! [spec]: https://github.com/apache/parquet-format/blob/master/VariantEncoding.md

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, Scalar, Struct, TimeUnit, Timezone};

/// The one specification version this reads and writes.
///
/// It is the low nibble of the metadata header byte; a metadata byte
/// stating another version is refused rather than guessed at.
pub const VARIANT_VERSION: u8 = 1;

// The four basic types, the low two bits of a `value_metadata` byte.
const PRIMITIVE: u8 = 0;
const SHORT_STRING: u8 = 1;
const OBJECT: u8 = 2;
const ARRAY: u8 = 3;

// The primitive types, the `value_header` of a primitive.
const NULL: u8 = 0;
const TRUE: u8 = 1;
const FALSE: u8 = 2;
const INT8: u8 = 3;
const INT16: u8 = 4;
const INT32: u8 = 5;
const INT64: u8 = 6;
const DOUBLE: u8 = 7;
const DECIMAL4: u8 = 8;
const DECIMAL8: u8 = 9;
const DECIMAL16: u8 = 10;
const DATE: u8 = 11;
const TIMESTAMP_MICROS: u8 = 12;
const TIMESTAMP_NTZ_MICROS: u8 = 13;
const FLOAT: u8 = 14;
const BINARY: u8 = 15;
const STRING: u8 = 16;
const TIME_NTZ_MICROS: u8 = 17;
const TIMESTAMP_NANOS: u8 = 18;
const TIMESTAMP_NTZ_NANOS: u8 = 19;
const UUID: u8 = 20;

/// The longest string whose length the short-string header folds in.
const SHORT_STRING_MAX: usize = 63;

/// The most digits a variant decimal carries.
const DECIMAL_DIGITS: u32 = 38;
const DECIMAL4_DIGITS: u32 = 9;
const DECIMAL8_DIGITS: u32 = 18;

/// The element count from which an object or an array states it in four
/// bytes rather than one.
const LARGE_FROM: usize = 256;

/// One variant value: the metadata dictionary and the value payload.
///
/// The two buffers are what a `variant` column stores per row - a Parquet
/// group of two `BYTE_ARRAY` children, an Avro record of two `bytes`
/// fields, an Arrow struct of two binaries - and they travel together
/// because a value's object keys live in the metadata.
///
/// ```
/// use yggdryl::{Scalar, Variant};
///
/// # fn main() -> yggdryl::Result<()> {
/// let quote = Scalar::from_struct([("symbol", Scalar::from("AAPL"))])?;
/// let variant = Variant::encode(&quote)?;
/// assert_eq!(variant.metadata()[0] & 0x0f, 1, "the specification version");
/// assert_eq!(variant.scalar()?, quote);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Variant {
    metadata: Arc<[u8]>,
    value: Arc<[u8]>,
}

impl Variant {
    /// One variant over bytes a medium stored, metadata first.
    ///
    /// The metadata header is read here - the version, the offset width,
    /// the dictionary and its keys - so a pair this answers has keys to
    /// resolve against. The value payload is walked by [`Self::scalar`],
    /// which is where a malformed value is refused.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal, positioned at the byte it could not
    /// read: metadata of another version, a dictionary cut short, an
    /// offset past the key bytes, a key that is not UTF-8, or an empty
    /// value payload.
    pub fn new(metadata: impl Into<Arc<[u8]>>, value: impl Into<Arc<[u8]>>) -> Result<Self> {
        let variant = Self {
            metadata: metadata.into(),
            value: value.into(),
        };
        Dictionary::over(&variant.metadata)?;
        if variant.value.is_empty() {
            return Err(refuse(0, "a value payload of no bytes"));
        }
        Ok(variant)
    }

    /// This value as the variant encoding.
    ///
    /// The keys of every object in the tree are gathered first, sorted and
    /// deduplicated into the metadata dictionary, so the value payload
    /// names each key by its index and the header states `sorted_strings`.
    /// Sizes are the narrowest the payload allows: a count under 256 is
    /// one byte, an offset is one to four.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal for the values the standard has no
    /// spelling for: a decimal past thirty-eight digits, a time that is not a
    /// whole microsecond, a zoned time or
    /// duration, a mapping keyed by anything but text, a payload past four
    /// gibibytes, or a tree nested deeper than the parse limit.
    pub fn encode(value: &Scalar) -> Result<Self> {
        if let Scalar::Variant(variant) = value {
            return Ok(variant.clone());
        }
        let mut keys = BTreeSet::new();
        collect_keys(value, 0, &mut keys)?;
        let dictionary: Vec<SmolStr> = keys.into_iter().collect();
        let mut payload = Vec::with_capacity(16);
        write_value(value, &dictionary, 0, &mut payload)?;
        Ok(Self {
            metadata: if dictionary.is_empty() {
                empty_metadata()
            } else {
                metadata_bytes(&dictionary)?.into()
            },
            value: payload.into(),
        })
    }

    /// The metadata bytes: the header, the dictionary size, its offsets
    /// and the key bytes.
    #[must_use]
    pub fn metadata(&self) -> &[u8] {
        &self.metadata
    }

    /// The value bytes: one `value_metadata` byte and its payload.
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// The value these bytes hold.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal, positioned at the byte it could not
    /// read: a payload cut short, a primitive type this version does not
    /// name, a field id past the dictionary, text that is not UTF-8, an
    /// object naming one key twice, a value nested deeper than the parse
    /// limit, or bytes left over after the value.
    pub fn scalar(&self) -> Result<Scalar> {
        let dictionary = Dictionary::over(&self.metadata)?;
        let mut reader = Reader {
            bytes: &self.value,
            at: 0,
            dictionary: &dictionary,
        };
        let value = reader.value(0)?;
        if reader.at != self.value.len() {
            return Err(refuse(
                reader.at,
                format_smolstr!(
                    "{} bytes left after the value",
                    self.value.len() - reader.at
                ),
            ));
        }
        Ok(value)
    }
}

impl Scalar {
    /// This value as [the variant encoding](Variant).
    ///
    /// # Errors
    ///
    /// [`Variant::encode`] carries the rule.
    pub fn into_variant(&self) -> Result<Variant> {
        Variant::encode(self)
    }

    /// The value one variant encoding holds.
    ///
    /// # Errors
    ///
    /// [`Variant::scalar`] carries the rule.
    pub fn from_variant(variant: &Variant) -> Result<Self> {
        variant.scalar()
    }
}

impl DataType {
    /// `value` cast to this datatype and encoded as [a variant](Variant).
    ///
    /// # Errors
    ///
    /// Returns the cast's refusal where the value is not one this datatype
    /// holds, or the codec's where the standard cannot spell it.
    pub fn encode_variant(&self, value: &Scalar) -> Result<Variant> {
        Variant::encode(&self.cast_scalar(value)?)
    }

    /// The value one variant encoding holds, cast to this datatype.
    ///
    /// # Errors
    ///
    /// Returns the codec's refusal, or the cast's where the bytes hold a
    /// value this datatype does not.
    pub fn decode_variant(&self, variant: &Variant) -> Result<Scalar> {
        self.cast_scalar(&variant.scalar()?)
    }
}

impl std::fmt::Display for Variant {
    /// The value the bytes hold, as the JSON text every variant reader
    /// shows a variant as.
    ///
    /// A payload this cannot read, or a value JSON cannot spell, writes its
    /// own size instead, because a display has no refusal to return: the
    /// bytes are still there, and [`Self::scalar`] is where reading them
    /// fails by name.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self
            .scalar()
            .and_then(|value| crate::json::into_json_scalar(&value))
        {
            Ok(text) => formatter.write_str(&text),
            Err(_) => write!(formatter, "variant({} bytes)", self.value.len()),
        }
    }
}

impl crate::Value for Variant {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Variant)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Variant(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Variant(value) => Some(value),
            _ => None,
        }
    }

    fn into_variant(&self) -> Result<Self> {
        Ok(self.clone())
    }

    fn from_variant(value: &Self) -> Result<Self> {
        Ok(value.clone())
    }
}

impl From<Variant> for Scalar {
    fn from(value: Variant) -> Self {
        Self::Variant(value)
    }
}

/// A refusal at one byte of a variant encoding.
fn refuse(position: usize, reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "variant",
        position,
        reason: reason.into(),
    }
}

// ------------------------------------------------------------------------
// Metadata.
// ------------------------------------------------------------------------

/// The number of bytes one unsigned value needs, one through four.
const fn width_of(value: usize) -> usize {
    if value <= 0xff {
        1
    } else if value <= 0xffff {
        2
    } else if value <= 0x00ff_ffff {
        3
    } else {
        4
    }
}

/// Append `value` as an unsigned little-endian integer of `width` bytes.
fn write_le(out: &mut Vec<u8>, value: usize, width: usize) {
    let bytes = (value as u64).to_le_bytes();
    out.extend_from_slice(&bytes[..width]);
}

/// Refuse a size no offset width this encoding has can state.
fn bounded(size: usize, what: &'static str) -> Result<usize> {
    if size > u32::MAX as usize {
        return Err(refuse(0, format_smolstr!("{what} past four gibibytes")));
    }
    Ok(size)
}

/// The metadata bytes for one sorted, deduplicated dictionary.
fn metadata_bytes(dictionary: &[SmolStr]) -> Result<Vec<u8>> {
    let total = bounded(
        dictionary.iter().map(SmolStr::len).sum::<usize>(),
        "a key dictionary",
    )?;
    let width = width_of(total.max(dictionary.len()));
    let mut bytes = Vec::with_capacity(1 + width * (dictionary.len() + 2) + total);
    // The keys are gathered through a sorted set, so `sorted_strings` is
    // always set and a reader may binary-search the field ids.
    bytes.push(VARIANT_VERSION | (1 << 4) | ((width as u8 - 1) << 6));
    write_le(&mut bytes, dictionary.len(), width);
    let mut offset = 0;
    for key in dictionary {
        write_le(&mut bytes, offset, width);
        offset += key.len();
    }
    write_le(&mut bytes, offset, width);
    for key in dictionary {
        bytes.extend_from_slice(key.as_bytes());
    }
    Ok(bytes)
}

/// The one canonical metadata buffer for a value with no object keys.
fn empty_metadata() -> Arc<[u8]> {
    static EMPTY: OnceLock<Arc<[u8]>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from([0x11, 0, 0])))
}

/// The key dictionary one metadata buffer holds, read in place.
struct Dictionary<'a> {
    /// The key bytes, after the header and the offsets.
    keys: &'a str,
    /// The `count + 1` packed offsets into `keys`.
    offsets: &'a [u8],
    width: usize,
    count: usize,
}

impl<'a> Dictionary<'a> {
    /// Read and validate one metadata buffer's header and dictionary.
    fn over(metadata: &'a [u8]) -> Result<Self> {
        let header = *metadata
            .first()
            .ok_or_else(|| refuse(0, "metadata of no bytes"))?;
        let version = header & 0x0f;
        if version != VARIANT_VERSION {
            return Err(refuse(
                0,
                format_smolstr!("version {version} is not the {VARIANT_VERSION} this reads"),
            ));
        }
        let width = usize::from(header >> 6) + 1;
        let count = read_le(metadata, 1, width)
            .ok_or_else(|| refuse(1, "a dictionary size the metadata ends before"))?;
        let offsets_at = 1 + width;
        let offsets_len = count
            .checked_add(1)
            .and_then(|count| count.checked_mul(width))
            .ok_or_else(|| refuse(offsets_at, "a dictionary whose offset table is too large"))?;
        let keys_at = offsets_at
            .checked_add(offsets_len)
            .ok_or_else(|| refuse(offsets_at, "a dictionary whose offset table is too large"))?;
        let offsets = metadata.get(offsets_at..keys_at).ok_or_else(|| {
            refuse(
                offsets_at,
                format_smolstr!(
                    "{} dictionary offsets are not held",
                    count.saturating_add(1)
                ),
            )
        })?;
        let key_bytes = metadata
            .get(keys_at..)
            .ok_or_else(|| refuse(keys_at, "key bytes the metadata ends before"))?;

        let offset = |index: usize| {
            read_le(offsets, index * width, width)
                .expect("the complete offset table was borrowed above")
        };
        if offset(0) != 0 {
            return Err(refuse(
                offsets_at,
                "the first dictionary offset is not zero",
            ));
        }
        let mut previous = 0;
        for index in 0..=count {
            let current = offset(index);
            if current < previous {
                return Err(refuse(
                    offsets_at + index * width,
                    "a dictionary offset before the one before it",
                ));
            }
            previous = current;
        }
        if previous != key_bytes.len() {
            return Err(refuse(
                keys_at,
                format_smolstr!(
                    "{previous} key bytes announced, {} held exactly",
                    key_bytes.len()
                ),
            ));
        }

        let keys = std::str::from_utf8(key_bytes).map_err(|error| {
            refuse(
                keys_at + error.valid_up_to(),
                "dictionary key bytes are not UTF-8",
            )
        })?;
        let sorted = header & 0x10 != 0;
        let mut previous_key: Option<&str> = None;
        for index in 0..count {
            let start = offset(index);
            let end = offset(index + 1);
            let key = keys.get(start..end).ok_or_else(|| {
                refuse(
                    keys_at + start,
                    format_smolstr!("dictionary key {index} offsets split UTF-8 text"),
                )
            })?;
            if sorted {
                if previous_key.is_some_and(|previous| previous >= key) {
                    let reason = if previous_key == Some(key) {
                        "a dictionary claiming sorted strings has keys that are not unique"
                    } else {
                        "a dictionary claiming sorted strings has keys out of sorted order"
                    };
                    return Err(refuse(keys_at + start, reason));
                }
                previous_key = Some(key);
            }
        }
        Ok(Self {
            keys,
            offsets,
            width,
            count,
        })
    }

    /// The key one field id names.
    fn key(&self, id: usize, at: usize) -> Result<&'a str> {
        if id >= self.count {
            return Err(refuse(
                at,
                format_smolstr!(
                    "field id {id} is past the {} keys the dictionary holds",
                    self.count
                ),
            ));
        }
        let start = read_le(self.offsets, id * self.width, self.width)
            .expect("a validated dictionary holds every offset");
        let end = read_le(self.offsets, (id + 1) * self.width, self.width)
            .expect("a validated dictionary holds every offset");
        Ok(self
            .keys
            .get(start..end)
            .expect("every dictionary offset was validated on a UTF-8 boundary"))
    }
}

/// Read an unsigned little-endian integer of `width` bytes at `at`.
fn read_le(bytes: &[u8], at: usize, width: usize) -> Option<usize> {
    let held = bytes.get(at..at.checked_add(width)?)?;
    let mut value = 0_u64;
    for (shift, byte) in held.iter().enumerate() {
        value |= u64::from(*byte) << (shift * 8);
    }
    usize::try_from(value).ok()
}

// ------------------------------------------------------------------------
// Encoding.
// ------------------------------------------------------------------------

/// The text one object key is spelled as, or the refusal naming it.
fn key_text(key: &Scalar) -> Result<&str> {
    key.as_str().ok_or_else(|| {
        refuse(
            0,
            format_smolstr!("a variant object's keys are text; {} is not", key.kind()),
        )
    })
}

/// Gather every object key in the tree into the sorted dictionary.
fn collect_keys(value: &Scalar, depth: usize, keys: &mut BTreeSet<SmolStr>) -> Result<()> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return Err(refuse(0, "a value nested deeper than the parse limit"));
    }
    match value {
        Scalar::Arrow(_) => collect_keys(&value.into_native()?, depth, keys)?,
        Scalar::Sequence(held) => {
            for item in held.rows().iter() {
                collect_keys(item, depth + 1, keys)?;
            }
        }
        Scalar::Struct(held) => {
            for (name, child) in held.as_map() {
                keys.insert(name.clone());
                collect_keys(child, depth + 1, keys)?;
            }
        }
        Scalar::Mapping(held) => {
            for (key, child) in held.as_slice() {
                keys.insert(SmolStr::new(key_text(key)?));
                collect_keys(child, depth + 1, keys)?;
            }
        }
        // A variant nested in a value contributes the keys of the value it
        // holds, because that is what this encoding writes for it.
        Scalar::Variant(held) => collect_keys(&held.scalar()?, depth, keys)?,
        _ => {}
    }
    Ok(())
}

/// Append one primitive's header byte and its payload.
fn primitive(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(PRIMITIVE | (id << 2));
    out.extend_from_slice(payload);
}

/// Append one text payload: a short string where it fits the header, a
/// `string` primitive with a four-byte length where it does not.
fn write_text(out: &mut Vec<u8>, text: &str) -> Result<()> {
    let size = bounded(text.len(), "a text payload")?;
    if size <= SHORT_STRING_MAX {
        out.push(SHORT_STRING | ((size as u8) << 2));
    } else {
        out.push(PRIMITIVE | (STRING << 2));
        out.extend_from_slice(&(size as u32).to_le_bytes());
    }
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Append one `binary` primitive: a four-byte length and the bytes.
fn write_binary(out: &mut Vec<u8>, payload: &[u8]) -> Result<()> {
    let size = bounded(payload.len(), "a binary payload")?;
    out.push(PRIMITIVE | (BINARY << 2));
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

/// Append one integer at the narrowest of the widths that hold it, never
/// narrower than the width the value was held at.
fn write_integer(out: &mut Vec<u8>, value: i128, floor: u8) -> Result<()> {
    let id = if floor > INT8 || i128::from(i8::MIN) > value || value > i128::from(i8::MAX) {
        if floor > INT16 || i128::from(i16::MIN) > value || value > i128::from(i16::MAX) {
            if floor > INT32 || i128::from(i32::MIN) > value || value > i128::from(i32::MAX) {
                if i128::from(i64::MIN) > value || value > i128::from(i64::MAX) {
                    // Past int64 the only exact type left is a decimal of
                    // no scale, which the standard holds to 38 digits.
                    return write_decimal(out, value, 0, DECIMAL16);
                }
                INT64
            } else {
                INT32
            }
        } else {
            INT16
        }
    } else {
        INT8
    };
    match id {
        INT8 => primitive(out, INT8, &(value as i8).to_le_bytes()),
        INT16 => primitive(out, INT16, &(value as i16).to_le_bytes()),
        INT32 => primitive(out, INT32, &(value as i32).to_le_bytes()),
        _ => primitive(out, INT64, &(value as i64).to_le_bytes()),
    }
    Ok(())
}

/// Append one decimal: a one-byte scale in `0..=38` and the little-endian
/// unscaled value, at the narrowest width from `floor` up that holds it.
///
/// A negative scale is not one the standard states, so the coefficient is
/// scaled up to a scale of zero - the same number, spelled the way the
/// format spells it - and a scale past thirty-eight digits is refused.
fn write_decimal(out: &mut Vec<u8>, coefficient: i128, scale: i8, floor: u8) -> Result<()> {
    let mut coefficient = coefficient;
    let mut scale = scale;
    while scale < 0 {
        coefficient = coefficient.checked_mul(10).ok_or_else(|| {
            refuse(
                out.len(),
                format_smolstr!("a decimal of scale {scale} past what 128 bits hold at scale 0"),
            )
        })?;
        scale += 1;
    }
    if u32::from(scale.unsigned_abs()) > DECIMAL_DIGITS {
        return Err(refuse(
            out.len(),
            format_smolstr!("a scale of {scale}, past the {DECIMAL_DIGITS} digits a variant holds"),
        ));
    }
    let digits = decimal_digits(coefficient);
    if digits > DECIMAL_DIGITS {
        return Err(refuse(
            out.len(),
            format_smolstr!(
                "a coefficient of more than the {DECIMAL_DIGITS} digits a variant holds"
            ),
        ));
    }
    let scale = scale as u8;
    if floor <= DECIMAL4 && digits <= DECIMAL4_DIGITS {
        out.push(PRIMITIVE | (DECIMAL4 << 2));
        out.push(scale);
        out.extend_from_slice(&(coefficient as i32).to_le_bytes());
    } else if floor <= DECIMAL8 && digits <= DECIMAL8_DIGITS {
        out.push(PRIMITIVE | (DECIMAL8 << 2));
        out.push(scale);
        out.extend_from_slice(&(coefficient as i64).to_le_bytes());
    } else {
        out.push(PRIMITIVE | (DECIMAL16 << 2));
        out.push(scale);
        out.extend_from_slice(&coefficient.to_le_bytes());
    }
    Ok(())
}

/// The base-ten precision of one signed coefficient; zero has precision one.
fn decimal_digits(coefficient: i128) -> u32 {
    let coefficient = coefficient.unsigned_abs();
    if coefficient == 0 {
        1
    } else {
        coefficient.ilog10() + 1
    }
}

/// Refuse a coefficient whose precision its physical decimal cannot hold.
fn validate_decimal(coefficient: i128, digits: u32, physical: &str, at: usize) -> Result<()> {
    let actual = decimal_digits(coefficient);
    if actual > digits {
        return Err(refuse(
            at,
            format_smolstr!(
                "{physical} holds at most {digits} digits; its coefficient has {actual}"
            ),
        ));
    }
    Ok(())
}

/// The count of one temporal value at `unit`, or the refusal naming it.
fn count_at(value: &Scalar, unit: TimeUnit, at: usize) -> Result<i64> {
    value.temporal_count_at(unit).ok_or_else(|| {
        refuse(
            at,
            format_smolstr!("a {} is not a whole {unit}", value.kind()),
        )
    })
}

/// Refuse a clock value carrying a zone the standard's type does not.
fn naive(zone: &Timezone, what: &'static str, at: usize) -> Result<()> {
    if zone.is_naive() {
        return Ok(());
    }
    Err(refuse(
        at,
        format_smolstr!("a {what} carries no zone in a variant; {zone} is one"),
    ))
}

/// Append one value: its `value_metadata` byte, then its payload.
#[allow(clippy::too_many_lines)]
fn write_value(value: &Scalar, keys: &[SmolStr], depth: usize, out: &mut Vec<u8>) -> Result<()> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return Err(refuse(
            out.len(),
            "a value nested deeper than the parse limit",
        ));
    }
    match value {
        Scalar::Arrow(_) => write_value(&value.into_native()?, keys, depth, out)?,
        Scalar::Null => out.push(PRIMITIVE | (NULL << 2)),
        Scalar::Boolean(held) => {
            let id = if held.get() { TRUE } else { FALSE };
            out.push(PRIMITIVE | (id << 2));
        }
        Scalar::Int8(held) => primitive(out, INT8, &held.get().to_le_bytes()),
        Scalar::Int16(held) => primitive(out, INT16, &held.get().to_le_bytes()),
        Scalar::Int32(held) => primitive(out, INT32, &held.get().to_le_bytes()),
        Scalar::Int64(held) => primitive(out, INT64, &held.get().to_le_bytes()),
        // An unsigned width has no variant type of its own, so it takes
        // the next signed width up, which holds every value it does.
        Scalar::UInt8(held) => primitive(out, INT16, &i16::from(held.get()).to_le_bytes()),
        Scalar::UInt16(held) => primitive(out, INT32, &i32::from(held.get()).to_le_bytes()),
        Scalar::UInt32(held) => primitive(out, INT64, &i64::from(held.get()).to_le_bytes()),
        Scalar::UInt64(held) => write_integer(out, i128::from(held.get()), INT64)?,
        Scalar::Int128(held) => write_integer(out, held.get(), INT64)?,
        Scalar::UInt128(held) => {
            let held = i128::try_from(held.get()).map_err(|_| {
                refuse(
                    out.len(),
                    "a uint128 past what 128 signed bits, the widest a variant holds, state",
                )
            })?;
            write_integer(out, held, INT64)?;
        }
        // Every binary float the standard states is IEEE, and a half
        // widens to a single exactly.
        Scalar::Float16(held) => primitive(out, FLOAT, &held.as_f16().to_f32().to_le_bytes()),
        Scalar::Float32(held) => primitive(out, FLOAT, &held.as_f32().to_le_bytes()),
        Scalar::Float64(held) => primitive(out, DOUBLE, &held.as_f64().to_le_bytes()),
        Scalar::Decimal32(held) => {
            write_decimal(out, i128::from(held.coefficient()), held.scale(), DECIMAL4)?;
        }
        Scalar::Decimal64(held) => {
            write_decimal(out, i128::from(held.coefficient()), held.scale(), DECIMAL8)?;
        }
        Scalar::Decimal128(held) => {
            write_decimal(out, held.coefficient(), held.scale(), DECIMAL16)?
        }
        Scalar::Decimal256(held) => {
            let coefficient = held.coefficient().as_i128().ok_or_else(|| {
                refuse(
                    out.len(),
                    "a decimal256 coefficient past what 128 bits, the widest a variant holds, state",
                )
            })?;
            write_decimal(out, coefficient, held.scale(), DECIMAL16)?;
        }
        // A date is days since the epoch however the value counts them.
        Scalar::Date32(_) | Scalar::Date64(_) => {
            let days = count_at(value, TimeUnit::Day, out.len())?;
            let days = i32::try_from(days)
                .map_err(|_| refuse(out.len(), "a date past what 32 bits of days hold"))?;
            primitive(out, DATE, &days.to_le_bytes());
        }
        Scalar::Time32(held) => {
            naive(&held.timezone(), "time of day", out.len())?;
            let count = count_at(value, TimeUnit::Microsecond, out.len())?;
            primitive(out, TIME_NTZ_MICROS, &count.to_le_bytes());
        }
        Scalar::Time64(held) => {
            naive(&held.timezone(), "time of day", out.len())?;
            let count = count_at(value, TimeUnit::Microsecond, out.len())?;
            primitive(out, TIME_NTZ_MICROS, &count.to_le_bytes());
        }
        // An instant keeps its precision: nanoseconds have their own two
        // types, and every coarser unit is exact in microseconds.
        Scalar::DateTime64(held) => {
            let zoned = !held.timezone().is_naive();
            let (id, count) = if held.unit() == TimeUnit::Nanosecond {
                let id = if zoned {
                    TIMESTAMP_NANOS
                } else {
                    TIMESTAMP_NTZ_NANOS
                };
                (id, held.count())
            } else {
                let id = if zoned {
                    TIMESTAMP_MICROS
                } else {
                    TIMESTAMP_NTZ_MICROS
                };
                (id, count_at(value, TimeUnit::Microsecond, out.len())?)
            };
            primitive(out, id, &count.to_le_bytes());
        }
        // A duration and an interval have no variant type; each takes the
        // spelling the JSON codec gives it, so a reader sees what a JSON
        // reader would.
        Scalar::Duration32(held) => {
            naive(&held.timezone(), "duration", out.len())?;
            write_duration(out, i64::from(held.count()), held.unit())?;
        }
        Scalar::Duration64(held) => {
            naive(&held.timezone(), "duration", out.len())?;
            write_duration(out, held.count(), held.unit())?;
        }
        Scalar::Interval(held) => match held.unit() {
            TimeUnit::YearMonth => primitive(out, INT32, &held.months().to_le_bytes()),
            TimeUnit::DayTime => write_array(
                &[
                    Scalar::from(i64::from(held.days())),
                    Scalar::from(held.nanoseconds() / 1_000_000),
                ],
                keys,
                depth,
                out,
            )?,
            _ => write_array(
                &[
                    Scalar::from(i64::from(held.months())),
                    Scalar::from(i64::from(held.days())),
                    Scalar::from(held.nanoseconds()),
                ],
                keys,
                depth,
                out,
            )?,
        },
        Scalar::String(held) => write_text(out, held.as_str())?,
        Scalar::Bytes(held) => write_binary(out, held.as_bytes())?,
        Scalar::Uuid(held) => primitive(out, UUID, &held.into_bytes()),
        // A geospatial value is its WKB payload, which is bytes.
        Scalar::Geometry(held) => write_binary(out, held.as_bytes())?,
        Scalar::Geography(held) => write_binary(out, held.as_bytes())?,
        Scalar::Version(held) => write_text(out, &held.to_string())?,
        Scalar::Url(held) => write_text(out, &held.to_string())?,
        Scalar::Urn(held) => write_text(out, &held.to_string())?,
        Scalar::Timezone(held) => write_text(out, held.as_str())?,
        Scalar::MimeType(held) => write_text(out, held.as_str())?,
        Scalar::MediaType(held) => write_text(out, &held.to_string())?,
        Scalar::Sequence(held) => write_array(&held.rows(), keys, depth, out)?,
        Scalar::Struct(held) => {
            let entries: Vec<(&str, &Scalar)> = held
                .as_map()
                .iter()
                .map(|(name, child)| (name.as_str(), child))
                .collect();
            write_object(&entries, keys, depth, out)?;
        }
        Scalar::Mapping(held) => {
            let mut entries = BTreeMap::new();
            for (key, child) in held.as_slice() {
                let key = key_text(key)?;
                if entries.insert(key, child).is_some() {
                    return Err(refuse(
                        out.len(),
                        format_smolstr!("a mapping spelling the key {key:?} twice"),
                    ));
                }
            }
            let entries: Vec<(&str, &Scalar)> = entries.into_iter().collect();
            write_object(&entries, keys, depth, out)?;
        }
        // A variant inside a value is the value its bytes hold, restated
        // into this encoding's own dictionary.
        Scalar::Variant(held) => write_value(&held.scalar()?, keys, depth, out)?,
        // A registered code is its text under the standard's string.
        code @ crate::code_scalars!() => {
            write_text(out, code.as_str().expect("a code borrows its text"))?;
        }
    }
    Ok(())
}

/// Append one duration as the ISO-8601 text the JSON codec writes.
fn write_duration(out: &mut Vec<u8>, count: i64, unit: TimeUnit) -> Result<()> {
    match crate::temporal::format_duration(count, unit) {
        Some(text) => write_text(out, &text),
        None => {
            primitive(out, INT64, &count.to_le_bytes());
            Ok(())
        }
    }
}

/// Append one array: the header, the count, the offsets and the values.
fn write_array(items: &[Scalar], keys: &[SmolStr], depth: usize, out: &mut Vec<u8>) -> Result<()> {
    let mut fields = Vec::with_capacity(items.len() * 4);
    let mut offsets = Vec::with_capacity(items.len() + 1);
    for item in items {
        offsets.push(fields.len());
        write_value(item, keys, depth + 1, &mut fields)?;
    }
    offsets.push(bounded(fields.len(), "an array payload")?);
    let width = width_of(fields.len());
    let large = items.len() >= LARGE_FROM;
    out.push(ARRAY | (((u8::from(large) << 2) | (width as u8 - 1)) << 2));
    write_le(out, items.len(), if large { 4 } else { 1 });
    for offset in offsets {
        write_le(out, offset, width);
    }
    out.extend_from_slice(&fields);
    Ok(())
}

/// Append one object: the header, the count, the field ids in key order,
/// the offsets and the values.
fn write_object(
    entries: &[(&str, &Scalar)],
    keys: &[SmolStr],
    depth: usize,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut fields = Vec::with_capacity(entries.len() * 8);
    let mut offsets = Vec::with_capacity(entries.len() + 1);
    let mut ids = Vec::with_capacity(entries.len());
    // The entries arrive sorted by key - a struct holds one map and a
    // mapping was gathered into one - so the ids they name are in the
    // lexicographic order the specification requires.
    for (key, child) in entries {
        let id = keys
            .binary_search_by(|candidate| candidate.as_str().cmp(key))
            .expect("every object key was gathered into the dictionary");
        ids.push(id);
        offsets.push(fields.len());
        write_value(child, keys, depth + 1, &mut fields)?;
    }
    offsets.push(bounded(fields.len(), "an object payload")?);
    let id_width = width_of(ids.iter().copied().max().unwrap_or(0));
    let offset_width = width_of(fields.len());
    let large = entries.len() >= LARGE_FROM;
    let header = (u8::from(large) << 4) | ((id_width as u8 - 1) << 2) | (offset_width as u8 - 1);
    out.push(OBJECT | (header << 2));
    write_le(out, entries.len(), if large { 4 } else { 1 });
    for id in ids {
        write_le(out, id, id_width);
    }
    for offset in offsets {
        write_le(out, offset, offset_width);
    }
    out.extend_from_slice(&fields);
    Ok(())
}

// ------------------------------------------------------------------------
// Decoding.
// ------------------------------------------------------------------------

/// A validated table of child starts and the final end of their value bytes.
struct Segments<'a> {
    packed: &'a [u8],
    width: usize,
    end: usize,
    /// Physical starts in ascending order only when logical order differs.
    physical: Option<Vec<usize>>,
}

impl Segments<'_> {
    fn offset(&self, index: usize) -> usize {
        read_le(self.packed, index * self.width, self.width)
            .expect("a validated segment table holds every offset")
    }

    /// The physical byte range occupied by one logical child.
    fn range(&self, index: usize) -> (usize, usize) {
        let start = self.offset(index);
        let end = match &self.physical {
            None => self.offset(index + 1),
            Some(physical) => {
                let at = physical
                    .binary_search(&start)
                    .expect("every validated logical start is in physical order");
                physical.get(at + 1).copied().unwrap_or(self.end)
            }
        };
        (start, end)
    }
}

/// One pass over a value payload, refusing at the byte it could not read.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    dictionary: &'a Dictionary<'a>,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8> {
        let byte = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| refuse(self.at, "the payload ends before its value does"))?;
        self.at += 1;
        Ok(byte)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                refuse(
                    self.at,
                    format_smolstr!("{count} bytes announced, fewer held"),
                )
            })?;
        let held = &self.bytes[self.at..end];
        self.at = end;
        Ok(held)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut held = [0; N];
        held.copy_from_slice(self.take(N)?);
        Ok(held)
    }

    /// Read an unsigned little-endian integer of `width` bytes.
    fn unsigned(&mut self, width: usize) -> Result<usize> {
        let held = self.take(width)?;
        let mut value = 0_u64;
        for (shift, byte) in held.iter().enumerate() {
            value |= u64::from(*byte) << (shift * 8);
        }
        usize::try_from(value).map_err(|_| refuse(self.at, "a size wider than this platform holds"))
    }

    /// Read a four-byte length and the bytes it counts.
    fn sized(&mut self) -> Result<&'a [u8]> {
        let size = u32::from_le_bytes(self.array()?) as usize;
        self.take(size)
    }

    fn text(&mut self, bytes: &'a [u8]) -> Result<&'a str> {
        std::str::from_utf8(bytes).map_err(|_| refuse(self.at, "text that is not UTF-8"))
    }

    /// Read one value at `depth`.
    fn value(&mut self, depth: usize) -> Result<Scalar> {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            return Err(refuse(
                self.at,
                "a value nested deeper than the parse limit",
            ));
        }
        let metadata = self.byte()?;
        let header = metadata >> 2;
        match metadata & 0x03 {
            SHORT_STRING => {
                let bytes = self.take(usize::from(header))?;
                Ok(Scalar::from(self.text(bytes)?))
            }
            OBJECT => self.object(header, depth),
            ARRAY => self.array_value(header, depth),
            _ => self.primitive(header, depth),
        }
    }

    /// Read one primitive's payload.
    fn primitive(&mut self, id: u8, depth: usize) -> Result<Scalar> {
        Ok(match id {
            NULL => Scalar::Null,
            TRUE => Scalar::from(true),
            FALSE => Scalar::from(false),
            INT8 => Scalar::from(i8::from_le_bytes(self.array()?)),
            INT16 => Scalar::from(i16::from_le_bytes(self.array()?)),
            INT32 => Scalar::from(i32::from_le_bytes(self.array()?)),
            INT64 => Scalar::from(i64::from_le_bytes(self.array()?)),
            DOUBLE => Scalar::from(f64::from_le_bytes(self.array()?)),
            FLOAT => Scalar::from(f32::from_le_bytes(self.array()?)),
            DECIMAL4 => {
                let scale = self.scale()?;
                let coefficient = i32::from_le_bytes(self.array()?);
                validate_decimal(
                    i128::from(coefficient),
                    DECIMAL4_DIGITS,
                    "decimal4",
                    self.at,
                )?;
                Scalar::Decimal32(crate::Decimal32::new(coefficient, scale))
            }
            DECIMAL8 => {
                let scale = self.scale()?;
                let coefficient = i64::from_le_bytes(self.array()?);
                validate_decimal(
                    i128::from(coefficient),
                    DECIMAL8_DIGITS,
                    "decimal8",
                    self.at,
                )?;
                Scalar::Decimal64(crate::Decimal64::new(coefficient, scale))
            }
            DECIMAL16 => {
                let scale = self.scale()?;
                let coefficient = i128::from_le_bytes(self.array()?);
                validate_decimal(coefficient, DECIMAL_DIGITS, "decimal16", self.at)?;
                Scalar::Decimal128(crate::Decimal128::new(coefficient, scale))
            }
            DATE => Scalar::date32(i32::from_le_bytes(self.array()?)),
            TIME_NTZ_MICROS => Scalar::time64(
                i64::from_le_bytes(self.array()?),
                TimeUnit::Microsecond,
                Timezone::NAIVE,
            )?,
            TIMESTAMP_MICROS => Scalar::datetime64(
                i64::from_le_bytes(self.array()?),
                TimeUnit::Microsecond,
                Timezone::UTC,
            )?,
            TIMESTAMP_NTZ_MICROS => Scalar::datetime64(
                i64::from_le_bytes(self.array()?),
                TimeUnit::Microsecond,
                Timezone::NAIVE,
            )?,
            TIMESTAMP_NANOS => Scalar::datetime64(
                i64::from_le_bytes(self.array()?),
                TimeUnit::Nanosecond,
                Timezone::UTC,
            )?,
            TIMESTAMP_NTZ_NANOS => Scalar::datetime64(
                i64::from_le_bytes(self.array()?),
                TimeUnit::Nanosecond,
                Timezone::NAIVE,
            )?,
            BINARY => Scalar::from(self.sized()?),
            STRING => {
                let bytes = self.sized()?;
                Scalar::from(self.text(bytes)?)
            }
            UUID => Scalar::Uuid(crate::Uuid::from_bytes(&self.array::<16>()?)?),
            other => {
                let _ = depth;
                return Err(refuse(
                    self.at,
                    format_smolstr!("primitive type {other} is not one version 1 names"),
                ));
            }
        })
    }

    /// Read one decimal's scale byte.
    fn scale(&mut self) -> Result<i8> {
        let scale = self.byte()?;
        if u32::from(scale) > DECIMAL_DIGITS {
            return Err(refuse(
                self.at,
                format_smolstr!("a scale of {scale}, past the {DECIMAL_DIGITS} a variant holds"),
            ));
        }
        Ok(scale as i8)
    }

    /// Read one array: the count, the offsets and the values.
    fn array_value(&mut self, header: u8, depth: usize) -> Result<Scalar> {
        let width = usize::from(header & 0x03) + 1;
        let count = self.unsigned(if header & 0x04 == 0 { 1 } else { 4 })?;
        let segments = self.segments(count, width)?;
        let start = self.at;
        let end = start + segments.end;
        let values = Scalar::try_sequence(count, |index| {
            let (child_start, child_end) = segments.range(index);
            self.segment(
                start + child_start,
                start + child_end,
                depth + 1,
                index,
                None,
            )
        })?;
        self.at = end;
        Ok(values)
    }

    /// Read one object: the count, the field ids, the offsets and the
    /// values.
    fn object(&mut self, header: u8, depth: usize) -> Result<Scalar> {
        let offset_width = usize::from(header & 0x03) + 1;
        let id_width = usize::from((header >> 2) & 0x03) + 1;
        let count = self.unsigned(if header & 0x10 == 0 { 1 } else { 4 })?;
        let ids_at = self.at;
        let ids_len = count
            .checked_mul(id_width)
            .ok_or_else(|| refuse(ids_at, "an object field-id table that is too large"))?;
        let ids = self.take(ids_len)?;
        let id = |index: usize| {
            read_le(ids, index * id_width, id_width)
                .expect("the complete field-id table was borrowed above")
        };
        let mut previous: Option<&str> = None;
        for index in 0..count {
            let key = self.dictionary.key(id(index), ids_at + index * id_width)?;
            if previous.is_some_and(|previous| previous >= key) {
                return Err(refuse(
                    ids_at + index * id_width,
                    format_smolstr!(
                        "object fields are not in unsigned UTF-8 name order at {key:?}"
                    ),
                ));
            }
            previous = Some(key);
        }
        let segments = self.segments(count, offset_width)?;
        let start = self.at;
        if count == 0 {
            self.at = start;
            return Scalar::from_struct(std::iter::empty::<(SmolStr, Scalar)>());
        }
        let mut entries = BTreeMap::new();
        for index in 0..count {
            let key = SmolStr::new(self.dictionary.key(id(index), ids_at + index * id_width)?);
            let (child_start, child_end) = segments.range(index);
            let value = self.segment(
                start + child_start,
                start + child_end,
                depth + 1,
                index,
                Some(key.as_str()),
            )?;
            entries.insert(key, value);
        }
        self.at = start + segments.end;
        Ok(Scalar::Struct(Struct::new(Arc::new(entries))))
    }

    /// Read and validate `count + 1` child offsets, each of `width` bytes.
    ///
    /// The offsets are relative to the first value, and the last one is
    /// the end of the last: a value out of that range is refused here
    /// rather than read from whatever follows the payload.
    fn segments(&mut self, count: usize, width: usize) -> Result<Segments<'a>> {
        let table_at = self.at;
        let table_len = count
            .checked_add(1)
            .and_then(|count| count.checked_mul(width))
            .ok_or_else(|| refuse(table_at, "a value-offset table that is too large"))?;
        let packed = self.take(table_len)?;
        let offset = |index: usize| {
            read_le(packed, index * width, width)
                .expect("the complete value-offset table was borrowed above")
        };
        let end = offset(count);
        if self
            .at
            .checked_add(end)
            .is_none_or(|last| last > self.bytes.len())
        {
            return Err(refuse(
                self.at,
                format_smolstr!(
                    "{end} value bytes announced, {} held",
                    self.bytes.len() - self.at
                ),
            ));
        }
        if count == 0 {
            if end != 0 {
                return Err(refuse(
                    table_at,
                    "an empty container whose final offset is not zero",
                ));
            }
            return Ok(Segments {
                packed,
                width,
                end,
                physical: None,
            });
        }

        let mut ordered = true;
        let mut previous = offset(0);
        for index in 0..count {
            let current = offset(index);
            if current >= end {
                return Err(refuse(
                    table_at + index * width,
                    "a child offset at or past the end of the values",
                ));
            }
            if index > 0 && current <= previous {
                ordered = false;
            }
            previous = current;
        }

        let physical = if ordered {
            if offset(0) != 0 {
                return Err(refuse(
                    table_at,
                    "the value bytes start before the first offset",
                ));
            }
            None
        } else {
            let mut physical = (0..count).map(offset).collect::<Vec<_>>();
            physical.sort_unstable();
            if physical[0] != 0 {
                return Err(refuse(
                    table_at,
                    "the value bytes start before the first offset",
                ));
            }
            if physical.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(refuse(
                    table_at,
                    "two children have the same physical offset",
                ));
            }
            Some(physical)
        };
        Ok(Segments {
            packed,
            width,
            end,
            physical,
        })
    }

    /// Decode exactly the bytes one child segment grants to its value.
    fn segment(
        &mut self,
        start: usize,
        end: usize,
        depth: usize,
        index: usize,
        field: Option<&str>,
    ) -> Result<Scalar> {
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or_else(|| refuse(start, "a child segment outside the value payload"))?;
        let mut child = Self {
            bytes,
            at: 0,
            dictionary: self.dictionary,
        };
        let value = child.value(depth).map_err(|error| match error {
            Error::Codec {
                format,
                position,
                reason,
            } => Error::Codec {
                format,
                position: start + position,
                reason: match field {
                    Some(name) => format_smolstr!("object field {name:?}: {reason}"),
                    None => format_smolstr!("array entry {index}: {reason}"),
                },
            },
            other => other,
        })?;
        if child.at != bytes.len() {
            return Err(refuse(
                start + child.at,
                match field {
                    Some(name) => format_smolstr!(
                        "object field {name:?} does not fill its declared physical segment"
                    ),
                    None => format_smolstr!(
                        "array entry {index} does not fill its declared physical segment"
                    ),
                },
            ));
        }
        self.at = end;
        Ok(value)
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the two binaries the canonical extension type lays out.
// ------------------------------------------------------------------------

/// The canonical Arrow extension name of the variant type.
///
/// The storage is a struct of the two binaries a [`Variant`] is, and the
/// extension metadata is the empty string. The name is Arrow's own
/// [Parquet Variant][canonical] canonical extension, so a column written
/// here is one an Arrow reader recognizes without a private agreement.
///
/// [canonical]: https://arrow.apache.org/docs/format/CanonicalExtensions.html#parquet-variant
pub const VARIANT_EXTENSION_NAME: &str = "arrow.parquet.variant";

/// The name of the child holding the key dictionary.
pub const VARIANT_METADATA_FIELD: &str = "metadata";

/// The name of the child holding the value payload.
pub const VARIANT_VALUE_FIELD: &str = "value";

mod arrow {
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};

    use super::{VARIANT_METADATA_FIELD, VARIANT_VALUE_FIELD};
    use crate::VariantType;

    impl VariantType {
        /// The Arrow storage a variant column lays out: a struct of two
        /// required binaries, `metadata` and `value`, under the
        /// [`VARIANT_EXTENSION_NAME`](super::VARIANT_EXTENSION_NAME)
        /// extension name.
        ///
        /// It is the shape Parquet, Avro, ORC and Iceberg all state for a
        /// variant column, so the projection is the same two children
        /// whichever medium the column lands in. A row holding no variant
        /// at all is the struct's own null; a row holding the variant null
        /// is a present struct whose value payload is the null byte.
        pub(crate) fn arrow_storage() -> ArrowDataType {
            ArrowDataType::Struct(variant_fields())
        }
    }

    /// The two children of the variant storage, in specification order.
    pub(crate) fn variant_fields() -> Fields {
        Fields::from(vec![
            Arc::new(ArrowField::new(
                VARIANT_METADATA_FIELD,
                ArrowDataType::Binary,
                false,
            )),
            Arc::new(ArrowField::new(
                VARIANT_VALUE_FIELD,
                ArrowDataType::Binary,
                false,
            )),
        ])
    }

    /// Reports whether an Arrow datatype is the variant storage.
    ///
    /// The two children are named, not positional, and either may be laid
    /// out as any of Arrow's three binary layouts - a foreign writer may
    /// have chosen `BinaryView` or `LargeBinary` - because the extension
    /// names the struct, never the layout inside it. A shredded column is
    /// this storage plus a `typed_value` child: its unshredded rows read
    /// as themselves, and a row that stored its value in `typed_value`
    /// refuses by name rather than reading as a different value.
    pub(crate) fn is_variant_storage(dtype: &ArrowDataType) -> bool {
        let ArrowDataType::Struct(fields) = dtype else {
            return false;
        };
        let binary = |name: &str, required: bool| {
            let mut named = fields.iter().filter(|field| field.name() == name);
            let Some(field) = named.next() else {
                return false;
            };
            named.next().is_none()
                && (!required || !field.is_nullable())
                && matches!(
                    field.data_type(),
                    ArrowDataType::Binary | ArrowDataType::LargeBinary | ArrowDataType::BinaryView
                )
        };
        binary(VARIANT_METADATA_FIELD, true) && binary(VARIANT_VALUE_FIELD, false)
    }
}

pub(crate) use arrow::{is_variant_storage, variant_fields};

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/variant.rs` pins and a caller cannot reach.
    //!
    //! The header bytes of the Variant binary encoding are private: a caller
    //! sees the [`Variant`](super::Variant) pair and the value it reads back,
    //! never the type byte that says how it was written. The pins are on the
    //! bytes themselves, so they name the constants, each restated here so
    //! nothing in the encoder becomes more public than it was.

    /// The basic type of a header byte that carries its own payload.
    pub const PRIMITIVE: u8 = super::PRIMITIVE;
    /// The basic type of a string short enough to fold into its header.
    pub const SHORT_STRING: u8 = super::SHORT_STRING;
    /// The basic type of a keyed object.
    pub const OBJECT: u8 = super::OBJECT;
    /// The primitive type of a four-byte signed integer.
    pub const INT32: u8 = super::INT32;
    /// The primitive type of a length-prefixed string.
    pub const STRING: u8 = super::STRING;
    /// The longest string whose length folds into its header byte.
    pub const SHORT_STRING_MAX: usize = super::SHORT_STRING_MAX;
}
