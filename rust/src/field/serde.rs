//! Structural JSON and Serde implementations for fields.

use std::fmt;
use std::sync::OnceLock;

use serde::de::{Error as DeError, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use super::Field;
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
