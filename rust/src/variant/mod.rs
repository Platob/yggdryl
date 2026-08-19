//! The Parquet Variant binary encoding of the one [`Value`](crate::Value) tree.
//!
//! A Variant is the self-describing semi-structured value Parquet, Iceberg v3,
//! and Doris exchange as two byte strings: a `metadata` dictionary of the
//! field names an object uses, and a `value` spelling the tree itself. The
//! project's [`Value`](crate::Value) already *is* a self-describing tree, so
//! this module is an encoding of that one value model - never a second value
//! model - implemented against the Parquet-format specification
//! (`VariantEncoding.md`) with no dependency, the way [`crate::avro`] and
//! [`crate::generic::wkb`] read their formats.
//!
//! [`encode_value`] returns the `(metadata, value)` pair for one tree;
//! [`decode_value`] reads the pair back. Decoding accepts everything the
//! specification permits a writer - unsorted dictionaries, oversized offset
//! widths, `is_large` on a small container - while encoding always writes the
//! canonical compact form: a sorted, deduplicated dictionary and the smallest
//! offset, id, and count widths that hold the data.
//!
//! ```
//! use yggdryl::{Value, variant};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let quote = Value::from_mapping([
//!     (Value::from("symbol"), Value::from("AAPL")),
//!     (Value::from("size"), Value::from(100_i64)),
//! ])?;
//!
//! let (metadata, value) = variant::encode_value(&quote)?;
//! let decoded = variant::decode_value(&metadata, &value)?;
//!
//! // Variant objects are unordered: fields come back in dictionary order.
//! assert_eq!(decoded.get_key_str("symbol"), quote.get_key_str("symbol"));
//! assert_eq!(decoded.get_key_str("size"), quote.get_key_str("size"));
//! # Ok(())
//! # }
//! ```
//!
//! # How each `Value` kind is spelled
//!
//! | `Value` variant | Variant spelling | Round trip |
//! |---|---|---|
//! | `Null` | primitive `null` (0) | identical |
//! | `Bool` | primitive `true`/`false` (1/2) | identical |
//! | `I8`/`I16`/`I32`/`I64` | `int8`/`int16`/`int32`/`int64` (3-6) | identical |
//! | `U8`/`U16`/`U32` | the next signed width: `int16`/`int32`/`int64` | comes back signed, same value |
//! | `U64`/`I128`/`U128` | `int64` when the value fits `i64`, refused otherwise | comes back as `I64` |
//! | `F32` | primitive `float` (14) | identical |
//! | `F64` | primitive `double` (7) | identical |
//! | `Decimal` | `decimal4`/`decimal8`/`decimal16` (8-10) by unscaled width | identical |
//! | `String` | short string when under 64 bytes, `string` (16) otherwise | identical |
//! | `Bytes` | primitive `binary` (15) | identical |
//! | `Date` | primitive `date` (11) | identical |
//! | `Timestamp` (UTC) | `timestamp` micros (12) or nanos (18); seconds and millis widen to micros | instant preserved |
//! | `DateTime` | ntz `timestamp` micros (13) or nanos (19); seconds and millis widen to micros | reading preserved |
//! | `Time` | `time` (17), always microseconds; whole-micro nanos divide down | value preserved |
//! | `Sequence` | array | identical |
//! | `Mapping` (string keys) | object | fields in dictionary order |
//! | `Record` | object over its field names | comes back as a `Mapping` |
//!
//! Integer widening stays inside the integer kind, which the workspace already
//! calls lossless; crossing kinds - a `u64` respelled as a scale-zero decimal -
//! would come back as a different kind of number, so it is refused instead.
//!
//! # What is refused, by name
//!
//! Encoding refuses - with the `$`-rooted path of the offending node - every
//! value the format cannot carry back:
//!
//! - `Duration`: no Variant primitive holds an elapsed count.
//! - `Geospatial`: spelling WKB as the `binary` primitive would decode as
//!   plain bytes, silently dropping the geospatial reading.
//! - A `Timestamp` in any zone but UTC: the Variant instant is UTC-adjusted
//!   and carries no zone name, so any other zone could not come back.
//! - `U64`/`I128`/`U128` values beyond `i64`: no wider Variant integer exists.
//! - A `Decimal` whose scale is outside 0..=38 or whose unscaled value needs
//!   more than 38 digits: outside the specification's decimal range.
//! - A second-or-millisecond count whose microsecond widening overflows, and
//!   a nanosecond `Time` that is not a whole number of microseconds.
//! - A `Mapping` key that is not a string: object keys are dictionary strings.
//!
//! # What decoding hands back
//!
//! - `uuid` (20) decodes as its sixteen big-endian bytes - `Value::Bytes` -
//!   because the tree has no UUID kind and the workspace already spells a UUID
//!   over `fixed[16]`. Re-encoding writes those bytes as `binary`.
//! - Both `timestamp with time zone` spellings decode with the zone
//!   [`Timezone::UTC`](crate::Timezone::UTC), which is the only zone the
//!   format can state.
//! - Objects decode as [`Value::Mapping`](crate::Value) in the sorted field
//!   order the specification mandates on ids and offsets; an object is
//!   unordered by definition, so no insertion order exists to preserve.
//! - A primitive type id this revision does not define (21 and above) is
//!   refused with the id and its byte position, as is a metadata version
//!   other than 1.
//!
//! # Named non-goal: shredding
//!
//! The companion `VariantShredding.md` specification - extracting typed
//! subcolumns beside the binary pair - is a storage-layout concern of the
//! Parquet and Iceberg writers, not of the byte encoding, and is deliberately
//! not implemented here.

mod decode;
mod encode;
mod metadata;

pub use decode::{decode_value, decode_value_with_limits};
pub use encode::encode_value;

use smol_str::SmolStr;

use crate::Error;

/// The basic-type bits of a value-metadata byte: a primitive value.
const BASIC_PRIMITIVE: u8 = 0;
/// The basic-type bits of a value-metadata byte: a short string.
const BASIC_SHORT_STRING: u8 = 1;
/// The basic-type bits of a value-metadata byte: an object.
const BASIC_OBJECT: u8 = 2;
/// The basic-type bits of a value-metadata byte: an array.
const BASIC_ARRAY: u8 = 3;

/// Primitive type id: the null value, no value data.
const PRIMITIVE_NULL: u8 = 0;
/// Primitive type id: boolean true, no value data.
const PRIMITIVE_TRUE: u8 = 1;
/// Primitive type id: boolean false, no value data.
const PRIMITIVE_FALSE: u8 = 2;
/// Primitive type id: one signed byte.
const PRIMITIVE_INT8: u8 = 3;
/// Primitive type id: a two-byte little-endian signed integer.
const PRIMITIVE_INT16: u8 = 4;
/// Primitive type id: a four-byte little-endian signed integer.
const PRIMITIVE_INT32: u8 = 5;
/// Primitive type id: an eight-byte little-endian signed integer.
const PRIMITIVE_INT64: u8 = 6;
/// Primitive type id: an IEEE little-endian double.
const PRIMITIVE_DOUBLE: u8 = 7;
/// Primitive type id: a scale byte then a four-byte unscaled value.
const PRIMITIVE_DECIMAL4: u8 = 8;
/// Primitive type id: a scale byte then an eight-byte unscaled value.
const PRIMITIVE_DECIMAL8: u8 = 9;
/// Primitive type id: a scale byte then a sixteen-byte unscaled value.
const PRIMITIVE_DECIMAL16: u8 = 10;
/// Primitive type id: days since the epoch, four bytes little-endian.
const PRIMITIVE_DATE: u8 = 11;
/// Primitive type id: UTC-adjusted microseconds since the epoch.
const PRIMITIVE_TIMESTAMP_MICROS: u8 = 12;
/// Primitive type id: naive microseconds since the epoch.
const PRIMITIVE_TIMESTAMP_NTZ_MICROS: u8 = 13;
/// Primitive type id: an IEEE little-endian float.
const PRIMITIVE_FLOAT: u8 = 14;
/// Primitive type id: a four-byte size then that many bytes.
const PRIMITIVE_BINARY: u8 = 15;
/// Primitive type id: a four-byte size then UTF-8 bytes.
const PRIMITIVE_STRING: u8 = 16;
/// Primitive type id: naive microseconds since midnight.
const PRIMITIVE_TIME_MICROS: u8 = 17;
/// Primitive type id: UTC-adjusted nanoseconds since the epoch.
const PRIMITIVE_TIMESTAMP_NANOS: u8 = 18;
/// Primitive type id: naive nanoseconds since the epoch.
const PRIMITIVE_TIMESTAMP_NTZ_NANOS: u8 = 19;
/// Primitive type id: sixteen big-endian UUID bytes.
const PRIMITIVE_UUID: u8 = 20;
/// The largest primitive type id this specification revision defines.
const PRIMITIVE_MAX: u8 = PRIMITIVE_UUID;

/// The metadata version this implementation reads and writes.
const METADATA_VERSION: u8 = 1;
/// The byte length below which a string folds into the short-string form.
const SHORT_STRING_LIMIT: usize = 64;
/// The largest unscaled decimal magnitude the specification's 38-digit
/// precision cap admits: `10^38`, exclusive.
const DECIMAL_MAGNITUDE_LIMIT: i128 = 100_000_000_000_000_000_000_000_000_000_000_000_000;

/// Report a malformed Variant buffer at a byte position.
fn codec(position: usize, reason: SmolStr) -> Error {
    Error::Codec {
        format: "variant",
        position,
        reason,
    }
}

/// The unit word for a byte count, so a one-byte message reads as prose.
const fn byte_word(count: usize) -> &'static str {
    if count == 1 { "byte" } else { "bytes" }
}

/// The fewest bytes that hold `largest` as an unsigned little-endian value:
/// the width encoders pick for dictionary offsets, field ids, and field
/// offsets, from one to four bytes.
const fn byte_width(largest: u32) -> usize {
    if largest <= 0xFF {
        1
    } else if largest <= 0xFFFF {
        2
    } else if largest <= 0xFF_FFFF {
        3
    } else {
        4
    }
}

/// Append `value` as `width` little-endian bytes.
fn push_unsigned(target: &mut Vec<u8>, value: u32, width: usize) {
    target.extend_from_slice(&value.to_le_bytes()[..width]);
}

#[cfg(test)]
mod tests;
