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

use super::ascii::AsciiFamily;
use super::boolean::Boolean;
use super::bytes::Bytes;
use super::decimal::Decimal;
use super::decimal::scalars as decimal;
use super::enumeration::Enum;
use super::floating::scalars::{Float16, Float32, Float64, Floating};
use super::geospatial::Geospatial;
use super::integer::scalars::Integer;
use super::nested::{Children, Mapping, Nested, Record, Sequence};
use super::temporal::scalars::{Temporal, temporal_key};
use super::text::Text;
use super::uuid::Uuid;

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
    ///
    /// Values whose physical parameters cannot be represented by a valid
    /// [`DataType`] return a typed error instead of guessing or panicking.
    fn dtype(&self) -> Result<DataType>;
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
    fn dtype(&self) -> Result<DataType>;
    /// Widen this family value to the dynamic scalar root.
    fn into_scalar(self) -> Scalar;
    /// Narrow a dynamic scalar to this family without re-validating it.
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}

/// The shared deterministic scalar spanning native and structured formats.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Scalar {
    /// The null value.
    Null,
    /// A boolean.
    Boolean(Boolean),
    /// A signed or unsigned exact integer.
    Integer(Integer),
    /// An IEEE floating-point value.
    Floating(Floating),
    /// An exact coefficient-and-scale decimal.
    Decimal(Decimal),
    /// A temporal or interval value.
    Temporal(Temporal),
    /// A Unicode string retaining its storage representation.
    Text(Text),
    /// Validated ASCII text or a registered code.
    Ascii(AsciiFamily),
    /// An RFC 9562 identifier.
    Uuid(Uuid),
    /// One identity-preserving member of a shared static enum.
    Enum(Enum),
    /// Opaque bytes retaining their storage representation.
    Bytes(Bytes),
    /// Geometry or geography as validated Well-Known Binary.
    Geospatial(Geospatial),
    /// A schema-free ordered, mapped, or named nested value.
    Nested(Nested),
}

const _: () = assert!(std::mem::size_of::<Scalar>() == 48);

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
            Self::Boolean(value) => tagged(serializer, "bool", &value.get()),
            Self::Integer(value) => match value {
                Integer::I8(value) => tagged(serializer, "i8", &value.get()),
                Integer::I16(value) => tagged(serializer, "i16", &value.get()),
                Integer::I32(value) => tagged(serializer, "i32", &value.get()),
                Integer::I64(value) => tagged(serializer, "i64", &value.get()),
                Integer::U8(value) => tagged(serializer, "u8", &value.get()),
                Integer::U16(value) => tagged(serializer, "u16", &value.get()),
                Integer::U32(value) => tagged(serializer, "u32", &value.get()),
                Integer::U64(value) => tagged(serializer, "u64", &value.get()),
                Integer::I128(value) => tagged(serializer, "i128", &value.get()),
                Integer::U128(value) => tagged(serializer, "u128", &value.get()),
            },
            Self::Floating(value) => match value {
                Floating::F16(value) => tagged(serializer, "f16", value),
                Floating::F32(value) => tagged(serializer, "f32", value),
                Floating::F64(value) => tagged(serializer, "f64", value),
            },
            Self::Decimal(value) => match value {
                Decimal::D32(value) => tagged(
                    serializer,
                    "d32",
                    &Pair(&value.coefficient(), &value.scale()),
                ),
                Decimal::D64(value) => tagged(
                    serializer,
                    "d64",
                    &Pair(&value.coefficient(), &value.scale()),
                ),
                Decimal::D128(value) => tagged(
                    serializer,
                    "d128",
                    &Pair(&value.coefficient(), &value.scale()),
                ),
                Decimal::D256(value) => tagged(
                    serializer,
                    "d256",
                    &Pair(&value.coefficient(), &value.scale()),
                ),
            },
            Self::Text(value) => match value {
                Text::Utf8(value) => tagged(serializer, "string", &value.as_str()),
                Text::LargeUtf8(value) => tagged(serializer, "large_utf8", &value.as_str()),
                Text::Utf8View(value) => tagged(serializer, "utf8_view", &value.as_str()),
            },
            Self::Ascii(value) => match value {
                AsciiFamily::Ascii(value) => tagged(serializer, "ascii", &value.as_str()),
                AsciiFamily::FixedAscii(value) => tagged(
                    serializer,
                    "fixed_ascii",
                    &Pair(&value.as_str(), &value.width()),
                ),
                AsciiFamily::Country(value) => tagged(serializer, "country", &value.as_str()),
                AsciiFamily::Currency(value) => tagged(serializer, "currency", &value.as_str()),
                AsciiFamily::Mic(value) => tagged(serializer, "mic", &value.as_str()),
                AsciiFamily::Cfi(value) => tagged(serializer, "cfi", &value.as_str()),
            },
            Self::Uuid(value) => tagged(serializer, "uuid", &value.to_string()),
            Self::Enum(value) => tagged(serializer, "enum", value),
            Self::Bytes(value) => match value {
                Bytes::Binary(value) => tagged(serializer, "bytes", &value.as_bytes()),
                Bytes::FixedSizeBinary(value) => {
                    tagged(serializer, "fixed_size_binary", &value.as_bytes())
                }
                Bytes::LargeBinary(value) => tagged(serializer, "large_binary", &value.as_bytes()),
                Bytes::BinaryView(value) => tagged(serializer, "binary_view", &value.as_bytes()),
            },
            Self::Geospatial(value) => match value {
                Geospatial::Geometry(value) => tagged(serializer, "geospatial", &value.as_bytes()),
                Geospatial::Geography(value) => tagged(serializer, "geography", &value.as_bytes()),
            },
            // A temporal is its classic ISO spelling wherever it has one; a
            // reading with no classic spelling keeps its structural parts.
            Self::Temporal(Temporal::Date32(value)) => match super::iso::format_date(value.count())
            {
                Some(spelled) if value.unit() == TimeUnit::Day && value.timezone().is_naive() => {
                    tagged(serializer, "date32", &spelled)
                }
                _ => tagged(
                    serializer,
                    "date32",
                    &Triple(&value.count(), &value.unit(), &value.timezone()),
                ),
            },
            Self::Temporal(Temporal::Date64(value)) => tagged(
                serializer,
                "date64",
                &Triple(&value.count(), &value.unit(), &value.timezone()),
            ),
            Self::Temporal(Temporal::Time32(value)) => {
                match super::iso::format_time(i64::from(value.count()), value.unit()) {
                    Some(spelled) if value.timezone().is_naive() => {
                        tagged(serializer, "time32", &spelled)
                    }
                    _ => tagged(
                        serializer,
                        "time32",
                        &Triple(&value.count(), &value.unit(), &value.timezone()),
                    ),
                }
            }
            Self::Temporal(Temporal::Time64(value)) => {
                match super::iso::format_time(value.count(), value.unit()) {
                    Some(spelled) if value.timezone().is_naive() => {
                        tagged(serializer, "time64", &spelled)
                    }
                    _ => tagged(
                        serializer,
                        "time64",
                        &Triple(&value.count(), &value.unit(), &value.timezone()),
                    ),
                }
            }
            Self::Temporal(Temporal::DateTime64(value)) => {
                let spelled = if value.timezone().is_naive() {
                    super::iso::format_datetime(value.count(), value.unit())
                } else {
                    super::iso::format_timestamp(value.count(), value.unit(), &value.timezone())
                };
                match spelled {
                    Some(spelled) => tagged(serializer, "datetime64", &spelled),
                    None => tagged(
                        serializer,
                        "datetime64",
                        &Triple(&value.count(), &value.unit(), &value.timezone()),
                    ),
                }
            }
            Self::Temporal(Temporal::Duration32(value)) => {
                match super::iso::format_duration(i64::from(value.count()), value.unit()) {
                    Some(spelled) if value.timezone().is_naive() => {
                        tagged(serializer, "duration32", &spelled)
                    }
                    _ => tagged(
                        serializer,
                        "duration32",
                        &Triple(&value.count(), &value.unit(), &value.timezone()),
                    ),
                }
            }
            Self::Temporal(Temporal::Duration64(value)) => {
                match super::iso::format_duration(value.count(), value.unit()) {
                    Some(spelled) if value.timezone().is_naive() => {
                        tagged(serializer, "duration64", &spelled)
                    }
                    _ => tagged(
                        serializer,
                        "duration64",
                        &Triple(&value.count(), &value.unit(), &value.timezone()),
                    ),
                }
            }
            Self::Temporal(Temporal::Interval(value)) => tagged(serializer, "interval", value),
            Self::Nested(Nested::Sequence(values)) => {
                tagged(serializer, "sequence", &values.as_slice())
            }
            Self::Nested(Nested::Mapping(entries)) => {
                tagged(serializer, "mapping", &entries.as_slice())
            }
            Self::Nested(Nested::Record(entries)) => {
                tagged(serializer, "record", &entries.as_map())
            }
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
            D32(i32, i8),
            D64(i64, i8),
            D128(i128, i8),
            D256(I256, i8),
            String(SmolStr),
            LargeUtf8(SmolStr),
            Utf8View(SmolStr),
            Ascii(SmolStr),
            FixedAscii(SmolStr, i32),
            Country(SmolStr),
            Currency(SmolStr),
            Mic(SmolStr),
            Cfi(SmolStr),
            Uuid(SmolStr),
            Enum(Enum),
            Bytes(Arc<[u8]>),
            FixedSizeBinary(Arc<[u8]>),
            LargeBinary(Arc<[u8]>),
            BinaryView(Arc<[u8]>),
            Geospatial(Arc<[u8]>),
            Geography(Arc<[u8]>),
            Date32(Temporal32),
            Date64(Temporal64),
            Time32(Temporal32),
            Time64(Temporal64),
            #[serde(rename = "datetime64")]
            DateTime64(Temporal64),
            Duration32(Temporal32),
            Duration64(Temporal64),
            Interval(super::temporal::Interval),
            Sequence(Vec<Scalar>),
            Mapping(Vec<(Scalar, Scalar)>),
            Record(RecordEntries),
        }

        match StructuralValue::deserialize(deserializer)? {
            StructuralValue::Null => Ok(Self::Null),
            StructuralValue::Bool(value) => Ok(Self::from(value)),
            StructuralValue::I8(value) => Ok(Self::from(value)),
            StructuralValue::I16(value) => Ok(Self::from(value)),
            StructuralValue::I32(value) => Ok(Self::from(value)),
            StructuralValue::I64(value) => Ok(Self::from(value)),
            StructuralValue::U8(value) => Ok(Self::from(value)),
            StructuralValue::U16(value) => Ok(Self::from(value)),
            StructuralValue::U32(value) => Ok(Self::from(value)),
            StructuralValue::U64(value) => Ok(Self::from(value)),
            StructuralValue::I128(value) => Ok(Self::from(value)),
            StructuralValue::U128(value) => Ok(Self::from(value)),
            StructuralValue::F16(value) => Ok(Self::Floating(Floating::F16(value))),
            StructuralValue::F32(value) => Ok(Self::Floating(Floating::F32(value))),
            StructuralValue::F64(value) => Ok(Self::Floating(Floating::F64(value))),
            StructuralValue::D32(unscaled, scale) => Ok(Self::Decimal(Decimal::D32(
                super::decimal::Decimal32::new(unscaled, scale),
            ))),
            StructuralValue::D64(unscaled, scale) => Ok(Self::Decimal(Decimal::D64(
                super::decimal::Decimal64::new(unscaled, scale),
            ))),
            StructuralValue::D128(unscaled, scale) => Ok(Self::d128(unscaled, scale)),
            StructuralValue::D256(unscaled, scale) => Ok(Self::d256(unscaled, scale)),
            StructuralValue::String(value) => Ok(Self::from(value)),
            StructuralValue::LargeUtf8(value) => Ok(Self::Text(Text::LargeUtf8(
                super::text::LargeUtf8::new(value),
            ))),
            StructuralValue::Utf8View(value) => Ok(Self::Text(Text::Utf8View(
                super::text::Utf8View::new(value),
            ))),
            StructuralValue::Ascii(value) => super::ascii::Ascii::new(value)
                .map(|value| Self::Ascii(AsciiFamily::Ascii(value)))
                .map_err(D::Error::custom),
            StructuralValue::FixedAscii(value, width) => {
                super::ascii::FixedAscii::new(value, width)
                    .map(|value| Self::Ascii(AsciiFamily::FixedAscii(value)))
                    .map_err(D::Error::custom)
            }
            StructuralValue::Country(value) => super::ascii::Country::new(value)
                .map(|value| Self::Ascii(AsciiFamily::Country(value)))
                .map_err(D::Error::custom),
            StructuralValue::Currency(value) => super::ascii::Currency::new(value)
                .map(|value| Self::Ascii(AsciiFamily::Currency(value)))
                .map_err(D::Error::custom),
            StructuralValue::Mic(value) => super::ascii::Mic::new(value)
                .map(|value| Self::Ascii(AsciiFamily::Mic(value)))
                .map_err(D::Error::custom),
            StructuralValue::Cfi(value) => super::ascii::Cfi::new(value)
                .map(|value| Self::Ascii(AsciiFamily::Cfi(value)))
                .map_err(D::Error::custom),
            StructuralValue::Uuid(value) => Uuid::from_bytes(value.as_bytes())
                .map(Self::Uuid)
                .map_err(D::Error::custom),
            StructuralValue::Enum(value) => Ok(Self::Enum(value)),
            StructuralValue::Bytes(value) => Ok(Self::from(value)),
            StructuralValue::FixedSizeBinary(value) => Ok(Self::Bytes(Bytes::FixedSizeBinary(
                super::bytes::FixedSizeBinary::new(value),
            ))),
            StructuralValue::LargeBinary(value) => Ok(Self::Bytes(Bytes::LargeBinary(
                super::bytes::LargeBinary::new(value),
            ))),
            StructuralValue::BinaryView(value) => Ok(Self::Bytes(Bytes::BinaryView(
                super::bytes::BinaryView::new(value),
            ))),
            StructuralValue::Geospatial(value) => super::geospatial::Geometry::new(value)
                .map(|value| Self::Geospatial(Geospatial::Geometry(value)))
                .map_err(D::Error::custom),
            StructuralValue::Geography(value) => super::geospatial::Geography::new(value)
                .map(|value| Self::Geospatial(Geospatial::Geography(value)))
                .map_err(D::Error::custom),
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
            StructuralValue::Interval(value) => Ok(Self::Temporal(Temporal::Interval(value))),
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
                return left.1.cmp(&right.1).then_with(|| left.2.cmp(&right.2));
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
            Self::Boolean(left) => same_kind!(Self::Boolean(right) => left.cmp(right)),
            Self::Integer(_) => unreachable!("every integer width returned above"),
            Self::Floating(_) => unreachable!("all float widths returned above"),
            Self::Decimal(_) => unreachable!("all decimal widths returned above"),
            Self::Temporal(left) => same_kind!(Self::Temporal(right) => left.cmp(right)),
            Self::Text(left) => same_kind!(Self::Text(right) => left.cmp(right)),
            Self::Ascii(left) => same_kind!(Self::Ascii(right) => left.cmp(right)),
            Self::Uuid(left) => same_kind!(Self::Uuid(right) => left.cmp(right)),
            Self::Enum(left) => same_kind!(Self::Enum(right) => left.cmp(right)),
            Self::Bytes(left) => same_kind!(Self::Bytes(right) => left.cmp(right)),
            Self::Geospatial(left) => same_kind!(Self::Geospatial(right) => left.cmp(right)),
            Self::Nested(left) => same_kind!(Self::Nested(right) => left.cmp(right)),
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
            Self::Boolean(value) => value.hash(state),
            Self::Integer(_) => unreachable!("integer values returned above"),
            Self::Floating(_) => unreachable!("float values returned above"),
            Self::Decimal(_) => unreachable!("decimal values returned above"),
            Self::Temporal(value) => value.hash(state),
            Self::Text(value) => value.hash(state),
            Self::Ascii(value) => value.hash(state),
            Self::Uuid(value) => value.hash(state),
            Self::Enum(value) => value.hash(state),
            Self::Bytes(value) => value.hash(state),
            Self::Geospatial(value) => value.hash(state),
            Self::Nested(value) => value.hash(state),
        }
    }
}

fn decimal_value(value: &Scalar) -> Option<(I256, i8)> {
    value.as_decimal()
}

/// The family, normalized count, and zone of one temporal.
fn temporal_value(value: &Scalar) -> Option<(super::TemporalFamily, (u8, i128), Timezone)> {
    let temporal = value.as_temporal()?;
    let count = match temporal {
        Temporal::Date32(value) => i64::from(value.count()),
        Temporal::Date64(value) => value.count(),
        Temporal::Time32(value) => i64::from(value.count()),
        Temporal::Time64(value) => value.count(),
        Temporal::DateTime64(value) => value.count(),
        Temporal::Duration32(value) => i64::from(value.count()),
        Temporal::Duration64(value) => value.count(),
        Temporal::Interval(_) => return None,
    };
    Some((
        (*temporal).family(),
        temporal_key(count, (*temporal).unit()),
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
        Scalar::Boolean(_) => 1,
        Scalar::Integer(_) => 2,
        Scalar::Floating(_) => 3,
        Scalar::Decimal(_) => 4,
        Scalar::Text(_) => 5,
        Scalar::Bytes(_) => 6,
        Scalar::Temporal(Temporal::Date32(_) | Temporal::Date64(_)) => 7,
        Scalar::Temporal(Temporal::Time32(_) | Temporal::Time64(_)) => 8,
        Scalar::Temporal(Temporal::DateTime64(_)) => 9,
        Scalar::Temporal(Temporal::Duration32(_) | Temporal::Duration64(_)) => 10,
        Scalar::Nested(Nested::Sequence(_)) => 11,
        Scalar::Nested(Nested::Mapping(_)) => 12,
        Scalar::Nested(Nested::Record(_)) => 13,
        Scalar::Geospatial(_) => 14,
        Scalar::Enum(_) => 15,
        Scalar::Temporal(Temporal::Interval(_)) => 16,
        Scalar::Uuid(_) => 17,
        Scalar::Ascii(_) => 18,
    }
}

impl Scalar {
    /// Return the most specific datatype identifier the value itself proves.
    ///
    /// Nested values report their most-general shape; a [`Field`](crate::Field)
    /// narrows a sequence to a list, fixed-size list, struct, or union. Static
    /// enum members report UTF-8 because their column representation remains a
    /// field-level choice.
    pub const fn id(&self) -> DataTypeId {
        match self {
            Self::Null => DataTypeId::Null,
            Self::Boolean(_) => DataTypeId::Boolean,
            Self::Integer(Integer::I8(_)) => DataTypeId::Int8,
            Self::Integer(Integer::I16(_)) => DataTypeId::Int16,
            Self::Integer(Integer::I32(_)) => DataTypeId::Int32,
            Self::Integer(Integer::I64(_)) => DataTypeId::Int64,
            Self::Integer(Integer::I128(_)) => DataTypeId::Int128,
            Self::Integer(Integer::U8(_)) => DataTypeId::UInt8,
            Self::Integer(Integer::U16(_)) => DataTypeId::UInt16,
            Self::Integer(Integer::U32(_)) => DataTypeId::UInt32,
            Self::Integer(Integer::U64(_)) => DataTypeId::UInt64,
            Self::Integer(Integer::U128(_)) => DataTypeId::UInt128,
            Self::Floating(Floating::F16(_)) => DataTypeId::Float16,
            Self::Floating(Floating::F32(_)) => DataTypeId::Float32,
            Self::Floating(Floating::F64(_)) => DataTypeId::Float64,
            Self::Decimal(Decimal::D32(_)) => DataTypeId::Decimal32,
            Self::Decimal(Decimal::D64(_)) => DataTypeId::Decimal64,
            Self::Decimal(Decimal::D128(_)) => DataTypeId::Decimal128,
            Self::Decimal(Decimal::D256(_)) => DataTypeId::Decimal256,
            Self::Temporal(Temporal::Date32(_)) => DataTypeId::Date32,
            Self::Temporal(Temporal::Date64(_)) => DataTypeId::Date64,
            Self::Temporal(Temporal::Time32(_)) => DataTypeId::Time32,
            Self::Temporal(Temporal::Time64(_)) => DataTypeId::Time64,
            Self::Temporal(Temporal::DateTime64(_)) => DataTypeId::DateTime64,
            Self::Temporal(Temporal::Duration32(_)) => DataTypeId::Duration32,
            Self::Temporal(Temporal::Duration64(_)) => DataTypeId::Duration64,
            Self::Temporal(Temporal::Interval(_)) => DataTypeId::Interval,
            Self::Text(Text::Utf8(_)) => DataTypeId::Utf8,
            Self::Text(Text::LargeUtf8(_)) => DataTypeId::LargeUtf8,
            Self::Text(Text::Utf8View(_)) => DataTypeId::Utf8View,
            Self::Ascii(AsciiFamily::Ascii(_)) => DataTypeId::Ascii,
            Self::Ascii(AsciiFamily::FixedAscii(_)) => DataTypeId::FixedAscii,
            Self::Ascii(AsciiFamily::Country(_)) => DataTypeId::Country,
            Self::Ascii(AsciiFamily::Currency(_)) => DataTypeId::Currency,
            Self::Ascii(AsciiFamily::Mic(_)) => DataTypeId::Mic,
            Self::Ascii(AsciiFamily::Cfi(_)) => DataTypeId::Cfi,
            Self::Uuid(_) => DataTypeId::Uuid,
            Self::Enum(_) => DataTypeId::Utf8,
            Self::Bytes(Bytes::Binary(_)) => DataTypeId::Binary,
            Self::Bytes(Bytes::FixedSizeBinary(_)) => DataTypeId::FixedSizeBinary,
            Self::Bytes(Bytes::LargeBinary(_)) => DataTypeId::LargeBinary,
            Self::Bytes(Bytes::BinaryView(_)) => DataTypeId::BinaryView,
            Self::Geospatial(Geospatial::Geometry(_)) => DataTypeId::Geometry,
            Self::Geospatial(Geospatial::Geography(_)) => DataTypeId::Geography,
            Self::Nested(Nested::Sequence(_)) => DataTypeId::List,
            Self::Nested(Nested::Mapping(_)) => DataTypeId::Map,
            Self::Nested(Nested::Record(_)) => DataTypeId::Struct,
        }
    }

    /// Return the datatype family the value itself proves.
    pub const fn family(&self) -> DataTypeKind {
        self.id().kind()
    }

    /// The canonical vocabulary name for this value's kind, such as `mapping`.
    ///
    /// This is the spelling every error message uses for an observed value, so
    /// a caller reading `expected string, got mapping` sees the same words the
    /// documentation and the bindings use.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean(_) => "boolean",
            Self::Integer(Integer::I8(_)) => "i8",
            Self::Integer(Integer::I16(_)) => "i16",
            Self::Integer(Integer::I32(_)) => "i32",
            Self::Integer(Integer::I64(_)) => "i64",
            Self::Integer(Integer::U8(_)) => "u8",
            Self::Integer(Integer::U16(_)) => "u16",
            Self::Integer(Integer::U32(_)) => "u32",
            Self::Integer(Integer::U64(_)) => "u64",
            Self::Integer(Integer::I128(_)) => "i128",
            Self::Integer(Integer::U128(_)) => "u128",
            Self::Floating(Floating::F16(_)) => "f16",
            Self::Floating(Floating::F32(_)) => "f32",
            Self::Floating(Floating::F64(_)) => "f64",
            Self::Decimal(Decimal::D32(_)) => "d32",
            Self::Decimal(Decimal::D64(_)) => "d64",
            Self::Decimal(Decimal::D128(_)) => "d128",
            Self::Decimal(Decimal::D256(_)) => "d256",
            Self::Text(Text::Utf8(_)) => "string",
            Self::Text(Text::LargeUtf8(_)) => "large_utf8",
            Self::Text(Text::Utf8View(_)) => "utf8_view",
            Self::Ascii(AsciiFamily::Ascii(_)) => "ascii",
            Self::Ascii(AsciiFamily::FixedAscii(_)) => "fixed_ascii",
            Self::Ascii(AsciiFamily::Country(_)) => "country",
            Self::Ascii(AsciiFamily::Currency(_)) => "currency",
            Self::Ascii(AsciiFamily::Mic(_)) => "mic",
            Self::Ascii(AsciiFamily::Cfi(_)) => "cfi",
            Self::Uuid(_) => "uuid",
            Self::Enum(_) => "enum",
            Self::Bytes(Bytes::Binary(_)) => "bytes",
            Self::Bytes(Bytes::FixedSizeBinary(_)) => "fixed_size_binary",
            Self::Bytes(Bytes::LargeBinary(_)) => "large_binary",
            Self::Bytes(Bytes::BinaryView(_)) => "binary_view",
            Self::Geospatial(Geospatial::Geometry(_)) => "geospatial",
            Self::Geospatial(Geospatial::Geography(_)) => "geography",
            Self::Temporal(Temporal::Date32(_)) => "date32",
            Self::Temporal(Temporal::Date64(_)) => "date64",
            Self::Temporal(Temporal::Time32(_)) => "time32",
            Self::Temporal(Temporal::Time64(_)) => "time64",
            Self::Temporal(Temporal::DateTime64(_)) => "datetime64",
            Self::Temporal(Temporal::Duration32(_)) => "duration32",
            Self::Temporal(Temporal::Duration64(_)) => "duration64",
            Self::Temporal(Temporal::Interval(_)) => "interval",
            Self::Nested(Nested::Sequence(_)) => "sequence",
            Self::Nested(Nested::Mapping(_)) => "mapping",
            Self::Nested(Nested::Record(_)) => "record",
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
    /// assert_eq!(Scalar::from(1).stable_hash(), Scalar::from(1).stable_hash());
    /// assert_eq!(
    ///     Scalar::from("AAPL").stable_hash(),
    ///     Scalar::from("AAPL").digest(DigestAlgorithm::Xxh3).as_u64().unwrap(),
    /// );
    /// ```
    pub fn stable_hash(&self) -> u64 {
        let mut state = crate::xxhash::Xxh3::new();
        self.write_bytes(&mut state);
        state.as_u64()
    }

    /// Construct an ordered sequence.
    pub fn from_sequence(values: impl IntoIterator<Item = Self>) -> Self {
        let values = values.into_iter().collect::<Vec<_>>();
        if values.is_empty() {
            static EMPTY: OnceLock<Arc<[Scalar]>> = OnceLock::new();
            return Self::Nested(Nested::Sequence(Sequence::new(Arc::clone(
                EMPTY.get_or_init(|| Arc::from([])),
            ))));
        }
        Self::Nested(Nested::Sequence(Sequence::new(Arc::<[Scalar]>::from(
            values,
        ))))
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
            return Ok(Self::Nested(Nested::Mapping(Mapping::new(Arc::clone(
                EMPTY.get_or_init(|| Arc::from([])),
            )))));
        }
        Ok(Self::Nested(Nested::Mapping(Mapping::new(Arc::<
            [(Scalar, Scalar)],
        >::from(
            entries
        )))))
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
            return Ok(Self::Nested(Nested::Record(Record::new(Arc::clone(
                EMPTY.get_or_init(|| Arc::new(BTreeMap::new())),
            )))));
        }
        Ok(Self::Nested(Nested::Record(Record::new(Arc::new(record)))))
    }

    /// Return a boolean when this is a boolean.
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(value.get()),
            _ => None,
        }
    }

    /// Return a string slice when this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_str()),
            Self::Ascii(value) => Some(value.as_str()),
            Self::Enum(value) => Some(value.as_str()),
            _ => None,
        }
    }

    /// Return the retained generic enum member.
    pub const fn as_enum(&self) -> Option<&Enum> {
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
            Self::Bytes(value) => Some(value.as_bytes()),
            Self::Geospatial(value) => Some(value.as_bytes()),
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
            Self::Nested(Nested::Sequence(values)) => Some(values.as_slice()),
            _ => None,
        }
    }

    /// Return mapping entries without allocating.
    pub fn as_mapping(&self) -> Option<&[(Self, Self)]> {
        match self {
            Self::Nested(Nested::Mapping(entries)) => Some(entries.as_slice()),
            _ => None,
        }
    }

    /// Return record fields in deterministic name order.
    pub fn as_record(&self) -> Option<&BTreeMap<SmolStr, Self>> {
        match self {
            Self::Nested(Nested::Record(entries)) => Some(entries.as_map()),
            _ => None,
        }
    }

    /// Return the number of direct children or mapping entries.
    pub fn len(&self) -> usize {
        match self {
            Self::Nested(value) => value.len(),
            _ => 0,
        }
    }

    /// Return whether this is an empty sequence or mapping.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Nested(value) if value.is_empty())
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
        if let Self::Nested(Nested::Record(entries)) = self {
            return entries.as_map().get(key);
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
            Self::Nested(Nested::Sequence(values)) => Children::Sequence(values.as_slice().iter()),
            Self::Nested(Nested::Mapping(entries)) => Children::Mapping(entries.as_slice().iter()),
            Self::Nested(Nested::Record(entries)) => Children::Record(entries.as_map().values()),
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
        matches!(self, Self::Nested(_))
    }

    /// Return whether this is a number of any width.
    pub const fn is_number(&self) -> bool {
        self.is_integer() || matches!(self, Self::Floating(_) | Self::Decimal(_))
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
                Self::Nested(Nested::Mapping(_) | Nested::Record(_)) => {
                    current.get_key_str(segment)?
                }
                Self::Nested(Nested::Sequence(_)) => current.get(segment.parse::<usize>().ok()?)?,
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
        if let Self::Nested(Nested::Record(entries)) = self {
            return entries.as_map().keys().map(SmolStr::as_str).collect();
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
        Ok(Self::Nested(Nested::Record(Record::new(Arc::new(rebuilt)))))
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
        Ok(Self::Nested(Nested::Record(Record::new(Arc::new(rebuilt)))))
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
