//! Stable structural serialization and deserialization.

use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use super::{DataType, TimeUnit, UnionFields, UnionMode};
use crate::types::DecimalType;
use crate::types::UuidType;
use crate::types::enums::EnumType;
use crate::types::sequence::SequenceType;
use crate::{Error, Field, Result, Scalar};

/// Structural JSON and Serde implementations for fields.
mod field {
    use std::fmt;

    use serde::de::{Error as DeError, Visitor};
    use serde::ser::SerializeStruct;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use smol_str::{SmolStr, format_smolstr};

    use crate::Scalar;
    use crate::types::serde::{integer, invalid, key};
    use crate::{DataType, Error, Field, Metadata, Result};

    impl Field {
        /// Deserializes and validates a field from structural JSON.
        pub fn from_json(value: &str) -> Result<Self> {
            serde_json::from_str(value).map_err(Error::from)
        }

        /// Deserializes and validates a field from structural JSON bytes.
        ///
        /// The reading half of [`Self::into_json_bytes`], so a document never has
        /// to become a `String` on its way in either.
        ///
        /// # Errors
        ///
        /// Returns an error when the bytes are not the UTF-8 JSON document a field
        /// serializes to, or when the field does not validate.
        pub fn from_json_bytes(value: &[u8]) -> Result<Self> {
            serde_json::from_slice(value).map_err(Error::from)
        }

        /// Consumes and serializes this value as deterministic structural JSON.
        pub fn into_json(self) -> Result<String> {
            Ok(serde_json::to_string(&self)?)
        }

        /// Consumes and serializes this value as deterministic structural JSON
        /// bytes.
        ///
        /// The same document [`Self::into_json`] renders, encoded rather than
        /// decoded, for a caller writing it straight to a file or a socket without
        /// a round trip through `String`. [`crate::Scalar`] spells the same pair
        /// `into_json_bytes` and `into_json`.
        ///
        /// # Errors
        ///
        /// Returns the error [`Self::into_json`] raises.
        pub fn into_json_bytes(self) -> Result<Vec<u8>> {
            Ok(serde_json::to_vec(&self)?)
        }
    }

    impl Serialize for Field {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let field_count = 4
                + usize::from(self.dictionary_id().is_some_and(|id| id != 0))
                + usize::from(self.dictionary_is_ordered().unwrap_or_default());
            let mut field = serializer.serialize_struct("Field", field_count)?;
            field.serialize_field("name", &self.name())?;
            field.serialize_field("dtype", &self.dtype())?;
            field.serialize_field("nullable", &self.is_nullable())?;
            if let Some(id) = self.dictionary_id().filter(|id| *id != 0) {
                field.serialize_field("dictionary_id", &DictionaryIdJson(id))?;
            }
            if self.dictionary_is_ordered().unwrap_or_default() {
                field.serialize_field("dictionary_is_ordered", &true)?;
            }
            field.serialize_field("metadata", &self.as_metadata())?;
            field.end()
        }
    }

    struct DictionaryIdJson(i64);

    impl Serialize for DictionaryIdJson {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.collect_str(&self.0)
        }
    }

    fn deserialize_dictionary_id<'de, D>(deserializer: D) -> std::result::Result<i64, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DictionaryIdVisitor;

        impl Visitor<'_> for DictionaryIdVisitor {
            type Value = i64;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a signed 64-bit integer or its decimal string")
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
                Ok(value)
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
            where
                E: DeError,
            {
                i64::try_from(value).map_err(|_| E::custom("dictionary id exceeds i64::MAX"))
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: DeError,
            {
                value
                    .parse()
                    .map_err(|_| E::custom("invalid signed 64-bit dictionary id"))
            }
        }

        deserializer.deserialize_any(DictionaryIdVisitor)
    }

    impl<'de> Deserialize<'de> for Field {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct FieldWire {
                name: SmolStr,
                dtype: DataType,
                nullable: bool,
                #[serde(default, deserialize_with = "deserialize_dictionary_id")]
                dictionary_id: i64,
                #[serde(default)]
                dictionary_is_ordered: bool,
                #[serde(default)]
                metadata: Metadata,
            }

            let value = FieldWire::deserialize(deserializer)?;
            let mut field =
                Self::new_with_metadata(value.name, value.dtype, value.nullable, value.metadata);
            // A stated sidecar is a claim about the datatype, so it is
            // checked: only a dictionary field can carry one.
            if value.dictionary_id != 0 || value.dictionary_is_ordered {
                field
                    .set_dictionary_options(value.dictionary_id, value.dictionary_is_ordered)
                    .map_err(D::Error::custom)?;
            }
            field.validate().map_err(D::Error::custom)?;
            Ok(field)
        }
    }

    // ---------------------------------------------------------------------------
    // The one structural mapping between a `Field` and the shared `Scalar`.
    //
    // A companion to the datatype conversion: together they are the *only*
    // structural model of a schema in the tree. JSON, YAML, and TOML are three
    // writers over it, so the three agree by construction rather than by three
    // sets of tests, and a schema becomes embeddable in any structured document
    // the crate already reads - a config file can carry a declared schema inline
    // beside the rest of its settings, with no JSON-string-inside-YAML awkwardness.
    // ---------------------------------------------------------------------------

    impl Field {
        /// Project this field onto the shared structural [`Scalar`].
        ///
        /// The mapping carries `name`, `dtype`, `nullable`, then
        /// `dictionary_id` only when it is non-zero and `dictionary_is_ordered`
        /// only when it is set - an unset optional attribute is *omitted*, never
        /// emitted as null, so the serialized form and the readable form agree
        /// about what is noise - and finally `metadata`. Key order is fixed, so
        /// two equal fields produce byte-identical output in every format.
        ///
        /// The dictionary id crosses as its decimal *string*, because a 64-bit
        /// identifier does not survive every JSON reader as a number.
        ///
        /// ```
        /// use yggdryl::{DataType, Field};
        ///
        /// # fn main() -> yggdryl::Result<()> {
        /// let field = DataType::Int64.required_field("id");
        /// let value = field.clone().into_value();
        ///
        /// // Unset optional attributes are absent rather than null.
        /// assert!(value.get_key_str("dictionary_id").is_none());
        /// assert_eq!(Field::from_value(value)?, field);
        /// # Ok(())
        /// # }
        /// ```
        #[must_use]
        pub fn into_value(self) -> Scalar {
            let mut entries: Vec<(Scalar, Scalar)> = Vec::with_capacity(6);
            entries.push((key("name"), Scalar::from(self.name())));
            entries.push((key("dtype"), self.dtype().clone().into_value()));
            entries.push((key("nullable"), Scalar::from(self.is_nullable())));
            if let Some(id) = self.dictionary_id().filter(|id| *id != 0) {
                // Decimal text, as the JSON path emits it: a 64-bit identifier
                // does not survive every reader as a number.
                entries.push((key("dictionary_id"), Scalar::from(format_smolstr!("{id}"))));
            }
            if self.dictionary_is_ordered().unwrap_or_default() {
                entries.push((key("dictionary_is_ordered"), Scalar::from(true)));
            }
            entries.push((
                key("metadata"),
                Scalar::from_mapping(
                    self.metadata_iter()
                        .map(|(name, value)| (key(name), Scalar::from(SmolStr::new(value)))),
                )
                .unwrap_or(Scalar::Null),
            ));
            Scalar::from_mapping(entries).unwrap_or(Scalar::Null)
        }

        /// Read a field back from the shared structural [`Scalar`].
        ///
        /// Fallible and validating, raising the same typed errors the JSON path
        /// raises: the datatype is rebuilt through [`DataType::from_value`], the
        /// metadata through [`Metadata`]'s own validation, and the assembled field
        /// through its own.
        ///
        /// ```
        /// use yggdryl::{DataType, Field};
        ///
        /// # fn main() -> yggdryl::Result<()> {
        /// let nested = DataType::from_fields([
        ///     DataType::Int64.required_field("id"),
        ///     DataType::utf8().nullable_field("venue"),
        /// ])?
        /// .required_field("row");
        ///
        /// assert_eq!(Field::from_value(nested.clone().into_value())?, nested);
        /// # Ok(())
        /// # }
        /// ```
        ///
        /// # Errors
        ///
        /// Returns an error naming the path and the expectation when the value is
        /// not a field mapping, a required key is missing or wrongly typed, or the
        /// assembled field does not validate.
        pub fn from_value(value: Scalar) -> Result<Self> {
            if value.as_mapping().is_none() && value.as_record().is_none() {
                return Err(invalid("$", "a field mapping", value.kind()));
            }
            let at = |name: &str| value.get_key_str(name);
            let name = at("name")
                .and_then(Scalar::as_str)
                .ok_or_else(|| invalid("$.name", "a field name", "nothing"))?;
            let dtype = DataType::from_value(
                at("dtype")
                    .ok_or_else(|| invalid("$.dtype()", "a datatype mapping", "nothing"))?
                    .clone(),
            )?;
            let nullable = match at("nullable") {
                Some(held) if held.as_bool().is_some() => held.as_bool().unwrap_or(false),
                other => {
                    return Err(invalid(
                        "$.is_nullable()",
                        "a boolean",
                        other.map_or("nothing", Scalar::kind),
                    ));
                }
            };

            let mut field = Self::new(name, dtype, nullable);
            // Only a dictionary carries the pair, and both settle together so a
            // half-declared state can never reach the field.
            let dictionary_id =
                match at("dictionary_id").filter(|held| !matches!(held, Scalar::Null)) {
                    Some(held) => i64::from(integer(Some(held), "dictionary_id")?),
                    None => 0,
                };
            let dictionary_is_ordered =
                at("dictionary_is_ordered").and_then(Scalar::as_bool) == Some(true);
            if dictionary_id != 0 || dictionary_is_ordered {
                field.set_dictionary_options(dictionary_id, dictionary_is_ordered)?;
            }

            if let Some(held) = at("metadata").filter(|held| !matches!(held, Scalar::Null)) {
                let mut collected = Vec::with_capacity(held.len());
                if let Some(pairs) = held.as_record() {
                    for (name, value) in pairs {
                        let value = value.as_str().ok_or_else(|| {
                            invalid(
                                &format!("$.metadata[{name:?}]"),
                                "a string metadata value",
                                value.kind(),
                            )
                        })?;
                        collected.push((name.clone(), SmolStr::new(value)));
                    }
                } else if let Some(pairs) = held.as_mapping() {
                    for (name, value) in pairs {
                        let name = name.as_str().ok_or_else(|| {
                            invalid("$.metadata", "string metadata keys", name.kind())
                        })?;
                        let value = value.as_str().ok_or_else(|| {
                            invalid(
                                &format!("$.metadata[{name:?}]"),
                                "a string metadata value",
                                value.kind(),
                            )
                        })?;
                        collected.push((SmolStr::new(name), SmolStr::new(value)));
                    }
                } else {
                    return Err(invalid(
                        "$.metadata",
                        "a mapping of string entries",
                        held.kind(),
                    ));
                }
                field.set_metadata(Metadata::from_entries(collected)?)?;
            }

            field.validate()?;
            Ok(field)
        }
    }

    impl From<&Field> for Scalar {
        fn from(value: &Field) -> Self {
            value.clone().into_value()
        }
    }

    impl TryFrom<Scalar> for Field {
        type Error = Error;

        fn try_from(value: Scalar) -> Result<Self> {
            Self::from_value(value)
        }
    }

    // ---------------------------------------------------------------------------
    // The three formats, all over the one `Scalar` conversion.
    //
    // `into_json` keeps the Serde path because `Field` is `Serialize`/`Deserialize`
    // for the serde ecosystem - it is nested inside other derived structures
    // across the tree, and AGENTS.md requires those traits on a native value. The
    // two are not a second structural model: the parity test in
    // `tests/field/serde.rs` dumps every shape through both routes and compares the
    // bytes, so the Serde impl cannot drift from the `Scalar` mapping without
    // failing a test. Every *other* format goes through `into_value` alone.
    // ---------------------------------------------------------------------------

    impl Field {
        /// Serialize this value as deterministic structural JSON, laid out as asked.
        ///
        /// The companion of [`Self::into_json`]; see
        /// [`json::into_bytes_with_formatting`](crate::text::json::into_bytes_with_formatting)
        /// for what each [`Indent`](crate::text::Indent) means.
        ///
        /// # Errors
        ///
        /// Returns the encoder's failure.
        pub fn into_json_with_formatting(
            self,
            formatting: crate::text::Formatting,
        ) -> Result<String> {
            text_of(crate::text::json::into_bytes_with_formatting(
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
            Self::from_value(crate::text::yaml::from_utf8(value)?)
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
        pub fn into_yaml_with_formatting(
            self,
            formatting: crate::text::Formatting,
        ) -> Result<String> {
            text_of(crate::text::yaml::into_bytes_with_formatting(
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
            Self::from_value(crate::text::toml::from_utf8(value)?)
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
        pub fn into_toml_with_formatting(
            self,
            formatting: crate::text::Formatting,
        ) -> Result<String> {
            text_of(crate::text::toml::into_bytes_with_formatting(
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
}

impl DataType {
    /// Deserializes the stable tagged-object representation from JSON.
    ///
    /// Nested fields and parameters are reconstructed through the same
    /// validation used by native constructors. Canonical datatype strings are
    /// intentionally handled by [`Self::from_str`], not this method.
    pub fn from_json(input: &str) -> Result<Self> {
        serde_json::from_str(input).map_err(Error::from)
    }

    /// Deserializes and validates a datatype from structural JSON bytes.
    ///
    /// The reading half of [`Self::into_json_bytes`], so a document never has
    /// to become a `String` on its way in either.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not the UTF-8 JSON document a
    /// datatype serializes to, or when the datatype does not validate.
    pub fn from_json_bytes(value: &[u8]) -> Result<Self> {
        serde_json::from_slice(value).map_err(Error::from)
    }

    /// Consumes this value and serializes it to deterministic structural JSON.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// Consumes this value and serializes it to deterministic structural JSON
    /// bytes.
    ///
    /// The same document [`Self::into_json`] renders, encoded rather than
    /// decoded, for a caller writing it straight to a file or a socket without
    /// a round trip through `String`. [`crate::Scalar`] spells the same pair
    /// `into_json_bytes` and `into_json`.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::into_json`] raises.
    pub fn into_json_bytes(self) -> Result<Vec<u8>> {
        serde_json::to_vec(&self).map_err(Error::from)
    }
}

const fn naive_timezone() -> crate::Timezone {
    crate::Timezone::NAIVE
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
    // `rename_all = "snake_case"` turns `UInt8` into `u_int8`, which no other
    // door in the crate spells - `DataTypeId::as_str`, the text parser,
    // `Display` and both bindings all say `uint8`. Named explicitly here, the
    // way thirteen other variants in these enums already are. The alias
    // keeps documents written under the old spelling readable.
    #[serde(rename = "uint8", alias = "u_int8")]
    UInt8 {},
    #[serde(rename = "uint16", alias = "u_int16")]
    UInt16 {},
    #[serde(rename = "uint32", alias = "u_int32")]
    UInt32 {},
    #[serde(rename = "uint64", alias = "u_int64")]
    UInt64 {},
    Float16 {},
    Float32 {},
    Float64 {},
    #[serde(rename = "datetime64")]
    DateTime64 {
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
    // The `binary` layout is the default, so a plain `binary` is the bare
    // tag and only what a byte datatype declares is written.
    Binary {
        #[serde(skip_serializing_if = "Option::is_none")]
        layout: Option<crate::types::BytesType>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fixed: Option<u32>,
    },
    // The `utf8` leaf is the default, so a plain `utf8` is the bare tag and
    // only what a string declares is written; the leaf names its charset, so
    // no `charset` key is written beside it.
    String {
        #[serde(skip_serializing_if = "Option::is_none")]
        layout: Option<crate::types::StringType>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fixed: Option<u32>,
    },
    Country {},
    Currency {},
    Mic {},
    Cfi {},
    Isin {},
    Cusip {},
    Sedol {},
    Bloomberg {},
    Side {},
    State {},
    // One word on the wire, so it does not take the snake_case the rest
    // of this enum derives.
    #[serde(rename = "timeinforce")]
    TimeInForce {},
    Uuid {},
    Uuidv4 {},
    Uuidv7 {},
    Uuidv8 {},
    Version {},
    Url {},
    Timezone {},
    // One word on the wire, as `timeinforce` is.
    #[serde(rename = "mimetype")]
    MimeType {},
    #[serde(rename = "mediatype")]
    MediaType {},
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
        algorithm: crate::EdgeAlgorithm,
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
            D::DateTime64 { unit, timezone } => Self::DateTime64 {
                unit: *unit,
                timezone: (!timezone.is_naive()).then(|| timezone.as_str()),
            },
            D::Date32 => Self::Date32 {},
            D::Date64 => Self::Date64 {},
            D::Time32(unit) => Self::Time32 { unit: *unit },
            D::Time64(unit) => Self::Time64 { unit: *unit },
            D::Duration32(unit) => Self::Duration32 { unit: *unit },
            D::Duration64(unit) => Self::Duration64 { unit: *unit },
            D::Interval(unit) => Self::Interval { unit: *unit },
            D::Bytes(parameters) => {
                let parameters = *parameters;
                Self::Binary {
                    layout: Some(parameters)
                        .filter(|layout| *layout != crate::types::BytesType::Binary),
                    max: parameters.max(),
                    fixed: parameters.fixed(),
                }
            }
            D::String(parameters) => {
                let parameters = *parameters;
                Self::String {
                    layout: Some(parameters)
                        .filter(|layout| *layout != crate::types::StringType::Utf8String),
                    max: parameters.max(),
                    fixed: parameters.fixed(),
                }
            }
            D::Country => Self::Country {},
            D::Currency => Self::Currency {},
            D::Mic => Self::Mic {},
            D::Cfi => Self::Cfi {},
            D::Isin => Self::Isin {},
            D::Cusip => Self::Cusip {},
            D::Sedol => Self::Sedol {},
            D::Bloomberg => Self::Bloomberg {},
            D::Side => Self::Side {},
            D::State => Self::State {},
            D::TimeInForce => Self::TimeInForce {},
            D::Uuid(UuidType::Uuid) => Self::Uuid {},
            D::Uuid(UuidType::Uuidv4) => Self::Uuidv4 {},
            D::Uuid(UuidType::Uuidv7) => Self::Uuidv7 {},
            D::Uuid(UuidType::Uuidv8) => Self::Uuidv8 {},
            D::Version => Self::Version {},
            D::Url => Self::Url {},
            D::Timezone => Self::Timezone {},
            D::MimeType => Self::MimeType {},
            D::MediaType => Self::MediaType {},
            D::Sequence(SequenceType::List(field)) => Self::List { field },
            D::Sequence(SequenceType::ListView(field)) => Self::ListView { field },
            D::Sequence(SequenceType::FixedSizeList(field, length)) => Self::FixedSizeList {
                field,
                length: *length,
            },
            D::Sequence(SequenceType::LargeList(field)) => Self::LargeList { field },
            D::Sequence(SequenceType::LargeListView(field)) => Self::LargeListView { field },
            D::Structure(fields) => Self::Struct {
                fields: fields.as_fields(),
            },
            D::Union(fields, mode) => Self::Union {
                mode: *mode,
                fields: UnionFieldsRef(fields),
            },
            D::Enum(EnumType::Dictionary(dictionary)) => Self::Dictionary {
                key: &dictionary.key,
                value: &dictionary.value,
            },
            D::Decimal(DecimalType::Decimal32 { precision, scale }) => Self::Decimal32 {
                precision: *precision,
                scale: *scale,
            },
            D::Decimal(DecimalType::Decimal64 { precision, scale }) => Self::Decimal64 {
                precision: *precision,
                scale: *scale,
            },
            D::Decimal(DecimalType::Decimal128 { precision, scale }) => Self::Decimal128 {
                precision: *precision,
                scale: *scale,
            },
            D::Decimal(DecimalType::Decimal256 { precision, scale }) => Self::Decimal256 {
                precision: *precision,
                scale: *scale,
            },
            D::Mapping(map) => Self::Map {
                entries: map.entries(),
                keys_sorted: map.keys_sorted(),
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
enum DataTypeWire {
    Null {},
    Boolean {},
    Int8 {},
    Int16 {},
    Int32 {},
    Int64 {},
    // `rename_all = "snake_case"` turns `UInt8` into `u_int8`, which no other
    // door in the crate spells - `DataTypeId::as_str`, the text parser,
    // `Display` and both bindings all say `uint8`. Named explicitly here, the
    // way thirteen other variants in these enums already are. The alias
    // keeps documents written under the old spelling readable.
    #[serde(rename = "uint8", alias = "u_int8")]
    UInt8 {},
    #[serde(rename = "uint16", alias = "u_int16")]
    UInt16 {},
    #[serde(rename = "uint32", alias = "u_int32")]
    UInt32 {},
    #[serde(rename = "uint64", alias = "u_int64")]
    UInt64 {},
    Float16 {},
    Float32 {},
    Float64 {},
    #[serde(rename = "datetime64")]
    DateTime64 {
        unit: TimeUnit,
        #[serde(default = "naive_timezone")]
        timezone: crate::Timezone,
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
    Binary {
        #[serde(default)]
        layout: crate::types::BytesType,
        #[serde(default)]
        max: Option<u32>,
        #[serde(default)]
        fixed: Option<u32>,
    },
    String {
        #[serde(default)]
        layout: crate::types::StringType,
        #[serde(default)]
        charset: Option<crate::Charset>,
        #[serde(default)]
        max: Option<u32>,
        #[serde(default)]
        fixed: Option<u32>,
    },
    Country {},
    Currency {},
    Mic {},
    Cfi {},
    Isin {},
    Cusip {},
    Sedol {},
    Bloomberg {},
    Side {},
    #[serde(rename = "state")]
    State {},
    #[serde(rename = "timeinforce")]
    TimeInForce {},
    Uuid {},
    Uuidv4 {},
    Uuidv7 {},
    Uuidv8 {},
    Version {},
    Url {},
    Timezone {},
    // One word on the wire, as `timeinforce` is.
    #[serde(rename = "mimetype")]
    MimeType {},
    #[serde(rename = "mediatype")]
    MediaType {},
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
        algorithm: Option<crate::EdgeAlgorithm>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnionMemberValue {
    type_id: i8,
    field: Field,
}

impl TryFrom<DataTypeWire> for DataType {
    type Error = Error;

    #[allow(clippy::too_many_lines)]
    fn try_from(value: DataTypeWire) -> Result<Self> {
        Ok(match value {
            DataTypeWire::Null {} => Self::Null,
            DataTypeWire::Boolean {} => Self::Boolean,
            DataTypeWire::Int8 {} => Self::Int8,
            DataTypeWire::Int16 {} => Self::Int16,
            DataTypeWire::Int32 {} => Self::Int32,
            DataTypeWire::Int64 {} => Self::Int64,
            DataTypeWire::UInt8 {} => Self::UInt8,
            DataTypeWire::UInt16 {} => Self::UInt16,
            DataTypeWire::UInt32 {} => Self::UInt32,
            DataTypeWire::UInt64 {} => Self::UInt64,
            DataTypeWire::Float16 {} => Self::Float16,
            DataTypeWire::Float32 {} => Self::Float32,
            DataTypeWire::Float64 {} => Self::Float64,
            DataTypeWire::DateTime64 { unit, timezone } => Self::datetime64(unit, timezone)?,
            DataTypeWire::Date32 {} => Self::Date32,
            DataTypeWire::Date64 {} => Self::Date64,
            DataTypeWire::Time32 { unit } => Self::time32(unit)?,
            DataTypeWire::Time64 { unit } => Self::time64(unit)?,
            DataTypeWire::Duration32 { unit } => Self::duration32(unit)?,
            DataTypeWire::Duration64 { unit } => Self::duration64(unit)?,
            DataTypeWire::Interval { unit } => Self::Interval(unit),
            DataTypeWire::Binary { layout, max, fixed } => {
                Self::bytes(bytes_parameters(layout, max, fixed)?)?
            }
            DataTypeWire::String {
                layout,
                charset,
                max,
                fixed,
            } => Self::string(string_parameters(layout, charset, max, fixed)?)?,
            DataTypeWire::Country {} => Self::Country,
            DataTypeWire::Currency {} => Self::Currency,
            DataTypeWire::Mic {} => Self::Mic,
            DataTypeWire::Cfi {} => Self::Cfi,
            DataTypeWire::Isin {} => Self::Isin,
            DataTypeWire::Cusip {} => Self::Cusip,
            DataTypeWire::Sedol {} => Self::Sedol,
            DataTypeWire::Bloomberg {} => Self::Bloomberg,
            DataTypeWire::Side {} => Self::Side,
            DataTypeWire::State {} => Self::State,
            DataTypeWire::TimeInForce {} => Self::TimeInForce,
            DataTypeWire::Uuid {} => Self::Uuid(UuidType::Uuid),
            DataTypeWire::Uuidv4 {} => Self::Uuid(UuidType::Uuidv4),
            DataTypeWire::Uuidv7 {} => Self::Uuid(UuidType::Uuidv7),
            DataTypeWire::Uuidv8 {} => Self::Uuid(UuidType::Uuidv8),
            DataTypeWire::Version {} => Self::Version,
            DataTypeWire::Url {} => Self::Url,
            DataTypeWire::Timezone {} => Self::Timezone,
            DataTypeWire::MimeType {} => Self::MimeType,
            DataTypeWire::MediaType {} => Self::MediaType,
            DataTypeWire::List { field } => Self::list(field),
            DataTypeWire::ListView { field } => Self::list_view(field),
            DataTypeWire::FixedSizeList { field, length } => Self::fixed_size_list(field, length)?,
            DataTypeWire::LargeList { field } => Self::large_list(field),
            DataTypeWire::LargeListView { field } => Self::large_list_view(field),
            DataTypeWire::Struct { fields } => Self::from_fields(fields)?,
            DataTypeWire::Union { mode, fields } => Self::union(
                fields
                    .into_iter()
                    .map(|member| (member.type_id, member.field)),
                mode,
            )?,
            DataTypeWire::Dictionary { key, value } => Self::dictionary(*key, *value)?,
            DataTypeWire::Decimal32 { precision, scale } => Self::decimal32(precision, scale)?,
            DataTypeWire::Decimal64 { precision, scale } => Self::decimal64(precision, scale)?,
            DataTypeWire::Decimal128 { precision, scale } => Self::decimal128(precision, scale)?,
            DataTypeWire::Decimal256 { precision, scale } => Self::decimal256(precision, scale)?,
            DataTypeWire::Map {
                entries,
                keys_sorted,
            } => Self::map(entries, keys_sorted)?,
            DataTypeWire::RunEndEncoded { run_ends, values } => {
                Self::run_end_encoded(run_ends, values)?
            }
            DataTypeWire::Variant {} => Self::Variant,
            DataTypeWire::Geometry { crs } => Self::geometry(Some(&crs))?,
            DataTypeWire::Geography { crs, algorithm } => Self::geography(Some(&crs), algorithm)?,
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
        let value = Self::try_from(DataTypeWire::deserialize(deserializer)?)
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
    /// use yggdryl::Scalar;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
    /// let value = row.clone().into_value();
    ///
    /// assert_eq!(value.get_key_str("type").and_then(Scalar::as_str), Some("struct"));
    /// assert_eq!(DataType::from_value(value)?, row);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn into_value(self) -> Scalar {
        use DataType as D;
        let mut entries: Vec<(Scalar, Scalar)> = Vec::with_capacity(4);
        let mut tag = |name: &str| entries.push((key(TYPE_KEY), Scalar::from(SmolStr::new(name))));
        match &self {
            D::Null => tag("null"),
            D::Boolean => tag("boolean"),
            D::Int8 => tag("int8"),
            D::Int16 => tag("int16"),
            D::Int32 => tag("int32"),
            D::Int64 => tag("int64"),
            D::UInt8 => tag("uint8"),
            D::UInt16 => tag("uint16"),
            D::UInt32 => tag("uint32"),
            D::UInt64 => tag("uint64"),
            D::Float16 => tag("float16"),
            D::Float32 => tag("float32"),
            D::Float64 => tag("float64"),
            D::Date32 => tag("date32"),
            D::Date64 => tag("date64"),
            D::Country => tag("country"),
            D::Currency => tag("currency"),
            D::Mic => tag("mic"),
            D::Cfi => tag("cfi"),
            D::Isin => tag("isin"),
            D::Cusip => tag("cusip"),
            D::Sedol => tag("sedol"),
            D::Bloomberg => tag("bloomberg"),
            D::Side => tag("side"),
            D::State => tag("state"),
            D::TimeInForce => tag("timeinforce"),
            D::Uuid(family) => tag(family.id().as_str()),
            D::Version => tag("version"),
            D::Url => tag("url"),
            D::Timezone => tag("timezone"),
            D::MimeType => tag("mimetype"),
            D::MediaType => tag("mediatype"),
            D::DateTime64 { unit, timezone } => {
                tag("datetime64");
                entries.push((key("unit"), unit_value(*unit)));
                // Omitted for the explicit NAIVE marker, exactly as the JSON
                // path skips it for an Arrow-compatible wall-clock type.
                if !timezone.is_naive() {
                    entries.push((
                        key("timezone"),
                        Scalar::from(SmolStr::new(timezone.as_str())),
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
            D::Bytes(parameters) => {
                let parameters = *parameters;
                tag("binary");
                if parameters != crate::types::BytesType::Binary {
                    entries.push((
                        key("layout"),
                        Scalar::from(SmolStr::new_static(parameters.as_str())),
                    ));
                }
                if let Some(max) = parameters.max() {
                    entries.push((key("max"), Scalar::from(max)));
                }
                if let Some(fixed) = parameters.fixed() {
                    entries.push((key("fixed"), Scalar::from(fixed)));
                }
            }
            D::String(parameters) => {
                let parameters = *parameters;
                tag("string");
                if parameters != crate::types::StringType::Utf8String {
                    entries.push((
                        key("layout"),
                        Scalar::from(SmolStr::new_static(parameters.as_str())),
                    ));
                }
                if let Some(max) = parameters.max() {
                    entries.push((key("max"), Scalar::from(max)));
                }
                if let Some(fixed) = parameters.fixed() {
                    entries.push((key("fixed"), Scalar::from(fixed)));
                }
            }
            D::Sequence(SequenceType::List(field)) => {
                tag("list");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::Sequence(SequenceType::ListView(field)) => {
                tag("list_view");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::Sequence(SequenceType::FixedSizeList(field, length)) => {
                tag("fixed_size_list");
                entries.push((key("field"), field.as_ref().clone().into_value()));
                entries.push((key("length"), Scalar::from(*length)));
            }
            D::Sequence(SequenceType::LargeList(field)) => {
                tag("large_list");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::Sequence(SequenceType::LargeListView(field)) => {
                tag("large_list_view");
                entries.push((key("field"), field.as_ref().clone().into_value()));
            }
            D::Structure(fields) => {
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
                    Scalar::from(SmolStr::new(match mode {
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
            D::Enum(EnumType::Dictionary(dictionary)) => {
                tag("dictionary");
                entries.push((key("key"), dictionary.key.clone().into_value()));
                entries.push((key("value"), dictionary.value.clone().into_value()));
            }
            D::Decimal(DecimalType::Decimal32 { precision, scale }) => {
                decimal(&mut entries, "decimal32", *precision, *scale)
            }
            D::Decimal(DecimalType::Decimal64 { precision, scale }) => {
                decimal(&mut entries, "decimal64", *precision, *scale)
            }
            D::Decimal(DecimalType::Decimal128 { precision, scale }) => {
                decimal(&mut entries, "decimal128", *precision, *scale);
            }
            D::Decimal(DecimalType::Decimal256 { precision, scale }) => {
                decimal(&mut entries, "decimal256", *precision, *scale);
            }
            D::Mapping(map) => {
                tag("map");
                entries.push((key("entries"), map.entries().clone().into_value()));
                entries.push((key("keys_sorted"), Scalar::from(map.keys_sorted())));
            }
            D::RunEndEncoded(encoded) => {
                tag("run_end_encoded");
                entries.push((key("run_ends"), encoded.run_ends.clone().into_value()));
                entries.push((key("values"), encoded.values.clone().into_value()));
            }
            D::Variant => tag("variant"),
            D::Geometry(geospatial) => {
                tag("geometry");
                entries.push((key("crs"), Scalar::from(SmolStr::new(geospatial.crs()))));
            }
            D::Geography(geospatial) => {
                tag("geography");
                entries.push((key("crs"), Scalar::from(SmolStr::new(geospatial.crs()))));
                entries.push((
                    key("algorithm"),
                    Scalar::from(SmolStr::new(
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
    /// let list = DataType::list(DataType::utf8().nullable_field("item"));
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
        let bound = |name: &str| -> Result<u32> {
            u32::try_from(integer(at(name), name)?).map_err(|_| {
                invalid(
                    &format!("$.{name}"),
                    "a byte bound",
                    "an out-of-range value",
                )
            })
        };
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

        let dtype = match tag.as_str() {
            "null" => Self::Null,
            "boolean" => Self::Boolean,
            "int8" => Self::Int8,
            "int16" => Self::Int16,
            "int32" => Self::Int32,
            "int64" => Self::Int64,
            // The written spelling, and the one `rename_all = "snake_case"`
            // used to produce. Documents carrying the old one still read.
            "uint8" | "u_int8" => Self::UInt8,
            "uint16" | "u_int16" => Self::UInt16,
            "uint32" | "u_int32" => Self::UInt32,
            "uint64" | "u_int64" => Self::UInt64,
            "float16" => Self::Float16,
            "float32" => Self::Float32,
            "float64" => Self::Float64,
            "date32" => Self::Date32,
            "date64" => Self::Date64,
            "country" => Self::Country,
            "currency" => Self::Currency,
            "mic" => Self::Mic,
            "cfi" => Self::Cfi,
            "isin" => Self::Isin,
            "cusip" => Self::Cusip,
            "sedol" => Self::Sedol,
            "bloomberg" => Self::Bloomberg,
            "side" => Self::Side,
            "state" => Self::State,
            "timeinforce" => Self::TimeInForce,
            "uuid" => Self::Uuid(UuidType::Uuid),
            "uuidv4" => Self::Uuid(UuidType::Uuidv4),
            "uuidv7" => Self::Uuid(UuidType::Uuidv7),
            "uuidv8" => Self::Uuid(UuidType::Uuidv8),
            "version" => Self::Version,
            "url" => Self::Url,
            "timezone" => Self::Timezone,
            "mimetype" => Self::MimeType,
            "mediatype" => Self::MediaType,
            "datetime64" => {
                let timezone = match at("timezone").filter(|held| !matches!(held, Scalar::Null)) {
                    Some(held) => {
                        let text = held.as_str().ok_or_else(|| {
                            invalid("$.timezone", "a time zone name", "a non-string value")
                        })?;
                        text.parse()?
                    }
                    None => crate::Timezone::NAIVE,
                };
                Self::datetime64(unit("unit")?, timezone)?
            }
            "time32" => Self::time32(unit("unit")?)?,
            "time64" => Self::time64(unit("unit")?)?,
            "duration32" => Self::duration32(unit("unit")?)?,
            "duration64" => Self::duration64(unit("unit")?)?,
            "interval" => Self::Interval(unit("unit")?),
            "binary" => {
                let layout = match at("layout") {
                    None => crate::types::BytesType::Binary,
                    Some(held) => {
                        let name = held.as_str().ok_or_else(|| {
                            invalid("$.layout", "a name", format_smolstr!("{}", held.kind()))
                        })?;
                        crate::types::BytesType::from_str(name)?
                    }
                };
                let max = at("max").map(|_| bound("max")).transpose()?;
                let fixed = at("fixed").map(|_| bound("fixed")).transpose()?;
                Self::bytes(bytes_parameters(layout, max, fixed)?)?
            }
            "string" => {
                let name = |field: &str| -> Result<Option<SmolStr>> {
                    match at(field) {
                        None => Ok(None),
                        Some(held) => held.as_str().map(SmolStr::new).map(Some).ok_or_else(|| {
                            invalid(
                                &format!("$.{field}"),
                                "a name",
                                format_smolstr!("{}", held.kind()),
                            )
                        }),
                    }
                };
                let layout = match name("layout")? {
                    Some(layout) => crate::types::StringType::from_str(&layout)?,
                    None => crate::types::StringType::Utf8String,
                };
                let charset = match name("charset")? {
                    Some(charset) => Some(crate::Charset::from_str(&charset)?),
                    None => None,
                };
                let max = at("max").map(|_| bound("max")).transpose()?;
                let fixed = at("fixed").map(|_| bound("fixed")).transpose()?;
                Self::string(string_parameters(layout, charset, max, fixed)?)?
            }
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
                    Some(held) if held.as_bool().is_some() => held.as_bool().unwrap_or(false),
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
        dtype.validate()?;
        Ok(dtype)
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
        (key("type_id"), Scalar::from(type_id)),
        (key("field"), field.clone().into_value()),
    ])
    .unwrap_or(Scalar::Null)
}

/// Append the decimal tag and its two parameters, in emission order.
fn decimal(entries: &mut Vec<(Scalar, Scalar)>, name: &str, precision: u8, scale: i8) {
    entries.push((key(TYPE_KEY), Scalar::from(SmolStr::new(name))));
    entries.push((key("precision"), Scalar::from(precision)));
    entries.push((key("scale"), Scalar::from(scale)));
}

/// The parameters a `string` document declares.
///
/// The bound is written under the key its layout gives it - `fixed` on the
/// fixed layout, `max` on every other - and a bound under the other key is
/// refused rather than read as the one the layout has. A `charset` key
/// restates the leaf in that charset's family, so an older document that
/// spelled `{"layout":"large_string","charset":"windows-1252"}` still names
/// `large_cp1252`.
fn string_parameters(
    layout: crate::types::StringType,
    charset: Option<crate::Charset>,
    max: Option<u32>,
    fixed: Option<u32>,
) -> Result<crate::types::StringType> {
    use crate::types::StringType;

    let named = match charset {
        Some(charset) => layout.with_charset(charset)?,
        None => layout,
    };
    let leaf = match (named.is_fixed(), fixed, max) {
        (true, Some(width), None) => named.with_bound(width)?,
        (true, None, None) => {
            return Err(invalid("$.fixed", "a width on the fixed layout", 0));
        }
        (true, _, Some(max)) => {
            return Err(invalid("$.max", "a fixed width on the fixed layout", max));
        }
        (false, Some(fixed), _) => {
            return Err(invalid("$.fixed", "a maximum on a variable layout", fixed));
        }
        // A maximum makes the column sized whichever unbounded shape named
        // it, in the charset that shape carries.
        (false, None, Some(max)) => {
            StringType::SizedUtf8String(max).with_charset(named.charset())?
        }
        (false, None, None) if named.max().is_some() => {
            return Err(invalid("$.max", "a maximum on the sized layout", 0));
        }
        (false, None, None) => named,
    };
    leaf.validate()?;
    Ok(leaf)
}

/// The parameters a `binary` document declares, under the same rule.
fn bytes_parameters(
    layout: crate::types::BytesType,
    max: Option<u32>,
    fixed: Option<u32>,
) -> Result<crate::types::BytesType> {
    use crate::types::BytesType;

    let leaf = match (layout, fixed, max) {
        (BytesType::FixedBinary(_), Some(width), None) => BytesType::FixedBinary(width),
        (BytesType::FixedBinary(_), None, None) => {
            return Err(invalid("$.fixed", "a width on the fixed layout", 0));
        }
        (BytesType::FixedBinary(_), _, Some(max)) => {
            return Err(invalid("$.max", "a fixed width on the fixed layout", max));
        }
        (other, Some(fixed), _) => {
            let _ = other;
            return Err(invalid("$.fixed", "a maximum on a variable layout", fixed));
        }
        (_, None, Some(max)) => BytesType::SizedBinary(max),
        (BytesType::SizedBinary(_), None, None) => {
            return Err(invalid("$.max", "a maximum on the sized layout", 0));
        }
        (other, None, None) => other,
    };
    leaf.validate()?;
    Ok(leaf)
}

/// A mapping key, which is always a plain string in a schema document.
pub(crate) fn key(name: &str) -> Scalar {
    Scalar::from(SmolStr::new(name))
}

/// A time unit as its snake_case full name, exactly as the JSON path emits it.
///
/// Not [`TimeUnit::as_str`], which answers the canonical *short* spelling
/// (`us`): the serialized vocabulary is the full name, and `from_str` accepts
/// both, so the two stay interchangeable on the read side.
fn unit_value(unit: TimeUnit) -> Scalar {
    Scalar::from(SmolStr::new(match unit {
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
    let value = if let Some(value) = held.as_i128() {
        i64::try_from(value)
            .map_err(|_| invalid(&format!("$.{name}"), "an integer", "an out-of-range value"))?
    } else {
        // A structured-text document may carry a wide integer as text; the
        // JSON path already accepts the decimal-string spelling for the same
        // reason, so the two stay interchangeable.
        let Scalar::String(text) = held else {
            return Err(invalid(
                &format!("$.{name}"),
                "an integer",
                format_smolstr!("{}", held.kind()),
            ));
        };
        text.as_str().parse::<i64>().map_err(|_| {
            invalid(
                &format!("$.{name}"),
                "an integer",
                format_smolstr!("{:?}", text.as_str()),
            )
        })?
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
    /// [`json::into_bytes_with_formatting`](crate::text::json::into_bytes_with_formatting)
    /// for what each [`Indent`](crate::text::Indent) means.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_json_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::text::json::into_bytes_with_formatting(
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
        Self::from_value(crate::text::yaml::from_utf8(value)?)
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
        text_of(crate::text::yaml::into_bytes_with_formatting(
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
        Self::from_value(crate::text::toml::from_utf8(value)?)
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
        text_of(crate::text::toml::into_bytes_with_formatting(
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
