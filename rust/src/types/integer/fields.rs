//! Signed and unsigned integer field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Int8, "int8", crate::DataType::Int8);
define_field_types!(Int16, "int16", crate::DataType::Int16);
define_field_types!(Int32, "int32", crate::DataType::Int32);
define_field_types!(Int64, "int64", crate::DataType::Int64);
define_field_types!(UInt8, "uint8", crate::DataType::UInt8);
define_field_types!(UInt16, "uint16", crate::DataType::UInt16);
define_field_types!(UInt32, "uint32", crate::DataType::UInt32);
define_field_types!(UInt64, "uint64", crate::DataType::UInt64);

/// An Int8-typed field.
pub type Int8Field = TypedField<Int8>;
/// An Int16-typed field.
pub type Int16Field = TypedField<Int16>;
/// An Int32-typed field.
pub type Int32Field = TypedField<Int32>;
/// An Int64-typed field.
pub type Int64Field = TypedField<Int64>;
/// A UInt8-typed field.
pub type UInt8Field = TypedField<UInt8>;
/// A UInt16-typed field.
pub type UInt16Field = TypedField<UInt16>;
/// A UInt32-typed field.
pub type UInt32Field = TypedField<UInt32>;
/// A UInt64-typed field.
pub type UInt64Field = TypedField<UInt64>;
