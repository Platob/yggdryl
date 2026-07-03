//! The [`Optional`] data type.

use std::sync::OnceLock;

use crate::{DataError, DataType, RawDataType, RawLogical, Union};

/// The logical `optional` data type: a value of the value type `D`, or null —
/// physically stored as the sparse two-variant [`Union`] between
/// [`Null`](crate::Null) and `D` ([`Union::optional`]).
///
/// It is the first concrete logical type ([`RawLogical`] and, with a codec,
/// [`Logical`](crate::Logical)): [`storage`](RawLogical::storage) returns
/// the backing [`Union`], and the Arrow surface delegates to it (`arrow_format` /
/// `to_arrow` describe the union — Arrow has no separate "optional" type, so this
/// type has no [`DataTypeId`](crate::DataTypeId)). The typed layer delegates the
/// other way: the [`DataType<T>`] byte codec is the *value type's* codec, so an
/// `Optional<Int64>` reads and writes plain `i64` bytes.
///
/// The storage union is a pure function of the value type, so it is built lazily on
/// first use and plays no part in equality.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64, Optional, RawDataType, RawLogical, RawOptional};
///
/// let optional = Optional::new(Int64);
/// assert_eq!(optional.name(), "optional");
/// assert_eq!(optional.value_type().name(), "int64");
///
/// // Physically the null-or-int64 union...
/// assert_eq!(optional.storage().name(), "union");
/// assert_eq!(optional.arrow_format(), "+us:0,1");
/// assert_eq!(optional.byte_width(), None);
///
/// // ...while the typed codec is the value type's.
/// assert_eq!(optional.native_to_bytes(&42), Int64.native_to_bytes(&42));
/// assert_eq!(optional.native_from_bytes(&[0xFF; 8]).unwrap(), -1);
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert_eq!(Optional::<Int64>::from_arrow(&optional.to_arrow()).unwrap(), optional);
/// ```
#[derive(Debug)]
pub struct Optional<D> {
    value_type: D,
    storage: OnceLock<Union>,
}

impl<D: RawDataType> Optional<D> {
    /// The optional of `value_type`.
    pub fn new(value_type: D) -> Self {
        Self {
            value_type,
            storage: OnceLock::new(),
        }
    }
}

impl<D: RawDataType> super::RawOptional<D> for Optional<D> {
    fn value_type(&self) -> &D {
        &self.value_type
    }
}

impl<T, D: DataType<T>> crate::Logical<T> for Optional<D> {
    type Storage = Union;
}

impl<T, D: DataType<T>> super::TypedOptional<T> for Optional<D> {
    type ValueType = D;
}

impl<D: RawDataType + Default> Default for Optional<D> {
    fn default() -> Self {
        Self::new(D::default())
    }
}

impl<D: Clone> Clone for Optional<D> {
    fn clone(&self) -> Self {
        Self {
            value_type: self.value_type.clone(),
            storage: self.storage.clone(),
        }
    }
}

impl<D: PartialEq> PartialEq for Optional<D> {
    // The storage union is a function of the value type, so equality is the value
    // type alone.
    fn eq(&self, other: &Self) -> bool {
        self.value_type == other.value_type
    }
}

impl<D: Eq> Eq for Optional<D> {}

impl<D: RawDataType> RawDataType for Optional<D> {
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
        if null_id != Union::NULL_TYPE_ID
            || value_id != Union::VALUE_TYPE_ID
            || null_field.name() != "null"
            || !null_field.is_nullable()
            || null_field.data_type() != &arrow_schema::DataType::Null
            || !null_field.metadata().is_empty()
            || value_field.is_nullable()
            || !value_field.metadata().is_empty()
        {
            return Err(incompatible());
        }
        // The value child redirects to the value type's own from_arrow.
        let value_type = D::from_arrow(value_field.data_type())?;
        if value_field.name() != value_type.name() {
            return Err(incompatible());
        }
        Ok(Self::new(value_type))
    }
}

impl<T, D: DataType<T>> DataType<T> for Optional<D> {
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

impl<D: RawDataType> RawLogical<Union> for Optional<D> {
    fn storage(&self) -> &Union {
        self.storage
            .get_or_init(|| Union::optional(&self.value_type))
    }
}
