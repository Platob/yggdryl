//! The one value every part of the project speaks.
//!
//! A [`Scalar`] is any native value: null, a boolean, a signed or unsigned
//! integer at every width from 8 to 128 bits, a float at 16, 32, or 64 bits,
//! an exact decimal, text, bytes, width-typed temporals, an ordered sequence,
//! an arbitrary-key mapping, or a name-sorted record. It is what [`crate::text::json`], [`crate::text::yaml`], and
//! [`crate::text::toml`] parse into and write from, what a [`crate::Field`] validates
//! and canonicalizes, and what the language bindings convert their own objects
//! into - so a value crosses every boundary in the project without being
//! re-modelled on the way.
//!
//! Every kind that carries a unit or a scale carries it as a typed field rather
//! than as a free-form name over an untyped payload, because a name nothing
//! validates is not a type. [`Scalar::dtype`] reads the datatype straight
//! off the variant for exactly that reason. There is deliberately no `Variant`
//! kind: a variant value is a `Scalar` - a self-describing tree - so the binary
//! form is an encoding of the one value model, not a second value model.
//!
//! ```
//! use yggdryl::Scalar;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let quote = Scalar::from_record([
//!     ("symbol", Scalar::from("AAPL")),
//!     ("price", Scalar::d128(125, 1)),
//! ])?;
//!
//! assert_eq!(quote.get_key_str("symbol").and_then(Scalar::as_utf8), Some("AAPL"));
//! assert_eq!(quote.len(), 2);
//! # Ok(())
//! # }
//! ```

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::{
    DataType, DataTypeId, DataTypeKind, Error, FieldType, I256, Result, TimeUnit, Timezone,
};

use super::decimal::scalars as decimal;
use super::enum_scalar::EnumScalar;
use super::floating::scalars::{Float16, Float32, Float64};
use super::integer::scalars::Integer;
use super::nested::Children;
use super::temporal::scalars::temporal_key;

/// One concrete scalar representation.
///
/// Implementors are the final leaves below a [`Scalar`] family. Narrowing an
/// existing scalar only projects a reference; validation remains owned by
/// [`DataType::scalar`](crate::DataType::scalar) and
/// [`Field::scalar`](crate::Field::scalar).
pub trait ScalarValue:
    Sized + Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + 'static
{
    /// The family enum containing this representation.
    type Family: ScalarFamily;
    /// The zero-sized marker naming this representation's datatype.
    type Type: FieldType;

    /// The exact representation identifier.
    const ID: DataTypeId;
    /// The representation's datatype family.
    const KIND: DataTypeKind;

    /// Return the datatype this value materializes into.
    fn dtype(&self) -> DataType;
    /// Widen this leaf to its family enum.
    fn into_family(self) -> Self::Family;
    /// Narrow a family value to this leaf.
    fn from_family(family: &Self::Family) -> Option<&Self>;
    /// Widen this leaf to the dynamic scalar root.
    fn into_scalar(self) -> Scalar;
    /// Narrow a dynamic scalar to this leaf without re-validating it.
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}

/// One dynamic family of scalar representations.
pub trait ScalarFamily: Sized + Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash {
    /// The datatype family shared by every member.
    const KIND: DataTypeKind;

    /// Return the exact representation identifier.
    fn id(&self) -> DataTypeId;
    /// Return the exact datatype carried by this value.
    fn dtype(&self) -> DataType;
    /// Widen this family value to the dynamic scalar root.
    fn into_scalar(self) -> Scalar;
    /// Narrow a dynamic scalar to this family without re-validating it.
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}

/// The shared deterministic scalar spanning native and structured formats.
#[derive(Clone, Debug)]
pub enum Scalar {
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
    /// A 16-bit floating-point value.
    F16(Float16),
    /// A 32-bit floating-point value.
    F32(Float32),
    /// A 64-bit floating-point value.
    F64(Float64),
    /// An exact decimal with a signed 128-bit coefficient and scale.
    D128(i128, i8),
    /// An exact decimal with a signed 256-bit coefficient and scale.
    D256(I256, i8),
    /// A Unicode string.
    String(SmolStr),
    /// One identity-preserving member of a shared static enum.
    Enum(EnumScalar),
    /// Arbitrary bytes.
    Bytes(Arc<[u8]>),
    /// A geometry or geography value, as Well-Known Binary.
    ///
    /// WKB rather than a parsed coordinate tree because WKB is what every
    /// geospatial encoding stores and exchanges; [`crate::types::geospatial::wkb`] reads
    /// the bytes wherever a text spelling or a bound is needed.
    Geospatial(Arc<[u8]>),
    /// A 32-bit day count with its explicit day unit and zone marker.
    Date32(i32, TimeUnit, Timezone),
    /// A 64-bit millisecond date count and zone marker.
    Date64(i64, TimeUnit, Timezone),
    /// A 32-bit time-of-day count, unit, and zone marker.
    Time32(i32, TimeUnit, Timezone),
    /// A 64-bit time-of-day count, unit, and zone marker.
    Time64(i64, TimeUnit, Timezone),
    /// A 64-bit epoch count, unit, and non-optional zone.
    ///
    /// [`Timezone::NAIVE`] represents a wall-clock reading without a zone.
    DateTime64(i64, TimeUnit, Timezone),
    /// A 32-bit elapsed count, unit, and explicit zone-free marker.
    Duration32(i32, TimeUnit, Timezone),
    /// A 64-bit elapsed count, unit, and explicit zone-free marker.
    Duration64(i64, TimeUnit, Timezone),
    /// An ordered sequence.
    Sequence(Arc<[Scalar]>),
    /// An insertion-ordered mapping with arbitrary unique keys.
    Mapping(Arc<[(Scalar, Scalar)]>),
    /// A deterministic row mapping sorted by field name.
    Record(Arc<BTreeMap<SmolStr, Scalar>>),
}

impl Serialize for Scalar {
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

        // The layout is an adjacently tagged {"type", "value"} document; a
        // unit variant carries the tag alone.
        fn tagged<S: serde::Serializer, T: Serialize>(
            serializer: S,
            tag: &'static str,
            value: &T,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut document = serializer.serialize_struct("Scalar", 2)?;
            document.serialize_field("type", tag)?;
            document.serialize_field("value", value)?;
            document.end()
        }

        match self {
            Self::Null => {
                let mut document = serializer.serialize_struct("Scalar", 1)?;
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
            Self::F16(value) => tagged(serializer, "f16", value),
            Self::F32(value) => tagged(serializer, "f32", value),
            Self::F64(value) => tagged(serializer, "f64", value),
            Self::D128(unscaled, scale) => tagged(serializer, "d128", &Pair(unscaled, scale)),
            Self::D256(unscaled, scale) => tagged(serializer, "d256", &Pair(unscaled, scale)),
            Self::String(value) => tagged(serializer, "string", value),
            Self::Enum(value) => tagged(serializer, "enum", value),
            Self::Bytes(value) => tagged(serializer, "bytes", value),
            Self::Geospatial(value) => tagged(serializer, "geospatial", value),
            // A temporal is its classic ISO spelling wherever it has one; a
            // reading with no classic spelling keeps its structural parts.
            Self::Date32(days, unit, zone) => match super::iso::format_date(*days) {
                Some(spelled) if *unit == TimeUnit::Day && zone.is_naive() => {
                    tagged(serializer, "date32", &spelled)
                }
                _ => tagged(serializer, "date32", &Triple(days, unit, zone)),
            },
            Self::Date64(count, unit, zone) => {
                tagged(serializer, "date64", &Triple(count, unit, zone))
            }
            Self::Time32(count, unit, zone) => {
                match super::iso::format_time(i64::from(*count), *unit) {
                    Some(spelled) if zone.is_naive() => tagged(serializer, "time32", &spelled),
                    _ => tagged(serializer, "time32", &Triple(count, unit, zone)),
                }
            }
            Self::Time64(count, unit, zone) => match super::iso::format_time(*count, *unit) {
                Some(spelled) if zone.is_naive() => tagged(serializer, "time64", &spelled),
                _ => tagged(serializer, "time64", &Triple(count, unit, zone)),
            },
            Self::DateTime64(count, unit, zone) => {
                let spelled = if zone.is_naive() {
                    super::iso::format_datetime(*count, *unit)
                } else {
                    super::iso::format_timestamp(*count, *unit, zone)
                };
                match spelled {
                    Some(spelled) => tagged(serializer, "datetime64", &spelled),
                    None => tagged(serializer, "datetime64", &Triple(count, unit, zone)),
                }
            }
            Self::Duration32(count, unit, zone) => {
                match super::iso::format_duration(i64::from(*count), *unit) {
                    Some(spelled) if zone.is_naive() => tagged(serializer, "duration32", &spelled),
                    _ => tagged(serializer, "duration32", &Triple(count, unit, zone)),
                }
            }
            Self::Duration64(count, unit, zone) => match super::iso::format_duration(*count, *unit)
            {
                Some(spelled) if zone.is_naive() => tagged(serializer, "duration64", &spelled),
                _ => tagged(serializer, "duration64", &Triple(count, unit, zone)),
            },
            Self::Sequence(values) => tagged(serializer, "sequence", &values.as_ref()),
            Self::Mapping(entries) => tagged(serializer, "mapping", &entries.as_ref()),
            Self::Record(entries) => tagged(serializer, "record", &entries.as_ref()),
        }
    }
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        /// A 32-bit temporal payload.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Temporal32 {
            Iso(SmolStr),
            Triple(i32, TimeUnit, Timezone),
        }

        /// A 64-bit temporal payload.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Temporal64 {
            Iso(SmolStr),
            Triple(i64, TimeUnit, Timezone),
        }

        /// Record entries kept in input order until the canonical constructor
        /// sorts them and rejects duplicate field names.
        struct RecordEntries(Vec<(SmolStr, Scalar)>);

        impl<'de> Deserialize<'de> for RecordEntries {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct Visitor;

                impl<'de> serde::de::Visitor<'de> for Visitor {
                    type Value = RecordEntries;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("a record object with unique field names")
                    }

                    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
                    where
                        A: serde::de::MapAccess<'de>,
                    {
                        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or_default());
                        while let Some(key) = map.next_key::<SmolStr>()? {
                            entries.push((key, map.next_value()?));
                        }
                        Ok(RecordEntries(entries))
                    }
                }

                deserializer.deserialize_map(Visitor)
            }
        }

        // This mirror must stay variant-for-variant identical to `Scalar`: a
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
            F16(Float16),
            F32(Float32),
            F64(Float64),
            D128(i128, i8),
            D256(I256, i8),
            String(SmolStr),
            Enum(EnumScalar),
            Bytes(Arc<[u8]>),
            Geospatial(Arc<[u8]>),
            Date32(Temporal32),
            Date64(Temporal64),
            Time32(Temporal32),
            Time64(Temporal64),
            #[serde(rename = "datetime64")]
            DateTime64(Temporal64),
            Duration32(Temporal32),
            Duration64(Temporal64),
            Sequence(Vec<Scalar>),
            Mapping(Vec<(Scalar, Scalar)>),
            Record(RecordEntries),
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
            StructuralValue::F16(value) => Ok(Self::F16(value)),
            StructuralValue::F32(value) => Ok(Self::F32(value)),
            StructuralValue::F64(value) => Ok(Self::F64(value)),
            StructuralValue::D128(unscaled, scale) => Ok(Self::D128(unscaled, scale)),
            StructuralValue::D256(unscaled, scale) => Ok(Self::D256(unscaled, scale)),
            StructuralValue::String(value) => Ok(Self::String(value)),
            StructuralValue::Enum(value) => Ok(Self::Enum(value)),
            StructuralValue::Bytes(value) => Ok(Self::from(value)),
            StructuralValue::Geospatial(value) => Ok(Self::Geospatial(value)),
            StructuralValue::Date32(Temporal32::Triple(count, unit, zone)) => {
                Self::date32_in(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralValue::Date32(Temporal32::Iso(spelled)) => super::iso::parse_date(&spelled)
                .map(Self::date32)
                .map_err(D::Error::custom),
            StructuralValue::Date64(Temporal64::Triple(count, unit, zone)) => {
                Self::date64_in(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralValue::Date64(Temporal64::Iso(spelled)) => super::iso::parse_date(&spelled)
                .map(|days| Self::date64(i64::from(days) * 86_400_000))
                .map_err(D::Error::custom),
            StructuralValue::Time32(Temporal32::Triple(count, unit, zone)) => {
                Self::time32(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralValue::Time32(Temporal32::Iso(spelled)) => super::iso::parse_time(&spelled)
                .and_then(|(count, unit)| {
                    i32::try_from(count)
                        .map(|count| (count, unit))
                        .map_err(|_| Error::InvalidRecord {
                            path: SmolStr::new_static("$"),
                            reason: SmolStr::new(format!("time count {count} does not fit time32")),
                        })
                })
                .and_then(|(count, unit)| Self::time32(count, unit, Timezone::NAIVE))
                .map_err(D::Error::custom),
            StructuralValue::Time64(Temporal64::Triple(count, unit, zone)) => {
                Self::time64(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralValue::Time64(Temporal64::Iso(spelled)) => super::iso::parse_time(&spelled)
                .and_then(|(count, unit)| Self::time64(count, unit, Timezone::NAIVE))
                .map_err(D::Error::custom),
            StructuralValue::DateTime64(Temporal64::Triple(count, unit, zone)) => {
                Self::datetime64(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralValue::DateTime64(Temporal64::Iso(spelled)) => {
                super::iso::parse_timestamp(&spelled)
                    .and_then(|(count, unit, zone)| Self::datetime64(count, unit, zone))
                    .or_else(|_| {
                        super::iso::parse_datetime(&spelled).and_then(|(count, unit)| {
                            Self::datetime64(count, unit, Timezone::NAIVE)
                        })
                    })
                    .map_err(D::Error::custom)
            }
            StructuralValue::Duration32(Temporal32::Triple(count, unit, zone)) => {
                if !zone.is_naive() {
                    return Err(D::Error::custom("duration32 timezone must be NAIVE"));
                }
                Self::duration32(count, unit).map_err(D::Error::custom)
            }
            StructuralValue::Duration32(Temporal32::Iso(spelled)) => {
                super::iso::parse_duration(&spelled)
                    .and_then(|(count, unit)| {
                        i32::try_from(count)
                            .map(|count| (count, unit))
                            .map_err(|_| Error::InvalidRecord {
                                path: SmolStr::new_static("$"),
                                reason: SmolStr::new(format!(
                                    "duration count {count} does not fit duration32"
                                )),
                            })
                    })
                    .and_then(|(count, unit)| Self::duration32(count, unit))
                    .map_err(D::Error::custom)
            }
            StructuralValue::Duration64(Temporal64::Triple(count, unit, zone)) => {
                if !zone.is_naive() {
                    return Err(D::Error::custom("duration64 timezone must be NAIVE"));
                }
                Self::duration64(count, unit).map_err(D::Error::custom)
            }
            StructuralValue::Duration64(Temporal64::Iso(spelled)) => {
                super::iso::parse_duration(&spelled)
                    .and_then(|(count, unit)| Self::duration64(count, unit))
                    .map_err(D::Error::custom)
            }
            StructuralValue::Sequence(values) => Ok(Self::from_sequence(values)),
            StructuralValue::Mapping(entries) => {
                Self::from_mapping(entries).map_err(D::Error::custom)
            }
            StructuralValue::Record(entries) => {
                Self::from_record(entries.0).map_err(D::Error::custom)
            }
        }
    }
}

impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Scalar {}

impl PartialOrd for Scalar {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scalar {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.as_integer(), other.as_integer()) {
            (Some(left), Some(right)) => return compare_integer(left, right),
            (Some(_), None) | (None, Some(_)) => {
                return value_rank(self).cmp(&value_rank(other));
            }
            (None, None) => {}
        }
        // Floats are one family across widths, exactly as the integers are:
        // an `f32` widens to `f64` without loss, so `F32(1.5)` and `F64(1.5)`
        // are one value, not two kinds that happen to print alike.
        if let (Some(left), Some(right)) = (self.as_float(), other.as_float()) {
            return left.cmp(&right);
        }
        if let (Some(left), Some(right)) = (decimal_value(self), decimal_value(other)) {
            return decimal::compare(left.0, left.1, right.0, right.1);
        }
        if let (Some(left), Some(right)) = (temporal_value(self), temporal_value(other)) {
            if left.0 == right.0 {
                return left.1.cmp(&right.1).then_with(|| left.2.cmp(right.2));
            }
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
            Self::F16(_) | Self::F32(_) | Self::F64(_) => {
                unreachable!("all float widths returned above")
            }
            Self::D128(..) | Self::D256(..) => {
                unreachable!("both decimal widths returned above")
            }
            Self::String(left) => same_kind!(Self::String(right) => left.cmp(right)),
            Self::Enum(left) => same_kind!(Self::Enum(right) => left.cmp(right)),
            Self::Bytes(left) => same_kind!(Self::Bytes(right) => left.cmp(right)),
            Self::Geospatial(left) => same_kind!(Self::Geospatial(right) => left.cmp(right)),
            Self::Date32(..)
            | Self::Date64(..)
            | Self::Time32(..)
            | Self::Time64(..)
            | Self::DateTime64(..)
            | Self::Duration32(..)
            | Self::Duration64(..) => unreachable!("temporal families returned above"),
            Self::Sequence(left) => same_kind!(Self::Sequence(right) => left.cmp(right)),
            Self::Mapping(left) => same_kind!(Self::Mapping(right) => left.cmp(right)),
            Self::Record(left) => same_kind!(Self::Record(right) => left.cmp(right)),
        }
    }
}

impl Hash for Scalar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        value_rank(self).hash(state);
        if let Some(integer) = self.as_integer() {
            integer.hash(state);
            return;
        }
        // All float widths hash their common 64-bit reading, which is what
        // keeps `Hash` agreeing with `Ord` across the family.
        if let Some(float) = self.as_float() {
            float.hash(state);
            return;
        }
        if let Some((unscaled, scale)) = decimal_value(self) {
            decimal::normalize(unscaled, scale).hash(state);
            return;
        }
        if let Some((_, count, zone)) = temporal_value(self) {
            count.hash(state);
            zone.hash(state);
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
            Self::F16(_) | Self::F32(_) | Self::F64(_) => {
                unreachable!("float values returned above")
            }
            Self::D128(..) | Self::D256(..) => unreachable!("decimal values returned above"),
            Self::String(value) => value.hash(state),
            Self::Enum(value) => value.hash(state),
            Self::Bytes(value) | Self::Geospatial(value) => value.hash(state),
            Self::Date32(..)
            | Self::Date64(..)
            | Self::Time32(..)
            | Self::Time64(..)
            | Self::DateTime64(..)
            | Self::Duration32(..)
            | Self::Duration64(..) => unreachable!("temporal values returned above"),
            Self::Sequence(value) => value.hash(state),
            Self::Mapping(value) => value.hash(state),
            Self::Record(value) => value.hash(state),
        }
    }
}

fn decimal_value(value: &Scalar) -> Option<(I256, i8)> {
    value.as_decimal()
}

/// The family, normalized count, and zone of one temporal.
fn temporal_value(value: &Scalar) -> Option<(super::TemporalFamily, (u8, i128), &Timezone)> {
    let temporal = value.as_temporal()?;
    Some((
        temporal.family(),
        temporal_key(temporal.count(), temporal.unit()),
        temporal.timezone(),
    ))
}

fn compare_integer(left: Integer, right: Integer) -> Ordering {
    left.cmp(&right)
}

/// The total-ordering key that separates one kind of value from another.
///
/// This number is wire-visible: it decides the order of an Arrow dictionary's
/// values and of any caller who sorts values, so it is a numbering to keep, not
/// an implementation detail. It runs in one coherent sweep - nothing, then the
/// numbers, then the text, then the instants, then the containers - so a kind
/// added later takes the next free number rather than displacing an existing
/// one.
const fn value_rank(value: &Scalar) -> u8 {
    match value {
        Scalar::Null => 0,
        Scalar::Bool(_) => 1,
        Scalar::I8(_)
        | Scalar::I16(_)
        | Scalar::I32(_)
        | Scalar::I64(_)
        | Scalar::U8(_)
        | Scalar::U16(_)
        | Scalar::U32(_)
        | Scalar::U64(_)
        | Scalar::I128(_)
        | Scalar::U128(_) => 2,
        Scalar::F16(_) | Scalar::F32(_) | Scalar::F64(_) => 3,
        Scalar::D128(..) | Scalar::D256(..) => 4,
        Scalar::String(_) => 5,
        Scalar::Bytes(_) => 6,
        Scalar::Date32(..) | Scalar::Date64(..) => 7,
        Scalar::Time32(..) | Scalar::Time64(..) => 8,
        Scalar::DateTime64(..) => 9,
        Scalar::Duration32(..) | Scalar::Duration64(..) => 10,
        Scalar::Sequence(_) => 11,
        Scalar::Mapping(_) => 12,
        Scalar::Record(_) => 13,
        Scalar::Geospatial(_) => 14,
        Scalar::Enum(_) => 15,
    }
}

impl Scalar {
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
            Self::F16(_) => "f16",
            Self::F32(_) => "f32",
            Self::F64(_) => "f64",
            Self::D128(..) => "d128",
            Self::D256(..) => "d256",
            Self::String(_) => "string",
            Self::Enum(_) => "enum",
            Self::Bytes(_) => "bytes",
            Self::Geospatial(_) => "geospatial",
            Self::Date32(..) => "date32",
            Self::Date64(..) => "date64",
            Self::Time32(..) => "time32",
            Self::Time64(..) => "time64",
            Self::DateTime64(..) => "datetime64",
            Self::Duration32(..) => "duration32",
            Self::Duration64(..) => "duration64",
            Self::Sequence(_) => "sequence",
            Self::Mapping(_) => "mapping",
            Self::Record(_) => "record",
        }
    }

    /// Return the deterministic 64-bit hash used by every binding.
    ///
    /// This is XXH3-64 over [`Self::write_bytes`], the value's canonical byte
    /// representation, so the value and its [`Self::digest`] have one
    /// definition. Equal values hash identically across integer, float,
    /// decimal, and temporal widths, because that feed writes each family's
    /// canonical form rather than its storage width.
    ///
    /// ```
    /// use yggdryl::{DigestAlgorithm, Scalar};
    ///
    /// assert_eq!(Scalar::I8(1).stable_hash(), Scalar::I64(1).stable_hash());
    /// assert_eq!(
    ///     Scalar::from("AAPL").stable_hash(),
    ///     Scalar::from("AAPL").digest(DigestAlgorithm::Xxh3_64).as_u64().unwrap(),
    /// );
    /// ```
    pub fn stable_hash(&self) -> u64 {
        let mut state = crate::xxhash::Xxh3_64::new();
        self.write_bytes(&mut state);
        state.as_u64()
    }

    /// Construct an ordered sequence.
    pub fn from_sequence(values: impl IntoIterator<Item = Self>) -> Self {
        let values = values.into_iter().collect::<Vec<_>>();
        if values.is_empty() {
            static EMPTY: OnceLock<Arc<[Scalar]>> = OnceLock::new();
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
            // `Scalar`'s hash reads canonical content only, never the
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
            static EMPTY: OnceLock<Arc<[(Scalar, Scalar)]>> = OnceLock::new();
            return Ok(Self::Mapping(Arc::clone(
                EMPTY.get_or_init(|| Arc::from([])),
            )));
        }
        Ok(Self::Mapping(entries.into()))
    }

    /// Construct a deterministic record sorted by field name.
    ///
    /// # Errors
    ///
    /// Returns an error when a field name occurs more than once.
    pub fn from_record<K, I>(entries: I) -> Result<Self>
    where
        K: Into<SmolStr>,
        I: IntoIterator<Item = (K, Self)>,
    {
        let mut record = BTreeMap::new();
        for (index, (name, value)) in entries.into_iter().enumerate() {
            if record.insert(name.into(), value).is_some() {
                return Err(Error::Codec {
                    format: "value",
                    position: index,
                    reason: "record contains a duplicate field name".into(),
                });
            }
        }
        if record.is_empty() {
            static EMPTY: OnceLock<Arc<BTreeMap<SmolStr, Scalar>>> = OnceLock::new();
            return Ok(Self::Record(Arc::clone(
                EMPTY.get_or_init(|| Arc::new(BTreeMap::new())),
            )));
        }
        Ok(Self::Record(Arc::new(record)))
    }

    /// Return a boolean when this is a boolean.
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Return a string slice when this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value.as_str()),
            Self::Enum(value) => Some(value.as_str()),
            _ => None,
        }
    }

    /// Return the retained generic enum member.
    pub const fn as_enum(&self) -> Option<&EnumScalar> {
        match self {
            Self::Enum(value) => Some(value),
            _ => None,
        }
    }

    /// Return borrowed UTF-8 text when this is a string.
    pub fn as_utf8(&self) -> Option<&str> {
        self.as_str()
    }

    /// Return bytes when this is a byte value.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) | Self::Geospatial(value) => Some(value),
            _ => None,
        }
    }

    /// Encode this value as compact JSON bytes.
    pub fn as_json_bytes(&self) -> Result<Vec<u8>> {
        crate::text::json::into_bytes(self)
    }

    /// Encode this value as compact JSON UTF-8.
    pub fn as_json_utf8(&self) -> Result<String> {
        crate::text::json::into_utf8(self)
    }

    /// Return the Well-Known Binary payload without allocating.
    ///
    /// A geospatial column also accepts plain [`Self::Bytes`] on the way in -
    /// canonicalization is what rewrites it - so this reads both spellings.
    pub fn as_wkb(&self) -> Option<&[u8]> {
        self.as_bytes()
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

    /// Return record fields in deterministic name order.
    pub fn as_record(&self) -> Option<&BTreeMap<SmolStr, Self>> {
        match self {
            Self::Record(entries) => Some(entries),
            _ => None,
        }
    }

    /// Return the number of direct children or mapping entries.
    pub fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
            Self::Record(entries) => entries.len(),
            _ => 0,
        }
    }

    /// Return whether this is an empty sequence or mapping.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Sequence(values) if values.is_empty())
            || matches!(self, Self::Mapping(entries) if entries.is_empty())
            || matches!(self, Self::Record(entries) if entries.is_empty())
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
        if let Self::Record(entries) = self {
            return entries.get(key);
        }
        self.as_mapping()?
            .iter()
            .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
    }

    /// Iterate over sequence values, mapping keys, or record field values.
    ///
    /// Use [`Self::record_iter`] when both a record field's name and value are
    /// needed.
    pub fn iter(&self) -> Children<'_> {
        match self {
            Self::Sequence(values) => Children::Sequence(values.iter()),
            Self::Mapping(entries) => Children::Mapping(entries.iter()),
            Self::Record(entries) => Children::Record(entries.values()),
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

    /// Iterate over record name/value pairs in deterministic name order.
    pub fn record_iter(&self) -> std::collections::btree_map::Iter<'_, SmolStr, Self> {
        static EMPTY: OnceLock<BTreeMap<SmolStr, Scalar>> = OnceLock::new();
        self.as_record()
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeMap::new))
            .iter()
    }

    /// Return whether this is the null value.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Return whether this value holds other values.
    pub const fn is_container(&self) -> bool {
        matches!(self, Self::Sequence(_) | Self::Mapping(_) | Self::Record(_))
    }

    /// Return whether this is a number of any width.
    pub const fn is_number(&self) -> bool {
        self.is_integer()
            || matches!(
                self,
                Self::F16(_) | Self::F32(_) | Self::F64(_) | Self::D128(..) | Self::D256(..)
            )
    }

    /// Look one value up by a dotted path of mapping keys and sequence indexes.
    ///
    /// `"legs.0.price"` reads the mapping key `legs`, then index `0`, then the
    /// key `price`. A segment that does not resolve returns `None`, so probing a
    /// shape needs no nested matching.
    ///
    /// ```
    /// use yggdryl::Scalar;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = Scalar::from_mapping([(
    ///     Scalar::from("legs"),
    ///     Scalar::from_sequence([Scalar::from_mapping([(
    ///         Scalar::from("price"),
    ///         Scalar::from(12_i64),
    ///     )])?]),
    /// )])?;
    ///
    /// assert_eq!(order.path("legs.0.price").and_then(Scalar::as_i64), Some(12));
    /// assert!(order.path("legs.9.price").is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn path(&self, path: &str) -> Option<&Self> {
        let mut current = self;
        for segment in path.split('.').filter(|segment| !segment.is_empty()) {
            current = match current {
                Self::Mapping(_) | Self::Record(_) => current.get_key_str(segment)?,
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
        if let Self::Record(entries) = self {
            return entries.keys().map(SmolStr::as_str).collect();
        }
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

    /// Return this record with one named field added or replaced.
    pub fn with_field(&self, name: impl Into<SmolStr>, value: impl Into<Self>) -> Result<Self> {
        let entries = self.as_record().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: smol_str::format_smolstr!(
                "expected a record to set a field on, got {}",
                self.kind()
            ),
        })?;
        let mut rebuilt = entries.clone();
        rebuilt.insert(name.into(), value.into());
        Ok(Self::Record(Arc::new(rebuilt)))
    }

    /// Return this record without `name`, preserving deterministic order.
    pub fn without_field(&self, name: &str) -> Result<Self> {
        let entries = self.as_record().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: smol_str::format_smolstr!(
                "expected a record to remove a field from, got {}",
                self.kind()
            ),
        })?;
        if !entries.contains_key(name) {
            return Ok(self.clone());
        }
        let mut rebuilt = entries.clone();
        rebuilt.remove(name);
        Ok(Self::Record(Arc::new(rebuilt)))
    }
}

fn duplicate_key_error(index: usize) -> Error {
    Error::Codec {
        format: "value",
        position: index,
        reason: "mapping contains a duplicate key".into(),
    }
}

#[cfg(test)]
#[path = "scalar/tests.rs"]
mod tests;
