//! The 64-bit floating-point data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The IEEE 754 double-precision floating-point type, mapping to Arrow
    /// `Float64`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Float64, PrimitiveType};
    ///
    /// assert_eq!(Float64::BIT_WIDTH, 64);
    /// assert_eq!(Float64::from_arrow(&Float64.to_arrow()), Ok(Float64));
    /// ```
    Float64, f64, 64, Float64, "float64"
);
