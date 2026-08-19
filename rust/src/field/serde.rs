//! Structural JSON and Serde implementations for fields.

use std::fmt;
use std::sync::OnceLock;

use serde::de::{Error as DeError, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use super::Field;
use crate::datatype::serde::{integer, invalid, key};
use crate::generic::Value;
use crate::{DataType, Error, Metadata, Result};

impl Field {
    /// Deserializes and validates a field from structural JSON.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// Serializes this value as deterministic structural JSON.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Consumes and serializes this value as deterministic structural JSON.
    pub fn into_json(self) -> Result<String> {
        Ok(serde_json::to_string(&self)?)
    }
}

impl Serialize for Field {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let field_count =
            4 + usize::from(self.dictionary_id != 0) + usize::from(self.dictionary_is_ordered);
        let mut field = serializer.serialize_struct("Field", field_count)?;
        field.serialize_field("name", &self.name)?;
        field.serialize_field("data_type", &self.data_type)?;
        field.serialize_field("nullable", &self.nullable)?;
        if self.dictionary_id != 0 {
            field.serialize_field("dictionary_id", &DictionaryIdJson(self.dictionary_id))?;
        }
        if self.dictionary_is_ordered {
            field.serialize_field("dictionary_is_ordered", &true)?;
        }
        field.serialize_field("metadata", &self.metadata)?;
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
        struct FieldValue {
            name: SmolStr,
            data_type: DataType,
            nullable: bool,
            #[serde(default, deserialize_with = "deserialize_dictionary_id")]
            dictionary_id: i64,
            #[serde(default)]
            dictionary_is_ordered: bool,
            #[serde(default)]
            metadata: Metadata,
        }

        let value = FieldValue::deserialize(deserializer)?;
        let field = Self {
            name: value.name,
            data_type: value.data_type,
            nullable: value.nullable,
            dictionary_id: value.dictionary_id,
            dictionary_is_ordered: value.dictionary_is_ordered,
            metadata: value.metadata,
            arrow: OnceLock::new(),
        };
        field.validate().map_err(D::Error::custom)?;
        Ok(field)
    }
}

// ---------------------------------------------------------------------------
// The one structural mapping between a `Field` and the shared `Value`.
//
// A companion to the datatype conversion: together they are the *only*
// structural model of a schema in the tree. JSON, YAML, and TOML are three
// writers over it, so the three agree by construction rather than by three
// sets of tests, and a schema becomes embeddable in any structured document
// the crate already reads - a config file can carry a declared schema inline
// beside the rest of its settings, with no JSON-string-inside-YAML awkwardness.
// ---------------------------------------------------------------------------

impl Field {
    /// Project this field onto the shared structural [`Value`].
    ///
    /// The mapping carries `name`, `data_type`, `nullable`, then
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
    /// let value = field.to_value();
    ///
    /// // Unset optional attributes are absent rather than null.
    /// assert!(value.get_key_str("dictionary_id").is_none());
    /// assert_eq!(Field::from_value(value)?, field);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut entries: Vec<(Value, Value)> = Vec::with_capacity(6);
        entries.push((key("name"), Value::String(self.name.clone())));
        entries.push((key("data_type"), self.data_type.to_value()));
        entries.push((key("nullable"), Value::Bool(self.nullable)));
        if self.dictionary_id != 0 {
            // Decimal text, as the JSON path emits it: a 64-bit identifier
            // does not survive every reader as a number.
            entries.push((
                key("dictionary_id"),
                Value::String(format_smolstr!("{}", self.dictionary_id)),
            ));
        }
        if self.dictionary_is_ordered {
            entries.push((key("dictionary_is_ordered"), Value::Bool(true)));
        }
        entries.push((
            key("metadata"),
            Value::from_mapping(
                self.metadata_iter()
                    .map(|(name, value)| (key(name), Value::String(SmolStr::new(value)))),
            )
            .unwrap_or(Value::Null),
        ));
        Value::from_mapping(entries).unwrap_or(Value::Null)
    }

    /// Read a field back from the shared structural [`Value`].
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
    ///     DataType::Utf8.nullable_field("venue"),
    /// ])?
    /// .required_field("row");
    ///
    /// assert_eq!(Field::from_value(nested.to_value())?, nested);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the path and the expectation when the value is
    /// not a field mapping, a required key is missing or wrongly typed, or the
    /// assembled field does not validate.
    pub fn from_value(value: Value) -> Result<Self> {
        let entries = value
            .as_mapping()
            .ok_or_else(|| invalid("$", "a field mapping", value.kind()))?;
        let at = |name: &str| -> Option<&Value> {
            entries
                .iter()
                .find(|(held, _)| held.as_str() == Some(name))
                .map(|(_, held)| held)
        };
        let name = at("name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("$.name", "a field name", "nothing"))?;
        let data_type = DataType::from_value(
            at("data_type")
                .ok_or_else(|| invalid("$.data_type", "a datatype mapping", "nothing"))?
                .clone(),
        )?;
        let nullable = match at("nullable") {
            Some(Value::Bool(held)) => *held,
            other => {
                return Err(invalid(
                    "$.nullable",
                    "a boolean",
                    other.map_or("nothing", Value::kind),
                ));
            }
        };

        let mut field = Self::new(name, data_type, nullable);
        // Only a dictionary carries the pair, and both settle together so a
        // half-declared state can never reach the field.
        let dictionary_id = match at("dictionary_id").filter(|held| !matches!(held, Value::Null)) {
            Some(held) => i64::from(integer(Some(held), "dictionary_id")?),
            None => 0,
        };
        let dictionary_is_ordered = matches!(at("dictionary_is_ordered"), Some(Value::Bool(true)));
        if dictionary_id != 0 || dictionary_is_ordered {
            field.set_dictionary_options(dictionary_id, dictionary_is_ordered)?;
        }

        if let Some(held) = at("metadata").filter(|held| !matches!(held, Value::Null)) {
            let pairs = held
                .as_mapping()
                .ok_or_else(|| invalid("$.metadata", "a mapping of string entries", held.kind()))?;
            let mut collected = Vec::with_capacity(pairs.len());
            for (name, value) in pairs {
                let name = name
                    .as_str()
                    .ok_or_else(|| invalid("$.metadata", "string metadata keys", name.kind()))?;
                let value = value.as_str().ok_or_else(|| {
                    invalid(
                        &format!("$.metadata[{name:?}]"),
                        "a string metadata value",
                        value.kind(),
                    )
                })?;
                collected.push((SmolStr::new(name), SmolStr::new(value)));
            }
            field.set_metadata(Metadata::from_entries(collected)?)?;
        }

        field.validate()?;
        Ok(field)
    }
}

impl From<&Field> for Value {
    fn from(value: &Field) -> Self {
        value.to_value()
    }
}

impl TryFrom<Value> for Field {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self> {
        Self::from_value(value)
    }
}

// ---------------------------------------------------------------------------
// The three formats, all over the one `Value` conversion.
//
// `to_json` keeps the Serde path because `Field` is `Serialize`/`Deserialize`
// for the serde ecosystem - it is nested inside other derived structures
// across the tree, and AGENTS.md requires those traits on a native value. The
// two are not a second structural model: the parity test in
// `tests/field/serde.rs` dumps every shape through both routes and compares the
// bytes, so the Serde impl cannot drift from the `Value` mapping without
// failing a test. Every *other* format goes through `to_value` alone.
// ---------------------------------------------------------------------------

impl Field {
    /// Serialize this value as deterministic structural JSON, laid out as asked.
    ///
    /// The companion of [`Self::to_json`]; see
    /// [`json::to_vec_with_formatting`](crate::json::to_vec_with_formatting)
    /// for what each [`Indent`](crate::text::Indent) means.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn to_json_with_formatting(&self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::json::to_vec_with_formatting(
            &self.to_value(),
            formatting,
        )?)
    }

    /// Consume and serialize as structural JSON, laid out as asked.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_json_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        self.to_json_with_formatting(formatting)
    }

    /// Deserialize and validate from structural YAML.
    ///
    /// The same structure [`Self::from_json`] reads, in YAML's syntax - so a
    /// configuration document can carry a declared schema inline beside the
    /// rest of its settings, with no JSON-string-inside-YAML awkwardness.
    ///
    /// # Errors
    ///
    /// Returns the parser's failure, or the structural refusal naming the path
    /// and the expectation.
    pub fn from_yaml(value: &str) -> Result<Self> {
        Self::from_value(crate::yaml::from_str(value)?)
    }

    /// Serialize as YAML: block style, one key per line, one indent per level.
    ///
    /// Key order is the same as the JSON emit, so the two are comparable side
    /// by side, and an unset optional attribute is omitted rather than emitted
    /// as null.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn to_yaml(&self) -> Result<String> {
        self.to_yaml_with_formatting(crate::text::Formatting::default())
    }

    /// Serialize as YAML, laid out as asked.
    ///
    /// Block style at the requested width; flow style only if a caller
    /// explicitly asks for [`Indent::None`](crate::text::Indent::None).
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn to_yaml_with_formatting(&self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::yaml::to_vec_with_formatting(
            &self.to_value(),
            formatting,
        )?)
    }

    /// Consume and serialize as YAML.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_yaml(self) -> Result<String> {
        self.to_yaml()
    }

    /// Consume and serialize as YAML, laid out as asked.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_yaml_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        self.to_yaml_with_formatting(formatting)
    }

    /// Deserialize and validate from structural TOML.
    ///
    /// # Errors
    ///
    /// Returns the parser's failure, or the structural refusal naming the path
    /// and the expectation.
    pub fn from_toml(value: &str) -> Result<Self> {
        Self::from_value(crate::toml::from_str(value)?)
    }

    /// Serialize as TOML.
    ///
    /// TOML has no null, and this model never needs one: an unset optional
    /// attribute is *omitted* rather than faked, so nothing is lost on the way
    /// out and `from_toml` reads the same value back.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn to_toml(&self) -> Result<String> {
        self.to_toml_with_formatting(crate::text::Formatting::default())
    }

    /// Serialize as TOML, laid out as asked.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn to_toml_with_formatting(&self, formatting: crate::text::Formatting) -> Result<String> {
        text_of(crate::toml::to_vec_with_formatting(
            &self.to_value(),
            formatting,
        )?)
    }

    /// Consume and serialize as TOML.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_toml(self) -> Result<String> {
        self.to_toml()
    }

    /// Consume and serialize as TOML, laid out as asked.
    ///
    /// # Errors
    ///
    /// Returns the encoder's failure.
    pub fn into_toml_with_formatting(self, formatting: crate::text::Formatting) -> Result<String> {
        self.to_toml_with_formatting(formatting)
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
