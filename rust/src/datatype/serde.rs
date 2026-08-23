//! Stable structural serialization and deserialization.

use ::serde::ser::SerializeSeq;
use ::serde::{Deserialize, Deserializer, Serialize, Serializer};

use smol_str::{SmolStr, format_smolstr};

use crate::generic::Scalar;
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
    Duration32 {
        unit: TimeUnit,
    },
    Duration64 {
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
    Variant {},
    Geometry {
        crs: &'a str,
    },
    Geography {
        crs: &'a str,
        algorithm: crate::generic::EdgeAlgorithm,
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
            D::Duration32(unit) => Self::Duration32 { unit: *unit },
            D::Duration64(unit) => Self::Duration64 { unit: *unit },
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
            D::Variant => Self::Variant {},
            D::Geometry(geospatial) => Self::Geometry {
                crs: geospatial.crs(),
            },
            D::Geography(geospatial) => Self::Geography {
                crs: geospatial.crs(),
                // A geography's algorithm is always present; the constructor
                // filled the default, so the stored value is the truth.
                algorithm: geospatial.algorithm().unwrap_or_default(),
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
    Duration32 {
        unit: TimeUnit,
    },
    Duration64 {
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
    Variant {},
    Geometry {
        crs: SmolStr,
    },
    Geography {
        crs: SmolStr,
        #[serde(default)]
        algorithm: Option<crate::generic::EdgeAlgorithm>,
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
            DataTypeValue::Duration32 { unit } => Self::duration32(unit)?,
            DataTypeValue::Duration64 { unit } => Self::duration64(unit)?,
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
            DataTypeValue::Variant {} => Self::Variant,
            DataTypeValue::Geometry { crs } => Self::geometry(Some(&crs))?,
            DataTypeValue::Geography { crs, algorithm } => Self::geography(Some(&crs), algorithm)?,
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

// ---------------------------------------------------------------------------
// The one structural mapping between a `DataType` and the shared `Scalar`.
//
// Every serialized form of a schema goes through here. JSON, YAML, and TOML
// are three writers over one model rather than three hand-written serializers,
// which is what makes them agree by construction instead of by three sets of
// tests. The shape is exactly what `into_json` has always emitted - the same
// keys, the same order, the same conditional omissions - so re-expressing the
// JSON path over this conversion changes no byte a caller could observe.
// ---------------------------------------------------------------------------

/// The tag key every datatype mapping opens with.
const TYPE_KEY: &str = "type";

impl DataType {
    /// Project this datatype onto the shared structural [`Scalar`].
    ///
    /// A tagged mapping - `{"type": "decimal128", "precision": 9, "scale": 2}` -
    /// whose keys are emitted in a fixed order so two equal datatypes produce
    /// byte-identical output in every format. Nested datatypes recurse through
    /// the same conversion, so a struct's children, a list's item, a map's key
    /// and value, a union's variants, and a dictionary's index and value are
    /// all described the one way.
    ///
    /// Infallible: every value of this type is representable.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::generic::Scalar;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
    /// let value = row.clone().into_value();
    ///
    /// assert_eq!(value.get_key_str("type").and_then(Scalar::as_utf8), Some("struct"));
    /// assert_eq!(DataType::from_value(value)?, row);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn into_value(self) -> Scalar {
        use DataType as D;
        let mut entries: Vec<(Scalar, Scalar)> = Vec::with_capacity(4);
        let mut tag =
            |name: &str| entries.push((key(TYPE_KEY), Scalar::String(SmolStr::new(name))));
        match &self {
            D::Null => tag("null"),
            D::Boolean => tag("boolean"),
            D::Int8 => tag("int8"),
            D::Int16 => tag("int16"),
            D::Int32 => tag("int32"),
            D::Int64 => tag("int64"),
            D::UInt8 => tag("u_int8"),
            D::UInt16 => tag("u_int16"),
            D::UInt32 => tag("u_int32"),
            D::UInt64 => tag("u_int64"),
            D::Float16 => tag("float16"),
            D::Float32 => tag("float32"),
            D::Float64 => tag("float64"),
            D::Date32 => tag("date32"),
            D::Date64 => tag("date64"),
            D::Binary => tag("binary"),
            D::LargeBinary => tag("large_binary"),
            D::BinaryView => tag("binary_view"),
            D::Utf8 => tag("utf8"),
            D::LargeUtf8 => tag("large_utf8"),
            D::Utf8View => tag("utf8_view"),
            D::Timestamp(unit, timezone) => {
                tag("timestamp");
                entries.push((key("unit"), unit_value(*unit)));
                // Omitted rather than null when absent, exactly as the JSON
                // path skips it - a naive timestamp carries no zone at all.
                if let Some(timezone) = timezone {
                    entries.push((
                        key("timezone"),
                        Scalar::String(SmolStr::new(timezone.as_str())),
                    ));
                }
            }
            D::Time32(unit) => {
                tag("time32");
                entries.push((key("unit"), unit_value(*unit)));
            }
            D::Time64(unit) => {
                tag("time64");
                entries.push((key("unit"), unit_value(*unit)));
            }
            D::Duration32(unit) => {
                tag("duration32");
                entries.push((key("unit"), unit_value(*unit)));
            }
            D::Duration64(unit) => {
                tag("duration64");
                entries.push((key("unit"), unit_value(*unit)));
            }
            D::Interval(unit) => {
                tag("interval");
                entries.push((key("unit"), unit_value(*unit)));
            }
            D::FixedSizeBinary(width) => {
                tag("fixed_size_binary");
                entries.push((key("width"), Scalar::I32(*width)));
            }
            D::List(field) => {
                tag("list");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::ListView(field) => {
                tag("list_view");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::FixedSizeList(field, length) => {
                tag("fixed_size_list");
                entries.push((key("field"), field.as_ref().clone().into_value()));
                entries.push((key("length"), Scalar::I32(*length)));
            }
            D::LargeList(field) => {
                tag("large_list");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::LargeListView(field) => {
                tag("large_list_view");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::Struct(fields) => {
                tag("struct");
                entries.push((
                    key("fields"),
                    Scalar::from_sequence(
                        fields
                            .as_fields()
                            .iter()
                            .map(|field| field.clone().into_value()),
                    ),
                ));
            }
            D::Union(fields, mode) => {
                tag("union");
                entries.push((
                    key("mode"),
                    Scalar::String(SmolStr::new(match mode {
                        UnionMode::Sparse => "sparse",
                        UnionMode::Dense => "dense",
                    })),
                ));
                entries.push((
                    key("fields"),
                    Scalar::from_sequence(
                        fields
                            .iter()
                            .map(|(type_id, field)| union_member(type_id, field)),
                    ),
                ));
            }
            D::Dictionary(dictionary) => {
                tag("dictionary");
                entries.push((key("key"), dictionary.key.clone().into_value()));
                entries.push((key("value"), dictionary.value.clone().into_value()));
            }
            D::Decimal32 { precision, scale } => {
                decimal(&mut entries, "decimal32", *precision, *scale)
            }
            D::Decimal64 { precision, scale } => {
                decimal(&mut entries, "decimal64", *precision, *scale)
            }
            D::Decimal128 { precision, scale } => {
                decimal(&mut entries, "decimal128", *precision, *scale);
            }
            D::Decimal256 { precision, scale } => {
                decimal(&mut entries, "decimal256", *precision, *scale);
            }
            D::Map(map) => {
                tag("map");
                entries.push((key("entries"), map.entries.clone().into_value()));
                entries.push((key("keys_sorted"), Scalar::Bool(map.keys_sorted)));
            }
            D::RunEndEncoded(encoded) => {
                tag("run_end_encoded");
                entries.push((key("run_ends"), encoded.run_ends.clone().into_value()));
                entries.push((key("values"), encoded.values.clone().into_value()));
            }
            D::Variant => tag("variant"),
            D::Geometry(geospatial) => {
                tag("geometry");
                entries.push((key("crs"), Scalar::String(SmolStr::new(geospatial.crs()))));
            }
            D::Geography(geospatial) => {
                tag("geography");
                entries.push((key("crs"), Scalar::String(SmolStr::new(geospatial.crs()))));
                entries.push((
                    key("algorithm"),
                    Scalar::String(SmolStr::new(
                        geospatial.algorithm().unwrap_or_default().as_str(),
                    )),
                ));
            }
        }
        // The keys are distinct literals, so the mapping cannot be rejected.
        Scalar::from_mapping(entries).unwrap_or(Scalar::Null)
    }

    /// Read a datatype back from the shared structural [`Scalar`].
    ///
    /// Fallible and validating: a malformed or incomplete mapping is refused
    /// with the same typed error the JSON path raises, and every constructed
    /// datatype goes through the native constructor's own validation rather
    /// than being assembled by hand.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let list = DataType::list(DataType::Utf8.nullable_field("item"));
    /// assert_eq!(DataType::from_value(list.clone().into_value())?, list);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the path and the expectation when the value is
    /// not a datatype mapping, its `type` names nothing this model holds, a
    /// required parameter is missing or wrongly typed, or the resulting
    /// datatype does not validate.
    #[allow(clippy::too_many_lines)]
    pub fn from_value(value: Scalar) -> Result<Self> {
        if value.as_mapping().is_none() && value.as_record().is_none() {
            return Err(invalid(
                "$",
                "a datatype mapping",
                format_smolstr!("{}", value.kind()),
            ));
        }
        let tag = value
            .get_key_str(TYPE_KEY)
            .and_then(Scalar::as_str)
            .ok_or_else(|| invalid("$.type", "a datatype name", "nothing"))?
            .to_owned();
        let at = |name: &str| -> Option<&Scalar> { value.get_key_str(name) };
        let unit = |name: &str| -> Result<TimeUnit> {
            let text = at(name)
                .and_then(Scalar::as_str)
                .ok_or_else(|| invalid(&format!("$.{name}"), "a time unit", "nothing"))?;
            text.parse()
        };
        let width = |name: &str| -> Result<i32> { integer(at(name), name) };
        let child = |name: &str| -> Result<Field> {
            let held = at(name)
                .ok_or_else(|| invalid(&format!("$.{name}"), "a field mapping", "nothing"))?;
            Field::from_value(held.clone())
        };
        let nested = |name: &str| -> Result<Self> {
            let held = at(name)
                .ok_or_else(|| invalid(&format!("$.{name}"), "a datatype mapping", "nothing"))?;
            Self::from_value(held.clone())
        };
        let precision_scale = || -> Result<(u8, i8)> {
            let precision = u8::try_from(integer(at("precision"), "precision")?).map_err(|_| {
                invalid(
                    "$.precision",
                    "a decimal precision",
                    "an out-of-range value",
                )
            })?;
            let scale = i8::try_from(integer(at("scale"), "scale")?)
                .map_err(|_| invalid("$.scale", "a decimal scale", "an out-of-range value"))?;
            Ok((precision, scale))
        };

        let data_type = match tag.as_str() {
            "null" => Self::Null,
            "boolean" => Self::Boolean,
            "int8" => Self::Int8,
            "int16" => Self::Int16,
            "int32" => Self::Int32,
            "int64" => Self::Int64,
            "u_int8" => Self::UInt8,
            "u_int16" => Self::UInt16,
            "u_int32" => Self::UInt32,
            "u_int64" => Self::UInt64,
            "float16" => Self::Float16,
            "float32" => Self::Float32,
            "float64" => Self::Float64,
            "date32" => Self::Date32,
            "date64" => Self::Date64,
            "binary" => Self::Binary,
            "large_binary" => Self::LargeBinary,
            "binary_view" => Self::BinaryView,
            "utf8" => Self::Utf8,
            "large_utf8" => Self::LargeUtf8,
            "utf8_view" => Self::Utf8View,
            "timestamp" => {
                let timezone = match at("timezone").filter(|held| !matches!(held, Scalar::Null)) {
                    Some(held) => {
                        let text = held.as_str().ok_or_else(|| {
                            invalid("$.timezone", "a time zone name", "a non-string value")
                        })?;
                        Some(text.parse()?)
                    }
                    None => None,
                };
                Self::Timestamp(unit("unit")?, timezone)
            }
            "time32" => Self::time32(unit("unit")?)?,
            "time64" => Self::time64(unit("unit")?)?,
            "duration32" => Self::duration32(unit("unit")?)?,
            "duration64" => Self::duration64(unit("unit")?)?,
            "interval" => Self::Interval(unit("unit")?),
            "fixed_size_binary" => Self::fixed_size_binary(width("width")?)?,
            "list" => Self::list(child("field")?),
            "list_view" => Self::list_view(child("field")?),
            "fixed_size_list" => Self::fixed_size_list(child("field")?, width("length")?)?,
            "large_list" => Self::large_list(child("field")?),
            "large_list_view" => Self::large_list_view(child("field")?),
            "struct" => {
                let fields = at("fields")
                    .and_then(Scalar::as_sequence)
                    .ok_or_else(|| invalid("$.fields", "a sequence of fields", "nothing"))?;
                let mut children = Vec::with_capacity(fields.len());
                for held in fields {
                    children.push(Field::from_value(held.clone())?);
                }
                Self::from_fields(children)?
            }
            "union" => {
                let mode = match at("mode").and_then(Scalar::as_str) {
                    Some("sparse") => UnionMode::Sparse,
                    Some("dense") => UnionMode::Dense,
                    other => {
                        return Err(invalid(
                            "$.mode",
                            "\"sparse\" or \"dense\"",
                            format_smolstr!("{other:?}"),
                        ));
                    }
                };
                let members = at("fields")
                    .and_then(Scalar::as_sequence)
                    .ok_or_else(|| invalid("$.fields", "a sequence of union members", "nothing"))?;
                let mut variants = Vec::with_capacity(members.len());
                for held in members {
                    let type_id = i8::try_from(integer(held.get_key_str("type_id"), "type_id")?)
                        .map_err(|_| {
                            invalid(
                                "$.fields[].type_id",
                                "an 8-bit type id",
                                "an out-of-range value",
                            )
                        })?;
                    let field = held
                        .get_key_str("field")
                        .ok_or_else(|| invalid("$.fields[].field", "a field mapping", "nothing"))?;
                    variants.push((type_id, Field::from_value(field.clone())?));
                }
                Self::union(variants, mode)?
            }
            "dictionary" => Self::dictionary(nested("key")?, nested("value")?)?,
            "decimal32" => {
                let (precision, scale) = precision_scale()?;
                Self::decimal32(precision, scale)?
            }
            "decimal64" => {
                let (precision, scale) = precision_scale()?;
                Self::decimal64(precision, scale)?
            }
            "decimal128" => {
                let (precision, scale) = precision_scale()?;
                Self::decimal128(precision, scale)?
            }
            "decimal256" => {
                let (precision, scale) = precision_scale()?;
                Self::decimal256(precision, scale)?
            }
            "map" => {
                let keys_sorted = match at("keys_sorted") {
                    Some(Scalar::Bool(held)) => *held,
                    other => {
                        return Err(invalid(
                            "$.keys_sorted",
                            "a boolean",
                            other.map_or("nothing", Scalar::kind),
                        ));
                    }
                };
                Self::map(child("entries")?, keys_sorted)?
            }
            "run_end_encoded" => Self::run_end_encoded(child("run_ends")?, child("values")?)?,
            "variant" => Self::Variant,
            "geometry" => {
                let crs = at("crs").and_then(Scalar::as_str);
                Self::geometry(crs)?
            }
            "geography" => {
                let crs = at("crs").and_then(Scalar::as_str);
                let algorithm = match at("algorithm").filter(|held| !matches!(held, Scalar::Null)) {
                    Some(held) => {
                        let text = held.as_str().ok_or_else(|| {
                            invalid(
                                "$.algorithm",
                                "an edge algorithm name",
                                "a non-string value",
                            )
                        })?;
                        Some(text.parse()?)
                    }
                    None => None,
                };
                Self::geography(crs, algorithm)?
            }
            other => {
                return Err(invalid(
                    "$.type",
                    "a datatype this model holds",
                    format_smolstr!("{other:?}"),
                ));
            }
        };
        data_type.validate()?;
        Ok(data_type)
    }
}

impl From<&DataType> for Scalar {
    fn from(value: &DataType) -> Self {
        value.clone().into_value()
    }
}

impl TryFrom<Scalar> for DataType {
    type Error = Error;

    fn try_from(value: Scalar) -> Result<Self> {
        Self::from_value(value)
    }
}

/// One union member as the `{type_id, field}` pair the JSON shape uses.
fn union_member(type_id: i8, field: &Field) -> Scalar {
    Scalar::from_mapping([
        (key("type_id"), Scalar::I8(type_id)),
        (key("field"), field.clone().into_value()),
    ])
    .unwrap_or(Scalar::Null)
}

/// Append the decimal tag and its two parameters, in emission order.
fn decimal(entries: &mut Vec<(Scalar, Scalar)>, name: &str, precision: u8, scale: i8) {
    entries.push((key(TYPE_KEY), Scalar::String(SmolStr::new(name))));
    entries.push((key("precision"), Scalar::U8(precision)));
    entries.push((key("scale"), Scalar::I8(scale)));
}

/// A mapping key, which is always a plain string in a schema document.
pub(crate) fn key(name: &str) -> Scalar {
    Scalar::String(SmolStr::new(name))
}

/// A time unit as its snake_case full name, exactly as the JSON path emits it.
///
/// Not [`TimeUnit::as_str`], which answers the canonical *short* spelling
/// (`us`): the serialized vocabulary is the full name, and `from_str` accepts
/// both, so the two stay interchangeable on the read side.
fn unit_value(unit: TimeUnit) -> Scalar {
    Scalar::String(SmolStr::new(match unit {
        TimeUnit::Day => "day",
        TimeUnit::Second => "second",
        TimeUnit::Millisecond => "millisecond",
        TimeUnit::Microsecond => "microsecond",
        TimeUnit::Nanosecond => "nanosecond",
        TimeUnit::YearMonth => "year_month",
        TimeUnit::DayTime => "day_time",
        TimeUnit::MonthDayNano => "month_day_nano",
    }))
}

/// Read an integer parameter, accepting every width the model may carry it in.
pub(crate) fn integer(held: Option<&Scalar>, name: &str) -> Result<i32> {
    let held = held.ok_or_else(|| invalid(&format!("$.{name}"), "an integer", "nothing"))?;
    let value = match held {
        Scalar::I8(value) => i64::from(*value),
        Scalar::I16(value) => i64::from(*value),
        Scalar::I32(value) => i64::from(*value),
        Scalar::I64(value) => *value,
        Scalar::U8(value) => i64::from(*value),
        Scalar::U16(value) => i64::from(*value),
        Scalar::U32(value) => i64::from(*value),
        Scalar::U64(value) => i64::try_from(*value)
            .map_err(|_| invalid(&format!("$.{name}"), "an integer", "an out-of-range value"))?,
        // A structured-text document may carry a wide integer as text; the
        // JSON path already accepts the decimal-string spelling for the same
        // reason, so the two stay interchangeable.
        Scalar::String(text) => text.parse::<i64>().map_err(|_| {
            invalid(
                &format!("$.{name}"),
                "an integer",
                format_smolstr!("{text:?}"),
            )
        })?,
        other => {
            return Err(invalid(
                &format!("$.{name}"),
                "an integer",
                format_smolstr!("{}", other.kind()),
            ));
        }
    };
    i32::try_from(value).map_err(|_| {
        invalid(
            &format!("$.{name}"),
            "a 32-bit integer",
            "an out-of-range value",
        )
    })
}

/// A typed structural failure naming the path, the expectation, and the actual.
pub(crate) fn invalid(
    path: &str,
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(path),
        reason: crate::text::expected_got(expected.to_string(), actual.to_string()),
    }
}

// ---------------------------------------------------------------------------
// The three formats, all over the one `Scalar` conversion.
//
// `into_json` keeps the Serde path because `DataType` is `Serialize`/`Deserialize`
// for the serde ecosystem - it is nested inside other derived structures
// across the tree, and AGENTS.md requires those traits on a native value. The
// two are not a second structural model: the parity test in
// `tests/field/serde.rs` dumps every shape through both routes and compares the
// bytes, so the Serde impl cannot drift from the `Scalar` mapping without
// failing a test. Every *other* format goes through `into_value` alone.
// ---------------------------------------------------------------------------

impl DataType {
    /// Serialize this value as deterministic structural JSON, laid out as asked.
    ///
    /// The companion of [`Self::into_json`]; see
    /// [`json::into_bytes_with_formatting`](crate::json::into_bytes_with_formatting)
    /// for what each [`Indent`](crate::text::Indent) means.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_json_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::json::into_bytes_with_formatting(
            &self.into_value(),
            formatting,
        )?)
    }

    /// Deserialize and validate the same structure as [`Self::from_json`].
    ///
    /// # Errors
    ///
    /// Returns the parser's failure, or the structural refusal naming the path
    /// and the expectation.
    pub fn from_yaml(value: &str) -> Result<Self> {
        Self::from_value(crate::yaml::from_utf8(value)?)
    }

    /// Consume and serialize as YAML.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_yaml(self) -> Result<String> {
        self.into_yaml_with_formatting(crate::text::Formatting::default())
    }

    /// Consume and serialize as YAML, laid out as asked.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_yaml_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::yaml::into_bytes_with_formatting(
            &self.into_value(),
            formatting,
        )?)
    }

    /// Deserialize and validate from structural TOML.
    ///
    /// # Errors
    ///
    /// Returns the parser's failure, or the structural refusal naming the path
    /// and the expectation.
    pub fn from_toml(value: &str) -> Result<Self> {
        Self::from_value(crate::toml::from_utf8(value)?)
    }

    /// Consume and serialize as TOML.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_toml(self) -> Result<String> {
        self.into_toml_with_formatting(crate::text::Formatting::default())
    }

    /// Consume and serialize as TOML, laid out as asked.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_toml_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::toml::into_bytes_with_formatting(
            &self.into_value(),
            formatting,
        )?)
    }
}

/// A dumped document as text, or the encoder's own UTF-8 failure.
///
/// Every writer here emits UTF-8 by construction, so this only ever converts.
fn text_of(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|error| Error::Codec {
        format: "text",
        position: 0,
        reason: smol_str::format_smolstr!("expected UTF-8 output, got {error}"),
    })
}
