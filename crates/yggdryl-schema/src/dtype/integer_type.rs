//! The primitive integer [`DataType`]s — the signed `Int8`…`Int256` and unsigned
//! `UInt8`…`UInt256`, generated together by one macro. Each is a `DataType<T>` over
//! its native Rust value type (`i8`…`i128` / [`I256`], `u8`…`u128` / [`U256`]).
//!
//! ```
//! use yggdryl_schema::{DataType, DataTypeId, I256, Int32Type, Int256Type, UInt8Type};
//!
//! assert_eq!(Int32Type::new().type_name(), "int32");
//! assert_eq!(Int32Type::new().default(), 0i32);
//! assert_eq!(UInt8Type::new().type_id(), DataTypeId::UInt8);
//! assert_eq!(Int256Type::new().default(), I256::ZERO);
//!
//! // Each scalar type round-trips through an Arrow type node.
//! let node = Int32Type::new().to_arrow_scalar();
//! assert_eq!(node.format(), "i");
//! assert_eq!(Int32Type::from_arrow_scalar(&node).unwrap(), Int32Type::new());
//! ```

use crate::arrow::{ArrowError, ArrowSchema};
use crate::dtype::{DataType, DataTypeId, PrimitiveType};
use yggdryl_core::{I256, U256};

/// Defines a parameterless primitive integer type over its native value type: a
/// zero-sized struct implementing [`DataType`] + [`PrimitiveType`].
macro_rules! integer_types {
    ($($name:ident => $id:ident : $type_name:literal : $native:ty),+ $(,)?) => {$(
        #[doc = concat!("The `", $type_name, "` primitive integer [`DataType`] (native `", stringify!($native), "`).")]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name;

        impl $name {
            #[doc = concat!("A new `", $type_name, "` type.")]
            pub fn new() -> Self {
                Self
            }

            #[doc = concat!("This `", $type_name, "` type as a scalar Arrow type node.")]
            pub fn to_arrow_scalar(&self) -> ArrowSchema {
                ArrowSchema::primitive(self.type_id())
            }

            #[doc = concat!("A `", $type_name, "` type from a scalar Arrow node, erroring unless its format matches.")]
            pub fn from_arrow_scalar(schema: &ArrowSchema) -> Result<Self, ArrowError> {
                crate::arrow::check_id(DataTypeId::$id, schema.primitive_id()?)?;
                Ok(Self)
            }
        }

        impl DataType<$native> for $name {
            fn type_id(&self) -> DataTypeId {
                DataTypeId::$id
            }

            fn type_name(&self) -> &str {
                $type_name
            }
        }

        impl PrimitiveType<$native> for $name {}
    )+};
}

integer_types! {
    Int8Type => Int8 : "int8" : i8,
    Int16Type => Int16 : "int16" : i16,
    Int32Type => Int32 : "int32" : i32,
    Int64Type => Int64 : "int64" : i64,
    Int128Type => Int128 : "int128" : i128,
    Int256Type => Int256 : "int256" : I256,
    UInt8Type => UInt8 : "uint8" : u8,
    UInt16Type => UInt16 : "uint16" : u16,
    UInt32Type => UInt32 : "uint32" : u32,
    UInt64Type => UInt64 : "uint64" : u64,
    UInt128Type => UInt128 : "uint128" : u128,
    UInt256Type => UInt256 : "uint256" : U256,
}
