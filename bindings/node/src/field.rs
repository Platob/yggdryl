//! The `Field` napi class — a named, nullable `DataType` graph node.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_schema::{Field as CoreField, MergeStrategy};

use crate::datatype::DataType;
use crate::{err, to_mapping};

/// A named, nullable `DataType` with metadata, an optional parent (for graph
/// traversal) and child accessors. A struct-typed field is a schema.
#[napi]
pub struct Field {
    pub(crate) inner: CoreField,
}

fn wrap(inner: CoreField) -> Field {
    Field { inner }
}

#[napi]
impl Field {
    /// Build from a name, `DataType` and nullability (default `true`).
    #[napi(constructor)]
    pub fn new(name: String, data_type: &DataType, nullable: Option<bool>) -> Self {
        wrap(CoreField::new(
            name,
            data_type.inner.clone(),
            nullable.unwrap_or(true),
        ))
    }

    /// Parse a `"name: type"` field string (`not null` suffix = non-nullable).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str(value: String) -> Result<Self> {
        CoreField::from_str(&value).map(wrap).map_err(err)
    }

    /// Build from an object (`name` / `type` / `nullable` / `comment`).
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(fields: HashMap<String, String>) -> Result<Self> {
        CoreField::from_mapping(&to_mapping(fields))
            .map(wrap)
            .map_err(err)
    }

    /// Parse from the structural JSON of `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        CoreField::from_json(&value).map(wrap).map_err(err)
    }

    // ---- accessors ----

    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    #[napi(getter, js_name = "dataType")]
    pub fn data_type(&self) -> DataType {
        DataType {
            inner: self.inner.data_type().clone(),
        }
    }

    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    /// The metadata object.
    #[napi(getter)]
    pub fn metadata(&self) -> HashMap<String, String> {
        self.inner.metadata().clone().into_iter().collect()
    }

    /// The `comment` metadata, if any.
    #[napi(getter)]
    pub fn comment(&self) -> Option<String> {
        self.inner.comment().map(str::to_string)
    }

    /// Read one metadata value.
    #[napi(js_name = "getMetadata")]
    pub fn get_metadata(&self, key: String) -> Option<String> {
        self.inner.get_metadata(&key).map(str::to_string)
    }

    /// Set one metadata entry in place.
    #[napi(js_name = "setMetadata")]
    pub fn set_metadata(&mut self, key: String, value: String) {
        self.inner.set_metadata(key, value);
    }

    /// Remove one metadata entry in place, returning the old value.
    #[napi(js_name = "removeMetadata")]
    pub fn remove_metadata(&mut self, key: String) -> Option<String> {
        self.inner.remove_metadata(&key)
    }

    /// Set the `comment` metadata in place.
    #[napi(js_name = "setComment")]
    pub fn set_comment(&mut self, comment: String) {
        self.inner.set_comment(comment);
    }

    // ---- builders (non-mutating) ----

    #[napi(js_name = "withName")]
    pub fn with_name(&self, name: String) -> Self {
        wrap(self.inner.clone().with_name(name))
    }

    #[napi(js_name = "withDataType")]
    pub fn with_data_type(&self, data_type: &DataType) -> Self {
        wrap(self.inner.clone().with_data_type(data_type.inner.clone()))
    }

    #[napi(js_name = "withNullable")]
    pub fn with_nullable(&self, nullable: bool) -> Self {
        wrap(self.inner.clone().with_nullable(nullable))
    }

    #[napi(js_name = "withMetadata")]
    pub fn with_metadata(&self, metadata: HashMap<String, String>) -> Self {
        wrap(self.inner.clone().with_metadata(to_mapping(metadata)))
    }

    #[napi(js_name = "withMetadataEntry")]
    pub fn with_metadata_entry(&self, key: String, value: String) -> Self {
        wrap(self.inner.clone().with_metadata_entry(key, value))
    }

    #[napi(js_name = "withComment")]
    pub fn with_comment(&self, comment: String) -> Self {
        wrap(self.inner.clone().with_comment(comment))
    }

    #[napi(js_name = "withoutMetadata")]
    pub fn without_metadata(&self) -> Self {
        wrap(self.inner.clone().without_metadata())
    }

    /// A copy overriding any component passed and keeping the rest (parent is not carried).
    #[napi]
    pub fn copy(
        &self,
        name: Option<String>,
        data_type: Option<&DataType>,
        nullable: Option<bool>,
        metadata: Option<HashMap<String, String>>,
    ) -> Self {
        wrap(self.inner.copy(
            name,
            data_type.map(|d| d.inner.clone()),
            nullable,
            metadata.map(to_mapping),
        ))
    }

    // ---- graph ----

    /// The navigational parent, if linked.
    #[napi(getter)]
    pub fn parent(&self) -> Option<Field> {
        self.inner.parent().cloned().map(wrap)
    }

    #[napi(js_name = "withParent")]
    pub fn with_parent(&self, parent: &Field) -> Self {
        wrap(self.inner.clone().with_parent(parent.inner.clone()))
    }

    /// Set the parent in place.
    #[napi(js_name = "setParent")]
    pub fn set_parent(&mut self, parent: &Field) {
        self.inner.set_parent(parent.inner.clone());
    }

    #[napi(js_name = "withoutParent")]
    pub fn without_parent(&self) -> Self {
        wrap(self.inner.clone().without_parent())
    }

    /// The topmost ancestor reachable via `parent` (or `self`).
    #[napi]
    pub fn root(&self) -> Field {
        wrap(self.inner.root().clone())
    }

    /// A copy with parent links wired throughout the struct tree.
    #[napi(js_name = "withLinkedChildren")]
    pub fn with_linked_children(&self) -> Self {
        wrap(self.inner.clone().with_linked_children())
    }

    /// The child fields (empty unless this is a struct).
    #[napi]
    pub fn children(&self) -> Vec<Field> {
        self.inner.children().iter().cloned().map(wrap).collect()
    }

    /// The number of child fields.
    #[napi(getter, js_name = "childCount")]
    pub fn child_count(&self) -> u32 {
        self.inner.child_count() as u32
    }

    /// The child at `index`, if any.
    #[napi(js_name = "childAt")]
    pub fn child_at(&self, index: u32) -> Option<Field> {
        self.inner.child_at(index as usize).cloned().map(wrap)
    }

    /// The first child matching `name` (case-insensitive).
    #[napi]
    pub fn child(&self, name: String) -> Option<Field> {
        self.inner.child(&name).cloned().map(wrap)
    }

    /// The first child matching `name` exactly (case-sensitive).
    #[napi(js_name = "childExact")]
    pub fn child_exact(&self, name: String) -> Option<Field> {
        self.inner.child_exact(&name).cloned().map(wrap)
    }

    /// The index of the first child matching `name` (case-insensitive).
    #[napi(js_name = "childIndex")]
    pub fn child_index(&self, name: String) -> Option<u32> {
        self.inner.child_index(&name).map(|i| i as u32)
    }

    // ---- merge ----

    /// Merge with `other` (names must match) under a strategy.
    #[napi]
    pub fn merge(&self, other: &Field, strategy: Option<String>) -> Result<Field> {
        let strategy =
            MergeStrategy::from_str(strategy.as_deref().unwrap_or("promote")).map_err(err)?;
        self.inner
            .merge(&other.inner, strategy)
            .map(wrap)
            .map_err(err)
    }

    // ---- serialisation ----

    /// Render to an object (`name` / `type` / `nullable` / `comment`).
    #[napi(js_name = "toMapping")]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// The canonical string as bytes.
    #[napi(js_name = "toBytes")]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// `true` if the two fields are equal (parent is ignored).
    #[napi]
    pub fn equals(&self, other: &Field) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to a lossless structural JSON string.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }
}
