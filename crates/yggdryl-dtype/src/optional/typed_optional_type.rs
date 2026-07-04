//! The [`TypedOptionalType`] data type.

use std::sync::OnceLock;

use crate::{DataError, DataType, Logical, OptionalType, TypedDataType, UnionType};

/// The statically-typed [`OptionalType`](crate::OptionalType): a value of a value
/// type `D` known at compile time, or null.
///
/// Where the dynamic [`OptionalType`](crate::OptionalType) carries its value type
/// only as the storage union's Arrow field, `TypedOptionalType<D>` keeps the concrete
/// value type `D`, so it adds the [`TypedOptional`](crate::TypedOptional) surface —
/// the value-type accessor and the [`TypedDataType<T>`] byte codec. The typed codec
/// is the *value type's* codec, so a `TypedOptionalType<Int64Type>` reads and writes
/// plain `i64` bytes (while [`byte_width`](DataType::byte_width) stays the union
/// storage's, `None`). [`erase`](TypedOptionalType::erase) drops the static type back
/// to a dynamic [`OptionalType`](crate::OptionalType).
///
/// The storage union is a pure function of the value type, so it is built lazily on
/// first use and plays no part in equality.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Logical, OptionalType, TypedDataType, TypedOptional, TypedOptionalType};
///
/// let optional = TypedOptionalType::new(Int64Type);
/// assert_eq!(optional.name(), "optional");
/// assert_eq!(optional.value_type().name(), "int64");
///
/// // Physically the null-or-int64 union...
/// assert_eq!(optional.storage().name(), "union");
/// assert_eq!(optional.arrow_format(), "+us:0,1");
///
/// // ...while the typed codec is the value type's.
/// assert_eq!(optional.native_to_bytes(&42), Int64Type.native_to_bytes(&42));
/// assert_eq!(optional.native_from_bytes(&[0xFF; 8]).unwrap(), -1);
///
/// // Erase to the dynamic optional; from_arrow is the exact inverse of to_arrow.
/// assert_eq!(optional.erase(), OptionalType::from_arrow(&optional.to_arrow()).unwrap());
/// assert_eq!(TypedOptionalType::<Int64Type>::from_arrow(&optional.to_arrow()).unwrap(), optional);
/// ```
#[derive(Debug)]
pub struct TypedOptionalType<D> {
    value_type: D,
    storage: OnceLock<UnionType>,
}

impl<D: DataType> TypedOptionalType<D> {
    /// The optional of `value_type`.
    pub fn new(value_type: D) -> Self {
        Self {
            value_type,
            storage: OnceLock::new(),
        }
    }

    /// Drop the static value type, returning the dynamic [`OptionalType`].
    pub fn erase(&self) -> OptionalType {
        OptionalType::new(&self.value_type)
    }
}

impl<D: DataType> super::Optional for TypedOptionalType<D> {}

impl<T, D: TypedDataType<T>> super::TypedOptional<T> for TypedOptionalType<D> {
    type ValueType = D;

    fn value_type(&self) -> &D {
        &self.value_type
    }
}

impl<T, D: TypedDataType<T>> crate::TypedLogical<T> for TypedOptionalType<D> {}

impl<D: DataType + Default> Default for TypedOptionalType<D> {
    fn default() -> Self {
        Self::new(D::default())
    }
}

impl<D: Clone> Clone for TypedOptionalType<D> {
    fn clone(&self) -> Self {
        Self {
            value_type: self.value_type.clone(),
            storage: self.storage.clone(),
        }
    }
}

impl<D: PartialEq> PartialEq for TypedOptionalType<D> {
    // The storage union is a function of the value type, so equality is the value
    // type alone.
    fn eq(&self, other: &Self) -> bool {
        self.value_type == other.value_type
    }
}

impl<D: Eq> Eq for TypedOptionalType<D> {}

impl<D: DataType> DataType for TypedOptionalType<D> {
    fn name(&self) -> &str {
        "optional"
    }

    fn arrow_format(&self) -> String {
        self.storage().arrow_format()
    }

    fn byte_width(&self) -> Option<usize> {
        self.storage().byte_width()
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        self.storage().to_arrow()
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        // Reuse the dynamic optional's structural validation, then decode the value
        // and confirm its variant is named after the value type.
        let dynamic = OptionalType::from_arrow(data_type)?;
        let value_field = crate::Optional::value_field(&dynamic);
        let value_type = D::from_arrow(value_field.data_type())?;
        if value_field.name() != value_type.name() {
            return Err(DataError::IncompatibleArrowType {
                expected: "a sparse union of a null variant and a value variant (type ids 0 and 1)"
                    .to_string(),
                got: data_type.to_string(),
            });
        }
        Ok(Self::new(value_type))
    }
}

impl<T, D: TypedDataType<T>> TypedDataType<T> for TypedOptionalType<D> {
    fn native_to_bytes(&self, value: &T) -> Vec<u8> {
        self.value_type.native_to_bytes(value)
    }

    fn native_from_bytes(&self, bytes: &[u8]) -> Result<T, DataError> {
        self.value_type.native_from_bytes(bytes)
    }

    // The codec is the value type's, so its width is too (the physical
    // `byte_width` is the union storage's, `None`).
    fn codec_byte_width(&self) -> Option<usize> {
        self.value_type.codec_byte_width()
    }

    fn default_value(&self) -> T {
        self.value_type.default_value()
    }
}

impl<D: DataType> Logical for TypedOptionalType<D> {
    type Storage = UnionType;

    fn storage(&self) -> &UnionType {
        self.storage
            .get_or_init(|| UnionType::optional(&self.value_type))
    }
}
