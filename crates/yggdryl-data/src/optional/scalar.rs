//! The [`OptionalScalar`] scalar of the [`OptionalType`](super::OptionalType) data type.

use super::OptionalType;
use crate::{DataError, RawDataType, RawLogical, RawScalar, RawUnion, Scalar, UnionType};

/// A single value of the [`OptionalType`] of the value type `D` — an inner scalar `S`,
/// or the null variant.
///
/// Where a plain scalar (e.g. [`Int64Scalar`](crate::Int64Scalar)) models nullness
/// as a missing value of its own type, an `OptionalScalar` models it as a *union
/// variant*: its data type is the logical [`OptionalType<D>`](OptionalType), whose storage
/// is the sparse null-or-value [`UnionType`], and its Arrow form is a one-element
/// `UnionArray` whose type id selects the null or the value child. Access redirects
/// to the inner scalar: [`value`](RawScalar::value) and every `as_*` accessor
/// answer through `S` — so a conversion error names the *value type* actually
/// holding the value (``int64 scalars have no str conversion``), while the null
/// variant errors with [`DataError::NullValue`]. A null inner scalar *normalizes to the null variant* — the
/// two representations of null are one state, so equality,
/// [`scalar`](OptionalScalar::scalar) (which answers `None` for it) and the Arrow
/// round trip all agree.
///
/// ```
/// use yggdryl_data::{Int64, Int64Scalar, OptionalScalar, RawDataType, RawLogical, RawScalar};
///
/// let answer = OptionalScalar::new(Int64Scalar::new(42));
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
/// assert_eq!(answer.as_i64().unwrap(), 42); // redirected to the inner scalar
/// assert_eq!(answer.data_type().name(), "optional");
/// assert_eq!(answer.data_type().storage().name(), "union");
/// assert_eq!(answer.data_type().arrow_format(), "+us:0,1");
///
/// let missing: OptionalScalar<Int64, Int64Scalar> = OptionalScalar::null();
/// assert!(missing.is_null());
/// assert!(missing.as_i64().is_err()); // a null holds no value
///
/// // The Arrow form is a one-element union array, and from_arrow redirects the
/// // value child back through the inner scalar's own from_arrow.
/// let arrow = answer.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(OptionalScalar::from_arrow(arrow.as_ref()).unwrap(), answer);
/// ```
#[derive(Debug)]
pub struct OptionalScalar<D, S> {
    data_type: OptionalType<D>,
    value: Option<S>,
}

impl<D: RawDataType + Default, S: RawScalar<D>> OptionalScalar<D, S> {
    /// A scalar holding the value variant `scalar`. A null inner scalar
    /// *normalizes to the null variant* — the two representations of null are one
    /// state, so equality, [`scalar`](OptionalScalar::scalar) (which then answers
    /// `None`) and the Arrow round trip all agree.
    pub fn new(scalar: S) -> Self {
        Self {
            data_type: OptionalType::default(),
            value: (!scalar.is_null()).then_some(scalar),
        }
    }

    /// The null variant.
    pub fn null() -> Self {
        Self {
            data_type: OptionalType::default(),
            value: None,
        }
    }

    /// The inner scalar, when this holds the value variant.
    pub fn scalar(&self) -> Option<&S> {
        self.value.as_ref()
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Default for OptionalScalar<D, S> {
    fn default() -> Self {
        Self::null()
    }
}

impl<D: Clone, S: Clone> Clone for OptionalScalar<D, S> {
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            value: self.value.clone(),
        }
    }
}

impl<D, S: PartialEq> PartialEq for OptionalScalar<D, S> {
    // The data type is a function of `D`, identical for every instance, so
    // equality is the value alone.
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<D, S: Eq> Eq for OptionalScalar<D, S> {}

impl<D: RawDataType + Default, S: RawScalar<D>> From<S> for OptionalScalar<D, S> {
    /// A scalar holding the value variant `scalar`.
    fn from(scalar: S) -> Self {
        Self::new(scalar)
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> From<Option<S>> for OptionalScalar<D, S> {
    /// A scalar holding the value variant, or the null variant for `None`.
    fn from(scalar: Option<S>) -> Self {
        match scalar {
            Some(scalar) => Self::new(scalar),
            None => Self::null(),
        }
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> RawScalar<OptionalType<D>>
    for OptionalScalar<D, S>
{
    type Value = S::Value;

    fn data_type(&self) -> &OptionalType<D> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.value.as_ref().is_none_or(|scalar| scalar.is_null())
    }

    fn value(&self) -> Option<&S::Value> {
        self.value.as_ref().and_then(|scalar| scalar.value())
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        let storage = self.data_type.storage();
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
        let value_child = match &self.value {
            Some(scalar) if !scalar.is_null() => scalar.to_arrow(),
            _ => arrow_array::new_null_array(value_field.data_type(), 1),
        };
        let children = vec![
            std::sync::Arc::new(arrow_array::NullArray::new(1)) as arrow_array::ArrayRef,
            value_child,
        ];
        let array = arrow_array::UnionArray::try_new(
            storage.fields().clone(),
            vec![type_id].into(),
            None, // sparse
            children,
        )
        .expect("a one-element sparse union of the declared fields is valid");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout and redirects the value child's type
        // to `D`; then the value child itself redirects to `S`.
        let data_type = OptionalType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::UnionArray>()
            .expect("a value with a union data type is a union array");
        let value = if array.type_id(0) == UnionType::NULL_TYPE_ID {
            None
        } else {
            let value = array.value(0);
            let scalar = S::from_arrow(value.as_ref())?;
            (!scalar.is_null()).then_some(scalar)
        };
        Ok(Self { data_type, value })
    }

    fn as_i8(&self) -> Result<i8, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_i8()
    }
    fn as_i16(&self) -> Result<i16, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_i16()
    }
    fn as_i32(&self) -> Result<i32, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_i32()
    }
    fn as_i64(&self) -> Result<i64, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_i64()
    }
    fn as_u8(&self) -> Result<u8, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_u8()
    }
    fn as_u16(&self) -> Result<u16, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_u16()
    }
    fn as_u32(&self) -> Result<u32, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_u32()
    }
    fn as_u64(&self) -> Result<u64, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_u64()
    }
    fn as_f32(&self) -> Result<f32, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_f32()
    }
    fn as_f64(&self) -> Result<f64, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_f64()
    }
    fn as_bool(&self) -> Result<bool, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_bool()
    }
    fn as_str(&self) -> Result<&str, DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_str()
    }
    fn as_bytes(&self) -> Result<&[u8], DataError> {
        self.value.as_ref().ok_or(DataError::NullValue)?.as_bytes()
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Scalar<<S as RawScalar<D>>::Value>
    for OptionalScalar<D, S>
{
    type Type = OptionalType<D>;
}
