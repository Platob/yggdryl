//! The 32-bit floating-point data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The IEEE 754 single-precision floating-point type, mapping to Arrow
    /// `Float32`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Float32, PrimitiveType};
    ///
    /// assert_eq!(Float32::BIT_WIDTH, 32);
    /// assert_eq!(Float32::from_arrow(&Float32.to_arrow()), Ok(Float32));
    /// ```
    Float32, f32, 32, Float32, "float32"
);
