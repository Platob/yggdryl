//! [`StructScalar`] — one **struct value**: a nullable row of the struct's fields, each a type-erased
//! [`AnyScalar`](crate::io::AnyScalar). It is what [`StructSerie::row`](super::StructSerie::row) yields.

use super::StructType;
use crate::io::{AnyField, AnyScalar, DataTypeId, ScalarType};

/// A single **struct value** — a row: the struct's schema (its ordered child [`AnyField`]s), one
/// erased [`AnyScalar`] per field, and whether the struct value itself is null.
///
/// It is a hashable value type: two struct values are equal iff they have the same schema and either
/// are both null, or hold equal per-field values. A **null** struct's phantom child values are
/// ignored (two same-schema null structs are equal, like `Scalar::null() == Scalar::null()`).
#[derive(Debug, Clone)]
pub struct StructScalar {
    fields: Vec<AnyField>,
    values: Vec<AnyScalar>,
    null: bool,
}

impl StructScalar {
    /// A present struct value from its schema fields and one [`AnyScalar`] per field.
    pub fn new(fields: Vec<AnyField>, values: Vec<AnyScalar>) -> Self {
        Self {
            fields,
            values,
            null: false,
        }
    }

    /// A null struct value carrying its (logically-absent) per-field values.
    pub fn null(fields: Vec<AnyField>, values: Vec<AnyScalar>) -> Self {
        Self {
            fields,
            values,
            null: true,
        }
    }

    /// The number of fields.
    pub fn num_fields(&self) -> usize {
        self.fields.len()
    }

    /// The child field descriptors, in order.
    pub fn fields(&self) -> &[AnyField] {
        &self.fields
    }

    /// The field descriptor at `index`, or `None` if out of range.
    pub fn field(&self, index: usize) -> Option<&AnyField> {
        self.fields.get(index)
    }

    /// The value at `index`, or `None` if out of range.
    pub fn value(&self, index: usize) -> Option<&AnyScalar> {
        self.values.get(index)
    }

    /// The value of the field named `name` (first match), or `None`.
    pub fn value_named(&self, name: &str) -> Option<&AnyScalar> {
        let index = self.fields.iter().position(|f| f.name() == name)?;
        self.values.get(index)
    }

    /// Whether the struct value is null.
    pub fn is_null(&self) -> bool {
        self.null
    }

    /// The element [`DataTypeId`] — always [`Struct`](DataTypeId::Struct).
    pub fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    /// The typed [`StructType`] descriptor of this value.
    pub fn data_type(&self) -> StructType {
        StructType::new(self.fields.clone())
    }
}

impl PartialEq for StructScalar {
    fn eq(&self, other: &Self) -> bool {
        if self.null != other.null || self.fields != other.fields {
            return false;
        }
        // A null struct's per-field values are logically absent, so they do not affect identity.
        self.null || self.values == other.values
    }
}

impl Eq for StructScalar {}

impl core::hash::Hash for StructScalar {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.fields.hash(state);
        self.null.hash(state);
        if !self.null {
            self.values.hash(state);
        }
    }
}

impl ScalarType for StructScalar {
    type Data = StructType;

    fn data_type(&self) -> StructType {
        self.data_type()
    }

    fn is_null(&self) -> bool {
        self.null
    }
}
