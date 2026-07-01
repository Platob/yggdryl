//! The [`Struct`] nested scalar.

use crate::{Any, Scalar};
use yggdryl_schema::{
    Any as AnyValue, AnyField, AnyType, Field, Struct as StructValue, StructField,
};

/// A struct scalar — a row built from a **collection** of child [`Any`] scalars. It
/// pairs a [`StructField`] (the child fields) with a [`Struct`](yggdryl_schema::Struct)
/// value (the child values), so it is a [`Scalar`] over that value and mirrors
/// [`StructField`](yggdryl_schema::StructField). Because a child scalar can itself be a
/// struct (via `From<Struct>` for [`Any`]), scalars nest recursively.
///
/// ```
/// use yggdryl_scalar::{Int32, Scalar, Struct};
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// let row = Struct::new(
///     "point",
///     vec![
///         Int32::from(1).with_name("x".to_string()).into(),
///         Int32::from(2).with_name("y".to_string()).into(),
///     ],
/// );
/// assert_eq!(row.name(), "point");
/// assert_eq!(row.len(), 2);
/// assert_eq!(row.scalars()[0].name(), "x");
/// assert_eq!(row.dtype().type_id(), DataTypeId::Struct);
/// // Navigate to a child scalar and read its atom.
/// assert_eq!(row.scalar_by("y").unwrap().as_i32(), Some(2));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Struct {
    field: StructField,
    value: StructValue,
}

impl Struct {
    /// A struct scalar named `name` from a collection of child scalars — its field is
    /// their fields, its value their values.
    pub fn new(name: impl Into<String>, scalars: Vec<Any>) -> Self {
        let fields = scalars.iter().map(|s| s.field().clone()).collect();
        let values = scalars.iter().map(|s| s.value().clone()).collect();
        Self {
            field: StructField::new(name, fields),
            value: StructValue::new(values),
        }
    }

    /// The scalar from its explicit field and value.
    pub fn from_parts(field: StructField, value: StructValue) -> Self {
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

    /// The number of child scalars.
    pub fn len(&self) -> usize {
        self.value.len()
    }

    /// Whether the struct holds no child scalars.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// The child scalars, rebuilt by pairing each child field with its value.
    pub fn scalars(&self) -> Vec<Any> {
        self.field
            .dtype()
            .fields()
            .iter()
            .zip(self.value.values())
            .map(|(field, value)| Any::from_parts(field.clone(), value.clone()))
            .collect()
    }

    /// The child scalar at `index`, if any — pairing the child field with its value.
    pub fn scalar_at(&self, index: usize) -> Option<Any> {
        let field = self.field.dtype().field_at(index)?;
        let value = self.value.get(index)?;
        Some(Any::from_parts(field.clone(), value.clone()))
    }

    /// The first child scalar named `name`, if any.
    pub fn scalar_by(&self, name: &str) -> Option<Any> {
        let index = self
            .field
            .dtype()
            .fields()
            .iter()
            .position(|field| field.name() == name)?;
        self.scalar_at(index)
    }
}

impl From<Vec<Any>> for Struct {
    fn from(scalars: Vec<Any>) -> Self {
        Self::new("", scalars)
    }
}

impl From<Struct> for Any {
    fn from(scalar: Struct) -> Self {
        let field = AnyField::from_parts(
            scalar.field.name().to_owned(),
            AnyType::struct_type(scalar.field.dtype().clone()),
            scalar.field.nullable(),
            scalar.field.metadata().cloned(),
        );
        Any::from_parts(field, AnyValue::Struct(scalar.value))
    }
}

impl Scalar<StructValue> for Struct {
    type Field = StructField;

    fn field(&self) -> &StructField {
        &self.field
    }

    fn value(&self) -> &StructValue {
        &self.value
    }
}
