//! The primitive integer [`DataType`]s — the signed `Int8`…`Int64` and unsigned
//! `UInt8`…`UInt64`, generated together by one macro (a closely-related family, like
//! the binary types will be).
//!
//! ```
//! use yggdryl_schema::{DataType, DataTypeId, Int32Type, NestedFields, UInt8Type};
//!
//! assert_eq!(Int32Type::new().type_name(), "int32");
//! assert_eq!(UInt8Type::new().type_id(), DataTypeId::UInt8);
//! assert!(Int32Type::new().children_fields().is_empty()); // a primitive has no children
//! ```

use crate::dtype::{DataType, DataTypeId, PrimitiveType};
use crate::nested_fields::NestedFields;

/// Defines a parameterless primitive integer type: a zero-sized struct implementing
/// [`DataType`] + [`PrimitiveType`], with empty children.
macro_rules! integer_types {
    ($($name:ident => $id:ident : $type_name:literal),+ $(,)?) => {$(
        #[doc = concat!("The `", $type_name, "` primitive integer [`DataType`].")]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name;

        impl $name {
            #[doc = concat!("A new `", $type_name, "` type.")]
            pub fn new() -> Self {
                Self
            }
        }

        impl NestedFields for $name {}

        impl DataType for $name {
            fn type_id(&self) -> DataTypeId {
                DataTypeId::$id
            }

            fn type_name(&self) -> &str {
                $type_name
            }

            fn clone_box(&self) -> Box<dyn DataType> {
                Box::new(*self)
            }
        }

        impl PrimitiveType for $name {}
    )+};
}

integer_types! {
    Int8Type => Int8 : "int8",
    Int16Type => Int16 : "int16",
    Int32Type => Int32 : "int32",
    Int64Type => Int64 : "int64",
    Int128Type => Int128 : "int128",
    Int256Type => Int256 : "int256",
    UInt8Type => UInt8 : "uint8",
    UInt16Type => UInt16 : "uint16",
    UInt32Type => UInt32 : "uint32",
    UInt64Type => UInt64 : "uint64",
    UInt128Type => UInt128 : "uint128",
    UInt256Type => UInt256 : "uint256",
}
