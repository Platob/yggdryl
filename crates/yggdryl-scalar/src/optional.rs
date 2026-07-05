//! The dynamic [`OptionalScalar`] scalar of the
//! [`OptionalType`](yggdryl_dtype::OptionalType) data type.

use crate::Scalar;
use arrow_array::ArrayRef;
use yggdryl_dtype::{DataError, DataType, Logical, OptionalType, Union, UnionType};

/// A single value of the [`OptionalType`](yggdryl_dtype::OptionalType) with its value
/// type erased — the null-or-value union variant, backed by an optional one-element
/// Arrow value array, carrying a dynamic [`OptionalType`](yggdryl_dtype::OptionalType).
///
/// It is the untyped base of the statically-typed
/// [`TypedOptionalScalar<D, S>`](crate::TypedOptionalScalar): it implements only the
/// base [`Scalar`] surface ([`to_arrow_scalar`](Scalar::to_arrow_scalar) /
/// [`to_arrow_array`](Scalar::to_arrow_array) / [`from_arrow`](Scalar::from_arrow), all
/// reference-count bumps), since the value scalar type is erased — the value-redirecting
/// `as_*` accessors and the [`TypedScalar`](crate::TypedScalar) surface live on
/// `TypedOptionalScalar<D, S>`, which [`erase`](crate::TypedOptionalScalar::erase)s back
/// to this type. The erased base cannot reach the inner scalar's typed contract, so its
/// `as_*` accessors default to their error.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataType, Int64Type};
/// use yggdryl_scalar::{Int64Scalar, OptionalScalar, Scalar, TypedOptionalScalar};
///
/// // A dynamic optional is reached by erasing a typed one, or from Arrow.
/// let missing = TypedOptionalScalar::<Int64Type, Int64Scalar>::null().erase();
/// assert!(missing.is_null());
/// assert_eq!(missing.data_type().name(), "optional");
///
/// let answer = TypedOptionalScalar::new(Int64Scalar::new(42)).erase();
/// assert!(!answer.is_null());
/// assert_eq!(OptionalScalar::from_arrow(answer.to_arrow_scalar().into_inner().as_ref()).unwrap(), answer);
/// ```
#[derive(Debug, Clone)]
pub struct OptionalScalar {
    data_type: OptionalType,
    value: Option<ArrayRef>,
}

impl OptionalScalar {
    /// A dynamic optional over an already-built one-element Arrow `value` array of
    /// the value variant (shared zero-copy) of the given dynamic `data_type`, or the
    /// null variant for `None`.
    pub(crate) fn from_parts(data_type: OptionalType, value: Option<ArrayRef>) -> Self {
        Self { data_type, value }
    }
}

impl PartialEq for OptionalScalar {
    // The backing value arrays compare by value through `dyn Array` equality, so two
    // optionals are equal when their value variants (or both null) are; the null
    // variant is distinct from every present value.
    fn eq(&self, other: &Self) -> bool {
        match (&self.value, &other.value) {
            (None, None) => true,
            (Some(left), Some(right)) => left.as_ref() == right.as_ref(),
            _ => false,
        }
    }
}

impl Eq for OptionalScalar {}

impl Scalar for OptionalScalar {
    type DataType = OptionalType;
    type Value = dyn arrow_array::Array;

    fn data_type(&self) -> &OptionalType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }

    fn value(&self) -> Option<&(dyn arrow_array::Array + 'static)> {
        self.value.as_deref()
    }

    fn to_arrow_scalar(&self) -> arrow_array::Scalar<ArrayRef> {
        let storage = Logical::storage(&self.data_type);
        let (_, value_field) = storage
            .fields()
            .iter()
            .find(|(id, _)| *id == UnionType::VALUE_TYPE_ID)
            .expect("an optional union has a value variant");
        let type_id = if self.is_null() {
            UnionType::NULL_TYPE_ID
        } else {
            UnionType::VALUE_TYPE_ID
        };
        // Sparse layout: both children are one element long; the unselected child
        // holds a null.
        let value_child = self
            .value
            .clone()
            .unwrap_or_else(|| arrow_array::new_null_array(value_field.data_type(), 1));
        let children = vec![
            std::sync::Arc::new(arrow_array::NullArray::new(1)) as ArrayRef,
            value_child,
        ];
        let array = arrow_array::UnionArray::try_new(
            storage.fields().clone(),
            vec![type_id].into(),
            None, // sparse
            children,
        )
        .expect("a one-element sparse union of the declared fields is valid");
        arrow_array::Scalar::new(std::sync::Arc::new(array))
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the union layout; the value child is shared
        // zero-copy, its element type still carried only as the union's Arrow field.
        let data_type = OptionalType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::UnionArray>()
            .expect("a value with a union data type is a union array");
        let value = if array.type_id(0) == UnionType::NULL_TYPE_ID {
            None
        } else {
            Some(array.value(0))
        };
        Ok(Self { data_type, value })
    }
}
