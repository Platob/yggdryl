//! [`StructScalar`] — one **struct value**: a nullable row of the struct's fields, each a
//! type-erased [`Value`](crate::io::nested::Value). It is what
//! [`StructSerie::get_row`](super::StructSerie::get_row) yields (wrapped in a
//! [`Value::Struct`](crate::io::nested::Value::Struct)).

use crate::io::nested::{ColumnField, Value};
use crate::io::{DataTypeId, ScalarType};

use super::StructType;

/// A single **struct value** — a row: the struct's schema (its ordered child
/// [`ColumnField`]s), one erased [`Value`] per field, and whether the struct value itself is null.
/// A null struct still carries its per-field values (Arrow keeps them under a null parent); they are
/// simply logically absent, so [`is_null`](StructScalar::is_null) is the authority.
///
/// It is a hashable value type: two struct values are equal iff they have the same schema and either
/// are both null, or hold equal per-field values. A **null** struct's phantom child values are
/// ignored (two same-schema null structs are equal, like `Scalar::null() == Scalar::null()`).
#[derive(Debug, Clone)]
pub struct StructScalar {
    fields: Vec<ColumnField>,
    values: Vec<Value>,
    null: bool,
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
        // Skip the logically-absent values under a null struct, so `eq` and `hash` agree.
        if !self.null {
            self.values.hash(state);
        }
    }
}

impl StructScalar {
    /// A present struct value from its schema fields and one [`Value`] per field.
    pub fn new(fields: Vec<ColumnField>, values: Vec<Value>) -> Self {
        Self {
            fields,
            values,
            null: false,
        }
    }

    /// A null struct value carrying its (logically-absent) per-field values.
    pub fn null(fields: Vec<ColumnField>, values: Vec<Value>) -> Self {
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
    pub fn fields(&self) -> &[ColumnField] {
        &self.fields
    }

    /// The field descriptor at `index`, or `None` if out of range.
    pub fn field(&self, index: usize) -> Option<&ColumnField> {
        self.fields.get(index)
    }

    /// The value at `index`, or `None` if out of range.
    pub fn value(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// The value of the field named `name` (first match), or `None`.
    pub fn value_named(&self, name: &str) -> Option<&Value> {
        let index = self.fields.iter().position(|f| f.name() == name)?;
        self.values.get(index)
    }

    /// Whether the struct value is null.
    pub fn is_null(&self) -> bool {
        self.null
    }

    /// The typed [`StructType`] descriptor of this value.
    pub fn data_type(&self) -> StructType {
        StructType::new(self.fields.clone())
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

impl StructScalar {
    /// The element [`DataTypeId`] — always [`Struct`](DataTypeId::Struct).
    pub fn type_id(&self) -> DataTypeId {
        DataTypeId::Struct
    }
}
