//! The `Field` napi class — a named `DataType` with optional byte metadata and the
//! reserved comment / index_name / index_level accessors.

use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_schema::Field as CoreField;

use crate::datatype::DataType;

/// A named, typed schema node with optional byte-keyed metadata.
#[napi]
pub struct Field {
    pub(crate) inner: CoreField,
}

#[napi]
impl Field {
    /// A field with the given `name` and `dtype`.
    #[napi(constructor)]
    pub fn new(name: String, dtype: &DataType) -> Self {
        Field {
            inner: CoreField::new(name, dtype.inner.clone()),
        }
    }

    /// The field name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name.clone()
    }
    #[napi(setter)]
    pub fn set_name(&mut self, value: String) {
        self.inner.name = value;
    }

    /// The field's `DataType`.
    #[napi(getter)]
    pub fn dtype(&self) -> DataType {
        DataType {
            inner: self.inner.dtype.clone(),
        }
    }
    #[napi(setter)]
    pub fn set_dtype(&mut self, value: &DataType) {
        self.inner.dtype = value.inner.clone();
    }

    // ---- metadata (bytes -> bytes) ----

    /// The raw metadata value for `key`, if present.
    #[napi(js_name = "getMetadata")]
    pub fn get_metadata(&self, key: Buffer) -> Option<Buffer> {
        self.inner
            .get_metadata(&key)
            .map(|value| Buffer::from(value.to_vec()))
    }
    /// Sets a raw metadata `key` to `value` (in place).
    #[napi(js_name = "setMetadata")]
    pub fn set_metadata(&mut self, key: Buffer, value: Buffer) {
        self.inner.set_metadata(key.to_vec(), value.to_vec());
    }
    /// Removes a raw metadata `key`, returning its value.
    #[napi(js_name = "removeMetadata")]
    pub fn remove_metadata(&mut self, key: Buffer) -> Option<Buffer> {
        self.inner.remove_metadata(&key).map(Buffer::from)
    }

    // ---- reserved typed metadata (mutating setters) ----

    /// The field's comment, if any.
    #[napi(getter)]
    pub fn comment(&self) -> Option<String> {
        self.inner.comment()
    }
    #[napi(setter)]
    pub fn set_comment(&mut self, value: Option<String>) {
        self.inner.set_comment(value.as_deref());
    }

    /// The field's index name, if any.
    #[napi(getter, js_name = "indexName")]
    pub fn index_name(&self) -> Option<String> {
        self.inner.index_name()
    }
    #[napi(setter, js_name = "indexName")]
    pub fn set_index_name(&mut self, value: Option<String>) {
        self.inner.set_index_name(value.as_deref());
    }

    /// The field's index level (a `u16`), if any.
    #[napi(getter, js_name = "indexLevel")]
    pub fn index_level(&self) -> Option<u16> {
        self.inner.index_level()
    }
    #[napi(setter, js_name = "indexLevel")]
    pub fn set_index_level(&mut self, value: Option<u16>) {
        self.inner.set_index_level(value);
    }

    // ---- dunders ----

    /// `true` if the two fields are equal.
    #[napi]
    pub fn equals(&self, other: &Field) -> bool {
        self.inner == other.inner
    }

    /// A stable hash of the field.
    #[napi(js_name = "hashCode")]
    pub fn hash_code(&self) -> BigInt {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        BigInt::from(hasher.finish())
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        format!("Field({:?}, {})", self.inner.name, self.inner.dtype.name())
    }
}
