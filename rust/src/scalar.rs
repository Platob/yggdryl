//! The one value every part of the project speaks.
//!
//! A [`Scalar`] is any native value: null, a boolean, a signed or unsigned
//! integer at every width from 8 to 128 bits, a float at 16, 32, or 64 bits,
//! an exact decimal, text, bytes, width-typed temporals, an ordered sequence,
//! an arbitrary-key mapping, or a name-sorted record. It is what [`crate::json`], [`crate::yaml`], and
//! [`crate::toml`] parse into and write from, what a [`crate::Field`] validates
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
//! let quote = Scalar::from_struct([
//!     ("symbol", Scalar::from("AAPL")),
//!     ("price", Scalar::d128(125, 1)),
//! ])?;
//!
//! assert_eq!(quote.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
//! assert_eq!(quote.len(), 2);
//! # Ok(())
//! # }
//! ```

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::boolean::Boolean;
use crate::bytes::Bytes;
use crate::date::{Date32, Date64};
use crate::datetime::DateTime64;
use crate::decimal::{Decimal32, Decimal64, Decimal128, Decimal256};
use crate::duration::{Duration32, Duration64};
use crate::floating::{Float16, Float32, Float64};
use crate::geospatial::{Geography, Geometry};
use crate::integer::{
    Int8, Int16, Int32, Int64, Int128, UInt8, UInt16, UInt32, UInt64, UInt128,
    compare_integer_parts, integer_parts,
};
use crate::interval::Interval;
use crate::mapping::Map;
use crate::serie::Run;
use crate::string::Str;
use crate::structure::Struct;
use crate::temporal::scalars::temporal_key;
use crate::time::{Time32, Time64};
use crate::uuid::Uuid;
use crate::value::Children;
use crate::version::Version;
use crate::{
    BloombergCode, Ccy, CfiCode, Country, CusipCode, FIGICode, IsinCode, MicCode, SedolCode, Side,
    State, TimeInForce, Unit, decimal,
};
use crate::{
    DataTypeId, DataTypeKind, Error, MediaType, MimeType, Result, TimeUnit, Timezone, i256,
};
use std::ops::Index;

use crate::Serie;

/// Make one canonical text value a scalar leaf of its own.
///
/// A value that parses, canonicalizes and renders itself - a version, a time
/// zone, a MIME type - is its own family: it holds no narrower leaf and no
/// wider one holds it, so every method here is the same four lines under a
/// different name. The wrapping variant is named because a value wider than
/// the enum rides behind a shared pointer instead.
macro_rules! text_leaf_value {
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
    // The string leaves, one variant per leaf in identifier order: the
    // characters, and the number a fixed or sized leaf states.
    /// UTF-8 text, 32-bit offsets.
    Utf8String(Str),
    /// UTF-8 text, 64-bit offsets.
    LargeUtf8String(Str),
    /// UTF-8 text, viewed.
    Utf8StringView(Str),
    /// UTF-8 text, viewed over 64-bit offsets.
    LargeUtf8StringView(Str),
    /// UTF-8 text in a padded slot of the width it carries.
    FixedUtf8String(Str, u32),
    /// UTF-8 text under the maximum, in stored bytes, it carries.
    SizedUtf8String(Str, u32),
    /// US-ASCII text, 32-bit offsets.
    AsciiString(Str),
    /// US-ASCII text, 64-bit offsets.
    LargeAsciiString(Str),
    /// US-ASCII text, viewed.
    AsciiStringView(Str),
    /// US-ASCII text, viewed over 64-bit offsets.
    LargeAsciiStringView(Str),
    /// US-ASCII text in a padded slot of the width it carries.
    FixedAsciiString(Str, u32),
    /// US-ASCII text under the maximum, in stored bytes, it carries.
    SizedAsciiString(Str, u32),
    /// Windows-1252 text, 32-bit offsets.
    Cp1252String(Str),
    /// Windows-1252 text, 64-bit offsets.
    LargeCp1252String(Str),
    /// Windows-1252 text, viewed.
    Cp1252StringView(Str),
    /// Windows-1252 text, viewed over 64-bit offsets.
    LargeCp1252StringView(Str),
    /// Windows-1252 text in a padded slot of the width it carries.
    FixedCp1252String(Str, u32),
    /// Windows-1252 text under the maximum, in stored bytes, it carries.
    SizedCp1252String(Str, u32),
    /// ISO 3166-1 alpha-2 country code.
    Country(Country),
    /// ISO 4217 currency code.
    Ccy(Ccy),
    /// ISO 10383 market identifier code.
    MicCode(MicCode),
    /// ISO 10962 classification code.
    CfiCode(CfiCode),
    /// FIX's side of a trade.
    Side(Side),
    /// What state one thing is in, ranked so the bytes sort by lifecycle.
    State(State),
    /// How long an order stands.
    TimeInForce(TimeInForce),
    /// ISO 6166 securities identification number.
    IsinCode(IsinCode),
    /// CUSIP securities identifier.
    CusipCode(CusipCode),
    /// SEDOL securities identifier.
    SedolCode(SedolCode),
    /// BloombergCode securities identifier.
    BloombergCode(BloombergCode),
    /// An RFC 9562 identifier.
    Uuid(Uuid),
    /// A canonical, numerically ordered version.
    Version(Version),
    /// A validated, canonical location.
    ///
    /// Behind one shared pointer: a parsed [`crate::Url`] is far wider than
    /// this enum, and a column of them is cloned once per row.
    Url(Arc<crate::Url>),
    /// A validated, canonical resource name, behind one shared pointer for
    /// the reason a URL is.
    Urn(Arc<crate::Urn>),
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
    // The byte leaves, one variant per leaf in identifier order: the payload,
    // and the number a fixed or sized leaf states.
    /// Bytes, 32-bit offsets.
    Binary(Bytes),
    /// Bytes, 64-bit offsets.
    LargeBinary(Bytes),
    /// Bytes, viewed.
    BinaryView(Bytes),
    /// Bytes, viewed over 64-bit offsets.
    LargeBinaryView(Bytes),
    /// Bytes of exactly the width they carry.
    FixedBinary(Bytes, u32),
    /// Bytes under the maximum they carry.
    SizedBinary(Bytes, u32),
    /// Planar geometry as validated Well-Known Binary.
    Geometry(Geometry),
    /// Geographic coordinates as validated Well-Known Binary.
    Geography(Geography),
    /// Many values under 32-bit offsets: a run, or a column of its item.
    ///
    /// Each of the five sequence variants is the serie layout the value
    /// declares - what [`Scalar::dtype`] answers and what a column crossing
    /// Arrow is laid out as. The rows are its identity: two sequences of
    /// equal rows are equal whichever layout declares them.
    Serie(Serie),
    /// Many values under 32-bit offsets and sizes.
    SerieView(Serie),
    /// Exactly as many values per row as the declaring field fixes.
    FixedSizeSerie(Serie),
    /// Many values under 64-bit offsets.
    LargeSerie(Serie),
    /// Many values under 64-bit offsets and sizes.
    LargeSerieView(Serie),
    /// An insertion-ordered mapping of arbitrary keys.
    Map(Map),
    /// A mapping whose keys are held sorted.
    SortedMap(Map),
    /// A schema-free record of values sorted by field name.
    Struct(Struct),
    /// One semi-structured value in [the Parquet Variant binary
    /// encoding](crate::Variant): its metadata dictionary and its value
    /// payload, the pair a `variant` column stores per row.
    ///
    /// A variant *is* those bytes, the way a geometry is its WKB: the value
    /// inside them is [`Variant::scalar`](crate::Variant::scalar), and a
    /// cast either way encodes or decodes. Two variants are equal when
    /// their bytes are, which is why a value cast into a variant is
    /// canonically encoded - keys sorted, sizes narrowest.
    Variant(crate::Variant),
    /// ANSI X9.145 Financial Instrument Global Identifier.
    FIGICode(FIGICode),
    /// The unit a quantity is stated in.
    Unit(Unit),
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
            // One tag for every string. The ordinary value - the `utf8`
            // leaf - writes its characters and nothing else, which is what
            // it always wrote; any other leaf writes the whole declaration.
            string_scalars!(_) => {
                let (leaf, text) = self.string_leaf().expect("a string leaf");
                tagged(
                    serializer,
                    "string",
                    &crate::string::StringWire { leaf, text },
                )
            }
            // A code writes its text under its own datatype's name.
            code_scalars!() => tagged(
                serializer,
                self.id().as_str(),
                &self
                    .code_storage()
                    .expect("a code borrowed its storage")
                    .as_str(),
            ),
            Self::Uuid(value) => {
                let mut slot = [0_u8; crate::Uuid::TEXT_LEN];
                tagged(serializer, "uuid", &value.render(&mut slot))
            }
            Self::Version(value) => tagged(serializer, "version", value),
            Self::Timezone(value) => tagged(serializer, "timezone", &value.as_str()),
            Self::MimeType(value) => tagged(serializer, "mimetype", &value.as_str()),
            Self::MediaType(value) => tagged(serializer, "mediatype", &value.to_string()),
            Self::Url(value) => tagged(serializer, "url", &value.to_string()),
            Self::Urn(value) => tagged(serializer, "urn", &value.to_string()),
            // One tag for every byte value: the ordinary payload writes its
            // bytes and nothing else, and any other leaf writes the whole
            // declaration.
            bytes_scalars!(_) => {
                let (leaf, payload) = self.bytes_leaf().expect("a byte leaf");
                tagged(
                    serializer,
                    "bytes",
                    &crate::bytes::BytesWire { leaf, payload },
                )
            }
            Self::Geometry(value) => tagged(serializer, "geometry", &value.as_bytes()),
            Self::Geography(value) => tagged(serializer, "geography", &value.as_bytes()),
            // A temporal is its classic ISO spelling wherever it has one; a
            // reading with no classic spelling keeps its structural parts.
            Self::Date32(value) => match crate::temporal::format_date(value.count()) {
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
                match crate::temporal::format_time(i64::from(value.count()), value.unit()) {
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
                match crate::temporal::format_time(value.count(), value.unit()) {
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
                    crate::temporal::format_datetime(value.count(), value.unit())
                } else {
                    crate::temporal::format_timestamp(
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
                match crate::temporal::format_duration(i64::from(value.count()), value.unit()) {
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
                match crate::temporal::format_duration(value.count(), value.unit()) {
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
            // One tag per layout, the layout's own name. A run writes its
            // rows; a column writes the field that types them beside them,
            // which its rows alone cannot say - and the payload's shape says
            // which of the two it is.
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => tagged(serializer, self.kind(), values),
            Self::Map(entries) => tagged(serializer, "map", &entries.as_slice()),
            Self::SortedMap(entries) => tagged(serializer, "sorted_map", &entries.as_slice()),
            Self::Struct(entries) => tagged(serializer, "struct", &entries.as_map()),
            // The two buffers as they are: a variant is its bytes, and
            // re-encoding the value they hold would restate a foreign
            // writer's dictionary as this one's.
            Self::Variant(value) => {
                tagged(serializer, "variant", &(value.metadata(), value.value()))
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

        /// A serie's payload in either shape the wire holds it: a run's rows,
        /// or a column's field beside its rows.
        enum Held {
            Rows(Vec<Scalar>),
            Column(Serie),
        }

        impl<'de> Deserialize<'de> for Held {
            fn deserialize<D: Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                struct Visitor;

                impl<'de> serde::de::Visitor<'de> for Visitor {
                    type Value = Held;

                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        formatter.write_str("a serie's rows, or its field beside its rows")
                    }

                    fn visit_seq<A: serde::de::SeqAccess<'de>>(
                        self,
                        seq: A,
                    ) -> std::result::Result<Held, A::Error> {
                        Vec::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
                            .map(Held::Rows)
                    }

                    fn visit_map<A: serde::de::MapAccess<'de>>(
                        self,
                        map: A,
                    ) -> std::result::Result<Held, A::Error> {
                        Serie::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                            .map(Held::Column)
                    }
                }

                deserializer.deserialize_any(Visitor)
            }
        }

        impl Held {
            /// The serie either shape holds.
            fn into_serie(self) -> Serie {
                match self {
                    Self::Rows(values) => Serie::new(values),
                    Self::Column(column) => column,
                }
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
            String(crate::string::StringDocument),
            Country(SmolStr),
            Ccy(SmolStr),
            #[serde(rename = "mic")]
            MicCode(SmolStr),
            #[serde(rename = "cfi")]
            CfiCode(SmolStr),
            #[serde(rename = "isin")]
            IsinCode(SmolStr),
            #[serde(rename = "cusip")]
            CusipCode(SmolStr),
            #[serde(rename = "sedol")]
            SedolCode(SmolStr),
            #[serde(rename = "bloomberg")]
            BloombergCode(SmolStr),
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
            Urn(SmolStr),
            Bytes(crate::bytes::BytesDocument),
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
            Interval(crate::interval::Interval),
            // The serie family's tags before it took its own names - the rows
            // under `list`, a column under `serie` or `<layout>_serie` - each
            // still read, whichever shape the payload has.
            #[serde(alias = "list")]
            Serie(Held),
            #[serde(alias = "list_view", alias = "list_view_serie")]
            SerieView(Held),
            #[serde(alias = "fixed_size_list", alias = "fixed_size_list_serie")]
            FixedSizeSerie(Held),
            #[serde(alias = "large_list", alias = "large_list_serie")]
            LargeSerie(Held),
            #[serde(alias = "large_list_view", alias = "large_list_view_serie")]
            LargeSerieView(Held),
            Map(Vec<(Scalar, Scalar)>),
            SortedMap(Vec<(Scalar, Scalar)>),
            Struct(RecordEntries),
            Variant(Arc<[u8]>, Arc<[u8]>),
            #[serde(rename = "figi")]
            FIGICode(SmolStr),
            Unit(SmolStr),
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
                crate::decimal::Decimal32::new(unscaled, scale),
            )),
            StructuralWire::D64(unscaled, scale) => Ok(Self::Decimal64(
                crate::decimal::Decimal64::new(unscaled, scale),
            )),
            StructuralWire::D128(unscaled, scale) => Ok(Self::d128(unscaled, scale)),
            StructuralWire::D256(unscaled, scale) => Ok(Self::d256(unscaled, scale)),
            StructuralWire::String(value) => Ok(value.0),
            StructuralWire::Country(value) => crate::Country::new(value)
                .map(Self::Country)
                .map_err(D::Error::custom),
            StructuralWire::Ccy(value) => crate::Ccy::new(value)
                .map(Self::Ccy)
                .map_err(D::Error::custom),
            StructuralWire::MicCode(value) => crate::MicCode::new(value)
                .map(Self::MicCode)
                .map_err(D::Error::custom),
            StructuralWire::CfiCode(value) => crate::CfiCode::new(value)
                .map(Self::CfiCode)
                .map_err(D::Error::custom),
            StructuralWire::IsinCode(value) => crate::IsinCode::new(value)
                .map(Self::IsinCode)
                .map_err(D::Error::custom),
            StructuralWire::CusipCode(value) => crate::CusipCode::new(value)
                .map(Self::CusipCode)
                .map_err(D::Error::custom),
            StructuralWire::BloombergCode(value) => crate::BloombergCode::new(value)
                .map(Self::BloombergCode)
                .map_err(serde::de::Error::custom),
            StructuralWire::FIGICode(value) => crate::FIGICode::new(value)
                .map(Self::FIGICode)
                .map_err(D::Error::custom),
            StructuralWire::Unit(value) => crate::Unit::new(value)
                .map(Self::Unit)
                .map_err(D::Error::custom),
            StructuralWire::SedolCode(value) => crate::SedolCode::new(value)
                .map(Self::SedolCode)
                .map_err(D::Error::custom),
            // A side and a state are read by their spelling, exactly as a
            // column reads them.
            StructuralWire::Side(value) => crate::Side::read(&value)
                .map(Self::Side)
                .map_err(D::Error::custom),
            StructuralWire::State(value) => crate::State::read(&value)
                .map(Self::State)
                .map_err(D::Error::custom),
            StructuralWire::TimeInForce(value) => crate::TimeInForce::new(value)
                .map(Self::TimeInForce)
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
            StructuralWire::Urn(value) => crate::Urn::from_str(value.as_str())
                .map(|value| Self::Urn(Arc::new(value)))
                .map_err(D::Error::custom),
            StructuralWire::Bytes(value) => Ok(value.0),
            StructuralWire::Geometry(value) => crate::geospatial::Geometry::new(value)
                .map(Self::Geometry)
                .map_err(D::Error::custom),
            StructuralWire::Geography(value) => crate::geospatial::Geography::new(value)
                .map(Self::Geography)
                .map_err(D::Error::custom),
            StructuralWire::Date32(Temporal32::Triple(count, unit, zone)) => {
                Self::date32_in(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Date32(Temporal32::Iso(spelled)) => {
                crate::temporal::parse_date(&spelled)
                    .map(Self::date32)
                    .map_err(D::Error::custom)
            }
            StructuralWire::Date64(Temporal64::Triple(count, unit, zone)) => {
                Self::date64_in(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Date64(Temporal64::Iso(spelled)) => {
                crate::temporal::parse_date(&spelled)
                    .map(|days| Self::date64(i64::from(days) * 86_400_000))
                    .map_err(D::Error::custom)
            }
            StructuralWire::Time32(Temporal32::Triple(count, unit, zone)) => {
                Self::time32(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::Time32(Temporal32::Iso(spelled)) => {
                crate::temporal::parse_time(&spelled)
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
                crate::temporal::parse_time(&spelled)
                    .and_then(|(count, unit)| Self::time64(count, unit, Timezone::NAIVE))
                    .map_err(D::Error::custom)
            }
            StructuralWire::DateTime64(Temporal64::Triple(count, unit, zone)) => {
                Self::datetime64(count, unit, zone).map_err(D::Error::custom)
            }
            StructuralWire::DateTime64(Temporal64::Iso(spelled)) => {
                crate::temporal::parse_timestamp(&spelled)
                    .and_then(|(count, unit, zone)| Self::datetime64(count, unit, zone))
                    .or_else(|_| {
                        crate::temporal::parse_datetime(&spelled).and_then(|(count, unit)| {
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
                crate::temporal::parse_duration(&spelled)
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
                crate::temporal::parse_duration(&spelled)
                    .and_then(|(count, unit)| Self::duration64(count, unit))
                    .map_err(D::Error::custom)
            }
            StructuralWire::Interval(value) => Ok(Self::Interval(value)),
            StructuralWire::Serie(Held::Rows(values)) => Ok(Self::from_sequence(values)),
            StructuralWire::Serie(Held::Column(column)) => Ok(Self::Serie(column)),
            StructuralWire::SerieView(held) => Ok(Self::SerieView(held.into_serie())),
            StructuralWire::FixedSizeSerie(held) => Ok(Self::FixedSizeSerie(held.into_serie())),
            StructuralWire::LargeSerie(held) => Ok(Self::LargeSerie(held.into_serie())),
            StructuralWire::LargeSerieView(held) => Ok(Self::LargeSerieView(held.into_serie())),
            StructuralWire::Map(entries) => Self::from_mapping(entries).map_err(D::Error::custom),
            StructuralWire::SortedMap(entries) => Self::from_mapping(entries)
                .map(|mapping| match mapping {
                    Self::Map(entries) => Self::SortedMap(entries),
                    other => other,
                })
                .map_err(D::Error::custom),
            StructuralWire::Struct(entries) => {
                Self::from_struct(entries.0).map_err(D::Error::custom)
            }
            StructuralWire::Variant(metadata, value) => crate::Variant::new(metadata, value)
                .map(Self::Variant)
                .map_err(D::Error::custom),
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
            // A value is one value whichever leaf holds it: every string
            // compares its characters, every byte value its payload.
            string_scalars!(left) => same_kind!(string_scalars!(right) => left.cmp(right)),
            Self::Country(_)
            | Self::Ccy(_)
            | Self::MicCode(_)
            | Self::CfiCode(_)
            | Self::Side(_)
            | Self::State(_)
            | Self::TimeInForce(_)
            | Self::IsinCode(_)
            | Self::CusipCode(_)
            | Self::SedolCode(_)
            | Self::BloombergCode(_)
            | Self::FIGICode(_)
            | Self::Unit(_) => code_key(self).cmp(&code_key(other)),
            Self::Uuid(left) => same_kind!(Self::Uuid(right) => left.cmp(right)),
            Self::Version(left) => same_kind!(Self::Version(right) => left.cmp(right)),
            Self::Timezone(left) => same_kind!(Self::Timezone(right) => left.cmp(right)),
            Self::MimeType(left) => same_kind!(Self::MimeType(right) => left.cmp(right)),
            Self::MediaType(left) => same_kind!(Self::MediaType(right) => left.cmp(right)),
            Self::Url(left) => same_kind!(Self::Url(right) => left.cmp(right)),
            Self::Urn(left) => same_kind!(Self::Urn(right) => left.cmp(right)),
            bytes_scalars!(left) => same_kind!(bytes_scalars!(right) => left.cmp(right)),
            Self::Geometry(_) | Self::Geography(_) => {
                unreachable!("both geospatial readings returned above")
            }
            Self::Serie(left)
            | Self::SerieView(left)
            | Self::FixedSizeSerie(left)
            | Self::LargeSerie(left)
            | Self::LargeSerieView(left) => {
                same_kind!((Self::Serie(right) | Self::SerieView(right) | Self::FixedSizeSerie(right) | Self::LargeSerie(right) | Self::LargeSerieView(right)) => left.cmp(right))
            }
            Self::Map(left) | Self::SortedMap(left) => {
                same_kind!((Self::Map(right) | Self::SortedMap(right)) => left.cmp(right))
            }
            Self::Struct(left) => same_kind!(Self::Struct(right) => left.cmp(right)),
            // A variant orders by its bytes, metadata first: its value is
            // not one this comparison decodes, and two encodings of one
            // value are one value only when their bytes agree.
            Self::Variant(left) => same_kind!(Self::Variant(right) => left.cmp(right)),
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
            string_scalars!(value) => value.hash(state),
            Self::Country(_)
            | Self::Ccy(_)
            | Self::MicCode(_)
            | Self::CfiCode(_)
            | Self::Side(_)
            | Self::State(_)
            | Self::TimeInForce(_)
            | Self::IsinCode(_)
            | Self::CusipCode(_)
            | Self::SedolCode(_)
            | Self::BloombergCode(_)
            | Self::FIGICode(_)
            | Self::Unit(_) => code_key(self).hash(state),
            Self::Uuid(value) => value.hash(state),
            Self::Version(value) => value.hash(state),
            Self::Timezone(value) => value.hash(state),
            Self::MimeType(value) => value.hash(state),
            Self::MediaType(value) => value.hash(state),
            Self::Url(value) => value.hash(state),
            Self::Urn(value) => value.hash(state),
            bytes_scalars!(value) => value.hash(state),
            Self::Geometry(value) => value.hash(state),
            Self::Geography(value) => value.hash(state),
            Self::Serie(value)
            | Self::SerieView(value)
            | Self::FixedSizeSerie(value)
            | Self::LargeSerie(value)
            | Self::LargeSerieView(value) => {
                0_isize.hash(state);
                value.hash(state);
            }
            Self::Map(value) | Self::SortedMap(value) => {
                1_isize.hash(state);
                value.hash(state);
            }
            Self::Struct(value) => {
                2_isize.hash(state);
                value.hash(state);
            }
            Self::Variant(value) => {
                3_isize.hash(state);
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
fn temporal_value(value: &Scalar) -> Option<(crate::TemporalKind, (u8, i128), Timezone)> {
    if matches!(value, Scalar::Interval(_)) {
        return None;
    }
    Some((
        value.temporal_kind()?,
        temporal_key(value.temporal_count()?, value.temporal_unit()?),
        value.temporal_timezone()?,
    ))
}

/// The twelve registered codes as one pattern.
///
/// A guard does not count towards exhaustiveness, so a match that must cover
/// every `Scalar` spells the codes out. This is where they are spelled, once;
/// [`Scalar::code_storage`] is the same list in value position.
macro_rules! code_scalars {
    () => {
        $crate::Scalar::Country(_)
            | $crate::Scalar::Ccy(_)
            | $crate::Scalar::MicCode(_)
            | $crate::Scalar::CfiCode(_)
            | $crate::Scalar::Side(_)
            | $crate::Scalar::State(_)
            | $crate::Scalar::TimeInForce(_)
            | $crate::Scalar::IsinCode(_)
            | $crate::Scalar::CusipCode(_)
            | $crate::Scalar::SedolCode(_)
            | $crate::Scalar::BloombergCode(_)
            | $crate::Scalar::FIGICode(_)
            | $crate::Scalar::Unit(_)
    };
}

/// The eighteen string leaves as one pattern, each binding its characters to
/// `$text`; the number a fixed or sized leaf states is not bound.
///
/// [`Scalar::as_string`] is the same list in value position, and
/// [`Scalar::string_parameters`] the leaf.
macro_rules! string_scalars {
    ($text:pat) => {
        $crate::Scalar::Utf8String($text)
            | $crate::Scalar::LargeUtf8String($text)
            | $crate::Scalar::Utf8StringView($text)
            | $crate::Scalar::LargeUtf8StringView($text)
            | $crate::Scalar::FixedUtf8String($text, _)
            | $crate::Scalar::SizedUtf8String($text, _)
            | $crate::Scalar::AsciiString($text)
            | $crate::Scalar::LargeAsciiString($text)
            | $crate::Scalar::AsciiStringView($text)
            | $crate::Scalar::LargeAsciiStringView($text)
            | $crate::Scalar::FixedAsciiString($text, _)
            | $crate::Scalar::SizedAsciiString($text, _)
            | $crate::Scalar::Cp1252String($text)
            | $crate::Scalar::LargeCp1252String($text)
            | $crate::Scalar::Cp1252StringView($text)
            | $crate::Scalar::LargeCp1252StringView($text)
            | $crate::Scalar::FixedCp1252String($text, _)
            | $crate::Scalar::SizedCp1252String($text, _)
    };
}

/// The six byte leaves as one pattern, each binding its payload to
/// `$payload`; the number a fixed or sized leaf states is not bound.
macro_rules! bytes_scalars {
    ($payload:pat) => {
        $crate::Scalar::Binary($payload)
            | $crate::Scalar::LargeBinary($payload)
            | $crate::Scalar::BinaryView($payload)
            | $crate::Scalar::LargeBinaryView($payload)
            | $crate::Scalar::FixedBinary($payload, _)
            | $crate::Scalar::SizedBinary($payload, _)
    };
}

/// The reading the twelve registered codes order and hash by.
///
/// They share one value rank, so the identity is what separates them: a
/// currency and a country whose bytes agree are two values.
fn code_key(value: &Scalar) -> (DataTypeId, &SmolStr) {
    (
        value.id(),
        value
            .code_storage()
            .expect("only a code reaches the shared code rank"),
    )
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
        string_scalars!(_) => 5,
        bytes_scalars!(_) => 6,
        Scalar::Date32(_) | Scalar::Date64(_) => 7,
        Scalar::Time32(_) | Scalar::Time64(_) => 8,
        Scalar::DateTime64(_) => 9,
        Scalar::Duration32(_) | Scalar::Duration64(_) => 10,
        Scalar::Serie(_)
        | Scalar::SerieView(_)
        | Scalar::FixedSizeSerie(_)
        | Scalar::LargeSerie(_)
        | Scalar::LargeSerieView(_) => 11,
        Scalar::Map(_) | Scalar::SortedMap(_) => 12,
        Scalar::Struct(_) => 13,
        Scalar::Geometry(_) | Scalar::Geography(_) => 14,
        // 15 was the enum member, since retired: a member is its name, so it
        // ranks with the text at 5. Only the order between kinds is read, so
        // the number stays unused rather than resequencing the rest - the
        // same convention `DataTypeId` keeps for its own retired byte.
        Scalar::Interval(_) => 16,
        Scalar::Uuid(_) => 17,
        Scalar::Country(_)
        | Scalar::Ccy(_)
        | Scalar::MicCode(_)
        | Scalar::CfiCode(_)
        | Scalar::Side(_)
        | Scalar::State(_)
        | Scalar::TimeInForce(_)
        | Scalar::IsinCode(_)
        | Scalar::CusipCode(_)
        | Scalar::SedolCode(_)
        | Scalar::BloombergCode(_)
        | Scalar::FIGICode(_)
        | Scalar::Unit(_) => 18,
        Scalar::Version(_) => 19,
        Scalar::Url(_) => 20,
        Scalar::Timezone(_) => 21,
        Scalar::MimeType(_) => 22,
        Scalar::MediaType(_) => 23,
        // 24 was the Arrow payload, since retired: a held column is the
        // serie it is, and ranks at 11.
        Scalar::Urn(_) => 25,
        // A variant is its own kind, ranked after the containers it can
        // hold: the bytes say what is inside, and nothing else orders by
        // what they decode to.
        Scalar::Variant(_) => 26,
    }
}

impl Scalar {
    /// Return the most specific datatype identifier the value itself proves.
    ///
    /// Nested values report their most-general shape; a [`Field`](crate::Field)
    /// narrows a sequence to a serie, fixed-size serie, struct, or union. Static
    /// enum members report UTF-8 because their column representation remains a
    /// field-level choice.
    pub const fn id(&self) -> DataTypeId {
        match self {
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
            // A string names the leaf it is stored in.
            Self::Utf8String(_) => DataTypeId::Utf8String,
            Self::LargeUtf8String(_) => DataTypeId::LargeUtf8String,
            Self::Utf8StringView(_) => DataTypeId::Utf8StringView,
            Self::LargeUtf8StringView(_) => DataTypeId::LargeUtf8StringView,
            Self::FixedUtf8String(_, _) => DataTypeId::FixedUtf8String,
            Self::SizedUtf8String(_, _) => DataTypeId::SizedUtf8String,
            Self::AsciiString(_) => DataTypeId::AsciiString,
            Self::LargeAsciiString(_) => DataTypeId::LargeAsciiString,
            Self::AsciiStringView(_) => DataTypeId::AsciiStringView,
            Self::LargeAsciiStringView(_) => DataTypeId::LargeAsciiStringView,
            Self::FixedAsciiString(_, _) => DataTypeId::FixedAsciiString,
            Self::SizedAsciiString(_, _) => DataTypeId::SizedAsciiString,
            Self::Cp1252String(_) => DataTypeId::Cp1252String,
            Self::LargeCp1252String(_) => DataTypeId::LargeCp1252String,
            Self::Cp1252StringView(_) => DataTypeId::Cp1252StringView,
            Self::LargeCp1252StringView(_) => DataTypeId::LargeCp1252StringView,
            Self::FixedCp1252String(_, _) => DataTypeId::FixedCp1252String,
            Self::SizedCp1252String(_, _) => DataTypeId::SizedCp1252String,
            Self::Country(_) => DataTypeId::Country,
            Self::Ccy(_) => DataTypeId::Ccy,
            Self::MicCode(_) => DataTypeId::MicCode,
            Self::CfiCode(_) => DataTypeId::CfiCode,
            Self::Side(_) => DataTypeId::Side,
            Self::State(_) => DataTypeId::State,
            Self::TimeInForce(_) => DataTypeId::TimeInForce,
            Self::IsinCode(_) => DataTypeId::IsinCode,
            Self::CusipCode(_) => DataTypeId::CusipCode,
            Self::SedolCode(_) => DataTypeId::SedolCode,
            Self::BloombergCode(_) => DataTypeId::BloombergCode,
            Self::FIGICode(_) => DataTypeId::FIGICode,
            Self::Unit(_) => DataTypeId::Unit,
            Self::Uuid(_) => DataTypeId::Uuid,
            Self::Version(_) => DataTypeId::Version,
            Self::Timezone(_) => DataTypeId::Timezone,
            Self::MimeType(_) => DataTypeId::MimeType,
            Self::MediaType(_) => DataTypeId::MediaType,
            Self::Url(_) => DataTypeId::Url,
            Self::Urn(_) => DataTypeId::Urn,
            Self::Binary(_) => DataTypeId::Binary,
            Self::LargeBinary(_) => DataTypeId::LargeBinary,
            Self::BinaryView(_) => DataTypeId::BinaryView,
            Self::LargeBinaryView(_) => DataTypeId::LargeBinaryView,
            Self::FixedBinary(_, _) => DataTypeId::FixedBinary,
            Self::SizedBinary(_, _) => DataTypeId::SizedBinary,
            Self::Geometry(_) => DataTypeId::Geometry,
            Self::Geography(_) => DataTypeId::Geography,
            Self::Serie(_) => DataTypeId::Serie,
            Self::SerieView(_) => DataTypeId::SerieView,
            Self::FixedSizeSerie(_) => DataTypeId::FixedSizeSerie,
            Self::LargeSerie(_) => DataTypeId::LargeSerie,
            Self::LargeSerieView(_) => DataTypeId::LargeSerieView,
            Self::Map(_) => DataTypeId::Map,
            Self::SortedMap(_) => DataTypeId::SortedMap,
            Self::Struct(_) => DataTypeId::Struct,
            Self::Variant(_) => DataTypeId::Variant,
        }
    }

    /// Return the datatype family the value itself proves: the one whose
    /// [range](DataTypeKind::range) [`Self::id`] is in.
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
            // The plain leaf keeps the family's own word; every other leaf
            // is its name.
            string_scalars!(_) => match self {
                Self::Utf8String(_) => "string",
                _ => self.id().as_str(),
            },
            Self::Country(_) => DataTypeId::Country.as_str(),
            Self::Ccy(_) => DataTypeId::Ccy.as_str(),
            Self::MicCode(_) => DataTypeId::MicCode.as_str(),
            Self::CfiCode(_) => DataTypeId::CfiCode.as_str(),
            Self::Side(_) => DataTypeId::Side.as_str(),
            Self::State(_) => DataTypeId::State.as_str(),
            Self::TimeInForce(_) => DataTypeId::TimeInForce.as_str(),
            Self::IsinCode(_) => DataTypeId::IsinCode.as_str(),
            Self::CusipCode(_) => DataTypeId::CusipCode.as_str(),
            Self::SedolCode(_) => DataTypeId::SedolCode.as_str(),
            Self::BloombergCode(_) => DataTypeId::BloombergCode.as_str(),
            Self::FIGICode(_) => DataTypeId::FIGICode.as_str(),
            Self::Unit(_) => DataTypeId::Unit.as_str(),
            Self::Uuid(_) => "uuid",
            Self::Version(_) => "version",
            Self::Timezone(_) => "timezone",
            Self::MimeType(_) => "mimetype",
            Self::MediaType(_) => "mediatype",
            Self::Url(_) => "url",
            Self::Urn(_) => "urn",
            bytes_scalars!(_) => match self {
                Self::Binary(_) => "bytes",
                _ => self.id().as_str(),
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
            Self::Serie(_) => "serie",
            Self::SerieView(_) => "serie_view",
            Self::FixedSizeSerie(_) => "fixed_size_serie",
            Self::LargeSerie(_) => "large_serie",
            Self::LargeSerieView(_) => "large_serie_view",
            Self::Map(_) => "map",
            Self::SortedMap(_) => "sorted_map",
            Self::Struct(_) => "struct",
            Self::Variant(_) => "variant",
        }
    }

    /// The one shared empty sequence, which every empty run answers with.
    fn empty_sequence() -> Self {
        static EMPTY: OnceLock<Arc<[Scalar]>> = OnceLock::new();
        Self::Serie(Serie::Run(Run::new(Arc::clone(
            EMPTY.get_or_init(|| Arc::from([])),
        ))))
    }

    /// The one shared empty mapping.
    fn empty_mapping() -> Self {
        static EMPTY: OnceLock<Arc<[(Scalar, Scalar)]>> = OnceLock::new();
        Self::Map(Map::new(Arc::clone(EMPTY.get_or_init(|| Arc::from([])))))
    }

    /// Construct an ordered sequence.
    ///
    /// The children are written straight into the shared slice they are
    /// stored in. A `Vec` on the way would allocate a second buffer and copy
    /// the whole run between the two, which is what a row build pays per row.
    pub fn from_sequence(values: impl IntoIterator<Item = Self>) -> Self {
        shared_children(values.into_iter()).map_or_else(Self::empty_sequence, |values| {
            Self::Serie(Serie::Run(Run::new(values)))
        })
    }

    /// Build a known-width sequence in its final shared storage.
    ///
    /// The callback is evaluated once for each index, in ascending order, and
    /// the first refusal stops the build. Null initializes the shared slice so
    /// a partial build remains safe to drop without a temporary `Vec`.
    pub(crate) fn try_sequence(
        len: usize,
        mut at: impl FnMut(usize) -> Result<Self>,
    ) -> Result<Self> {
        if len == 0 {
            return Ok(Self::empty_sequence());
        }
        let mut values = (0..len).map(|_| Self::Null).collect::<Arc<[_]>>();
        let unique =
            Arc::get_mut(&mut values).expect("newly collected sequence storage has one owner");
        for (index, value) in unique.iter_mut().enumerate() {
            *value = at(index)?;
        }
        Ok(Self::Serie(Serie::Run(Run::new(values))))
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
        Ok(Self::Map(Map::new(entries)))
    }

    /// Construct a deterministic record sorted by field name.
    ///
    /// # Errors
    ///
    /// Returns an error when a field name occurs more than once.
    pub fn from_struct<K, I>(entries: I) -> Result<Self>
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
            return Ok(Self::Struct(Struct::new(Arc::clone(
                EMPTY.get_or_init(|| Arc::new(BTreeMap::new())),
            ))));
        }
        Ok(Self::Struct(Struct::new(Arc::new(record))))
    }

    /// Return a boolean when this is a boolean.
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(value.get()),
            _ => None,
        }
    }

    /// Return whether this value reads as true where a condition is wanted.
    ///
    /// This is a coercion, not a reading: it answers for every value, where
    /// [`Self::as_bool`] answers only for a boolean and keeps its `None` for
    /// the three-valued logic a filter walks. Nothing falls back to it, so a
    /// caller asking "is this set?" gets an answer and a caller asking "is
    /// this a boolean?" still does not.
    ///
    /// Falsy is emptiness and zero, the way Python reads them: absence, a
    /// zero of any width, empty text or bytes, and an empty container. A
    /// container of falsy values is falsy too - a struct whose every field is
    /// null reads as unset rather than as present-but-empty - which
    /// [`Self::is_empty`] does not say, because that one only counts entries.
    ///
    /// Text is the one place this is wider than Python, and deliberately:
    /// `"false"`, `"no"`, `"off"` and `"0"` read as false, where Python calls
    /// every non-empty string true. Values arrive as text from CSV, FIX and
    /// query strings, and a column that spells false is not asking to be
    /// read as true. The reading is ASCII case-insensitive and trims.
    /// [`crate::Boolean`]'s own text reader stays strict - it is the
    /// String-to-Boolean *cast*, and a cast that guessed this widely would
    /// accept text no schema declared.
    ///
    /// ```
    /// use yggdryl::Scalar;
    ///
    /// assert!(Scalar::from(5).is_truthy());
    /// assert!(!Scalar::from(0).is_truthy());
    /// assert!(!Scalar::Null.is_truthy());
    /// assert!(!Scalar::from("").is_truthy());
    /// assert!(!Scalar::from("OFF").is_truthy());
    /// assert!(Scalar::from("anything else").is_truthy());
    /// assert!(!Scalar::from_sequence([Scalar::Null, Scalar::from(0)]).is_truthy());
    /// assert!(Scalar::from_sequence([Scalar::from(1)]).is_truthy());
    ///
    /// // A column answers as the run of its rows does, an empty one included.
    /// # use yggdryl::{DataType, Field, Serie};
    /// let field = Field::new("flag", DataType::Int64, false);
    /// let column = |rows: [Scalar; 1]| {
    ///     Scalar::from(Serie::from_scalars(field.clone(), rows).expect("one row"))
    /// };
    /// assert!(column([Scalar::from(1_i64)]).is_truthy());
    /// assert!(!column([Scalar::from(0_i64)]).is_truthy());
    /// assert!(!Scalar::from(Serie::empty(field).expect("an empty column")).is_truthy());
    /// ```
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        if let Self::Null = self {
            return false;
        }
        if let Some(value) = self.as_bool() {
            return value;
        }
        if let Some(value) = self.as_i128() {
            return value != 0;
        }
        if let Some((unscaled, _)) = self.as_decimal() {
            return unscaled != crate::i256::ZERO;
        }
        if let Some(value) = self.as_f64() {
            // NaN is not zero, so it is present. Only the two zeroes are not.
            return value != 0.0;
        }
        if let Some(text) = self.as_str() {
            let trimmed = text.trim();
            return !matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "" | "0" | "f" | "n" | "no" | "off" | "false"
            );
        }
        if let Some(bytes) = self.as_bytes() {
            return !bytes.is_empty();
        }
        if self.as_serie().is_some() {
            // Either leaf: a run's values lent, a column's rows built one
            // at a time, and an empty column is as false as an empty run.
            return self.iter().any(|value| value.is_truthy());
        }
        if let Some(entries) = self.as_mapping() {
            return entries.iter().any(|(_, value)| value.is_truthy());
        }
        if let Some(entries) = self.as_struct() {
            return entries.values().any(Self::is_truthy);
        }
        true
    }

    /// The text of a string value of any leaf, or of a registered code.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            string_scalars!(value) => Some(value.as_str()),
            value => value.code_storage().map(SmolStr::as_str),
        }
    }

    /// Borrow the validated storage when this is a registered code.
    ///
    /// The twelve codes are twelve variants, but every question but "which
    /// one" has the same answer for all of them, so this is where they are
    /// written out and [`Self::id`] is the other half: the identity leads, and
    /// the text follows it. A currency and a country whose bytes agree are two
    /// values, and they order by which code they are before they order by
    /// text.
    pub const fn code_storage(&self) -> Option<&SmolStr> {
        match self {
            Self::Country(value) => Some(value.storage()),
            Self::Ccy(value) => Some(value.storage()),
            Self::MicCode(value) => Some(value.storage()),
            Self::CfiCode(value) => Some(value.storage()),
            Self::Side(value) => Some(value.storage()),
            Self::State(value) => Some(value.storage()),
            Self::TimeInForce(value) => Some(value.storage()),
            Self::IsinCode(value) => Some(value.storage()),
            Self::CusipCode(value) => Some(value.storage()),
            Self::SedolCode(value) => Some(value.storage()),
            Self::BloombergCode(value) => Some(value.storage()),
            Self::FIGICode(value) => Some(value.storage()),
            Self::Unit(value) => Some(value.storage()),
            _ => None,
        }
    }

    /// Whether this value is a code drawn from a published registry.
    #[must_use]
    pub const fn is_code(&self) -> bool {
        DataTypeKind::Code.contains(self.id())
    }

    /// The payload of a byte value of any leaf, or of a geospatial value.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            bytes_scalars!(value) => Some(value.as_bytes()),
            Self::Geometry(value) => Some(value.as_bytes()),
            Self::Geography(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    /// Encode this value as compact JSON bytes.
    pub fn into_json_bytes(&self) -> Result<Vec<u8>> {
        crate::json::into_bytes(self)
    }

    /// Encode this value as compact JSON UTF-8.
    pub fn into_json(&self) -> Result<String> {
        crate::json::into_utf8(self)
    }

    /// Return the Well-Known Binary payload without allocating.
    ///
    /// A geospatial column also accepts plain bytes on the way in -
    /// canonicalization is what rewrites it - so this reads both spellings.
    pub fn as_wkb(&self) -> Option<&[u8]> {
        self.as_bytes()
    }

    /// Return a run's children without allocating: `None` for a column,
    /// which stores no value to lend.
    pub fn as_sequence(&self) -> Option<&[Self]> {
        match self {
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => values.as_slice(),
            _ => None,
        }
    }

    /// Return any sequence - a run or a column - without allocating.
    pub fn as_serie(&self) -> Option<&Serie> {
        match self {
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => Some(values),
            _ => None,
        }
    }

    /// Return a sequence's rows: a run's lent, a column's built.
    ///
    /// The door every reader of meaning takes; a column holds only rows its
    /// field accepts, so building them cannot refuse.
    pub fn sequence_rows(&self) -> Option<Cow<'_, [Self]>> {
        match self {
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => Some(values.rows()),
            _ => None,
        }
    }

    /// Return mapping entries without allocating.
    pub fn as_mapping(&self) -> Option<&[(Self, Self)]> {
        match self {
            Self::Map(entries) | Self::SortedMap(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    /// Return record fields in deterministic name order.
    pub fn as_struct(&self) -> Option<&BTreeMap<SmolStr, Self>> {
        match self {
            Self::Struct(entries) => Some(entries.as_map()),
            _ => None,
        }
    }

    /// Return the number of direct children or mapping entries.
    pub fn len(&self) -> usize {
        match self {
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => values.len(),
            Self::Map(entries) | Self::SortedMap(entries) => entries.as_slice().len(),
            Self::Struct(entries) => entries.as_map().len(),
            _ => 0,
        }
    }

    /// Return whether this is an empty sequence or mapping.
    pub fn is_empty(&self) -> bool {
        self.is_container() && self.len() == 0
    }

    /// Look up a sequence index, or `None` past the end.
    ///
    /// Borrowed for a run, built for a column: the one lookup that
    /// allocates on a column, by contract.
    pub fn get(&self, index: usize) -> Option<Cow<'_, Self>> {
        self.as_serie()?.get(index)
    }

    /// Look up a mapping key without allocating.
    pub fn get_key(&self, key: &Self) -> Option<&Self> {
        self.as_mapping()?
            .iter()
            .find_map(|(candidate, value)| (candidate == key).then_some(value))
    }

    /// Look up a string mapping key without constructing a temporary value.
    pub fn get_key_str(&self, key: &str) -> Option<&Self> {
        if let Self::Struct(entries) = self {
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
            Self::Serie(values)
            | Self::SerieView(values)
            | Self::FixedSizeSerie(values)
            | Self::LargeSerie(values)
            | Self::LargeSerieView(values) => values.iter(),
            Self::Map(entries) | Self::SortedMap(entries) => {
                Children::Mapping(entries.as_slice().iter())
            }
            Self::Struct(entries) => Children::Struct(entries.as_map().values()),
            _ => Children::Sequence([].iter()),
        }
    }

    /// Iterate over mapping entries without allocating.
    pub fn mapping_iter(&self) -> std::slice::Iter<'_, (Self, Self)> {
        self.as_mapping().unwrap_or_default().iter()
    }

    /// Iterate over record name/value pairs in deterministic name order.
    pub fn record_iter(&self) -> std::collections::btree_map::Iter<'_, SmolStr, Self> {
        static EMPTY: OnceLock<BTreeMap<SmolStr, Scalar>> = OnceLock::new();
        self.as_struct()
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeMap::new))
            .iter()
    }

    /// Return whether this is the null value.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Return whether this value holds other values.
    pub const fn is_container(&self) -> bool {
        matches!(
            self,
            Self::Serie(_)
                | Self::SerieView(_)
                | Self::FixedSizeSerie(_)
                | Self::LargeSerie(_)
                | Self::LargeSerieView(_)
                | Self::Map(_)
                | Self::SortedMap(_)
                | Self::Struct(_)
        )
    }

    /// Return whether this is a number of any width: an integer, a float or
    /// a decimal.
    pub const fn is_number(&self) -> bool {
        self.family().is_numeric()
    }

    /// Borrow the width leaf's own [`fmt::Display`], for a variant that holds
    /// one.
    ///
    /// Every integer, float, decimal, temporal and nested variant answers the
    /// leaf it holds, whose text is that leaf's plain spelling; every other
    /// variant answers `None`, because its spelling is the caller's to choose.
    pub(crate) fn leaf_display(&self) -> Option<&dyn fmt::Display> {
        let leaf: &dyn fmt::Display = match self {
            Self::Variant(value) => value,
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
            Self::Serie(value)
            | Self::SerieView(value)
            | Self::FixedSizeSerie(value)
            | Self::LargeSerie(value)
            | Self::LargeSerieView(value) => value,
            Self::Map(value) | Self::SortedMap(value) => value,
            Self::Struct(value) => value,
            Self::Null
            | Self::Boolean(_)
            | string_scalars!(_)
            | Self::Country(_)
            | Self::Ccy(_)
            | Self::MicCode(_)
            | Self::CfiCode(_)
            | Self::Side(_)
            | Self::State(_)
            | Self::TimeInForce(_)
            | Self::IsinCode(_)
            | Self::CusipCode(_)
            | Self::SedolCode(_)
            | Self::BloombergCode(_)
            | Self::FIGICode(_)
            | Self::Unit(_)
            | Self::Uuid(_)
            | Self::Version(_)
            | Self::Url(_)
            | Self::Urn(_)
            | bytes_scalars!(_)
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
    /// assert_eq!(order.path("legs.0.price").and_then(|price| price.as_i64()), Some(12));
    /// assert!(order.path("legs.9.price").is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn path(&self, path: &str) -> Option<Cow<'_, Self>> {
        let mut current = Cow::Borrowed(self);
        for segment in path.split('.').filter(|segment| !segment.is_empty()) {
            current = match current {
                Cow::Borrowed(held) => held.step(segment)?,
                // A row built out of a column is owned here, so what it
                // holds is cloned out rather than borrowed past its life.
                Cow::Owned(held) => Cow::Owned(held.step(segment)?.into_owned()),
            };
        }
        Some(current)
    }

    /// One step of a path: a key of a mapping or a record, a row of a
    /// sequence.
    fn step(&self, segment: &str) -> Option<Cow<'_, Self>> {
        match self {
            Self::Map(_) | Self::SortedMap(_) | Self::Struct(_) => {
                self.get_key_str(segment).map(Cow::Borrowed)
            }
            Self::Serie(_)
            | Self::SerieView(_)
            | Self::FixedSizeSerie(_)
            | Self::LargeSerie(_)
            | Self::LargeSerieView(_) => self.get(segment.parse::<usize>().ok()?),
            _ => None,
        }
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
        if let Self::Struct(entries) = self {
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
        let entries = self.as_struct().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: smol_str::format_smolstr!(
                "expected a record to set a field on, got {}",
                self.kind()
            ),
        })?;
        let mut rebuilt = entries.clone();
        rebuilt.insert(name.into(), value.into());
        Ok(Self::Struct(Struct::new(Arc::new(rebuilt))))
    }

    /// Return this record without `name`, preserving deterministic order.
    pub fn without_field(&self, name: &str) -> Result<Self> {
        let entries = self.as_struct().ok_or_else(|| Error::InvalidRecord {
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
        Ok(Self::Struct(Struct::new(Arc::new(rebuilt))))
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

pub(crate) use bytes_scalars;
pub(crate) use code_scalars;
pub(crate) use string_scalars;
pub(crate) use text_leaf_value;

impl From<Vec<Scalar>> for Scalar {
    fn from(value: Vec<Scalar>) -> Self {
        Self::from_sequence(value)
    }
}

impl FromIterator<Scalar> for Scalar {
    fn from_iter<T: IntoIterator<Item = Scalar>>(iter: T) -> Self {
        Self::from_sequence(iter)
    }
}

impl Index<&Scalar> for Scalar {
    type Output = Scalar;

    fn index(&self, key: &Scalar) -> &Self::Output {
        self.get_key(key).expect("mapping key is not present")
    }
}

impl Index<&str> for Scalar {
    type Output = Scalar;

    fn index(&self, key: &str) -> &Self::Output {
        self.get_key_str(key).expect("mapping key is not present")
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/scalar.rs` pins and a caller cannot reach.
    //!
    //! Three crate-private readers: the fallible sequence build every
    //! recursive reader fills through, the rank sweep that decides how two
    //! kinds sort - which is wire-visible, because it orders dictionary
    //! values - and the sign-and-magnitude reader every integer width
    //! compares through. Each is behind a forwarder, so nothing here is more
    //! public than it was.

    use std::cmp::Ordering;
    use std::fmt;

    use crate::{Result, Scalar};

    /// Build a sequence of `len` values, stopping at the first refusal.
    pub fn try_sequence(len: usize, at: impl FnMut(usize) -> Result<Scalar>) -> Result<Scalar> {
        Scalar::try_sequence(len, at)
    }

    /// The sort rank of a value's kind.
    #[must_use]
    pub fn value_rank(value: &Scalar) -> u8 {
        super::value_rank(value)
    }

    /// The leaf's own spelling, where the leaf owns one.
    #[must_use]
    pub fn leaf_display(value: &Scalar) -> Option<&dyn fmt::Display> {
        value.leaf_display()
    }

    /// Read an integer value as a sign and a magnitude.
    #[must_use]
    pub fn integer_parts(value: &Scalar) -> Option<(bool, u128)> {
        crate::integer::integer_parts(value)
    }

    /// Order two sign-and-magnitude readings.
    #[must_use]
    pub fn compare_integer_parts(left: (bool, u128), right: (bool, u128)) -> Ordering {
        crate::integer::compare_integer_parts(left, right)
    }

    /// The deterministic hash a value's `Hash` feed produces.
    ///
    /// The pin is on the bytes themselves, which are wire-visible, and
    /// `crate::hashing::stable_hash_of` is crate-private.
    #[must_use]
    pub fn stable_hash_of(value: &Scalar) -> u64 {
        crate::hashing::stable_hash_of(value)
    }

    /// The well-known-binary an empty point is spelled as.
    ///
    /// The crate root declares `default` privately, so the bytes the rank
    /// sweep builds its one geometry from are unreachable otherwise.
    #[must_use]
    pub fn point_empty_wkb() -> &'static [u8] {
        crate::default::POINT_EMPTY_WKB.as_slice()
    }
}
