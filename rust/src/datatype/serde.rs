//! Stable structural serialization and deserialization.

use ::serde::ser::SerializeSeq;
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, Field, Result};

use super::{DataType, TimeUnit, UnionFields, UnionMode};

impl DataType {
    /// Deserializes the stable tagged-object representation from JSON.
    ///
    /// Nested fields and parameters are reconstructed through the same
    /// validation used by native constructors. Canonical datatype strings are
    /// intentionally handled by [`Self::from_str`], not this method.
    pub fn from_json(input: &str) -> Result<Self> {
        serde_json::from_str(input).map_err(Error::from)
    }

    /// Serializes this value to deterministic structural JSON.
    ///
    /// The representation is a tagged object such as
    /// `{"type":"decimal128","precision":38,"scale":6}`. Invalid
    /// caller-built enum states are rejected before any output is emitted.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(Error::from)
    }

    /// Consumes this value and serializes it to deterministic structural JSON.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DataTypeRef<'a> {
    Null {},
    Boolean {},
    Int8 {},
    Int16 {},
    Int32 {},
    Int64 {},
    UInt8 {},
    UInt16 {},
    UInt32 {},
    UInt64 {},
    Float16 {},
    Float32 {},
    Float64 {},
    Timestamp {
        unit: TimeUnit,
        #[serde(skip_serializing_if = "Option::is_none")]
        timezone: Option<&'a str>,
    },
    Date32 {},
    Date64 {},
    Time32 {
        unit: TimeUnit,
    },
    Time64 {
        unit: TimeUnit,
    },
    Duration {
        unit: TimeUnit,
    },
    Interval {
        unit: TimeUnit,
    },
    Binary {},
    FixedSizeBinary {
        width: i32,
    },
    LargeBinary {},
    BinaryView {},
    Utf8 {},
    LargeUtf8 {},
    Utf8View {},
    List {
        field: &'a Field,
    },
    ListView {
        field: &'a Field,
    },
    FixedSizeList {
        field: &'a Field,
        length: i32,
    },
    LargeList {
        field: &'a Field,
    },
    LargeListView {
        field: &'a Field,
    },
    Struct {
        fields: &'a [Field],
    },
    Union {
        mode: UnionMode,
        fields: UnionFieldsRef<'a>,
    },
    Dictionary {
        key: &'a DataType,
        value: &'a DataType,
    },
    Decimal32 {
        precision: u8,
        scale: i8,
    },
    Decimal64 {
        precision: u8,
        scale: i8,
    },
    Decimal128 {
        precision: u8,
        scale: i8,
    },
    Decimal256 {
        precision: u8,
        scale: i8,
    },
    Map {
        entries: &'a Field,
        keys_sorted: bool,
    },
    RunEndEncoded {
        run_ends: &'a Field,
        values: &'a Field,
    },
}

#[derive(Clone, Copy)]
struct UnionFieldsRef<'a>(&'a UnionFields);

#[derive(Serialize)]
struct UnionMemberRef<'a> {
    type_id: i8,
    field: &'a Field,
}

impl Serialize for UnionFieldsRef<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for (type_id, field) in self.0.iter() {
            sequence.serialize_element(&UnionMemberRef { type_id, field })?;
        }
        sequence.end()
    }
}

impl<'a> From<&'a DataType> for DataTypeRef<'a> {
    #[allow(clippy::too_many_lines)]
    fn from(value: &'a DataType) -> Self {
        use DataType as D;
        match value {
            D::Null => Self::Null {},
            D::Boolean => Self::Boolean {},
            D::Int8 => Self::Int8 {},
            D::Int16 => Self::Int16 {},
            D::Int32 => Self::Int32 {},
            D::Int64 => Self::Int64 {},
            D::UInt8 => Self::UInt8 {},
            D::UInt16 => Self::UInt16 {},
            D::UInt32 => Self::UInt32 {},
            D::UInt64 => Self::UInt64 {},
            D::Float16 => Self::Float16 {},
            D::Float32 => Self::Float32 {},
            D::Float64 => Self::Float64 {},
            D::Timestamp(unit, timezone) => Self::Timestamp {
                unit: *unit,
                timezone: timezone.as_ref().map(crate::Timezone::as_str),
            },
            D::Date32 => Self::Date32 {},
            D::Date64 => Self::Date64 {},
            D::Time32(unit) => Self::Time32 { unit: *unit },
            D::Time64(unit) => Self::Time64 { unit: *unit },
            D::Duration(unit) => Self::Duration { unit: *unit },
            D::Interval(unit) => Self::Interval { unit: *unit },
            D::Binary => Self::Binary {},
            D::FixedSizeBinary(width) => Self::FixedSizeBinary { width: *width },
            D::LargeBinary => Self::LargeBinary {},
            D::BinaryView => Self::BinaryView {},
            D::Utf8 => Self::Utf8 {},
            D::LargeUtf8 => Self::LargeUtf8 {},
            D::Utf8View => Self::Utf8View {},
            D::List(field) => Self::List { field },
            D::ListView(field) => Self::ListView { field },
            D::FixedSizeList(field, length) => Self::FixedSizeList {
                field,
                length: *length,
            },
            D::LargeList(field) => Self::LargeList { field },
            D::LargeListView(field) => Self::LargeListView { field },
            D::Struct(fields) => Self::Struct {
                fields: fields.as_fields(),
            },
            D::Union(fields, mode) => Self::Union {
                mode: *mode,
                fields: UnionFieldsRef(fields),
            },
            D::Dictionary(dictionary) => Self::Dictionary {
                key: &dictionary.key,
                value: &dictionary.value,
            },
            D::Decimal32 { precision, scale } => Self::Decimal32 {
                precision: *precision,
                scale: *scale,
            },
            D::Decimal64 { precision, scale } => Self::Decimal64 {
                precision: *precision,
                scale: *scale,
            },
            D::Decimal128 { precision, scale } => Self::Decimal128 {
                precision: *precision,
                scale: *scale,
            },
            D::Decimal256 { precision, scale } => Self::Decimal256 {
                precision: *precision,
                scale: *scale,
            },
            D::Map(map) => Self::Map {
                entries: &map.entries,
                keys_sorted: map.keys_sorted,
            },
            D::RunEndEncoded(encoded) => Self::RunEndEncoded {
                run_ends: &encoded.run_ends,
                values: &encoded.values,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum DataTypeValue {
    Null {},
    Boolean {},
    Int8 {},
    Int16 {},
    Int32 {},
    Int64 {},
    UInt8 {},
    UInt16 {},
    UInt32 {},
    UInt64 {},
    Float16 {},
    Float32 {},
    Float64 {},
    Timestamp {
        unit: TimeUnit,
        #[serde(default)]
        timezone: Option<crate::Timezone>,
    },
    Date32 {},
    Date64 {},
    Time32 {
        unit: TimeUnit,
    },
    Time64 {
        unit: TimeUnit,
    },
    Duration {
        unit: TimeUnit,
    },
    Interval {
        unit: TimeUnit,
    },
    Binary {},
    FixedSizeBinary {
        width: i32,
    },
    LargeBinary {},
    BinaryView {},
    Utf8 {},
    LargeUtf8 {},
    Utf8View {},
    List {
        field: Field,
    },
    ListView {
        field: Field,
    },
    FixedSizeList {
        field: Field,
        length: i32,
    },
    LargeList {
        field: Field,
    },
    LargeListView {
        field: Field,
    },
    Struct {
        fields: Vec<Field>,
    },
    Union {
        mode: UnionMode,
        fields: Vec<UnionMemberValue>,
    },
    Dictionary {
        key: Box<DataType>,
        value: Box<DataType>,
    },
    Decimal32 {
        precision: u8,
        scale: i8,
    },
    Decimal64 {
        precision: u8,
        scale: i8,
    },
    Decimal128 {
        precision: u8,
        scale: i8,
    },
    Decimal256 {
        precision: u8,
        scale: i8,
    },
    Map {
        entries: Field,
        keys_sorted: bool,
    },
    RunEndEncoded {
        run_ends: Field,
        values: Field,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnionMemberValue {
    type_id: i8,
    field: Field,
}

impl TryFrom<DataTypeValue> for DataType {
    type Error = Error;

    #[allow(clippy::too_many_lines)]
    fn try_from(value: DataTypeValue) -> Result<Self> {
        Ok(match value {
            DataTypeValue::Null {} => Self::Null,
            DataTypeValue::Boolean {} => Self::Boolean,
            DataTypeValue::Int8 {} => Self::Int8,
            DataTypeValue::Int16 {} => Self::Int16,
            DataTypeValue::Int32 {} => Self::Int32,
            DataTypeValue::Int64 {} => Self::Int64,
            DataTypeValue::UInt8 {} => Self::UInt8,
            DataTypeValue::UInt16 {} => Self::UInt16,
            DataTypeValue::UInt32 {} => Self::UInt32,
            DataTypeValue::UInt64 {} => Self::UInt64,
            DataTypeValue::Float16 {} => Self::Float16,
            DataTypeValue::Float32 {} => Self::Float32,
            DataTypeValue::Float64 {} => Self::Float64,
            DataTypeValue::Timestamp { unit, timezone } => Self::Timestamp(unit, timezone),
            DataTypeValue::Date32 {} => Self::Date32,
            DataTypeValue::Date64 {} => Self::Date64,
            DataTypeValue::Time32 { unit } => Self::time32(unit)?,
            DataTypeValue::Time64 { unit } => Self::time64(unit)?,
            DataTypeValue::Duration { unit } => Self::Duration(unit),
            DataTypeValue::Interval { unit } => Self::Interval(unit),
            DataTypeValue::Binary {} => Self::Binary,
            DataTypeValue::FixedSizeBinary { width } => Self::fixed_size_binary(width)?,
            DataTypeValue::LargeBinary {} => Self::LargeBinary,
            DataTypeValue::BinaryView {} => Self::BinaryView,
            DataTypeValue::Utf8 {} => Self::Utf8,
            DataTypeValue::LargeUtf8 {} => Self::LargeUtf8,
            DataTypeValue::Utf8View {} => Self::Utf8View,
            DataTypeValue::List { field } => Self::list(field),
            DataTypeValue::ListView { field } => Self::list_view(field),
            DataTypeValue::FixedSizeList { field, length } => Self::fixed_size_list(field, length)?,
            DataTypeValue::LargeList { field } => Self::large_list(field),
            DataTypeValue::LargeListView { field } => Self::large_list_view(field),
            DataTypeValue::Struct { fields } => Self::from_fields(fields)?,
            DataTypeValue::Union { mode, fields } => Self::union(
                fields
                    .into_iter()
                    .map(|member| (member.type_id, member.field)),
                mode,
            )?,
            DataTypeValue::Dictionary { key, value } => Self::dictionary(*key, *value)?,
            DataTypeValue::Decimal32 { precision, scale } => Self::decimal32(precision, scale)?,
            DataTypeValue::Decimal64 { precision, scale } => Self::decimal64(precision, scale)?,
            DataTypeValue::Decimal128 { precision, scale } => Self::decimal128(precision, scale)?,
            DataTypeValue::Decimal256 { precision, scale } => Self::decimal256(precision, scale)?,
            DataTypeValue::Map {
                entries,
                keys_sorted,
            } => Self::map(entries, keys_sorted)?,
            DataTypeValue::RunEndEncoded { run_ends, values } => {
                Self::run_end_encoded(run_ends, values)?
            }
        })
    }
}

impl Serialize for DataType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(::serde::ser::Error::custom)?;
        DataTypeRef::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DataType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Self::try_from(DataTypeValue::deserialize(deserializer)?)
            .map_err(::serde::de::Error::custom)?;
        value.validate().map_err(::serde::de::Error::custom)?;
        Ok(value)
    }
}
