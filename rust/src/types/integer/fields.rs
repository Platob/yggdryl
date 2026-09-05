//! Signed and unsigned integer field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Int8Type, "int8", crate::DataType::Int8);
define_field_types!(Int16Type, "int16", crate::DataType::Int16);
define_field_types!(Int32Type, "int32", crate::DataType::Int32);
define_field_types!(Int64Type, "int64", crate::DataType::Int64);
define_field_types!(UInt8Type, "uint8", crate::DataType::UInt8);
define_field_types!(UInt16Type, "uint16", crate::DataType::UInt16);
define_field_types!(UInt32Type, "uint32", crate::DataType::UInt32);
define_field_types!(UInt64Type, "uint64", crate::DataType::UInt64);

/// An Int8-typed field.
pub type Int8Field = TypedField<Int8Type>;
/// An Int16-typed field.
pub type Int16Field = TypedField<Int16Type>;
/// An Int32-typed field.
pub type Int32Field = TypedField<Int32Type>;
/// An Int64-typed field.
pub type Int64Field = TypedField<Int64Type>;
/// A UInt8-typed field.
pub type UInt8Field = TypedField<UInt8Type>;
/// A UInt16-typed field.
pub type UInt16Field = TypedField<UInt16Type>;
/// A UInt32-typed field.
pub type UInt32Field = TypedField<UInt32Type>;
/// A UInt64-typed field.
pub type UInt64Field = TypedField<UInt64Type>;
