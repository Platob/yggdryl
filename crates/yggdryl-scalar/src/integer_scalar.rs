//! The primitive integer [`Scalar`]s — the signed `Int8`…`Int256` and unsigned
//! `UInt8`…`UInt256`, generated together by one macro to mirror
//! [`integer_field`](yggdryl_schema). Each pairs a native value with its
//! [`Field`](yggdryl_schema::Field) and builds straight from the native type.
//!
//! ```
//! use yggdryl_scalar::{Int64, Scalar};
//! use yggdryl_schema::{DataType, DataTypeId};
//!
//! let s = Int64::from(42).with_name("answer".to_string());
//! assert_eq!(*s.value(), 42);
//! assert_eq!(s.name(), "answer");
//! assert_eq!(s.dtype().type_id(), DataTypeId::Int64);
//! ```

use crate::{Any, AnyField, AnyType, AnyValue, PrimitiveScalar, Scalar};
use yggdryl_schema::{
    DataTypeId, Field, Int128Field, Int16Field, Int256Field, Int32Field, Int64Field, Int8Field,
    UInt128Field, UInt16Field, UInt256Field, UInt32Field, UInt64Field, UInt8Field, I256, U256,
};

/// Defines a primitive integer scalar pairing a native value with its field: the
/// non-mutating `with_*` updates, a `From<native>` builder, a `From<Self>` bridge into
/// [`Any`], and a [`Scalar`] + [`PrimitiveScalar`] impl over the native type.
macro_rules! integer_scalars {
    ($($name:ident => $field:ident : $variant:ident : $type_name:literal : $native:ty),+ $(,)?) => {$(
        #[doc = concat!("A scalar `", $type_name, "` value paired with its [`", stringify!($field), "`](yggdryl_schema::", stringify!($field), ").")]
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name {
            field: $field,
            value: $native,
        }

        impl $name {
            #[doc = concat!("An unnamed `", $type_name, "` scalar holding `value`.")]
            pub fn new(value: $native) -> Self {
                Self { field: $field::new(""), value }
            }

            /// The scalar from its explicit field and value.
            pub fn from_parts(field: $field, value: $native) -> Self {
                Self { field, value }
            }

            /// A copy carrying `field`.
            pub fn with_field(&self, field: $field) -> Self {
                Self { field, value: self.value }
            }

            /// A copy renamed to `name`.
            pub fn with_name(&self, name: String) -> Self {
                self.with_field(self.field.with_name(name))
            }

            /// A copy holding `value`.
            pub fn with_value(&self, value: $native) -> Self {
                Self { field: self.field.clone(), value }
            }
        }

        impl From<$native> for $name {
            fn from(value: $native) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for Any {
            fn from(scalar: $name) -> Self {
                let field = AnyField::from_parts(
                    scalar.field.name().to_owned(),
                    AnyType::primitive(DataTypeId::$variant),
                    scalar.field.nullable(),
                    scalar.field.metadata().cloned(),
                );
                Any::from_parts(field, AnyValue::$variant(scalar.value))
            }
        }

        impl Scalar<$native> for $name {
            type Field = $field;

            fn field(&self) -> &$field {
                &self.field
            }

            fn value(&self) -> &$native {
                &self.value
            }
        }

        impl PrimitiveScalar<$native> for $name {}
    )+};
}

integer_scalars! {
    Int8 => Int8Field : Int8 : "int8" : i8,
    Int16 => Int16Field : Int16 : "int16" : i16,
    Int32 => Int32Field : Int32 : "int32" : i32,
    Int64 => Int64Field : Int64 : "int64" : i64,
    Int128 => Int128Field : Int128 : "int128" : i128,
    Int256 => Int256Field : Int256 : "int256" : I256,
    UInt8 => UInt8Field : UInt8 : "uint8" : u8,
    UInt16 => UInt16Field : UInt16 : "uint16" : u16,
    UInt32 => UInt32Field : UInt32 : "uint32" : u32,
    UInt64 => UInt64Field : UInt64 : "uint64" : u64,
    UInt128 => UInt128Field : UInt128 : "uint128" : u128,
    UInt256 => UInt256Field : UInt256 : "uint256" : U256,
}
