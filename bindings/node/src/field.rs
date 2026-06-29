//! Node wrapper for [`yggdryl_field::AnyField`].

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi::Either;
use napi_derive::napi;
use yggdryl_core::Jsonable;
use yggdryl_field::AnyField;

use crate::{anytype_from_either, anytype_to_either, to_napi_err, BinaryType, Utf8Type};

/// A named, nullable, typed field with string→string metadata.
#[napi]
pub struct Field {
    pub(crate) inner: AnyField,
}

#[napi]
impl Field {
    #[napi(constructor)]
    pub fn new(
        name: String,
        data_type: Either<&BinaryType, &Utf8Type>,
        nullable: Option<bool>,
        metadata: Option<HashMap<String, String>>,
    ) -> Self {
        let data_type = anytype_from_either(data_type);
        let mut field = AnyField::new(name, data_type, nullable.unwrap_or(true));
        if let Some(metadata) = metadata {
            field = field.with_metadata(metadata.into_iter().collect());
        }
        Field { inner: field }
    }

    /// The field's name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type (a `BinaryType` or `Utf8Type` object).
    #[napi(getter)]
    pub fn data_type(&self) -> Either<BinaryType, Utf8Type> {
        anytype_to_either(self.inner.data_type())
    }

    /// Whether the field admits null values.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    /// The field's metadata.
    #[napi(getter)]
    pub fn metadata(&self) -> HashMap<String, String> {
        self.inner.metadata().clone().into_iter().collect()
    }

    /// A copy with a different name.
    #[napi]
    pub fn with_name(&self, name: String) -> Field {
        Field {
            inner: self.inner.with_name(name),
        }
    }

    /// A copy with a different nullability.
    #[napi]
    pub fn with_nullable(&self, nullable: bool) -> Field {
        Field {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A copy with the given metadata.
    #[napi]
    pub fn with_metadata(&self, metadata: HashMap<String, String>) -> Field {
        Field {
            inner: self.inner.with_metadata(metadata.into_iter().collect()),
        }
    }

    /// A copy with the metadata cleared.
    #[napi]
    pub fn without_metadata(&self) -> Field {
        Field {
            inner: self.inner.without_metadata(),
        }
    }

    /// The component map (`name`, `type`, `nullable`, `metadata.<key>`).
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs a field from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<Field> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        AnyField::from_mapping(&mapping)
            .map(|inner| Field { inner })
            .map_err(to_napi_err)
    }

    /// The canonical, length-prefixed byte form.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// Reconstructs a field from its byte form.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> napi::Result<Field> {
        AnyField::from_bytes(data.as_ref())
            .map(|inner| Field { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.inner).expect("Field serializes to JSON")
    }

    /// The JSON string, formatted per the global `JsonParams`.
    #[napi(js_name = "toJsonString")]
    pub fn to_json_string(&self) -> String {
        self.inner.to_json()
    }

    /// Reconstructs a field from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<Field> {
        serde_json::from_value(value)
            .map(|inner| Field { inner })
            .map_err(to_napi_err)
    }

    /// The JSON bytes (JSON text encoded with the global charset).
    #[napi]
    pub fn to_bson(&self) -> Buffer {
        self.inner.to_bson().into()
    }

    /// Reconstructs a field from its JSON bytes.
    #[napi(factory)]
    pub fn from_bson(data: Buffer) -> napi::Result<Field> {
        AnyField::from_bson(data.as_ref())
            .map(|inner| Field { inner })
            .map_err(to_napi_err)
    }

    /// Structural equality with another `Field`.
    #[napi]
    pub fn equals(&self, other: &Field) -> bool {
        self.inner == other.inner
    }
}
