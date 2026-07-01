//! The [`Field`] — a named data type with optional metadata.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use crate::data_type::DataType;

/// Byte-keyed, byte-valued metadata attached to a [`Field`]. A `BTreeMap` keeps a
/// deterministic order, so equal fields hash equally.
pub type Metadata = BTreeMap<Vec<u8>, Vec<u8>>;

/// A named column: a `name`, its [`DataType`], and optional byte-keyed
/// [`Metadata`]. The `with_*` / [`without_metadata`](Field::without_metadata) /
/// [`copy`](Field::copy) updates are non-mutating and return a new field.
///
/// ```
/// use yggdryl_schema::{BinaryType, DataTypeId, Field};
///
/// let field = Field::new("payload", Box::new(BinaryType::new()));
/// assert_eq!(field.name(), "payload");
/// assert_eq!(field.dtype().type_id(), DataTypeId::Binary);
///
/// let renamed = field.with_name("body".to_string());
/// assert_eq!(field.name(), "payload"); // original untouched
/// assert_eq!(renamed.name(), "body");
/// ```
#[derive(Debug)]
pub struct Field {
    name: String,
    dtype: Box<dyn DataType>,
    metadata: Option<Metadata>,
}

impl Field {
    /// A field named `name` of type `dtype`, with no metadata.
    pub fn new(name: impl Into<String>, dtype: Box<dyn DataType>) -> Self {
        Self {
            name: name.into(),
            dtype,
            metadata: None,
        }
    }

    /// A field from its explicit parts.
    pub fn from_parts(name: String, dtype: Box<dyn DataType>, metadata: Option<Metadata>) -> Self {
        Self {
            name,
            dtype,
            metadata,
        }
    }

    /// The field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field's data type.
    pub fn dtype(&self) -> &dyn DataType {
        self.dtype.as_ref()
    }

    /// The field's metadata, if any.
    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }

    /// A copy with the given parts overridden; omitted parts are taken from `self`.
    pub fn copy(
        &self,
        name: Option<String>,
        dtype: Option<Box<dyn DataType>>,
        metadata: Option<Option<Metadata>>,
    ) -> Self {
        Self {
            name: name.unwrap_or_else(|| self.name.clone()),
            dtype: dtype.unwrap_or_else(|| self.dtype.clone_box()),
            metadata: metadata.unwrap_or_else(|| self.metadata.clone()),
        }
    }

    /// A copy renamed to `name`.
    pub fn with_name(&self, name: String) -> Self {
        self.copy(Some(name), None, None)
    }

    /// A copy with the data type replaced by `dtype`.
    pub fn with_dtype(&self, dtype: Box<dyn DataType>) -> Self {
        self.copy(None, Some(dtype), None)
    }

    /// A copy carrying `metadata`.
    pub fn with_metadata(&self, metadata: Metadata) -> Self {
        self.copy(None, None, Some(Some(metadata)))
    }

    /// A copy with the metadata cleared.
    pub fn without_metadata(&self) -> Self {
        self.copy(None, None, Some(None))
    }
}

// Hand-implemented (rather than derived) because `Box<dyn DataType>` is a trait
// object: each impl routes through the value-like hooks on [`DataType`].
impl Clone for Field {
    fn clone(&self) -> Self {
        self.copy(None, None, None)
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.dtype.dyn_eq(other.dtype.as_ref())
            && self.metadata == other.metadata
    }
}

impl Eq for Field {}

impl Hash for Field {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.dtype.dyn_hash(state);
        self.metadata.hash(state);
    }
}
