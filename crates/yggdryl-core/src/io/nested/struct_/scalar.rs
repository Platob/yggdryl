//! [`StructScalar`] — one **struct value**: a nullable row of the struct's fields, each a type-erased
//! [`AnyScalar`](crate::io::AnyScalar). It is what [`StructSerie::get_scalar`](super::StructSerie::get_scalar) yields.

use super::StructType;
use crate::io::field_carrier::field_accessors;
use crate::io::fixed::Field;
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
    /// The value's **own-header** field (`Struct` type_id) — its name, declared nullability, and
    /// metadata. Excluded from value identity (the child schema + per-field values are the identity).
    field: Field,
}

impl StructScalar {
    /// A present struct value from its schema fields and one [`AnyScalar`] per field.
    pub fn new(fields: Vec<AnyField>, values: Vec<AnyScalar>) -> Self {
        Self {
            fields,
            values,
            null: false,
            field: Field::of("", DataTypeId::Struct, 0, false),
        }
    }

    /// A null struct value carrying its (logically-absent) per-field values.
    pub fn null(fields: Vec<AnyField>, values: Vec<AnyScalar>) -> Self {
        Self {
            fields,
            values,
            null: true,
            field: Field::of("", DataTypeId::Struct, 0, false),
        }
    }

    field_accessors!();

    /// The erased [`AnyField`] this struct value contributes — a `Struct` field over its child
    /// fields, with **effective** nullability `self.nullable() || self.is_null()` and held metadata.
    ///
    /// DESIGN: this is `into_field` (consuming) rather than a no-arg `field()` because
    /// [`field`](StructScalar::field) is the by-index child-field accessor (a struct scalar is
    /// unreconstructable without its child names), mirroring [`StructSerie`](super::StructSerie).
    pub fn into_field(self) -> AnyField {
        let nullable = self.nullable() || self.null;
        AnyField::struct_(self.field.name(), self.fields, nullable)
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
