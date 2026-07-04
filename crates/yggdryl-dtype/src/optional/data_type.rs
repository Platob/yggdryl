//! The [`OptionalType`] data type.

use crate::{DataError, DataType, Logical, UnionType};

/// The logical `optional` data type: a value of some value type, or null —
/// physically stored as the sparse two-variant [`UnionType`] between
/// [`NullType`](crate::NullType) and the value type ([`UnionType::optional`]).
///
/// It is the first concrete logical type ([`Logical`]): [`storage`](Logical::storage)
/// returns the backing [`UnionType`], and the Arrow surface delegates to it
/// (`arrow_format` / `to_arrow` describe the union — Arrow has no separate "optional"
/// type, so this type has no [`DataTypeId`](crate::DataTypeId)). It carries its value
/// type only as the union's Arrow field, so it stays *untyped*; a statically-typed
/// optional carrying the value type's byte codec is
/// [`TypedOptionalType<D>`](crate::TypedOptionalType), whose
/// [`erase`](crate::TypedOptionalType::erase) drops back to this dynamic type.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Logical, Optional, OptionalType};
///
/// let optional = OptionalType::new(&Int64Type);
/// assert_eq!(optional.name(), "optional");
/// assert_eq!(optional.value_field().name(), "int64");
///
/// // Physically the null-or-int64 union.
/// assert_eq!(optional.storage().name(), "union");
/// assert_eq!(optional.arrow_format(), "+us:0,1");
/// assert_eq!(optional.byte_width(), None);
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert_eq!(OptionalType::from_arrow(&optional.to_arrow()).unwrap(), optional);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionalType {
    storage: UnionType,
}

impl OptionalType {
    /// The optional of `value_type`, storing it as the sparse null-or-value
    /// [`UnionType`].
    pub fn new(value_type: &dyn DataType) -> Self {
        Self {
            storage: UnionType::optional(value_type),
        }
    }
}

impl super::Optional for OptionalType {}

impl Logical for OptionalType {
    type Storage = UnionType;

    fn storage(&self) -> &UnionType {
        &self.storage
    }
}

impl DataType for OptionalType {
    fn name(&self) -> &str {
        "optional"
    }

    fn arrow_format(&self) -> String {
        self.storage.arrow_format()
    }

    fn byte_width(&self) -> Option<usize> {
        self.storage.byte_width()
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        self.storage.to_arrow()
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        let incompatible = || DataError::IncompatibleArrowType {
            expected: "a sparse union of a null variant and a value variant (type ids 0 and 1)"
                .to_string(),
            got: data_type.to_string(),
        };
        let arrow_schema::DataType::Union(fields, arrow_schema::UnionMode::Sparse) = data_type
        else {
            return Err(incompatible());
        };
        let mut children = fields.iter();
        let (Some((null_id, null_field)), Some((value_id, value_field)), None) =
            (children.next(), children.next(), children.next())
        else {
            return Err(incompatible());
        };
        if null_id != UnionType::NULL_TYPE_ID
            || value_id != UnionType::VALUE_TYPE_ID
            || null_field.name() != "null"
            || !null_field.is_nullable()
            || null_field.data_type() != &arrow_schema::DataType::Null
            || !null_field.metadata().is_empty()
            || value_field.is_nullable()
            || !value_field.metadata().is_empty()
        {
            return Err(incompatible());
        }
        Ok(Self {
            storage: UnionType::new(fields.clone(), arrow_schema::UnionMode::Sparse),
        })
    }
}
