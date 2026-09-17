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
//! assert_eq!(quote.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
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
    DataType, DataTypeId, DataTypeKind, Error, MediaType, MimeType, Result, TimeUnit, Timezone,
    i256,
};

use super::boolean::Boolean;
use super::bytes::Bytes;
use super::decimal::scalars as decimal;
use super::decimal::{Decimal32, Decimal64, Decimal128, Decimal256};
use super::enumeration::Enum;
use super::floating::scalars::{Float16, Float32, Float64};
use super::geospatial::{Geography, Geometry};
use super::integer::scalars::{compare_integer_parts, integer_parts};
use super::integer::{Int8, Int16, Int32, Int64, Int128, UInt8, UInt16, UInt32, UInt64, UInt128};
use super::nested::{Children, Mapping, Record, Sequence};
use super::string::{Code, Str};
use super::temporal::scalars::temporal_key;
use super::temporal::{
    Date32, Date64, DateTime64, Duration32, Duration64, Interval, Time32, Time64,
};
use super::uuid::Uuid;
use super::version::Version;

/// One concrete scalar representation.
///
/// Implementors are the final representation a [`Scalar`] variant holds.
/// Narrowing an existing scalar only projects a reference; validation remains
/// owned by [`DataType::scalar`](crate::DataType::scalar) and
/// [`Field::scalar`](crate::Field::scalar).
pub trait Value:
    Sized + Clone + fmt::Debug + fmt::Display + Eq + Ord + Hash + Send + Sync + 'static
{
    /// Return the datatype this value materializes into.
    ///
    /// Values whose physical parameters cannot be represented by a valid
    /// [`DataType`] return a typed error instead of guessing or panicking.
    fn dtype(&self) -> Result<DataType>;
    /// Widen this leaf to the dynamic scalar root.
    fn into_scalar(self) -> Scalar;
    /// Narrow a dynamic scalar to this leaf without re-validating it.
    fn from_scalar(value: &Scalar) -> Option<&Self>;
}

/// Make one canonical text value a scalar leaf of its own.
///
/// A value that parses, canonicalizes and renders itself - a version, a time
/// zone, a MIME type - is its own family: it holds no narrower leaf and no
/// wider one holds it, so every method here is the same four lines under a
/// different name. The wrapping variant is named because a value wider than
/// the enum rides behind a shared pointer instead.
macro_rules! text_scalar_value {
    ($leaf:ty, $variant:ident, $id:expr, $dtype:expr) => {

        impl Value for $leaf {

            fn dtype(&self) -> Result<DataType> {
                Ok($dtype)
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$variant(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$variant(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl From<$leaf> for Scalar {
            fn from(value: $leaf) -> Self {
                Self::$variant(value)
            }
        }
    };
}

pub(crate) use text_scalar_value;

/// The shared deterministic scalar spanning native and structured formats.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Scalar {
    /// The null value.
    Null,
    /// A boolean.
    Boolean(Boolean),
    /// A signed 8-bit integer.
    Int8(Int8),
    /// A signed 16-bit integer.
    Int16(Int16),
    /// A signed 32-bit integer.
    Int32(Int32),
    /// A signed 64-bit integer.
    Int64(Int64),
    /// An unsigned 8-bit integer.
    UInt8(UInt8),
    /// An unsigned 16-bit integer.
    UInt16(UInt16),
    /// An unsigned 32-bit integer.
    UInt32(UInt32),
    /// An unsigned 64-bit integer.
    UInt64(UInt64),
    /// A signed 128-bit integer.
    Int128(Int128),
    /// An unsigned 128-bit integer.
    UInt128(UInt128),
    /// An IEEE binary16 float.
    Float16(Float16),
    /// An IEEE binary32 float.
    Float32(Float32),
    /// An IEEE binary64 float.
    Float64(Float64),
    /// A 32-bit coefficient-and-scale decimal.
    Decimal32(Decimal32),
    /// A 64-bit coefficient-and-scale decimal.
    Decimal64(Decimal64),
    /// A 128-bit coefficient-and-scale decimal.
    Decimal128(Decimal128),
    /// A 256-bit coefficient-and-scale decimal.
    Decimal256(Decimal256),
    /// A 32-bit day-count date.
    Date32(Date32),
    /// A 64-bit millisecond-count date.
    Date64(Date64),
    /// A 32-bit second- or millisecond-count time of day.
    Time32(Time32),
    /// A 64-bit microsecond- or nanosecond-count time of day.
    Time64(Time64),
    /// A 64-bit epoch or wall-clock datetime.
    DateTime64(DateTime64),
    /// A 32-bit elapsed duration.
    Duration32(Duration32),
    /// A 64-bit elapsed duration.
    Duration64(Duration64),
    /// A calendar interval in one of its three layouts.
    Interval(Interval),
    /// A string: its characters, and the layout and charset it is stored
    /// under.
    String(Str),
    /// A registered code: an identity with a fixed US-ASCII storage.
    Code(Code),
    /// An RFC 9562 identifier.
    Uuid(Uuid),
    /// A canonical, numerically ordered version.
    Version(Version),
    /// A validated, canonical location.
    ///
    /// Behind one shared pointer: a parsed [`crate::Url`] is far wider than
    /// this enum, and a column of them is cloned once per row.
    Url(Arc<crate::Url>),
    /// A canonical time zone name, a fixed offset, or the zone-free marker.
    Timezone(Timezone),
    /// A validated, canonical MIME type.
    MimeType(MimeType),
    /// A MIME type with its charset and content codings.
    ///
    /// Behind one shared pointer: a media type carries a base, a charset and a
    /// coding list, which is wider than this enum, and a column of them is
    /// cloned once per row.
    MediaType(Arc<MediaType>),
    /// One identity-preserving member of a shared static enum.
    Enum(Enum),
    /// Opaque bytes retaining their storage representation.
    Bytes(Bytes),
    /// Planar geometry as validated Well-Known Binary.
    Geometry(Geometry),
    /// Geographic coordinates as validated Well-Known Binary.
    Geography(Geography),
    /// A schema-free ordered sequence of values.
    Sequence(Sequence),
    /// A schema-free insertion-ordered mapping of arbitrary keys.
    Mapping(Mapping),
    /// A schema-free record of values sorted by field name.
    Record(Record),
    /// An Arrow payload - one pinned row, a column, a table, or a stream -
    /// carrying the exact field that types it, behind one shared pointer so
    /// a clone shares the buffers rather than the rows.
    ///
    /// This is how a columnar value crosses a boundary as the scalar it is:
    /// a frame, a table or a reader handed to a binding lands here and reaches
    /// the record surface without becoming rows first. Reading it as a
    /// native value - [`as_sequence`](Self::as_sequence) and every other
    /// narrowing accessor - answers `None`; [`into_native`](Self::into_native)
    /// is the one crossing into the native tree, and it drains a stream.
    #[cfg(feature = "arrow")]
    Arrow(Arc<crate::arrow::ArrowScalar>),
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
            Self::Int8(value) => tagged(serializer, "i8", &value.get()),
            Self::Int16(value) => tagged(serializer, "i16", &value.get()),
            Self::Int32(value) => tagged(serializer, "i32", &value.get()),
            Self::Int64(value) => tagged(serializer, "i64", &value.get()),
            Self::UInt8(value) => tagged(serializer, "u8", &value.get()),
            Self::UInt16(value) => tagged(serializer, "u16", &value.get()),
            Self::UInt32(value) => tagged(serializer, "u32", &value.get()),
            Self::UInt64(value) => tagged(serializer, "u64", &value.get()),
            Self::Int128(value) => tagged(serializer, "i128", &value.get()),
            Self::UInt128(value) => tagged(serializer, "u128", &value.get()),
            Self::Float16(value) => tagged(serializer, "f16", value),
            Self::Float32(value) => tagged(serializer, "f32", value),
            Self::Float64(value) => tagged(serializer, "f64", value),
            Self::Decimal32(value) => tagged(
                serializer,
                "d32",
                &Pair(&value.coefficient(), &value.scale()),
            ),
            Self::Decimal64(value) => tagged(
                serializer,
                "d64",
                &Pair(&value.coefficient(), &value.scale()),
            ),
            Self::Decimal128(value) => tagged(
                serializer,
                "d128",
                &Pair(&value.coefficient(), &value.scale()),
            ),
            Self::Decimal256(value) => tagged(
                serializer,
                "d256",
                &Pair(&value.coefficient(), &value.scale()),
            ),
            // One tag for every string. The ordinary value - UTF-8, the
            // `string` layout - writes its characters and nothing else, which
            // is what it always wrote; a layout, a charset or a fixed width
            // is what makes a value carry more than that, and `Str` writes
            // the whole declaration rather than half of it.
            Self::String(value) => tagged(serializer, "string", value),
            // A code writes its text under its own datatype's name.
            Self::Code(value) => tagged(serializer, value.identifier().as_str(), &value.as_str()),
            Self::Uuid(value) => {
                let mut slot = [0_u8; crate::types::Uuid::TEXT_LEN];
                tagged(serializer, "uuid", &value.render(&mut slot))
            }
            Self::Version(value) => tagged(serializer, "version", value),
            Self::Timezone(value) => tagged(serializer, "timezone", &value.as_str()),
            Self::MimeType(value) => tagged(serializer, "mimetype", &value.as_str()),
            Self::MediaType(value) => tagged(serializer, "mediatype", &value.to_string()),
            Self::Url(value) => tagged(serializer, "url", &value.to_string()),
            Self::Enum(value) => tagged(serializer, "enum", value),
            // One tag for every byte value: the ordinary payload writes its
            // bytes and nothing else, and a layout or a fixed width is what
            // makes a value carry more than that.
            Self::Bytes(value) => tagged(serializer, "bytes", value),
            Self::Geometry(value) => tagged(serializer, "geometry", &value.as_bytes()),
            Self::Geography(value) => tagged(serializer, "geography", &value.as_bytes()),
            // A temporal is its classic ISO spelling wherever it has one; a
            // reading with no classic spelling keeps its structural parts.
            Self::Date32(value) => match super::temporal::iso::format_date(value.count()) {
                Some(spelled) if value.unit() == TimeUnit::Day && value.timezone().is_naive() => {
                    tagged(serializer, "date32", &spelled)
                }
                _ => tagged(
                    serializer,
                    "date32",
                    &Triple(&value.count(), &value.unit(), &value.timezone()),
                ),
            },
            Self::Date64(value) => tagged(
                serializer,
                "date64",
                &Triple(&value.count(), &value.unit(), &value.timezone()),
            ),
            Self::Time32(value) => {
                match super::temporal::iso::format_time(i64::from(value.count()), value.unit()) {
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
            Self::Time64(value) => {
                match super::temporal::iso::format_time(value.count(), value.unit()) {
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
            Self::DateTime64(value) => {
                let spelled = if value.timezone().is_naive() {
                    super::temporal::iso::format_datetime(value.count(), value.unit())
                } else {
                    super::temporal::iso::format_timestamp(
                        value.count(),
                        value.unit(),
                        &value.timezone(),
                    )
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
            Self::Duration32(value) => {
                match super::temporal::iso::format_duration(i64::from(value.count()), value.unit())
                {
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
            Self::Duration64(value) => {
                match super::temporal::iso::format_duration(value.count(), value.unit()) {
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
            Self::Interval(value) => tagged(serializer, "interval", value),
            Self::Sequence(values) => tagged(serializer, "sequence", &values.as_slice()),
            Self::Mapping(entries) => tagged(serializer, "mapping", &entries.as_slice()),
            Self::Record(entries) => tagged(serializer, "record", &entries.as_map()),
            // A stream is drained to be written, which is what serializing a
            // one-shot value means; a held shape is shared and stays readable.
            #[cfg(feature = "arrow")]
            Self::Arrow(value) => match (**value).clone().into_scalar() {
                Ok(native) => native.serialize(serializer),
                Err(error) => Err(serde::ser::Error::custom(error)),
            },
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

        // This mirror must cover every `Scalar` variant: one missing here is
        // not a compile error, it is a variant serde silently refuses to read
        // back. Its names are the wire tags, snake-cased, so they spell the
        // tag that is written rather than the Rust variant.
        #[derive(Deserialize)]
        #[serde(tag = "type", content = "value", rename_all = "snake_case")]
        enum StructuralWire {
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
            D256(i256, i8),
            String(Str),
            Country(SmolStr),
            Currency(SmolStr),
            Mic(SmolStr),
            Cfi(SmolStr),
            Isin(SmolStr),
            Cusip(SmolStr),
            Sedol(SmolStr),
            Bloomberg(SmolStr),
            Side(SmolStr),
            State(SmolStr),
            #[serde(rename = "timeinforce")]
            TimeInForce(SmolStr),
            Uuid(SmolStr),
            Version(Version),
            Timezone(SmolStr),
            #[serde(rename = "mimetype")]
            MimeType(SmolStr),
            #[serde(rename = "mediatype")]
            MediaType(SmolStr),
            Url(SmolStr),
            Enum(Enum),
            Bytes(Bytes),
            Geometry(Arc<[u8]>),
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

        match StructuralWire::deserialize(deserializer)? {
            StructuralWire::Null => Ok(Self::Null),
            StructuralWire::Bool(value) => Ok(Self::from(value)),
            StructuralWire::I8(value) => Ok(Self::from(value)),
            StructuralWire::I16(value) => Ok(Self::from(value)),
            StructuralWire::I32(value) => Ok(Self::from(value)),
            StructuralWire::I64(value) => Ok(Self::from(value)),
            StructuralWire::U8(value) => Ok(Self::from(value)),
            StructuralWire::U16(value) => Ok(Self::from(value)),
            StructuralWire::U32(value) => Ok(Self::from(value)),
            StructuralWire::U64(value) => Ok(Self::from(value)),
            StructuralWire::I128(value) => Ok(Self::from(value)),
            StructuralWire::U128(value) => Ok(Self::from(value)),
            StructuralWire::F16(value) => Ok(Self::Float16(value)),
            StructuralWire::F32(value) => Ok(Self::Float32(value)),
            StructuralWire::F64(value) => Ok(Self::Float64(value)),
            StructuralWire::D32(unscaled, scale) => Ok(Self::Decimal32(
                super::decimal::Decimal32::new(unscaled, scale),
            )),
            StructuralWire::D64(unscaled, scale) => Ok(Self::Decimal64(
                super::decimal::Decimal64::new(unscaled, scale),
            )),
            StructuralWire::D128(unscaled, scale) => Ok(Self::d128(unscaled, scale)),
            StructuralWire::D256(unscaled, scale) => Ok(Self::d256(unscaled, scale)),
            StructuralWire::String(value) => Ok(Self::String(value)),
            StructuralWire::Country(value) => super::string::Country::new(value)
                .map(|value| Self::Code(Code::Country(value)))
                .map_err(D::Error::custom),
            StructuralWire::Currency(value) => super::string::Currency::new(value)
                .map(|value| Self::Code(Code::Currency(value)))
                .map_err(D::Error::custom),
            StructuralWire::Mic(value) => super::string::Mic::new(value)
                .map(|value| Self::Code(Code::Mic(value)))
                .map_err(D::Error::custom),
            StructuralWire::Cfi(value) => super::string::Cfi::new(value)
                .map(|value| Self::Code(Code::Cfi(value)))
                .map_err(D::Error::custom),
            StructuralWire::Isin(value) => super::string::Isin::new(value)
                .map(|value| Self::Code(Code::Isin(value)))
                .map_err(D::Error::custom),
            StructuralWire::Cusip(value) => super::string::Cusip::new(value)
                .map(|value| Self::Code(Code::Cusip(value)))
                .map_err(D::Error::custom),
            StructuralWire::Bloomberg(value) => super::string::Bloomberg::new(value)
                .map(|value| Self::Code(Code::Bloomberg(value)))
                .map_err(serde::de::Error::custom),
            StructuralWire::Sedol(value) => super::string::Sedol::new(value)
                .map(|value| Self::Code(Code::Sedol(value)))
                .map_err(D::Error::custom),
            // A side and a state are read by their spelling, exactly as a
            // column reads them.
            StructuralWire::Side(value) => super::string::Side::read(&value)
                .map(|value| Self::Code(Code::Side(value)))
                .map_err(D::Error::custom),
            StructuralWire::State(value) => super::string::State::read(&value)
                .map(|value| Self::Code(Code::State(value)))
                .map_err(D::Error::custom),
            StructuralWire::TimeInForce(value) => super::string::TimeInForce::new(value)
                .map(|value| Self::Code(Code::TimeInForce(value)))
                .map_err(D::Error::custom),
            StructuralWire::Uuid(value) => Uuid::from_bytes(value.as_bytes())
                .map(Self::Uuid)
                .map_err(D::Error::custom),
            StructuralWire::Version(value) => Ok(Self::Version(value)),
            StructuralWire::Timezone(value) => Timezone::from_str(value.as_str())
                .map(Self::Timezone)
                .map_err(D::Error::custom),
            StructuralWire::MimeType(value) => MimeType::from_str(value.as_str())
                .map(Self::MimeType)
                .map_err(D::Error::custom),
            StructuralWire::MediaType(value) => MediaType::from_str(value.as_str())
                .map(|value| Self::MediaType(Arc::new(value)))
                .map_err(D::Error::custom),
            StructuralWire::Url(value) => crate::Url::from_str(value.as_str())
                .map(|value| Self::Url(Arc::new(value)))
                .map_err(D::Error::custom),
            StructuralWire::Enum(value) => Ok(Self::Enum(value)),
            StructuralWire::Bytes(value) => Ok(Self::Bytes(value)),
            StructuralWire::Geometry(value) => super::geospatial::Geometry::new(value)
                .map(Self::Geometry)
                .map_err(D::Error::custom),
            StructuralWire::Geography(value) => super::geospatial::Geography::new(value)
                .map(Self::Geography)
                .map_err(D::Error::custom),
            StructuralWire::Date32(Temporal32::Triple(count, unit, zone)) => {
                Self::date32_in(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Date32(Temporal32::Iso(spelled)) => {
                super::temporal::iso::parse_date(&spelled)
                    .map(Self::date32)
                    .map_err(D::Error::custom)
            }
            StructuralWire::Date64(Temporal64::Triple(count, unit, zone)) => {
                Self::date64_in(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Date64(Temporal64::Iso(spelled)) => {
                super::temporal::iso::parse_date(&spelled)
                    .map(|days| Self::date64(i64::from(days) * 86_400_000))
                    .map_err(D::Error::custom)
            }
            StructuralWire::Time32(Temporal32::Triple(count, unit, zone)) => {
                Self::time32(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Time32(Temporal32::Iso(spelled)) => {
                super::temporal::iso::parse_time(&spelled)
                    .and_then(|(count, unit)| {
                        i32::try_from(count)
                            .map(|count| (count, unit))
                            .map_err(|_| Error::InvalidRecord {
                                path: SmolStr::new_static("$"),
                                reason: SmolStr::new(format!(
                                    "time count {count} does not fit time32"
                                )),
                            })
                    })
                    .and_then(|(count, unit)| Self::time32(count, unit, Timezone::NAIVE))
                    .map_err(D::Error::custom)
            }
            StructuralWire::Time64(Temporal64::Triple(count, unit, zone)) => {
                Self::time64(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Time64(Temporal64::Iso(spelled)) => {
                super::temporal::iso::parse_time(&spelled)
                    .and_then(|(count, unit)| Self::time64(count, unit, Timezone::NAIVE))
                    .map_err(D::Error::custom)
            }
            StructuralWire::DateTime64(Temporal64::Triple(count, unit, zone)) => {
                Self::datetime64(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::DateTime64(Temporal64::Iso(spelled)) => {
                super::temporal::iso::parse_timestamp(&spelled)
                    .and_then(|(count, unit, zone)| Self::datetime64(count, unit, zone))
                    .or_else(|_| {
                        super::temporal::iso::parse_datetime(&spelled).and_then(|(count, unit)| {
                            Self::datetime64(count, unit, Timezone::NAIVE)
                        })
                    })
                    .map_err(D::Error::custom)
            }
            StructuralWire::Duration32(Temporal32::Triple(count, unit, zone)) => {
                if !zone.is_naive() {
                    return Err(D::Error::custom("duration32 timezone must be NAIVE"));
                }
                Self::duration32(count, unit).map_err(D::Error::custom)
            }
            StructuralWire::Duration32(Temporal32::Iso(spelled)) => {
                super::temporal::iso::parse_duration(&spelled)
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
            StructuralWire::Duration64(Temporal64::Triple(count, unit, zone)) => {
                if !zone.is_naive() {
                    return Err(D::Error::custom("duration64 timezone must be NAIVE"));
                }
                Self::duration64(count, unit).map_err(D::Error::custom)
            }
            StructuralWire::Duration64(Temporal64::Iso(spelled)) => {
                super::temporal::iso::parse_duration(&spelled)
                    .and_then(|(count, unit)| Self::duration64(count, unit))
                    .map_err(D::Error::custom)
            }
            StructuralWire::Interval(value) => Ok(Self::Interval(value)),
            StructuralWire::Sequence(values) => Ok(Self::from_sequence(values)),
            StructuralWire::Mapping(entries) => {
                Self::from_mapping(entries).map_err(D::Error::custom)
            }
            StructuralWire::Record(entries) => {
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
        match (integer_parts(self), integer_parts(other)) {
            (Some(left), Some(right)) => return compare_integer_parts(left, right),
            (Some(_), None) | (None, Some(_)) => {
                return value_rank(self).cmp(&value_rank(other));
            }
            (None, None) => {}
        }
        // Floats are one value space across widths, exactly as the integers
        // are: an `f32` widens to `f64` without loss, so `F32(1.5)` and
        // `F64(1.5)` are one value, not two kinds that happen to print alike.
        if let (Some(left), Some(right)) = (float_value(self), float_value(other)) {
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
        // Geometry and geography differ in the coordinate reference they
        // name, not in the bytes, so one WKB payload is one value under both.
        if let (Some(left), Some(right)) = (geospatial_bytes(self), geospatial_bytes(other)) {
            return left.cmp(right);
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
            Self::Int8(_)
            | Self::Int16(_)
            | Self::Int32(_)
            | Self::Int64(_)
            | Self::UInt8(_)
            | Self::UInt16(_)
            | Self::UInt32(_)
            | Self::UInt64(_)
            | Self::Int128(_)
            | Self::UInt128(_) => unreachable!("every integer width returned above"),
            Self::Float16(_) | Self::Float32(_) | Self::Float64(_) => {
                unreachable!("all float widths returned above")
            }
            Self::Decimal32(_) | Self::Decimal64(_) | Self::Decimal128(_) | Self::Decimal256(_) => {
                unreachable!("all decimal widths returned above")
            }
            // Two temporals of one family returned above, and two families
            // never share a rank.
            Self::Date32(_)
            | Self::Date64(_)
            | Self::Time32(_)
            | Self::Time64(_)
            | Self::DateTime64(_)
            | Self::Duration32(_)
            | Self::Duration64(_) => unreachable!("every temporal width returned above"),
            Self::Interval(left) => same_kind!(Self::Interval(right) => left.cmp(right)),
            Self::String(left) => same_kind!(Self::String(right) => left.cmp(right)),
            Self::Code(left) => same_kind!(Self::Code(right) => left.cmp(right)),
            Self::Uuid(left) => same_kind!(Self::Uuid(right) => left.cmp(right)),
            Self::Version(left) => same_kind!(Self::Version(right) => left.cmp(right)),
            Self::Timezone(left) => same_kind!(Self::Timezone(right) => left.cmp(right)),
            Self::MimeType(left) => same_kind!(Self::MimeType(right) => left.cmp(right)),
            Self::MediaType(left) => same_kind!(Self::MediaType(right) => left.cmp(right)),
            Self::Url(left) => same_kind!(Self::Url(right) => left.cmp(right)),
            Self::Enum(left) => same_kind!(Self::Enum(right) => left.cmp(right)),
            Self::Bytes(left) => same_kind!(Self::Bytes(right) => left.cmp(right)),
            Self::Geometry(_) | Self::Geography(_) => {
                unreachable!("both geospatial readings returned above")
            }
            Self::Sequence(left) => same_kind!(Self::Sequence(right) => left.cmp(right)),
            Self::Mapping(left) => same_kind!(Self::Mapping(right) => left.cmp(right)),
            Self::Record(left) => same_kind!(Self::Record(right) => left.cmp(right)),
            #[cfg(feature = "arrow")]
            Self::Arrow(left) => same_kind!(Self::Arrow(right) => left.cmp(right)),
        }
    }
}

impl Hash for Scalar {
    fn hash<H: Hasher>(&self, state: &mut H) {
        value_rank(self).hash(state);
        if let Some(parts) = integer_parts(self) {
            parts.hash(state);
            return;
        }
        // All float widths hash their common 64-bit reading, which is what
        // keeps `Hash` agreeing with `Ord` across the widths.
        if let Some(float) = float_value(self) {
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
            #[cfg(feature = "arrow")]
            Self::Arrow(value) => value.hash(state),
            Self::Null => {}
            Self::Boolean(value) => value.hash(state),
            Self::Int8(_)
            | Self::Int16(_)
            | Self::Int32(_)
            | Self::Int64(_)
            | Self::UInt8(_)
            | Self::UInt16(_)
            | Self::UInt32(_)
            | Self::UInt64(_)
            | Self::Int128(_)
            | Self::UInt128(_) => unreachable!("integer values returned above"),
            Self::Float16(_) | Self::Float32(_) | Self::Float64(_) => {
                unreachable!("float values returned above")
            }
            Self::Decimal32(_) | Self::Decimal64(_) | Self::Decimal128(_) | Self::Decimal256(_) => {
                unreachable!("decimal values returned above")
            }
            Self::Date32(_)
            | Self::Date64(_)
            | Self::Time32(_)
            | Self::Time64(_)
            | Self::DateTime64(_)
            | Self::Duration32(_)
            | Self::Duration64(_) => unreachable!("temporal values returned above"),
            // Interval, Sequence, Mapping and Record feed the discriminant their
            // retired width enum wrote before them, so deterministic hashes built
            // over `Hash` (a message's) keep their exact bytes.
            Self::Interval(value) => {
                7_isize.hash(state);
                value.hash(state);
            }
            Self::String(value) => value.hash(state),
            Self::Code(value) => value.hash(state),
            Self::Uuid(value) => value.hash(state),
            Self::Version(value) => value.hash(state),
            Self::Timezone(value) => value.hash(state),
            Self::MimeType(value) => value.hash(state),
            Self::MediaType(value) => value.hash(state),
            Self::Url(value) => value.hash(state),
            Self::Enum(value) => value.hash(state),
            Self::Bytes(value) => value.hash(state),
            Self::Geometry(value) => value.hash(state),
            Self::Geography(value) => value.hash(state),
            Self::Sequence(value) => {
                0_isize.hash(state);
                value.hash(state);
            }
            Self::Mapping(value) => {
                1_isize.hash(state);
                value.hash(state);
            }
            Self::Record(value) => {
                2_isize.hash(state);
                value.hash(state);
            }
        }
    }
}

/// The common 64-bit reading every float width orders and hashes by.
fn float_value(value: &Scalar) -> Option<Float64> {
    value.as_f64().map(Float64::from_f64)
}

fn decimal_value(value: &Scalar) -> Option<(i256, i8)> {
    value.as_decimal()
}

/// The validated WKB every geospatial reading orders and hashes by.
fn geospatial_bytes(value: &Scalar) -> Option<&[u8]> {
    match value {
        Scalar::Geometry(value) => Some(value.as_bytes()),
        Scalar::Geography(value) => Some(value.as_bytes()),
        _ => None,
    }
}

/// The family, normalized count, and zone of one temporal.
///
/// An interval answers `None`: its three components have no one count to
/// normalize, so it orders and hashes as itself.
fn temporal_value(value: &Scalar) -> Option<(super::TemporalFamily, (u8, i128), Timezone)> {
    if matches!(value, Scalar::Interval(_)) {
        return None;
    }
    Some((
        value.temporal_family()?,
        temporal_key(value.temporal_count()?, value.temporal_unit()?),
        value.temporal_timezone()?,
    ))
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
        Scalar::Int8(_)
        | Scalar::Int16(_)
        | Scalar::Int32(_)
        | Scalar::Int64(_)
        | Scalar::UInt8(_)
        | Scalar::UInt16(_)
        | Scalar::UInt32(_)
        | Scalar::UInt64(_)
        | Scalar::Int128(_)
        | Scalar::UInt128(_) => 2,
        Scalar::Float16(_) | Scalar::Float32(_) | Scalar::Float64(_) => 3,
        Scalar::Decimal32(_)
        | Scalar::Decimal64(_)
        | Scalar::Decimal128(_)
        | Scalar::Decimal256(_) => 4,
        Scalar::String(_) => 5,
        Scalar::Bytes(_) => 6,
        Scalar::Date32(_) | Scalar::Date64(_) => 7,
        Scalar::Time32(_) | Scalar::Time64(_) => 8,
        Scalar::DateTime64(_) => 9,
        Scalar::Duration32(_) | Scalar::Duration64(_) => 10,
        Scalar::Sequence(_) => 11,
        Scalar::Mapping(_) => 12,
        Scalar::Record(_) => 13,
        Scalar::Geometry(_) | Scalar::Geography(_) => 14,
        Scalar::Enum(_) => 15,
        Scalar::Interval(_) => 16,
        Scalar::Uuid(_) => 17,
        Scalar::Code(_) => 18,
        Scalar::Version(_) => 19,
        Scalar::Url(_) => 20,
        Scalar::Timezone(_) => 21,
        Scalar::MimeType(_) => 22,
        Scalar::MediaType(_) => 23,
        #[cfg(feature = "arrow")]
        Scalar::Arrow(_) => 24,
    }
}

impl Scalar {
    /// Return the most specific datatype identifier the value itself proves.
    ///
    /// Nested values report their most-general shape; a [`Field`](crate::Field)
    /// narrows a sequence to a list, fixed-size list, struct, or union. Static
    /// enum members report UTF-8 because their column representation remains a
    /// field-level choice.
    pub fn id(&self) -> DataTypeId {
        match self {
            // One pinned row is the value it holds; every wider shape is a
            // sequence of them.
            #[cfg(feature = "arrow")]
            Self::Arrow(value) => {
                if value.is_scalar() {
                    value.dtype().id()
                } else {
                    DataTypeId::List
                }
            }
            Self::Null => DataTypeId::Null,
            Self::Boolean(_) => DataTypeId::Boolean,
            Self::Int8(_) => DataTypeId::Int8,
            Self::Int16(_) => DataTypeId::Int16,
            Self::Int32(_) => DataTypeId::Int32,
            Self::Int64(_) => DataTypeId::Int64,
            Self::Int128(_) => DataTypeId::Int128,
            Self::UInt8(_) => DataTypeId::UInt8,
            Self::UInt16(_) => DataTypeId::UInt16,
            Self::UInt32(_) => DataTypeId::UInt32,
            Self::UInt64(_) => DataTypeId::UInt64,
            Self::UInt128(_) => DataTypeId::UInt128,
            Self::Float16(_) => DataTypeId::Float16,
            Self::Float32(_) => DataTypeId::Float32,
            Self::Float64(_) => DataTypeId::Float64,
            Self::Decimal32(_) => DataTypeId::Decimal32,
            Self::Decimal64(_) => DataTypeId::Decimal64,
            Self::Decimal128(_) => DataTypeId::Decimal128,
            Self::Decimal256(_) => DataTypeId::Decimal256,
            Self::Date32(_) => DataTypeId::Date32,
            Self::Date64(_) => DataTypeId::Date64,
            Self::Time32(_) => DataTypeId::Time32,
            Self::Time64(_) => DataTypeId::Time64,
            Self::DateTime64(_) => DataTypeId::DateTime64,
            Self::Duration32(_) => DataTypeId::Duration32,
            Self::Duration64(_) => DataTypeId::Duration64,
            Self::Interval(_) => DataTypeId::Interval,
            // A string names the layout it is stored in.
            Self::String(text) => text.layout().id(),
            Self::Code(code) => code.identifier(),
            Self::Uuid(_) => DataTypeId::Uuid,
            Self::Version(_) => DataTypeId::Version,
            Self::Timezone(_) => DataTypeId::Timezone,
            Self::MimeType(_) => DataTypeId::MimeType,
            Self::MediaType(_) => DataTypeId::MediaType,
            Self::Url(_) => DataTypeId::Url,
            Self::Enum(_) => DataTypeId::String,
            Self::Bytes(bytes) => bytes.layout().id(),
            Self::Geometry(_) => DataTypeId::Geometry,
            Self::Geography(_) => DataTypeId::Geography,
            Self::Sequence(_) => DataTypeId::List,
            Self::Mapping(_) => DataTypeId::Map,
            Self::Record(_) => DataTypeId::Struct,
        }
    }

    /// Return the datatype family the value itself proves.
    pub fn family(&self) -> DataTypeKind {
        self.id().kind()
    }

    /// The canonical vocabulary name for this value's kind, such as `mapping`.
    ///
    /// This is the spelling every error message uses for an observed value, so
    /// a caller reading `expected string, got mapping` sees the same words the
    /// documentation and the bindings use.
    pub const fn kind(&self) -> &'static str {
        match self {
            #[cfg(feature = "arrow")]
            Self::Arrow(_) => "arrow",
            Self::Null => "null",
            Self::Boolean(_) => "boolean",
            Self::Int8(_) => "i8",
            Self::Int16(_) => "i16",
            Self::Int32(_) => "i32",
            Self::Int64(_) => "i64",
            Self::UInt8(_) => "u8",
            Self::UInt16(_) => "u16",
            Self::UInt32(_) => "u32",
            Self::UInt64(_) => "u64",
            Self::Int128(_) => "i128",
            Self::UInt128(_) => "u128",
            Self::Float16(_) => "f16",
            Self::Float32(_) => "f32",
            Self::Float64(_) => "f64",
            Self::Decimal32(_) => "d32",
            Self::Decimal64(_) => "d64",
            Self::Decimal128(_) => "d128",
            Self::Decimal256(_) => "d256",
            Self::String(text) => text.layout().as_str(),
            Self::Code(code) => code.identifier().as_str(),
            Self::Uuid(_) => "uuid",
            Self::Version(_) => "version",
            Self::Timezone(_) => "timezone",
            Self::MimeType(_) => "mimetype",
            Self::MediaType(_) => "mediatype",
            Self::Url(_) => "url",
            Self::Enum(_) => "enum",
            Self::Bytes(bytes) => match bytes.layout() {
                super::bytes::BytesLayout::Binary => "bytes",
                other => other.as_str(),
            },
            Self::Geometry(_) => "geometry",
            Self::Geography(_) => "geography",
            Self::Date32(_) => "date32",
            Self::Date64(_) => "date64",
            Self::Time32(_) => "time32",
            Self::Time64(_) => "time64",
            Self::DateTime64(_) => "datetime64",
            Self::Duration32(_) => "duration32",
            Self::Duration64(_) => "duration64",
            Self::Interval(_) => "interval",
            Self::Sequence(_) => "sequence",
            Self::Mapping(_) => "mapping",
            Self::Record(_) => "record",
        }
    }

    /// The one shared empty sequence, which every empty run answers with.
    fn empty_sequence() -> Self {
        static EMPTY: OnceLock<Arc<[Scalar]>> = OnceLock::new();
        Self::Sequence(Sequence::new(Arc::clone(
            EMPTY.get_or_init(|| Arc::from([])),
        )))
    }

    /// The one shared empty mapping.
    fn empty_mapping() -> Self {
        static EMPTY: OnceLock<Arc<[(Scalar, Scalar)]>> = OnceLock::new();
        Self::Mapping(Mapping::new(Arc::clone(
            EMPTY.get_or_init(|| Arc::from([])),
        )))
    }

    /// Construct an ordered sequence.
    ///
    /// The children are written straight into the shared slice they are
    /// stored in. A `Vec` on the way would allocate a second buffer and copy
    /// the whole run between the two, which is what a row build pays per row.
    pub fn from_sequence(values: impl IntoIterator<Item = Self>) -> Self {
        shared_children(values.into_iter()).map_or_else(Self::empty_sequence, |values| {
            Self::Sequence(Sequence::new(values))
        })
    }

    /// Construct an insertion-ordered mapping, rejecting duplicate keys.
    pub fn from_mapping(entries: impl IntoIterator<Item = (Self, Self)>) -> Result<Self> {
        // The duplicate check reads the entries in place, so they are
        // collected once into the storage the mapping keeps.
        let Some(entries) = shared_children(entries.into_iter()) else {
            return Ok(Self::empty_mapping());
        };
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
        Ok(Self::Mapping(Mapping::new(entries)))
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
            return Ok(Self::Record(Record::new(Arc::clone(
                EMPTY.get_or_init(|| Arc::new(BTreeMap::new())),
            ))));
        }
        Ok(Self::Record(Record::new(Arc::new(record))))
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
            Self::String(value) => Some(value.as_str()),
            Self::Code(value) => Some(value.as_str()),
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

    /// Return bytes when this is a byte value.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value.as_bytes()),
            Self::Geometry(value) => Some(value.as_bytes()),
            Self::Geography(value) => Some(value.as_bytes()),
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
            Self::Sequence(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    /// Return mapping entries without allocating.
    pub fn as_mapping(&self) -> Option<&[(Self, Self)]> {
        match self {
            Self::Mapping(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    /// Return record fields in deterministic name order.
    pub fn as_record(&self) -> Option<&BTreeMap<SmolStr, Self>> {
        match self {
            Self::Record(entries) => Some(entries.as_map()),
            _ => None,
        }
    }

    /// Return the number of direct children or mapping entries.
    pub fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.as_slice().len(),
            Self::Mapping(entries) => entries.as_slice().len(),
            Self::Record(entries) => entries.as_map().len(),
            _ => 0,
        }
    }

    /// Return whether this is an empty sequence or mapping.
    pub fn is_empty(&self) -> bool {
        self.is_container() && self.len() == 0
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
            Self::Sequence(values) => Children::Sequence(values.as_slice().iter()),
            Self::Mapping(entries) => Children::Mapping(entries.as_slice().iter()),
            Self::Record(entries) => Children::Record(entries.as_map().values()),
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
                Self::Float16(_)
                    | Self::Float32(_)
                    | Self::Float64(_)
                    | Self::Decimal32(_)
                    | Self::Decimal64(_)
                    | Self::Decimal128(_)
                    | Self::Decimal256(_)
            )
    }

    /// Borrow the width leaf's own [`fmt::Display`], for a variant that holds
    /// one.
    ///
    /// Every integer, float, decimal, temporal and nested variant answers the
    /// leaf it holds, whose text is that leaf's plain spelling; every other
    /// variant answers `None`, because its spelling is the caller's to choose.
    pub(crate) fn leaf_display(&self) -> Option<&dyn fmt::Display> {
        let leaf: &dyn fmt::Display = match self {
            Self::Int8(value) => value,
            Self::Int16(value) => value,
            Self::Int32(value) => value,
            Self::Int64(value) => value,
            Self::UInt8(value) => value,
            Self::UInt16(value) => value,
            Self::UInt32(value) => value,
            Self::UInt64(value) => value,
            Self::Int128(value) => value,
            Self::UInt128(value) => value,
            Self::Float16(value) => value,
            Self::Float32(value) => value,
            Self::Float64(value) => value,
            Self::Decimal32(value) => value,
            Self::Decimal64(value) => value,
            Self::Decimal128(value) => value,
            Self::Decimal256(value) => value,
            Self::Date32(value) => value,
            Self::Date64(value) => value,
            Self::Time32(value) => value,
            Self::Time64(value) => value,
            Self::DateTime64(value) => value,
            Self::Duration32(value) => value,
            Self::Duration64(value) => value,
            Self::Interval(value) => value,
            Self::Sequence(value) => value,
            Self::Mapping(value) => value,
            Self::Record(value) => value,
            #[cfg(feature = "arrow")]
            Self::Arrow(_) => return None,
            Self::Null
            | Self::Boolean(_)
            | Self::String(_)
            | Self::Code(_)
            | Self::Uuid(_)
            | Self::Version(_)
            | Self::Url(_)
            | Self::Enum(_)
            | Self::Bytes(_)
            | Self::Geometry(_)
            | Self::Geography(_)
            | Self::Timezone(_)
            | Self::MimeType(_)
            | Self::MediaType(_) => return None,
        };
        Some(leaf)
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
        Ok(Self::Record(Record::new(Arc::new(rebuilt))))
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
        Ok(Self::Record(Record::new(Arc::new(rebuilt))))
    }
}

/// The shared slice a nested value stores, or `None` for an empty run.
///
/// An empty run answers with one shared value rather than a fresh allocation,
/// so a source that might be empty is drained through a `Vec` first: a size
/// hint is a hint and not a promise, and an iterator that under-reports must
/// still keep its children. Every other source is written straight into the
/// slice - one allocation, where a `Vec` on the way costs two and a copy of
/// the whole run between them.
fn shared_children<T>(values: impl Iterator<Item = T>) -> Option<Arc<[T]>> {
    let values: Arc<[T]> = if values.size_hint().1 == Some(0) {
        let drained = values.collect::<Vec<_>>();
        if drained.is_empty() {
            return None;
        }
        Arc::from(drained)
    } else {
        values.collect()
    };
    (!values.is_empty()).then_some(values)
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

#[cfg(feature = "arrow")]
impl Scalar {
    /// The native value an Arrow payload holds: the row of a pinned scalar, a
    /// sequence of items for a column, a sequence of rows for a table or a
    /// stream. Every other value is itself.
    ///
    /// This is the one crossing from the shared buffers into the native tree,
    /// so it is where a stream is drained; a held shape is shared and stays
    /// readable behind the value it came from.
    ///
    /// # Errors
    ///
    /// Returns an error when a value cannot be represented natively, when the
    /// stream was already read, or whatever the stream raised.
    pub fn into_native(&self) -> Result<Self> {
        match self {
            Self::Arrow(value) => Ok((**value).clone().into_scalar()?),
            other => Ok(other.clone()),
        }
    }

    /// Wrap one Arrow payload as the scalar it is.
    #[must_use]
    pub fn from_arrow(value: crate::arrow::ArrowScalar) -> Self {
        Self::Arrow(Arc::new(value))
    }

    /// Borrow the Arrow payload, if this value is one.
    #[must_use]
    pub fn as_arrow(&self) -> Option<&crate::arrow::ArrowScalar> {
        match self {
            Self::Arrow(value) => Some(value),
            _ => None,
        }
    }
}
