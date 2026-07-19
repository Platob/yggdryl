//! [`VarScalar`] — a **single variable-length value** (the one-element [`Scalar`] for a byte-blob
//! type). Unlike the fixed [`FixedScalar`](crate::typed::FixedScalar), which views a buffer, a
//! variable-length scalar simply owns its value.

use core::marker::PhantomData;

use crate::datatype_id::DataTypeId;
use crate::typed::{Scalar, VarType};

/// A single, possibly-null variable-length value (`Vec<u8>` for binary, `String` for UTF-8).
pub struct VarScalar<T: VarType> {
    value: Option<T::Owned>,
    _type: PhantomData<T>,
}

impl<T: VarType> VarScalar<T> {
    /// A **non-null** scalar holding `value`.
    pub fn of(value: T::Owned) -> Self {
        VarScalar {
            value: Some(value),
            _type: PhantomData,
        }
    }

    /// A **null** scalar.
    pub fn null() -> Self {
        VarScalar {
            value: None,
            _type: PhantomData,
        }
    }

    /// A scalar from an option.
    pub fn from_option(value: Option<T::Owned>) -> Self {
        VarScalar {
            value,
            _type: PhantomData,
        }
    }

    /// The value, or `None` when null.
    pub fn value(&self) -> Option<&T::Owned> {
        self.value.as_ref()
    }

    /// Consumes the scalar into its optional value.
    pub fn into_value(self) -> Option<T::Owned> {
        self.value
    }
}

impl<T: VarType> Scalar for VarScalar<T>
where
    T::Owned: Clone,
{
    type Value = T::Owned;

    fn data_type_id(&self) -> DataTypeId {
        T::DATA_TYPE_ID
    }

    fn len(&self) -> usize {
        1
    }

    fn is_valid(&self, index: usize) -> bool {
        index == 0 && self.value.is_some()
    }

    fn get(&self, index: usize) -> Option<T::Owned> {
        if index == 0 {
            self.value.clone()
        } else {
            None
        }
    }
}
