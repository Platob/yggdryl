//! The subtrait every floating-point data type satisfies.

use crate::{Float32Type, Float64Type, NumericType};

/// A [`NumericType`] whose values are IEEE 754 floating-point numbers.
///
/// ```
/// use yggdryl_schema::{Float64Type, FloatType, PrimitiveType};
///
/// fn width_of<T: FloatType>(_: &T) -> usize {
///     T::BIT_WIDTH
/// }
/// assert_eq!(width_of(&Float64Type), 64);
/// ```
pub trait FloatType: NumericType {}

impl FloatType for Float32Type {}
impl FloatType for Float64Type {}
