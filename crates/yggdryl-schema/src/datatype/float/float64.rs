//! The 64-bit floating-point data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The IEEE 754 double-precision floating-point type, mapping to Arrow
    /// `Float64Type`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Float64Type, PrimitiveType};
    ///
    /// assert_eq!(Float64Type::BIT_WIDTH, 64);
    /// assert_eq!(Float64Type::from_arrow(&Float64Type.to_arrow()), Ok(Float64Type));
    /// ```
    Float64Type, f64, 64, Float64, "float64"
);
