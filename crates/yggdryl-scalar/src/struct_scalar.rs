//! The [`StructScalar`] nested scalar.

use crate::{AnyScalar, Scalar};
use yggdryl_schema::{Any, AnyField, AnyType, Field, Struct, StructField};

/// A struct scalar — a row built from a **collection** of child [`AnyScalar`]s. It
/// pairs a [`StructField`] (the child fields) with a [`Struct`] value (the child
/// values), so it is a [`Scalar`] over [`Struct`] and mirrors
/// [`StructField`](yggdryl_schema::StructField). Because a child scalar can itself be a
/// struct (via [`From<StructScalar>`](AnyScalar) for [`AnyScalar`]), scalars nest
/// recursively.
///
/// ```
/// use yggdryl_scalar::{Int32Scalar, Scalar, StructScalar};
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// let row = StructScalar::new(
///     "point",
///     vec![
///         Int32Scalar::from(1).with_name("x".to_string()).into(),
///         Int32Scalar::from(2).with_name("y".to_string()).into(),
///     ],
/// );
/// assert_eq!(row.name(), "point");
/// assert_eq!(row.value().len(), 2);
/// assert_eq!(row.scalars()[0].name(), "x");
/// assert_eq!(row.dtype().type_id(), DataTypeId::Struct);
/// // Navigate to a child scalar and read its atom.
/// assert_eq!(row.scalar_by("y").unwrap().as_i32(), Some(2));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StructScalar {
    field: StructField,
    value: Struct,
}

impl StructScalar {
    /// A struct scalar named `name` from a collection of child scalars — its field is
    /// their fields, its value their values.
    pub fn new(name: impl Into<String>, scalars: Vec<AnyScalar>) -> Self {
        let fields = scalars.iter().map(|s| s.field().clone()).collect();
        let values = scalars.iter().map(|s| s.value().clone()).collect();
        Self {
            field: StructField::new(name, fields),
            value: Struct::new(values),
        }
    }

    /// The scalar from its explicit field and value.
    pub fn from_parts(field: StructField, value: Struct) -> Self {
        Self { field, value }
    }

    /// A copy carrying `field`.
    pub fn with_field(&self, field: StructField) -> Self {
        Self {
            field,
            value: self.value.clone(),
        }
    }

    /// A copy renamed to `name`.
    pub fn with_name(&self, name: String) -> Self {
        self.with_field(self.field.with_name(name))
    }

    /// The child scalars, rebuilt by pairing each child field with its value.
    pub fn scalars(&self) -> Vec<AnyScalar> {
        self.field
            .dtype()
            .fields()
            .iter()
            .zip(self.value.values())
            .map(|(field, value)| AnyScalar::from_parts(field.clone(), value.clone()))
            .collect()
    }

    /// The child scalar at `index`, if any — pairing the child field with its value.
    pub fn scalar_at(&self, index: usize) -> Option<AnyScalar> {
        let field = self.field.dtype().field_at(index)?;
        let value = self.value.get(index)?;
        Some(AnyScalar::from_parts(field.clone(), value.clone()))
    }

    /// The first child scalar named `name`, if any.
    pub fn scalar_by(&self, name: &str) -> Option<AnyScalar> {
        let index = self
            .field
            .dtype()
            .fields()
            .iter()
            .position(|field| field.name() == name)?;
        self.scalar_at(index)
    }
}

impl From<Vec<AnyScalar>> for StructScalar {
    fn from(scalars: Vec<AnyScalar>) -> Self {
        Self::new("", scalars)
    }
}

impl From<StructScalar> for AnyScalar {
    fn from(scalar: StructScalar) -> Self {
        let field = AnyField::from_parts(
            scalar.field.name().to_owned(),
            AnyType::struct_type(scalar.field.dtype().clone()),
            scalar.field.nullable(),
            scalar.field.metadata().cloned(),
        );
        AnyScalar::from_parts(field, Any::Struct(scalar.value))
    }
}

impl Scalar<Struct> for StructScalar {
    type Field = StructField;

    fn field(&self) -> &StructField {
        &self.field
    }

    fn value(&self) -> &Struct {
        &self.value
    }
}
