//! The [`TypedMapField`] field.

use crate::{Field, FieldFactory, TypedField};
use yggdryl_dtype::{DataError, DataType, TypedDataType, TypedMapType};

/// A nullable, statically-typed `map` field: a name paired with the
/// [`TypedMapType`](yggdryl_dtype::TypedMapType) from the key type `K` to the value
/// type `V`.
///
/// It is the typed counterpart of the dynamic [`MapField`](crate::MapField): it
/// carries both trait layers — the raw [`Field`] surface (its associated
/// [`DataType`](Field::DataType) is [`TypedMapType<K, V>`](TypedMapType)) and the
/// typed [`TypedField<TypedMapType<K, V>, Vec<(TK, TV)>>`] whenever both types have
/// codecs.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, TypedMap, TypedMapType, UInt8Type};
/// use yggdryl_field::{Field, FieldFactory, TypedMapField};
///
/// let ranks = TypedMapField::<UInt8Type, Int64Type>::new("ranks", true);
/// assert_eq!(ranks.name(), "ranks");
/// assert_eq!(ranks.data_type().name(), "map");
/// assert_eq!(ranks.data_type().key_type().name(), "uint8");
/// assert!(ranks.is_nullable());
/// assert_eq!(TypedMapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
/// assert_eq!(TypedMapType::new(UInt8Type, Int64Type).field("ranks", true), ranks);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedMapField<K, V> {
    name: String,
    data_type: TypedMapType<K, V>,
    nullable: bool,
}

impl<K: DataType + Default, V: DataType + Default> TypedMapField<K, V> {
    /// A `map` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: TypedMapType::default(),
            nullable,
        }
    }
}

impl<K: DataType, V: DataType> Field for TypedMapField<K, V> {
    type DataType = TypedMapType<K, V>;
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &TypedMapType<K, V> {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        let data_type = TypedMapType::from_arrow(field.data_type())?;
        crate::field::validate_field_metadata(field, "TypedMapType")?;
        Ok(Self {
            name: field.name().to_string(),
            data_type,
            nullable: field.is_nullable(),
        })
    }
}

impl<TK, TV, K: TypedDataType<TK>, V: TypedDataType<TV>>
    TypedField<TypedMapType<K, V>, Vec<(TK, TV)>> for TypedMapField<K, V>
{
}

impl<TK, TV, K: TypedDataType<TK> + Default, V: TypedDataType<TV> + Default>
    FieldFactory<Vec<(TK, TV)>> for TypedMapType<K, V>
{
    type Field = TypedMapField<K, V>;
    fn field(&self, name: impl Into<String>, nullable: bool) -> TypedMapField<K, V> {
        TypedMapField::new(name, nullable)
    }
}
