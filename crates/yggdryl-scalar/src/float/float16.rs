//! The [`Float16Scalar`] scalar.
//!
//! A single, possibly-null `float16` value (native [`half::f16`](yggdryl_dtype::half::f16))
//! of the [`Float16Type`](yggdryl_dtype::Float16Type) data type.
//!
//! Unlike [`Float32Scalar`](crate::Float32Scalar) / [`Float64Scalar`](crate::Float64Scalar),
//! `float16` is not a Rust primitive — `half::f16` converts through `.to_f32()` /
//! `from_f64()` rather than `as` casts — so this scalar is written out by hand rather
//! than through the `float_scalar!` macro, but it carries the same exact-or-error
//! contract: it reads as a wider float always (`f16` ⊂ `f32` ⊂ `f64`) and as an
//! integer only when it is a whole number in range.
//!
//! ```
//! use yggdryl_scalar::{Scalar, ScalarFactory, Float16Scalar};
//! use yggdryl_scalar::half::f16;
//! use yggdryl_scalar::yggdryl_dtype::Float16Type;
//!
//! let half = f16::from_f32(1.5);
//! assert_eq!(Float16Scalar::new(half).value(), Some(&half));
//! assert!(Float16Scalar::null().is_null());
//! assert_eq!(Float16Type.scalar(half), Float16Scalar::new(half)); // the data type is the factory
//!
//! // Widens to f32 / f64 exactly; reads as an integer only when whole.
//! assert_eq!(Float16Scalar::new(half).as_f32().unwrap(), 1.5f32);
//! assert_eq!(Float16Scalar::new(f16::from_f32(3.0)).as_i64().unwrap(), 3);
//! assert!(Float16Scalar::new(half).as_i64().is_err());
//! ```

use crate::{Scalar, ScalarFactory, TypedScalar};
use half::f16;
use yggdryl_dtype::{DataError, Float16Type};

/// A single, possibly-null `float16` value (native [`half::f16`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Float16Scalar {
    data_type: Float16Type,
    value: Option<f16>,
}

// Hashed by bit pattern, like [`Float32Scalar`](crate::Float32Scalar) — `-0.0`
// canonicalizes to `+0.0` (hashing equal, as they compare equal), a `NaN` hashes by
// its bits though it is unequal to itself by value. `Eq` is the pragmatic marker.
impl std::cmp::Eq for Float16Scalar {}

impl std::hash::Hash for Float16Scalar {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value
            .map(|value| if value.to_f32() == 0.0 { 0 } else { value.to_bits() })
            .hash(state);
    }
}

impl Float16Scalar {
    /// A `float16` scalar holding `value`.
    pub fn new(value: f16) -> Self {
        Self {
            data_type: Float16Type,
            value: Some(value),
        }
    }

    /// A null `float16` scalar.
    pub fn null() -> Self {
        Self {
            data_type: Float16Type,
            value: None,
        }
    }
}

impl Scalar for Float16Scalar {
    type DataType = Float16Type;
    type Value = f16;

    fn data_type(&self) -> &Float16Type {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.value.is_none()
    }

    fn value(&self) -> Option<&f16> {
        self.value.as_ref()
    }

    fn to_arrow_scalar(&self) -> arrow_array::ArrayRef {
        match self.value {
            Some(value) => {
                std::sync::Arc::new(arrow_array::Float16Array::from_iter_values([value]))
            }
            // Arrow arrays are immutable, so every null scalar shares one cached
            // one-null array; a clone is a reference-count bump.
            None => {
                static NULL: std::sync::OnceLock<arrow_array::ArrayRef> =
                    std::sync::OnceLock::new();
                NULL.get_or_init(|| std::sync::Arc::new(arrow_array::Float16Array::new_null(1)))
                    .clone()
            }
        }
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::Float16Array>()
            .ok_or_else(|| DataError::IncompatibleArrowType {
                expected: "Float16Type".to_string(),
                got: arrow_array::Array::data_type(array).to_string(),
            })?;
        Ok(if arrow_array::Array::is_null(array, 0) {
            Self::null()
        } else {
            Self::new(array.value(0))
        })
    }

    // The little-endian value bytes — the source of the unchecked reinterpret cast.
    fn value_le_bytes(&self) -> Result<Vec<u8>, DataError> {
        self.value
            .map(|value| value.to_le_bytes().to_vec())
            .ok_or(DataError::NullValue)
    }

    fn as_f16(&self) -> Result<f16, DataError> {
        self.value.ok_or(DataError::NullValue)
    }

    fn as_f32(&self) -> Result<f32, DataError> {
        // Always exact: every f16 has an exact f32.
        self.value.map(f16::to_f32).ok_or(DataError::NullValue)
    }

    fn as_f64(&self) -> Result<f64, DataError> {
        self.value.map(f16::to_f64).ok_or(DataError::NullValue)
    }

    fn as_i8(&self) -> Result<i8, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "i8")
    }
    fn as_i16(&self) -> Result<i16, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "i16")
    }
    fn as_i32(&self) -> Result<i32, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "i32")
    }
    fn as_i64(&self) -> Result<i64, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "i64")
    }
    fn as_u8(&self) -> Result<u8, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "u8")
    }
    fn as_u16(&self) -> Result<u16, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "u16")
    }
    fn as_u32(&self) -> Result<u32, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "u32")
    }
    fn as_u64(&self) -> Result<u64, DataError> {
        crate::float::float_to_int(self.value.ok_or(DataError::NullValue)?, "u64")
    }
}

impl TypedScalar<Float16Type, f16, arrow_array::Float16Array> for Float16Scalar {}

impl ScalarFactory<f16> for Float16Type {
    type Scalar = Float16Scalar;

    /// A `float16` scalar holding `value`.
    fn scalar(&self, value: f16) -> Float16Scalar {
        Float16Scalar::new(value)
    }

    /// The null `float16` scalar.
    fn null_scalar(&self) -> Float16Scalar {
        Float16Scalar::null()
    }

    /// The default `float16` scalar: a scalar holding `0.0`.
    fn default_scalar(&self) -> Float16Scalar {
        Float16Scalar::new(f16::default())
    }
}

impl From<f16> for Float16Scalar {
    /// A `float16` scalar holding `value`.
    fn from(value: f16) -> Self {
        Self::new(value)
    }
}

impl From<Option<f16>> for Float16Scalar {
    /// A `float16` scalar holding `value`, or the null scalar for `None`.
    fn from(value: Option<f16>) -> Self {
        match value {
            Some(value) => Self::new(value),
            None => Self::null(),
        }
    }
}
