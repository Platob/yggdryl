//! The 32-bit floating-point data type.

use crate::datatype::macros::primitive_data_type;

primitive_data_type!(
    /// The IEEE 754 single-precision floating-point type, mapping to Arrow
    /// `Float32Type`.
    ///
    /// ```
    /// use yggdryl_schema::{DataType, Float32Type, PrimitiveType};
    ///
    /// assert_eq!(Float32Type::BIT_WIDTH, 32);
    /// assert_eq!(Float32Type::from_arrow(&Float32Type.to_arrow()), Ok(Float32Type));
    /// ```
    Float32Type, f32, 32, Float32, "float32"
);
