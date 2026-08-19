//! The one value every part of the project speaks.
//!
//! A [`Value`] is any native value: null, a boolean, a signed or unsigned
//! integer at every width from 8 to 128 bits, a float at 32 or 64 bits, an
//! exact decimal, a string, bytes, one of the five temporals, an ordered
//! sequence, or an insertion-ordered mapping with arbitrary keys. It is what [`crate::json`], [`crate::yaml`], and
//! [`crate::toml`] parse into and write from, what a [`crate::Field`] validates
//! and canonicalizes, and what the language bindings convert their own objects
//! into - so a value crosses every boundary in the project without being
//! re-modelled on the way.
//!
//! Every kind that carries a unit or a scale carries it as a typed field rather
//! than as a free-form name over an untyped payload, because a name nothing
//! validates is not a type. [`Value::data_type`] reads the datatype straight
//! off the variant for exactly that reason. There is deliberately no `Variant`
//! kind: a variant value is a `Value` - a self-describing tree - so the binary
//! form is an encoding of the one value model, not a second value model.
//!
//! ```
//! use yggdryl::Value;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let quote = Value::from_mapping([
//!     (Value::from("symbol"), Value::from("AAPL")),
//!     (Value::from("price"), Value::from(12.5)),
//! ])?;
//!
//! assert_eq!(quote.get_key_str("symbol").and_then(Value::as_str), Some("AAPL"));
//! assert_eq!(quote.len(), 2);
//! # Ok(())
//! # }
//! ```

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::sync::{Arc, OnceLock};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, TimeUnit, Timezone};

use super::decimal;
use super::temporal::temporal_key;

/// A bit-preserving, totally ordered 64-bit floating-point value.
///
/// All NaN payloads are normalized at construction. Positive and negative
/// zero remain distinct so codecs can round-trip their exact representation.
#[derive(Clone, Copy, Default)]
pub struct Float(u64);

impl Float {
    /// Construct from a native `f64`.
    pub fn from_f64(value: f64) -> Self {
        if value.is_nan() {
            Self(f64::NAN.to_bits())
        } else {
            Self(value.to_bits())
        }
    }

    /// Return the native `f64` value.
    pub const fn as_f64(self) -> f64 {
        f64::from_bits(self.0)
    }

    /// Consume and return the native `f64` value.
    pub const fn into_f64(self) -> f64 {
        self.as_f64()
    }
}

impl fmt::Debug for Float {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f64().fmt(formatter)
    }
}

impl fmt::Display for Float {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f64().fmt(formatter)
    }
}

impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Float {}

impl PartialOrd for Float {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_f64().total_cmp(&other.as_f64())
    }
}

impl Hash for Float {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<f64> for Float {
    fn from(value: f64) -> Self {
        Self::from_f64(value)
    }
}

impl From<Float> for f64 {
    fn from(value: Float) -> Self {
        value.into_f64()
    }
}

impl Serialize for Float {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct Bits(u64);

        impl fmt::Display for Bits {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:016x}", self.0)
            }
        }

        serializer.collect_str(&Bits(self.0))
    }
}

impl<'de> Deserialize<'de> for Float {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FloatVisitor;

        impl<'de> serde::de::Visitor<'de> for FloatVisitor {
            type Value = Float;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact 0x-prefixed f64 bit string or floating number")
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Ok(Float::from_f64(value))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(Float::from_f64(value as f64))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Float::from_f64(value as f64))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let digits = value
                    .strip_prefix("0x")
                    .ok_or_else(|| E::custom("float bits must start with 0x"))?;
                if digits.len() != 16 {
                    return Err(E::custom("float bits must contain exactly 16 hex digits"));
                }
                let bits = u64::from_str_radix(digits, 16)
                    .map_err(|_| E::custom("float bits contain invalid hex"))?;
                Ok(Float::from_f64(f64::from_bits(bits)))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(FloatVisitor)
    }
}

/// A bit-preserving, totally ordered 32-bit floating-point value.
///
/// The narrow sibling of [`Float`], for a value that arrived through a
/// `Float32` column and must leave through one: widening it to 64 bits would
/// erase which width the column declared. All NaN payloads are normalized at
/// construction; the two zeros remain distinct.
#[derive(Clone, Copy, Default)]
pub struct Float32(u32);

impl Float32 {
    /// Construct from a native `f32`.
    pub fn from_f32(value: f32) -> Self {
        if value.is_nan() {
            Self(f32::NAN.to_bits())
        } else {
            Self(value.to_bits())
        }
    }

    /// Return the native `f32` value.
    pub const fn as_f32(self) -> f32 {
        f32::from_bits(self.0)
    }

    /// Return the value widened to `f64`, which is exact for every `f32`.
    pub const fn as_f64(self) -> f64 {
        self.as_f32() as f64
    }
}

impl fmt::Debug for Float32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f32().fmt(formatter)
    }
}

impl fmt::Display for Float32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_f32().fmt(formatter)
    }
}

impl PartialEq for Float32 {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Float32 {}

impl PartialOrd for Float32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Float32 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_f32().total_cmp(&other.as_f32())
    }
}

impl Hash for Float32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<f32> for Float32 {
    fn from(value: f32) -> Self {
        Self::from_f32(value)
    }
}

impl From<Float32> for f32 {
    fn from(value: Float32) -> Self {
        value.as_f32()
    }
}

impl Serialize for Float32 {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        struct Bits(u32);

        impl fmt::Display for Bits {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "0x{:08x}", self.0)
            }
        }

        serializer.collect_str(&Bits(self.0))
    }
}

impl<'de> Deserialize<'de> for Float32 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Float32Visitor;

        impl serde::de::Visitor<'_> for Float32Visitor {
            type Value = Float32;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an exact 0x-prefixed f32 bit string or floating number")
            }

            fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
                Ok(Float32::from_f32(value as f32))
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(Float32::from_f32(value as f32))
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
                Ok(Float32::from_f32(value as f32))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let digits = value
                    .strip_prefix("0x")
                    .ok_or_else(|| E::custom("float bits must start with 0x"))?;
                if digits.len() != 8 {
                    return Err(E::custom("f32 bits must contain exactly 8 hex digits"));
                }
                let bits = u32::from_str_radix(digits, 16)
                    .map_err(|_| E::custom("float bits contain invalid hex"))?;
                Ok(Float32::from_f32(f32::from_bits(bits)))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(Float32Visitor)
    }
}

/// A shared, deterministic structured-data value spanning JSON and YAML.
#[derive(Clone, Debug)]
pub enum Value {
    /// The null value.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed 8-bit integer.
    I8(i8),
    /// A signed 16-bit integer.
    I16(i16),
    /// A signed 32-bit integer.
    I32(i32),
    /// A signed 64-bit integer.
    I64(i64),
    /// An unsigned 8-bit integer.
    U8(u8),
    /// An unsigned 16-bit integer.
    U16(u16),
    /// An unsigned 32-bit integer.
    U32(u32),
    /// An unsigned 64-bit integer.
    U64(u64),
    /// A signed 128-bit integer.
    I128(i128),
    /// An unsigned 128-bit integer.
    U128(u128),
    /// A 32-bit floating-point value.
    F32(Float32),
    /// A 64-bit floating-point value.
    F64(Float),
    /// An exact decimal: an unscaled integer and the power of ten it divides by.
    ///
    /// A decimal is stored the way Arrow stores one - the coefficient and the
    /// scale, never a float - because `0.1` has no exact binary expansion, so a
    /// price that arrives as `0.1` must leave as `0.1` and not as the nearest
    /// double. The pair also carries the scale itself, which is data: `1.50`
    /// and `1.5` are the same number written to different precision, and a
    /// schema that declares scale 2 needs the first spelling back.
    Decimal(i128, i8),
    /// A Unicode string.
    String(SmolStr),
    /// Arbitrary bytes.
    Bytes(Arc<[u8]>),
    /// A geometry or geography value, as Well-Known Binary.
    ///
    /// WKB rather than a parsed coordinate tree because WKB is what every
    /// geospatial encoding stores and exchanges; [`crate::generic::wkb`] reads
    /// the bytes wherever a text spelling or a bound is needed.
    Geospatial(Arc<[u8]>),
    /// A count of days since the Unix epoch.
    Date(i32),
    /// A count of `TimeUnit` since midnight.
    Time(i64, TimeUnit),
    /// A count of `TimeUnit` since the Unix epoch, in a required zone.
    ///
    /// The zone is required because "no zone" is not a display detail of an
    /// instant - it is a different kind of reading, the naive wall clock, and
    /// that kind is [`Self::DateTime`]. Splitting them keeps every timestamp
    /// an instant someone can convert, and every naive reading honestly naive.
    Timestamp(i64, TimeUnit, Timezone),
    /// A naive wall-clock reading: a count of `TimeUnit` since the epoch, in
    /// no zone at all - what `DataType::Timestamp(unit, None)` stores.
    DateTime(i64, TimeUnit),
    /// An elapsed count of `TimeUnit`.
    Duration(i64, TimeUnit),
    /// An ordered sequence.
    Sequence(Arc<[Value]>),
    /// An insertion-ordered mapping with arbitrary unique keys.
    Mapping(Arc<[(Value, Value)]>),
    /// A typed row: a struct datatype and one value per field, in field order.
    ///
    /// The datatype is the schema half a [`Self::Mapping`] does not carry: the
    /// field names, their declared types, and their order. The values sit in
    /// exactly that order, so the pair round-trips a row without consulting
    /// anything outside itself. Text formats spell a record as the mapping of
    /// its field names to its values, which is what a record *is* in a format
    /// that has no schema of its own.
    Record(Arc<DataType>, Arc<[Value]>),
}

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        /// Two payload halves spelled as the array a tuple variant carries.
        struct Pair<'a, A: Serialize, B: Serialize>(&'a A, &'a B);
        impl<A: Serialize, B: Serialize> Serialize for Pair<'_, A, B> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeTuple;
                let mut tuple = serializer.serialize_tuple(2)?;
                tuple.serialize_element(self.0)?;
                tuple.serialize_element(self.1)?;
                tuple.end()
            }
        }

        /// Three payload halves spelled as the array a tuple variant carries.
        struct Triple<'a, A: Serialize, B: Serialize, C: Serialize>(&'a A, &'a B, &'a C);
        impl<A: Serialize, B: Serialize, C: Serialize> Serialize for Triple<'_, A, B, C> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeTuple;
                let mut tuple = serializer.serialize_tuple(3)?;
                tuple.serialize_element(self.0)?;
                tuple.serialize_element(self.1)?;
                tuple.serialize_element(self.2)?;
                tuple.end()
            }
        }

        // The layout is the adjacently tagged {"type", "value"} document the
        // derive produced before `Record` arrived - a unit variant carries the
        // tag alone - and `Record` spells its datatype as the canonical
        // string, so the wire never needs a serde implementation on the type
        // tree itself.
        fn tagged<S: serde::Serializer, T: Serialize>(
            serializer: S,
            tag: &'static str,
            value: &T,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut document = serializer.serialize_struct("Value", 2)?;
            document.serialize_field("type", tag)?;
            document.serialize_field("value", value)?;
            document.end()
        }

        match self {
            Self::Null => {
                let mut document = serializer.serialize_struct("Value", 1)?;
                document.serialize_field("type", "null")?;
                document.end()
            }
            Self::Bool(value) => tagged(serializer, "bool", value),
            Self::I8(value) => tagged(serializer, "i8", value),
            Self::I16(value) => tagged(serializer, "i16", value),
            Self::I32(value) => tagged(serializer, "i32", value),
            Self::I64(value) => tagged(serializer, "i64", value),
            Self::U8(value) => tagged(serializer, "u8", value),
            Self::U16(value) => tagged(serializer, "u16", value),
            Self::U32(value) => tagged(serializer, "u32", value),
            Self::U64(value) => tagged(serializer, "u64", value),
            Self::I128(value) => tagged(serializer, "i128", value),
            Self::U128(value) => tagged(serializer, "u128", value),
            Self::F32(value) => tagged(serializer, "f32", value),
            Self::F64(value) => tagged(serializer, "f64", value),
            Self::Decimal(unscaled, scale) => tagged(serializer, "decimal", &Pair(unscaled, scale)),
            Self::String(value) => tagged(serializer, "string", value),
            Self::Bytes(value) => tagged(serializer, "bytes", value),
            Self::Geospatial(value) => tagged(serializer, "geospatial", value),
            // A temporal is its classic ISO spelling wherever it has one; a
            // reading with no classic spelling keeps its structural parts.
            Self::Date(days) => match super::iso::format_date(*days) {
                Some(spelled) => tagged(serializer, "date", &spelled),
                None => tagged(serializer, "date", days),
            },
            Self::Time(count, unit) => match super::iso::format_time(*count, *unit) {
                Some(spelled) => tagged(serializer, "time", &spelled),
                None => tagged(serializer, "time", &Pair(count, unit)),
            },
            Self::Timestamp(count, unit, zone) => {
                match super::iso::format_timestamp(*count, *unit, zone) {
                    Some(spelled) => tagged(serializer, "timestamp", &spelled),
                    None => tagged(serializer, "timestamp", &Triple(count, unit, zone)),
                }
            }
            Self::DateTime(count, unit) => match super::iso::format_datetime(*count, *unit) {
                Some(spelled) => tagged(serializer, "datetime", &spelled),
                None => tagged(serializer, "datetime", &Pair(count, unit)),
            },
            Self::Duration(count, unit) => match super::iso::format_duration(*count, *unit) {
                Some(spelled) => tagged(serializer, "duration", &spelled),
                None => tagged(serializer, "duration", &Pair(count, unit)),
            },
            Self::Sequence(values) => tagged(serializer, "sequence", &values.as_ref()),
            Self::Mapping(entries) => tagged(serializer, "mapping", &entries.as_ref()),
            Self::Record(data_type, values) => tagged(
                serializer,
                "record",
                &Pair(&data_type.to_string(), &values.as_ref()),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        /// A temporal payload: the classic ISO string, or the structural pair
        /// a reading with no classic spelling keeps.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum CountPayload {
            Iso(SmolStr),
            Pair(i64, TimeUnit),
        }

        /// A date payload: the ISO string, or the raw day count.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum DatePayload {
            Iso(SmolStr),
            Days(i32),
        }

        /// A timestamp payload: the ISO string, or the structural triple. A
        /// triple whose zone is null is the naive reading older documents
        /// wrote, and reads back as the datetime it always was.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StampPayload {
            Iso(SmolStr),
            Triple(i64, TimeUnit, Option<Timezone>),
        }

        // This mirror must stay variant-for-variant identical to `Value`: a
        // variant missing here is not a compile error, it is a variant serde
        // silently refuses to read back.
        #[derive(Deserialize)]
        #[serde(tag = "type", content = "value", rename_all = "snake_case")]
        enum StructuralValue {
            Null,
            Bool(bool),
            I8(i8),
            I16(i16),
            I32(i32),
            I64(i64),
            U8(u8),
            U16(u16),
            U32(u32),
            U64(u64),
            I128(i128),
            U128(u128),
            F32(Float32),
            #[serde(alias = "float")]
            F64(Float),
            Decimal(i128, i8),
            String(SmolStr),
            Bytes(Arc<[u8]>),
            Geospatial(Arc<[u8]>),
            Date(DatePayload),
            Time(CountPayload),
            Timestamp(StampPayload),
            #[serde(rename = "datetime")]
            DateTime(CountPayload),
            Duration(CountPayload),
            Sequence(Vec<Value>),
            Mapping(Vec<(Value, Value)>),
            Record(SmolStr, Vec<Value>),
        }

        match StructuralValue::deserialize(deserializer)? {
            StructuralValue::Null => Ok(Self::Null),
            StructuralValue::Bool(value) => Ok(Self::Bool(value)),
            StructuralValue::I8(value) => Ok(Self::I8(value)),
            StructuralValue::I16(value) => Ok(Self::I16(value)),
            StructuralValue::I32(value) => Ok(Self::I32(value)),
            StructuralValue::I64(value) => Ok(Self::I64(value)),
            StructuralValue::U8(value) => Ok(Self::U8(value)),
            StructuralValue::U16(value) => Ok(Self::U16(value)),
            StructuralValue::U32(value) => Ok(Self::U32(value)),
            StructuralValue::U64(value) => Ok(Self::U64(value)),
            StructuralValue::I128(value) => Ok(Self::I128(value)),
            StructuralValue::U128(value) => Ok(Self::U128(value)),
            StructuralValue::F32(value) => Ok(Self::F32(value)),
            StructuralValue::F64(value) => Ok(Self::F64(value)),
            StructuralValue::Decimal(unscaled, scale) => Ok(Self::Decimal(unscaled, scale)),
            StructuralValue::String(value) => Ok(Self::String(value)),
            StructuralValue::Bytes(value) => Ok(Self::from(value)),
            StructuralValue::Geospatial(value) => Ok(Self::Geospatial(value)),
            StructuralValue::Date(DatePayload::Days(days)) => Ok(Self::Date(days)),
            StructuralValue::Date(DatePayload::Iso(spelled)) => super::iso::parse_date(&spelled)
                .map(Self::Date)
                .map_err(D::Error::custom),
            StructuralValue::Time(CountPayload::Pair(count, unit)) => Ok(Self::Time(count, unit)),
            StructuralValue::Time(CountPayload::Iso(spelled)) => super::iso::parse_time(&spelled)
                .map(|(count, unit)| Self::Time(count, unit))
                .map_err(D::Error::custom),
            StructuralValue::Timestamp(StampPayload::Triple(count, unit, Some(zone))) => {
                Ok(Self::Timestamp(count, unit, zone))
            }
            StructuralValue::Timestamp(StampPayload::Triple(count, unit, None)) => {
                Ok(Self::DateTime(count, unit))
            }
            StructuralValue::Timestamp(StampPayload::Iso(spelled)) => {
                super::iso::parse_timestamp(&spelled)
                    .map(|(count, unit, zone)| Self::Timestamp(count, unit, zone))
                    .map_err(D::Error::custom)
            }
            StructuralValue::DateTime(CountPayload::Pair(count, unit)) => {
                Ok(Self::DateTime(count, unit))
            }
            StructuralValue::DateTime(CountPayload::Iso(spelled)) => {
                super::iso::parse_datetime(&spelled)
                    .map(|(count, unit)| Self::DateTime(count, unit))
                    .map_err(D::Error::custom)
            }
            StructuralValue::Duration(CountPayload::Pair(count, unit)) => {
                Ok(Self::Duration(count, unit))
            }
            StructuralValue::Duration(CountPayload::Iso(spelled)) => {
                super::iso::parse_duration(&spelled)
                    .map(|(count, unit)| Self::Duration(count, unit))
                    .map_err(D::Error::custom)
            }
            StructuralValue::Sequence(values) => Ok(Self::from_sequence(values)),
            StructuralValue::Mapping(entries) => {
                Self::from_mapping(entries).map_err(D::Error::custom)
            }
            StructuralValue::Record(data_type, values) => {
                let data_type = DataType::from_str(&data_type).map_err(D::Error::custom)?;
                Self::record(data_type, values).map_err(D::Error::custom)
            }
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Value {}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        match (integer(self), integer(other)) {
            (Some(left), Some(right)) => return compare_integer(left, right),
            (Some(_), None) => return value_rank(self).cmp(&value_rank(other)),
            (None, Some(_)) => return value_rank(self).cmp(&value_rank(other)),
            (None, None) => {}
        }
        // Floats are one family across widths, exactly as the integers are:
        // an `f32` widens to `f64` without loss, so `F32(1.5)` and `F64(1.5)`
        // are one value, not two kinds that happen to print alike.
        if let (Some(left), Some(right)) = (float(self), float(other)) {
            return left.cmp(&right);
        }
        let rank = value_rank(self).cmp(&value_rank(other));
        if rank != Ordering::Equal {
            return rank;
        }
        // Matching on `self` alone, with `other` narrowed to the same kind, is
        // what makes a new variant a compile error here rather than a value
        // that silently orders equal to everything else of its kind.
        macro_rules! same_kind {
            ($pattern:pat => $ordering:expr) => {{
                let $pattern = other else {
                    unreachable!("an equal value rank holds an equal kind")
                };
                $ordering
            }};
        }
        match self {
            Self::Null => Ordering::Equal,
            Self::Bool(left) => same_kind!(Self::Bool(right) => left.cmp(right)),
            Self::I8(_)
            | Self::I16(_)
            | Self::I32(_)
            | Self::I64(_)
            | Self::U8(_)
            | Self::U16(_)
            | Self::U32(_)
            | Self::U64(_)
            | Self::I128(_)
            | Self::U128(_) => {
                unreachable!("every integer width returned above")
            }
            Self::F32(_) | Self::F64(_) => unreachable!("both float widths returned above"),
            Self::Decimal(unscaled, scale) => same_kind!(
                Self::Decimal(other_unscaled, other_scale) =>
                    decimal::compare(*unscaled, *scale, *other_unscaled, *other_scale)
            ),
            Self::String(left) => same_kind!(Self::String(right) => left.cmp(right)),
            Self::Bytes(left) => same_kind!(Self::Bytes(right) => left.cmp(right)),
            Self::Geospatial(left) => same_kind!(Self::Geospatial(right) => left.cmp(right)),
            Self::Date(left) => same_kind!(Self::Date(right) => left.cmp(right)),
            Self::Time(count, unit) => same_kind!(
                Self::Time(other_count, other_unit) =>
                    temporal_key(*count, *unit).cmp(&temporal_key(*other_count, *other_unit))
            ),
            Self::Timestamp(count, unit, zone) => same_kind!(
                Self::Timestamp(other_count, other_unit, other_zone) =>
                    temporal_key(*count, *unit)
                        .cmp(&temporal_key(*other_count, *other_unit))
                        .then_with(|| zone.cmp(other_zone))
            ),
            Self::DateTime(count, unit) => same_kind!(
                Self::DateTime(other_count, other_unit) =>
                    temporal_key(*count, *unit).cmp(&temporal_key(*other_count, *other_unit))
            ),
            Self::Duration(count, unit) => same_kind!(
                Self::Duration(other_count, other_unit) =>
                    temporal_key(*count, *unit).cmp(&temporal_key(*other_count, *other_unit))
            ),
            Self::Sequence(left) => same_kind!(Self::Sequence(right) => left.cmp(right)),
            Self::Mapping(left) => same_kind!(Self::Mapping(right) => left.cmp(right)),
            Self::Record(data_type, values) => same_kind!(
                Self::Record(other_type, other_values) =>
                    // The canonical spelling is the type's total order, and
                    // values only break the tie between equal types.
                    data_type
                        .to_string()
                        .cmp(&other_type.to_string())
                        .then_with(|| values.cmp(other_values))
            ),
        }
    }
}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        value_rank(self).hash(state);
        if let Some(integer) = integer(self) {
            integer.hash(state);
            return;
        }
        // Both float widths hash their common 64-bit reading, which is what
        // keeps `Hash` agreeing with `Ord` across the family.
        if let Some(float) = float(self) {
            float.hash(state);
            return;
        }
        match self {
            Self::Null => {}
            Self::Bool(value) => value.hash(state),
            Self::I8(_)
            | Self::I16(_)
            | Self::I32(_)
            | Self::I64(_)
            | Self::U8(_)
            | Self::U16(_)
            | Self::U32(_)
            | Self::U64(_)
            | Self::I128(_)
            | Self::U128(_) => unreachable!("integer values returned above"),
            Self::F32(_) | Self::F64(_) => unreachable!("float values returned above"),
            // Hashing the normalized pair is what keeps `Hash` agreeing with
            // `Ord`, which treats `1.50` and `1.5` as one number.
            Self::Decimal(unscaled, scale) => decimal::normalize(*unscaled, *scale).hash(state),
            Self::String(value) => value.hash(state),
            Self::Bytes(value) => value.hash(state),
            Self::Geospatial(value) => value.hash(state),
            Self::Date(value) => value.hash(state),
            Self::Time(count, unit) | Self::DateTime(count, unit) | Self::Duration(count, unit) => {
                temporal_key(*count, *unit).hash(state);
            }
            Self::Timestamp(count, unit, zone) => {
                temporal_key(*count, *unit).hash(state);
                zone.hash(state);
            }
            Self::Sequence(value) => value.hash(state),
            Self::Mapping(value) => value.hash(state),
            Self::Record(data_type, values) => {
                data_type.to_string().hash(state);
                values.hash(state);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum Integer {
    Negative(u128),
    NonNegative(u128),
}

fn integer(value: &Value) -> Option<Integer> {
    fn signed(value: i128) -> Integer {
        if value < 0 {
            Integer::Negative(value.unsigned_abs())
        } else {
            Integer::NonNegative(value as u128)
        }
    }
    match value {
        Value::I8(value) => Some(signed(i128::from(*value))),
        Value::I16(value) => Some(signed(i128::from(*value))),
        Value::I32(value) => Some(signed(i128::from(*value))),
        Value::I64(value) => Some(signed(i128::from(*value))),
        Value::I128(value) => Some(signed(*value)),
        Value::U8(value) => Some(Integer::NonNegative(u128::from(*value))),
        Value::U16(value) => Some(Integer::NonNegative(u128::from(*value))),
        Value::U32(value) => Some(Integer::NonNegative(u128::from(*value))),
        Value::U64(value) => Some(Integer::NonNegative(u128::from(*value))),
        Value::U128(value) => Some(Integer::NonNegative(*value)),
        _ => None,
    }
}

/// The common 64-bit reading either float width reduces to for ordering.
///
/// Widening an `f32` to `f64` is exact, so one total order covers the family;
/// the width stays on the value itself and on the wire.
fn float(value: &Value) -> Option<Float> {
    match value {
        Value::F32(value) => Some(Float::from_f64(value.as_f64())),
        Value::F64(value) => Some(*value),
        _ => None,
    }
}

fn compare_integer(left: Integer, right: Integer) -> Ordering {
    match (left, right) {
        (Integer::Negative(left), Integer::Negative(right)) => right.cmp(&left),
        (Integer::Negative(_), Integer::NonNegative(_)) => Ordering::Less,
        (Integer::NonNegative(_), Integer::Negative(_)) => Ordering::Greater,
        (Integer::NonNegative(left), Integer::NonNegative(right)) => left.cmp(&right),
    }
}

/// The total-ordering key that separates one kind of value from another.
///
/// This number is wire-visible: it decides the order of an Arrow dictionary's
/// values and of any caller who sorts values, so it is a numbering to keep, not
/// an implementation detail. It runs in one coherent sweep - nothing, then the
/// numbers, then the text, then the instants, then the containers - so a kind
/// added later takes the next free number rather than displacing an existing
/// one.
const fn value_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::I8(_)
        | Value::I16(_)
        | Value::I32(_)
        | Value::I64(_)
        | Value::U8(_)
        | Value::U16(_)
        | Value::U32(_)
        | Value::U64(_)
        | Value::I128(_)
        | Value::U128(_) => 2,
        Value::F32(_) | Value::F64(_) => 3,
        Value::Decimal(..) => 4,
        Value::String(_) => 5,
        Value::Bytes(_) => 6,
        Value::Date(_) => 7,
        Value::Time(..) => 8,
        Value::Timestamp(..) => 9,
        Value::Duration(..) => 10,
        Value::Sequence(_) => 11,
        Value::Mapping(_) => 12,
        Value::Record(..) => 13,
        // The naive reading arrived after the containers, and the numbering
        // is kept, so it takes the next free number rather than the slot
        // beside its zoned sibling.
        Value::DateTime(..) => 14,
        // A geometry arrived later still, and takes the next free number
        // rather than the slot beside its bytes sibling, for the same reason.
        Value::Geospatial(_) => 15,
    }
}

impl Value {
    /// The canonical vocabulary name for this value's kind, such as `mapping`.
    ///
    /// This is the spelling every error message uses for an observed value, so
    /// a caller reading `expected string, got mapping` sees the same words the
    /// documentation and the bindings use.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::I8(_) => "i8",
            Self::I16(_) => "i16",
            Self::I32(_) => "i32",
            Self::I64(_) => "i64",
            Self::U8(_) => "u8",
            Self::U16(_) => "u16",
            Self::U32(_) => "u32",
            Self::U64(_) => "u64",
            Self::I128(_) => "i128",
            Self::U128(_) => "u128",
            Self::F32(_) => "f32",
            Self::F64(_) => "f64",
            Self::Decimal(..) => "decimal",
            Self::String(_) => "string",
            Self::Bytes(_) => "bytes",
            Self::Geospatial(_) => "geospatial",
            Self::Date(_) => "date",
            Self::Time(..) => "time",
            Self::Timestamp(..) => "timestamp",
            Self::DateTime(..) => "datetime",
            Self::Duration(..) => "duration",
            Self::Sequence(_) => "sequence",
            Self::Mapping(_) => "mapping",
            Self::Record(..) => "record",
        }
    }

    /// Construct an ordered sequence.
    pub fn from_sequence(values: impl IntoIterator<Item = Self>) -> Self {
        let values = values.into_iter().collect::<Vec<_>>();
        if values.is_empty() {
            static EMPTY: OnceLock<Arc<[Value]>> = OnceLock::new();
            return Self::Sequence(Arc::clone(EMPTY.get_or_init(|| Arc::from([]))));
        }
        Self::Sequence(values.into())
    }

    /// Construct an insertion-ordered mapping, rejecting duplicate keys.
    pub fn from_mapping(entries: impl IntoIterator<Item = (Self, Self)>) -> Result<Self> {
        let entries = entries.into_iter().collect::<Vec<_>>();
        if entries.len() <= 16 {
            for (index, (key, _)) in entries.iter().enumerate() {
                if entries[..index].iter().any(|(existing, _)| existing == key) {
                    return Err(duplicate_key_error(index));
                }
            }
        } else {
            // `Value`'s hash reads canonical content only, never the
            // interior-mutable caches a datatype holds, so the key is stable.
            #[allow(clippy::mutable_key_type)]
            let mut seen = HashSet::with_capacity(entries.len());
            for (index, (key, _)) in entries.iter().enumerate() {
                if !seen.insert(key) {
                    return Err(duplicate_key_error(index));
                }
            }
        }
        if entries.is_empty() {
            static EMPTY: OnceLock<Arc<[(Value, Value)]>> = OnceLock::new();
            return Ok(Self::Mapping(Arc::clone(
                EMPTY.get_or_init(|| Arc::from([])),
            )));
        }
        Ok(Self::Mapping(entries.into()))
    }

    /// Build a typed row from a struct datatype and one value per field.
    ///
    /// The values arrive in the order the datatype declares its fields, which
    /// is the only order a record has.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype declares no fields to type the
    /// values with, or when the counts disagree.
    pub fn record(data_type: DataType, values: impl IntoIterator<Item = Self>) -> Result<Self> {
        let values = values.into_iter().collect::<Vec<_>>();
        let fields = data_type.field_len();
        if data_type.as_fields().is_none() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected a struct datatype to type a record, got {data_type}"
                ),
            });
        }
        if fields != values.len() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {fields} record values to match the datatype's fields, got {}",
                    values.len()
                ),
            });
        }
        Ok(Self::Record(Arc::new(data_type), values.into()))
    }

    /// Return the datatype and values when this is a record.
    pub fn as_record(&self) -> Option<(&DataType, &[Self])> {
        match self {
            Self::Record(data_type, values) => Some((data_type, values)),
            _ => None,
        }
    }

    /// Spell a record as the mapping of its field names to its values.
    ///
    /// This is what every text format emits for a record, because a record in
    /// a schemaless format *is* the named mapping; the datatype is what the
    /// mapping spelling drops. Any other value returns as it stands.
    #[must_use]
    pub fn record_to_mapping(&self) -> Self {
        let Self::Record(data_type, values) = self else {
            return self.clone();
        };
        let Some(fields) = data_type.as_fields() else {
            return Self::from_sequence(values.iter().cloned());
        };
        let entries: Vec<(Self, Self)> = fields
            .iter()
            .zip(values.iter())
            .map(|(field, value)| (Self::String(SmolStr::new(field.name())), value.clone()))
            .collect();
        Self::Mapping(entries.into())
    }

    /// Return a boolean when this is a boolean.
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Return a signed integer when it fits `i128`.
    pub const fn as_i128(&self) -> Option<i128> {
        match self {
            Self::I8(value) => Some(*value as i128),
            Self::I16(value) => Some(*value as i128),
            Self::I32(value) => Some(*value as i128),
            Self::I64(value) => Some(*value as i128),
            Self::I128(value) => Some(*value),
            Self::U8(value) => Some(*value as i128),
            Self::U16(value) => Some(*value as i128),
            Self::U32(value) => Some(*value as i128),
            Self::U64(value) => Some(*value as i128),
            Self::U128(value) if *value <= i128::MAX as u128 => Some(*value as i128),
            _ => None,
        }
    }

    /// Return an unsigned integer when it fits `u128`.
    pub const fn as_u128(&self) -> Option<u128> {
        match self {
            Self::I8(value) if *value >= 0 => Some(*value as u128),
            Self::I16(value) if *value >= 0 => Some(*value as u128),
            Self::I32(value) if *value >= 0 => Some(*value as u128),
            Self::I64(value) if *value >= 0 => Some(*value as u128),
            Self::I128(value) if *value >= 0 => Some(*value as u128),
            Self::U8(value) => Some(*value as u128),
            Self::U16(value) => Some(*value as u128),
            Self::U32(value) => Some(*value as u128),
            Self::U64(value) => Some(*value as u128),
            Self::U128(value) => Some(*value),
            _ => None,
        }
    }

    /// Return a floating-point value when this is a float of either width.
    ///
    /// The 32-bit width widens exactly, so no float answers differently here
    /// than it would at its own width.
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F32(value) => Some(value.as_f64()),
            Self::F64(value) => Some(value.as_f64()),
            _ => None,
        }
    }

    /// Return the 32-bit float when this is one.
    ///
    /// The wide float does not narrow here, because `as_f64` widening is
    /// exact and narrowing is not; a caller who wants the rounding asks for
    /// it with `as f32` where the loss is visible.
    pub const fn as_f32(&self) -> Option<f32> {
        match self {
            Self::F32(value) => Some(value.as_f32()),
            _ => None,
        }
    }

    /// Return a string slice when this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    /// Return bytes when this is a byte value.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }

    /// Return the Well-Known Binary payload without allocating.
    ///
    /// A geospatial column also accepts plain [`Self::Bytes`] on the way in -
    /// canonicalization is what rewrites it - so this reads both spellings.
    pub fn as_wkb(&self) -> Option<&[u8]> {
        match self {
            Self::Geospatial(value) => Some(value),
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }

    /// Return sequence children without allocating.
    pub fn as_sequence(&self) -> Option<&[Self]> {
        match self {
            Self::Sequence(values) => Some(values),
            _ => None,
        }
    }

    /// Return mapping entries without allocating.
    pub fn as_mapping(&self) -> Option<&[(Self, Self)]> {
        match self {
            Self::Mapping(entries) => Some(entries),
            _ => None,
        }
    }

    /// Return the number of direct children or mapping entries.
    pub fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
            _ => 0,
        }
    }

    /// Return whether this is an empty sequence or mapping.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Sequence(values) if values.is_empty())
            || matches!(self, Self::Mapping(entries) if entries.is_empty())
    }

    /// Look up a sequence index.
    pub fn get(&self, index: usize) -> Option<&Self> {
        self.as_sequence()?.get(index)
    }

    /// Look up a mapping key without allocating.
    pub fn get_key(&self, key: &Self) -> Option<&Self> {
        self.as_mapping()?
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value))
    }

    /// Look up a string mapping key without constructing a temporary value.
    pub fn get_key_str(&self, key: &str) -> Option<&Self> {
        self.as_mapping()?
            .iter()
            .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
    }

    /// Iterate over sequence values or mapping keys without allocating.
    pub fn iter(&self) -> Children<'_> {
        match self {
            Self::Sequence(values) => Children::Sequence(values.iter()),
            Self::Mapping(entries) => Children::Mapping(entries.iter()),
            _ => Children::Sequence([].iter()),
        }
    }

    /// Iterate over sequence children without allocating.
    pub fn sequence_iter(&self) -> std::slice::Iter<'_, Self> {
        self.as_sequence().unwrap_or_default().iter()
    }

    /// Iterate over mapping entries without allocating.
    pub fn mapping_iter(&self) -> std::slice::Iter<'_, (Self, Self)> {
        self.as_mapping().unwrap_or_default().iter()
    }

    /// Return whether this is the null value.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Return whether this value holds other values.
    pub const fn is_container(&self) -> bool {
        matches!(self, Self::Sequence(_) | Self::Mapping(_))
    }

    /// Return whether this is any integer, signed or unsigned, at any width.
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::I8(_)
                | Self::I16(_)
                | Self::I32(_)
                | Self::I64(_)
                | Self::U8(_)
                | Self::U16(_)
                | Self::U32(_)
                | Self::U64(_)
                | Self::I128(_)
                | Self::U128(_)
        )
    }

    /// Return whether this is a number of any width.
    pub const fn is_number(&self) -> bool {
        self.is_integer() || matches!(self, Self::F32(_) | Self::F64(_))
    }

    /// Read this value as an `i64`, when it fits.
    ///
    /// A wider integer that does not fit returns `None` rather than wrapping,
    /// so a caller never silently loses magnitude.
    pub const fn as_i64(&self) -> Option<i64> {
        match self.as_i128() {
            Some(value) if value >= i64::MIN as i128 && value <= i64::MAX as i128 => {
                Some(value as i64)
            }
            _ => None,
        }
    }

    /// Read this value as a `u64`, when it fits.
    pub const fn as_u64(&self) -> Option<u64> {
        match self.as_u128() {
            Some(value) if value <= u64::MAX as u128 => Some(value as u64),
            _ => None,
        }
    }

    /// Look one value up by a dotted path of mapping keys and sequence indexes.
    ///
    /// `"legs.0.price"` reads the mapping key `legs`, then index `0`, then the
    /// key `price`. A segment that does not resolve returns `None`, so probing a
    /// shape needs no nested matching.
    ///
    /// ```
    /// use yggdryl::Value;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = Value::from_mapping([(
    ///     Value::from("legs"),
    ///     Value::from_sequence([Value::from_mapping([(
    ///         Value::from("price"),
    ///         Value::from(12_i64),
    ///     )])?]),
    /// )])?;
    ///
    /// assert_eq!(order.path("legs.0.price").and_then(Value::as_i64), Some(12));
    /// assert!(order.path("legs.9.price").is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn path(&self, path: &str) -> Option<&Self> {
        let mut current = self;
        for segment in path.split('.').filter(|segment| !segment.is_empty()) {
            current = match current {
                Self::Mapping(_) => current.get_key_str(segment)?,
                Self::Sequence(_) => current.get(segment.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(current)
    }

    /// Return the value at `key`, or `default` when the key is absent or null.
    pub fn get_or<'value>(&'value self, key: &str, default: &'value Self) -> &'value Self {
        match self.get_key_str(key) {
            Some(value) if !value.is_null() => value,
            _ => default,
        }
    }

    /// Iterate over mapping entries as pairs.
    ///
    /// A value that is not a mapping iterates over nothing, which is what makes
    /// this usable on a value whose shape is not yet known.
    pub fn entries(&self) -> std::slice::Iter<'_, (Self, Self)> {
        self.mapping_iter()
    }

    /// Collect the string keys of a mapping in insertion order.
    ///
    /// Non-string keys are skipped, because a caller asking for names wants the
    /// ones it can use.
    pub fn keys(&self) -> Vec<&str> {
        self.mapping_iter()
            .filter_map(|(key, _)| key.as_str())
            .collect()
    }

    /// Return whether a mapping contains `key`.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get_key_str(key).is_some()
    }

    /// Return this value with one mapping key added or replaced.
    ///
    /// The order of existing keys is preserved and a new key is appended, so a
    /// rebuilt mapping reads in the order it was written.
    ///
    /// # Errors
    ///
    /// Returns an error when this value is not a mapping.
    pub fn with_key(&self, key: impl Into<Self>, value: impl Into<Self>) -> Result<Self> {
        let key = key.into();
        let value = value.into();
        let entries = self.as_mapping().ok_or_else(|| Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::format_smolstr!(
                "expected a mapping to set a key on, got {}",
                self.kind()
            ),
        })?;

        let mut rebuilt = Vec::with_capacity(entries.len() + 1);
        let mut replaced = false;
        for (existing, current) in entries {
            if existing == &key {
                rebuilt.push((key.clone(), value.clone()));
                replaced = true;
            } else {
                rebuilt.push((existing.clone(), current.clone()));
            }
        }
        if !replaced {
            rebuilt.push((key, value));
        }
        Self::from_mapping(rebuilt)
    }

    /// Return this value with one mapping key removed.
    ///
    /// A key that is not there is not an error; the value comes back unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error when this value is not a mapping.
    pub fn without_key(&self, key: &str) -> Result<Self> {
        let entries = self.as_mapping().ok_or_else(|| Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::format_smolstr!(
                "expected a mapping to remove a key from, got {}",
                self.kind()
            ),
        })?;
        Self::from_mapping(
            entries
                .iter()
                .filter(|(existing, _)| existing.as_str() != Some(key))
                .cloned(),
        )
    }
}

/// A borrowed iterator over sequence values or mapping keys.
pub enum Children<'a> {
    /// Sequence values.
    Sequence(std::slice::Iter<'a, Value>),
    /// Mapping keys.
    Mapping(std::slice::Iter<'a, (Value, Value)>),
}

impl<'a> Iterator for Children<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next(),
            Self::Mapping(entries) => entries.next().map(|(key, _)| key),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.len();
        (length, Some(length))
    }
}

impl DoubleEndedIterator for Children<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next_back(),
            Self::Mapping(entries) => entries.next_back().map(|(key, _)| key),
        }
    }
}

impl ExactSizeIterator for Children<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
        }
    }
}

impl std::iter::FusedIterator for Children<'_> {}

impl<'a> IntoIterator for &'a Value {
    type Item = &'a Value;
    type IntoIter = Children<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

fn duplicate_key_error(index: usize) -> Error {
    Error::Codec {
        format: "value",
        position: index,
        reason: "mapping contains a duplicate key".into(),
    }
}

impl From<()> for Value {
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

// A native integer keeps its width: an `i32` is an `I32`, not an `I64` that
// happens to fit, because the width is what a column declaration reads back.
macro_rules! width_value_from {
    ($($type:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$type> for Value {
            fn from(value: $type) -> Self {
                Self::$variant(value)
            }
        }
    )+};
}

width_value_from!(
    i8 => I8, i16 => I16, i32 => I32, i64 => I64,
    u8 => U8, u16 => U16, u32 => U32, u64 => U64,
);

impl From<i128> for Value {
    fn from(value: i128) -> Self {
        i64::try_from(value).map_or(Self::I128(value), Self::I64)
    }
}

impl From<u128> for Value {
    fn from(value: u128) -> Self {
        u64::try_from(value).map_or(Self::U128(value), Self::U64)
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::F32(Float32::from_f32(value))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::F64(Float::from_f64(value))
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(SmolStr::new(value))
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(SmolStr::from(value))
    }
}

impl From<SmolStr> for Value {
    fn from(value: SmolStr) -> Self {
        Self::String(value)
    }
}

impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        if value.is_empty() {
            static EMPTY: OnceLock<Arc<[u8]>> = OnceLock::new();
            return Self::Bytes(Arc::clone(EMPTY.get_or_init(|| Arc::from([]))));
        }
        Self::Bytes(value.into())
    }
}

impl From<&[u8]> for Value {
    fn from(value: &[u8]) -> Self {
        // Borrowed bytes are the shape every reader hands out, so taking them
        // directly saves the caller a `to_vec` whose only purpose was this call.
        if value.is_empty() {
            return Self::from(Vec::<u8>::new());
        }
        Self::Bytes(Arc::from(value))
    }
}

impl<const N: usize> From<&[u8; N]> for Value {
    fn from(value: &[u8; N]) -> Self {
        Self::from(value.as_slice())
    }
}

impl From<Arc<[u8]>> for Value {
    fn from(value: Arc<[u8]>) -> Self {
        if value.is_empty() {
            return Self::from(Vec::<u8>::new());
        }
        Self::Bytes(value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::from_sequence(value)
    }
}

impl FromIterator<Value> for Value {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        Self::from_sequence(iter)
    }
}

impl Index<usize> for Value {
    type Output = Value;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_sequence().expect("value is not a sequence")[index]
    }
}

impl Index<&Value> for Value {
    type Output = Value;

    fn index(&self, key: &Value) -> &Self::Output {
        self.get_key(key).expect("mapping key is not present")
    }
}

impl Index<&str> for Value {
    type Output = Value;

    fn index(&self, key: &str) -> &Self::Output {
        self.get_key_str(key).expect("mapping key is not present")
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    fn order() -> Value {
        Value::from_mapping([
            (Value::from("symbol"), Value::from("AAPL")),
            (
                Value::from("legs"),
                Value::from_sequence([
                    Value::from_mapping([(Value::from("price"), Value::from(12_i64))]).unwrap(),
                    Value::from_mapping([(Value::from("price"), Value::from(13_i64))]).unwrap(),
                ]),
            ),
            (Value::from("venue"), Value::Null),
        ])
        .unwrap()
    }

    #[test]
    fn a_dotted_path_walks_mappings_and_sequences() {
        let order = order();

        assert_eq!(order.path("symbol").and_then(Value::as_str), Some("AAPL"));
        assert_eq!(order.path("legs.1.price").and_then(Value::as_i64), Some(13));

        // A segment that does not resolve is absence, not an error.
        assert!(order.path("legs.9.price").is_none());
        assert!(order.path("symbol.price").is_none());
        assert!(order.path("missing").is_none());

        // An empty path is the value itself.
        assert_eq!(order.path(""), Some(&order));
    }

    #[test]
    fn narrowing_an_integer_refuses_to_lose_magnitude() {
        assert_eq!(Value::from(7_i64).as_i64(), Some(7));
        assert_eq!(Value::from(7_u64).as_u64(), Some(7));

        // A 128-bit value that does not fit is None rather than a wrapped one.
        assert_eq!(Value::from(i128::MAX).as_i64(), None);
        assert_eq!(Value::from(u128::MAX).as_u64(), None);
        assert_eq!(Value::from(-1_i64).as_u64(), None);
    }

    #[test]
    fn shape_predicates_answer_without_matching() {
        assert!(Value::Null.is_null());
        assert!(Value::from(1_i64).is_integer());
        assert!(Value::from(1.5).is_number());
        assert!(!Value::from(1.5).is_integer());
        assert!(order().is_container());
        assert!(!Value::from("AAPL").is_container());
    }

    #[test]
    fn mapping_helpers_read_and_rebuild_in_order() {
        let order = order();

        assert_eq!(order.keys(), vec!["symbol", "legs", "venue"]);
        assert!(order.contains_key("venue"));
        assert_eq!(order.entries().count(), 3);

        // A null value counts as absent for a default.
        let fallback = Value::from("XPAR");
        assert_eq!(order.get_or("venue", &fallback), &fallback);
        assert_eq!(order.get_or("symbol", &fallback), &Value::from("AAPL"));

        // Replacing keeps position; adding appends.
        let updated = order.with_key("venue", "XPAR").unwrap();
        assert_eq!(updated.keys(), vec!["symbol", "legs", "venue"]);
        assert_eq!(updated.path("venue").and_then(Value::as_str), Some("XPAR"));

        let added = order.with_key("currency", "EUR").unwrap();
        assert_eq!(added.keys(), vec!["symbol", "legs", "venue", "currency"]);

        let removed = order.without_key("venue").unwrap();
        assert_eq!(removed.keys(), vec!["symbol", "legs"]);
        // Removing something absent changes nothing.
        assert_eq!(removed.without_key("absent").unwrap(), removed);
    }

    #[test]
    fn a_geospatial_value_is_its_own_kind_over_its_bytes() {
        use std::hash::{Hash, Hasher};

        fn hash_of(value: &Value) -> u64 {
            let mut hasher = std::hash::DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        }

        let wkb: &[u8] = &[1, 1, 0, 0, 0];
        let point = Value::Geospatial(wkb.into());
        assert_eq!(point.kind(), "geospatial");

        // The same bytes under the bytes kind are a different value: the kind
        // is part of the identity, exactly as it is for string versus bytes.
        let bytes = Value::from(wkb);
        assert_ne!(point, bytes);
        assert_ne!(hash_of(&point), hash_of(&bytes));

        // Within the kind, the bytes compare, and equal values hash equal.
        assert_eq!(point, Value::Geospatial(wkb.into()));
        assert_eq!(hash_of(&point), hash_of(&Value::Geospatial(wkb.into())));
        assert!(point < Value::Geospatial([1u8, 2].as_slice().into()));
    }

    #[test]
    fn the_structural_wire_round_trips_a_geospatial_value() {
        let point = Value::Geospatial([1u8, 1, 0, 0, 0].as_slice().into());
        let encoded = serde_json::to_string(&point).unwrap();
        assert!(encoded.contains("\"type\":\"geospatial\""), "{encoded}");
        let decoded: Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, point);
    }

    #[test]
    fn rebuilding_a_value_that_is_not_a_mapping_says_what_it_is() {
        let message = Value::from("AAPL")
            .with_key("symbol", "AAPL")
            .unwrap_err()
            .to_string();
        assert!(message.contains("expected a mapping"), "{message}");
        assert!(message.contains("string"), "{message}");
    }
}
