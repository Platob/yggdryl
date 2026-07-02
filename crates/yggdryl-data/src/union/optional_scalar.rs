//! The [`OptionalScalar`] scalar: a [`Union`](super::Union) between null and a
//! value type.

use std::marker::PhantomData;
use std::sync::OnceLock;

use super::Union;
use crate::{DataError, RawDataType, RawScalar, Scalar};

/// Whether `data_type` is exactly the layout [`Union::optional`] produces for a
/// value type of arrow type `value_type` named `value_name` — checked structurally,
/// without building the expected [`Union`] (the hot path of `from_arrow`).
fn is_optional_layout(
    data_type: &arrow_schema::DataType,
    value_type: &arrow_schema::DataType,
    value_name: &str,
) -> bool {
    let arrow_schema::DataType::Union(fields, arrow_schema::UnionMode::Sparse) = data_type else {
        return false;
    };
    let mut children = fields.iter();
    let (Some((null_id, null_field)), Some((value_id, value_field)), None) =
        (children.next(), children.next(), children.next())
    else {
        return false;
    };
    null_id == Union::NULL_TYPE_ID
        && value_id == Union::VALUE_TYPE_ID
        && null_field.name() == "null"
        && null_field.is_nullable()
        && null_field.data_type() == &arrow_schema::DataType::Null
        && null_field.metadata().is_empty()
        && value_field.name() == value_name
        && !value_field.is_nullable()
        && value_field.data_type() == value_type
        && value_field.metadata().is_empty()
}

/// A single value of the two-variant [`Union`] between [`Null`](crate::Null) and a
/// value type — an inner scalar `S` of data type `D`, or the null variant.
///
/// Where a plain scalar (e.g. [`Int64Scalar`](crate::Int64Scalar)) models nullness
/// as a missing value of its own type, an `OptionalScalar` models it as a *union
/// variant*: its data type is [`Union::optional`]`(&D)` and its Arrow form is a
/// one-element `UnionArray` whose type id selects the null or the value child.
/// Access redirects to the inner scalar: [`value`](RawScalar::value) and every
/// `as_*` accessor answer through `S`.
///
/// ```
/// use yggdryl_data::{Int64, Int64Scalar, OptionalScalar, RawDataType, RawScalar};
///
/// let answer = OptionalScalar::new(Int64Scalar::new(42));
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
/// assert_eq!(answer.as_i64(), Some(42)); // redirected to the inner scalar
/// assert_eq!(answer.data_type().name(), "union");
/// assert_eq!(answer.data_type().arrow_format(), "+us:0,1");
///
/// let missing: OptionalScalar<Int64, Int64Scalar> = OptionalScalar::null();
/// assert!(missing.is_null());
/// assert_eq!(missing.as_i64(), None);
///
/// // The Arrow form is a one-element union array, and from_arrow redirects the
/// // value child back through the inner scalar's own from_arrow.
/// let arrow = answer.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(OptionalScalar::from_arrow(arrow.as_ref()).unwrap(), answer);
/// ```
#[derive(Debug)]
pub struct OptionalScalar<D, S> {
    // Fully determined by `D` (always `Union::optional(&D::default())`), so it is
    // built lazily on first access — construction stays allocation-free — and
    // plays no part in equality.
    data_type: OnceLock<Union>,
    value: Option<S>,
    value_type: PhantomData<D>,
}

impl<D: RawDataType + Default, S: RawScalar<D>> OptionalScalar<D, S> {
    /// A scalar holding the value variant `scalar`. A null inner scalar still
    /// [`is_null`](RawScalar::is_null) — the two representations agree.
    pub fn new(scalar: S) -> Self {
        Self {
            data_type: OnceLock::new(),
            value: Some(scalar),
            value_type: PhantomData,
        }
    }

    /// The null variant.
    pub fn null() -> Self {
        Self {
            data_type: OnceLock::new(),
            value: None,
            value_type: PhantomData,
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

impl<D, S: Clone> Clone for OptionalScalar<D, S> {
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            value: self.value.clone(),
            value_type: PhantomData,
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

impl<D: RawDataType + Default, S: RawScalar<D>> RawScalar<Union> for OptionalScalar<D, S> {
    type Value = S::Value;

    fn data_type(&self) -> &Union {
        self.data_type
            .get_or_init(|| Union::optional(&D::default()))
    }

    fn is_null(&self) -> bool {
        self.value.as_ref().is_none_or(|scalar| scalar.is_null())
    }

    fn value(&self) -> Option<&S::Value> {
        self.value.as_ref().and_then(|scalar| scalar.value())
    }

    fn to_arrow(&self) -> arrow_array::ArrayRef {
        let data_type = self.data_type();
        let (_, value_field) = data_type
            .fields()
            .iter()
            .find(|(id, _)| *id == Union::VALUE_TYPE_ID)
            .expect("an optional union has a value variant");
        let type_id = if self.is_null() {
            Union::NULL_TYPE_ID
        } else {
            Union::VALUE_TYPE_ID
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
            data_type.fields().clone(),
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
        let value_type = D::default();
        if !is_optional_layout(
            arrow_array::Array::data_type(array),
            &value_type.to_arrow(),
            value_type.name(),
        ) {
            return Err(DataError::IncompatibleArrowType {
                // Built only on the error path — the check itself is allocation-free.
                expected: Union::optional(&value_type).to_arrow().to_string(),
                got: arrow_array::Array::data_type(array).to_string(),
            });
        }
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::UnionArray>()
            .expect("a value with a union data type is a union array");
        if array.type_id(0) == Union::NULL_TYPE_ID {
            return Ok(Self::null());
        }
        // The value variant redirects to the inner scalar's own from_arrow, on the
        // one-element slice of the selected child.
        let value = array.value(0);
        Ok(Self::new(S::from_arrow(value.as_ref())?))
    }

    fn as_i8(&self) -> Option<i8> {
        self.value.as_ref().and_then(|scalar| scalar.as_i8())
    }
    fn as_i16(&self) -> Option<i16> {
        self.value.as_ref().and_then(|scalar| scalar.as_i16())
    }
    fn as_i32(&self) -> Option<i32> {
        self.value.as_ref().and_then(|scalar| scalar.as_i32())
    }
    fn as_i64(&self) -> Option<i64> {
        self.value.as_ref().and_then(|scalar| scalar.as_i64())
    }
    fn as_u8(&self) -> Option<u8> {
        self.value.as_ref().and_then(|scalar| scalar.as_u8())
    }
    fn as_u16(&self) -> Option<u16> {
        self.value.as_ref().and_then(|scalar| scalar.as_u16())
    }
    fn as_u32(&self) -> Option<u32> {
        self.value.as_ref().and_then(|scalar| scalar.as_u32())
    }
    fn as_u64(&self) -> Option<u64> {
        self.value.as_ref().and_then(|scalar| scalar.as_u64())
    }
    fn as_f32(&self) -> Option<f32> {
        self.value.as_ref().and_then(|scalar| scalar.as_f32())
    }
    fn as_f64(&self) -> Option<f64> {
        self.value.as_ref().and_then(|scalar| scalar.as_f64())
    }
    fn as_bool(&self) -> Option<bool> {
        self.value.as_ref().and_then(|scalar| scalar.as_bool())
    }
    fn as_str(&self) -> Option<&str> {
        self.value.as_ref().and_then(|scalar| scalar.as_str())
    }
}

impl<D: RawDataType + Default, S: RawScalar<D>> Scalar<<S as RawScalar<D>>::Value>
    for OptionalScalar<D, S>
{
    type Type = Union;
}
