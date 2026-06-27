//! The [`Field`] type — a named, nullable [`DataType`] with metadata, an optional
//! [`parent`](Field::parent) for graph traversal and child accessors. A field whose
//! type is a [`Struct`](DataType::Struct) is a schema.

use std::fmt;
use std::hash::{Hash, Hasher};

#[allow(unused_imports)]
use crate::log_event;
use crate::{DataType, MergeStrategy, SchemaError};
use std::collections::BTreeMap;

/// The metadata key used by the [`comment`](Field::comment) accessor.
const COMMENT_KEY: &str = "comment";

/// A named, nullable [`DataType`] with string metadata. Fields make the type system
/// recursive (a list item / struct member is a `Field`) and form a navigable graph
/// via an optional [`parent`](Field::parent) and the child accessors.
///
/// The optional `parent` is **navigational only**: it is excluded from equality,
/// hashing and serialization (so a field's identity is its name + type + nullability
/// + metadata, and cycles never break `Hash`/`serde`).
///
/// ```
/// use yggdryl_schema::{DataType, Field};
///
/// let f = Field::new("id", DataType::int(64, true), false).with_comment("primary key");
/// assert_eq!(f.name(), "id");
/// assert!(!f.is_nullable());
/// assert_eq!(f.comment(), Some("primary key"));
/// assert_eq!(f.to_str(), "id: int64 not null");
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Field {
    name: String,
    data_type: DataType,
    nullable: bool,
    metadata: BTreeMap<String, String>,
    /// Navigational parent — excluded from identity and serialization.
    #[cfg_attr(feature = "serde", serde(skip))]
    parent: Option<Box<Field>>,
}

// Identity ignores `parent` (it is navigational, and including it would make
// cyclic graphs un-hashable / un-comparable).
impl PartialEq for Field {
    fn eq(&self, other: &Field) -> bool {
        self.name == other.name
            && self.data_type == other.data_type
            && self.nullable == other.nullable
            && self.metadata == other.metadata
    }
}

impl Eq for Field {}

impl Hash for Field {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.data_type.hash(state);
        self.nullable.hash(state);
        self.metadata.hash(state);
    }
}

impl Field {
    /// Creates a field from its name, type and nullability (no metadata, no parent).
    pub fn new(name: impl Into<String>, data_type: DataType, nullable: bool) -> Field {
        Field {
            name: name.into(),
            data_type,
            nullable,
            metadata: BTreeMap::new(),
            parent: None,
        }
    }

    // ---- core accessors ----

    /// The field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's [`DataType`].
    pub fn data_type(&self) -> &DataType {
        &self.data_type
    }

    /// Whether the field admits nulls.
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// The field's metadata map (empty by default).
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    // ---- metadata getters / setters ----

    /// Reads one metadata value by key.
    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Sets one metadata entry in place (a mutating setter).
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Removes one metadata entry in place, returning the old value.
    pub fn remove_metadata(&mut self, key: &str) -> Option<String> {
        self.metadata.remove(key)
    }

    /// The `comment` metadata, if any — a common, named convenience accessor.
    pub fn comment(&self) -> Option<&str> {
        self.get_metadata(COMMENT_KEY)
    }

    /// Sets the `comment` metadata in place.
    pub fn set_comment(&mut self, comment: impl Into<String>) {
        self.set_metadata(COMMENT_KEY, comment);
    }

    // ---- non-mutating builders ----

    /// Returns a copy with the name replaced.
    pub fn with_name(mut self, name: impl Into<String>) -> Field {
        self.name = name.into();
        self
    }

    /// Returns a copy with the [`DataType`] replaced.
    pub fn with_data_type(mut self, data_type: DataType) -> Field {
        self.data_type = data_type;
        self
    }

    /// Returns a copy with the nullability replaced.
    pub fn with_nullable(mut self, nullable: bool) -> Field {
        self.nullable = nullable;
        self
    }

    /// Returns a copy with the whole metadata map replaced.
    pub fn with_metadata(mut self, metadata: BTreeMap<String, String>) -> Field {
        self.metadata = metadata;
        self
    }

    /// Returns a copy with one metadata entry added or replaced.
    pub fn with_metadata_entry(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Field {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Returns a copy with the `comment` metadata set.
    pub fn with_comment(mut self, comment: impl Into<String>) -> Field {
        self.metadata
            .insert(COMMENT_KEY.to_string(), comment.into());
        self
    }

    /// Returns a copy with the metadata cleared.
    pub fn without_metadata(mut self) -> Field {
        self.metadata.clear();
        self
    }

    /// Returns a copy, overriding each component for which `Some` is given. Call
    /// `copy(None, None, None, None)` to clone (the parent is not carried).
    pub fn copy(
        &self,
        name: Option<String>,
        data_type: Option<DataType>,
        nullable: Option<bool>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> Field {
        Field {
            name: name.unwrap_or_else(|| self.name.clone()),
            data_type: data_type.unwrap_or_else(|| self.data_type.clone()),
            nullable: nullable.unwrap_or(self.nullable),
            metadata: metadata.unwrap_or_else(|| self.metadata.clone()),
            parent: None,
        }
    }

    // ---- graph: parent (up) ----

    /// The navigational parent field, if linked.
    pub fn parent(&self) -> Option<&Field> {
        self.parent.as_deref()
    }

    /// Returns a copy whose parent is `parent`.
    pub fn with_parent(mut self, parent: Field) -> Field {
        self.parent = Some(Box::new(parent));
        self
    }

    /// Sets the parent in place.
    pub fn set_parent(&mut self, parent: Field) {
        self.parent = Some(Box::new(parent));
    }

    /// Returns a copy with no parent.
    pub fn without_parent(mut self) -> Field {
        self.parent = None;
        self
    }

    /// The topmost ancestor reachable through [`parent`](Field::parent) (or `self`).
    pub fn root(&self) -> &Field {
        let mut field = self;
        while let Some(parent) = field.parent.as_deref() {
            field = parent;
        }
        field
    }

    /// Returns a copy with parent links wired throughout the struct tree, so each
    /// descendant's [`parent`](Field::parent) reaches its container (ancestor
    /// headers carry name / nullability / metadata, but not sibling fields).
    pub fn with_linked_children(mut self) -> Field {
        let header = Box::new(self.header());
        if let DataType::Struct(fields) = &mut self.data_type {
            *fields = fields
                .iter()
                .map(|child| {
                    let mut linked = child.clone();
                    linked.parent = Some(header.clone());
                    linked.with_linked_children()
                })
                .collect();
        }
        self
    }

    /// A shallow header copy (no struct body) used as a parent pointer.
    fn header(&self) -> Field {
        Field {
            name: self.name.clone(),
            data_type: DataType::Struct(Vec::new()),
            nullable: self.nullable,
            metadata: self.metadata.clone(),
            parent: self.parent.clone(),
        }
    }

    // ---- graph: children (down) ----

    /// The child fields, if this field's type is a [`Struct`](DataType::Struct)
    /// (else an empty slice).
    pub fn children(&self) -> &[Field] {
        match &self.data_type {
            DataType::Struct(fields) => fields,
            _ => &[],
        }
    }

    /// The number of child fields.
    pub fn child_count(&self) -> usize {
        self.children().len()
    }

    /// The child at the given index, if any.
    pub fn child_at(&self, index: usize) -> Option<&Field> {
        self.children().get(index)
    }

    /// The first child whose name matches `name` **case-insensitively** (the
    /// common, allocation-free lookup).
    pub fn child(&self, name: &str) -> Option<&Field> {
        self.children()
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// The first child whose name matches `name` exactly (case-sensitive).
    pub fn child_exact(&self, name: &str) -> Option<&Field> {
        self.children().iter().find(|f| f.name == name)
    }

    /// The index of the first child matching `name` case-insensitively.
    pub fn child_index(&self, name: &str) -> Option<usize> {
        self.children()
            .iter()
            .position(|f| f.name.eq_ignore_ascii_case(name))
    }

    // ---- merge ----

    /// Merges this field with `other` under the chosen [`MergeStrategy`]: the names
    /// must match, the types merge ([`DataType::merge`]), the result is nullable if
    /// either side is, and the metadata is unioned (this field's entries win).
    pub fn merge(&self, other: &Field, strategy: MergeStrategy) -> Result<Field, SchemaError> {
        if self.name != other.name {
            return Err(SchemaError::NameMismatch {
                left: self.name.clone(),
                right: other.name.clone(),
            });
        }
        let data_type = self.data_type.merge(&other.data_type, strategy)?;
        let mut metadata = self.metadata.clone();
        for (key, value) in &other.metadata {
            metadata.entry(key.clone()).or_insert_with(|| value.clone());
        }
        Ok(Field {
            name: self.name.clone(),
            data_type,
            nullable: self.nullable || other.nullable,
            metadata,
            parent: None,
        })
    }

    // ---- conversions ----

    /// Parses a field from its canonical `"name: type"` string (a trailing
    /// ` not null` marks it non-nullable). Metadata / parent are not carried.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> Result<Field, SchemaError> {
        log_event!(trace, "Field::from_str {input:?}");
        crate::datatype::parse_field_str(input)
    }

    /// Builds a [`Field`] from a `BTreeMap` (`name`, `type` (required), `nullable`
    /// (default `true`), optional `comment`).
    pub fn from_mapping(fields: &BTreeMap<String, String>) -> Result<Field, SchemaError> {
        let name = fields.get("name").cloned().unwrap_or_default();
        let data_type = match fields.get("type") {
            Some(value) => DataType::from_str(value)?,
            None => return Err(SchemaError::Empty),
        };
        let nullable = fields
            .get("nullable")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
            .unwrap_or(true);
        let mut field = Field::new(name, data_type, nullable);
        if let Some(comment) = fields.get(COMMENT_KEY) {
            field.set_comment(comment.clone());
        }
        Ok(field)
    }

    /// Renders the canonical `"name: type"` string, with ` not null` when
    /// non-nullable — the inverse of [`from_str`](Field::from_str).
    pub fn to_str(&self) -> String {
        if self.nullable {
            format!("{}: {}", self.name, self.data_type.to_str())
        } else {
            format!("{}: {} not null", self.name, self.data_type.to_str())
        }
    }

    /// Renders to a component `BTreeMap` (`name` / `type` / `nullable`, plus
    /// `comment` if set).
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::from([
            ("name".to_string(), self.name.clone()),
            ("type".to_string(), self.data_type.to_str()),
            ("nullable".to_string(), self.nullable.to_string()),
        ]);
        if let Some(comment) = self.comment() {
            map.insert(COMMENT_KEY.to_string(), comment.to_string());
        }
        map
    }

    /// The canonical string as UTF-8 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_str().into_bytes()
    }

    /// Parses a field from the UTF-8 bytes of its canonical string.
    pub fn from_bytes(bytes: &[u8]) -> Result<Field, SchemaError> {
        let value =
            std::str::from_utf8(bytes).map_err(|_| SchemaError::Invalid("<bytes>".into()))?;
        Field::from_str(value)
    }

    /// Serialises to a lossless structural JSON string (preserves metadata).
    /// Requires the `json` feature.
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Field serialises")
    }

    /// Parses a [`Field`] from the structural JSON of [`to_json`](Field::to_json).
    #[cfg(feature = "json")]
    pub fn from_json(json: &str) -> Result<Field, SchemaError> {
        serde_json::from_str(json).map_err(|e| SchemaError::Invalid(e.to_string()))
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}
