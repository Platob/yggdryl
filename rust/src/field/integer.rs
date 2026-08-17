//! Signed and unsigned integer field markers.

use super::typed::define_field_types;

define_field_types!(Int8, "int8", crate::DataType::Int8);
define_field_types!(Int16, "int16", crate::DataType::Int16);
define_field_types!(Int32, "int32", crate::DataType::Int32);
define_field_types!(Int64, "int64", crate::DataType::Int64);
define_field_types!(UInt8, "uint8", crate::DataType::UInt8);
define_field_types!(UInt16, "uint16", crate::DataType::UInt16);
define_field_types!(UInt32, "uint32", crate::DataType::UInt32);
define_field_types!(UInt64, "uint64", crate::DataType::UInt64);
