//! The subtrait every numeric data type satisfies.

use crate::{
    Decimal128Type, Decimal256Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type,
    Int8Type, PrimitiveType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};

/// A fixed-width data type whose values are numbers — the shared root of
/// [`IntegerType`](crate::IntegerType), [`FloatType`](crate::FloatType) and
/// [`DecimalType`](crate::DecimalType), so numeric kernels are written once
/// against it.
///
/// ```
/// use yggdryl_schema::{Int64Type, NumericType, PrimitiveType};
///
/// fn width_of<T: NumericType>(_: &T) -> usize {
///     T::BIT_WIDTH
/// }
/// assert_eq!(width_of(&Int64Type), 64);
/// ```
pub trait NumericType: PrimitiveType {}

impl NumericType for Int8Type {}
impl NumericType for Int16Type {}
impl NumericType for Int32Type {}
impl NumericType for Int64Type {}
impl NumericType for UInt8Type {}
impl NumericType for UInt16Type {}
impl NumericType for UInt32Type {}
impl NumericType for UInt64Type {}
impl NumericType for Float32Type {}
impl NumericType for Float64Type {}
impl NumericType for Decimal128Type {}
impl NumericType for Decimal256Type {}
