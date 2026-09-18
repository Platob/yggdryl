//! Signed and unsigned integer field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Int8Type, Int8, crate::DataType::Int8);
define_field_types!(Int16Type, Int16, crate::DataType::Int16);
define_field_types!(Int32Type, Int32, crate::DataType::Int32);
define_field_types!(Int64Type, Int64, crate::DataType::Int64);
define_field_types!(UInt8Type, UInt8, crate::DataType::UInt8);
define_field_types!(UInt16Type, UInt16, crate::DataType::UInt16);
define_field_types!(UInt32Type, UInt32, crate::DataType::UInt32);
define_field_types!(UInt64Type, UInt64, crate::DataType::UInt64);

/// An Int8-typed field.
pub type Int8Field = TypedField<Int8Type>;
/// An Int32-typed field.
pub type Int32Field = TypedField<Int32Type>;
/// An Int64-typed field.
pub type Int64Field = TypedField<Int64Type>;
/// A UInt32-typed field.
pub type UInt32Field = TypedField<UInt32Type>;
/// A UInt64-typed field.
pub type UInt64Field = TypedField<UInt64Type>;
