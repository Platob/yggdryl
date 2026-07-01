//! The [`BinaryField`] — a field of [`BinaryType`].

use crate::dtype::{BinaryType, DataType};
use crate::field::{Field, Metadata, PrimitiveField};
use crate::nested_fields::NestedFields;

/// A field whose data type is [`BinaryType`] — the field-level counterpart of that
/// type, and the first concrete [`Field`] (a [`PrimitiveField`]). The `with_*` /
/// [`without_metadata`](BinaryField::without_metadata) / [`copy`](BinaryField::copy)
/// updates are non-mutating and return a new field.
///
/// ```
/// use yggdryl_schema::{BinaryField, DataTypeId, Field};
///
/// let field = BinaryField::new("payload");
/// assert_eq!(field.name(), "payload");
/// assert_eq!(field.dtype().type_id(), DataTypeId::Binary);
///
/// let renamed = field.with_name("body".to_string());
/// assert_eq!(field.name(), "payload"); // original untouched
/// assert_eq!(renamed.name(), "body");
/// ```
#[derive(Clone, Debug)]
pub struct BinaryField {
    name: String,
    dtype: BinaryType,
    metadata: Option<Metadata>,
}

impl BinaryField {
    /// A binary field named `name`, with no metadata.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dtype: BinaryType::new(),
            metadata: None,
        }
    }

    /// A binary field from its explicit parts.
    pub fn from_parts(name: String, metadata: Option<Metadata>) -> Self {
        Self {
            name,
            dtype: BinaryType::new(),
            metadata,
        }
    }

    /// A copy with the given parts overridden; omitted parts are taken from `self`.
    /// (The data type is fixed to [`BinaryType`] and so is not a parameter.)
    pub fn copy(&self, name: Option<String>, metadata: Option<Option<Metadata>>) -> Self {
        Self {
            name: name.unwrap_or_else(|| self.name.clone()),
            dtype: self.dtype,
            metadata: metadata.unwrap_or_else(|| self.metadata.clone()),
        }
    }

    /// A copy renamed to `name`.
    pub fn with_name(&self, name: String) -> Self {
        self.copy(Some(name), None)
    }

    /// A copy carrying `metadata`.
    pub fn with_metadata(&self, metadata: Metadata) -> Self {
        self.copy(None, Some(Some(metadata)))
    }

    /// A copy with the metadata cleared.
    pub fn without_metadata(&self) -> Self {
        self.copy(None, Some(None))
    }
}

// A binary field has no children — the empty `NestedFields` default is right.
impl NestedFields for BinaryField {}

impl Field for BinaryField {
    fn name(&self) -> &str {
        &self.name
    }

    fn dtype(&self) -> &dyn DataType {
        &self.dtype
    }

    fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }

    fn clone_box(&self) -> Box<dyn Field> {
        Box::new(self.clone())
    }
}

impl PrimitiveField for BinaryField {}
