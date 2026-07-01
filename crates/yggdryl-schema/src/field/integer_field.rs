//! The primitive integer [`Field`]s — the field-level counterparts of the integer
//! types (`Int8Field`…`UInt256Field`), generated together by one macro to mirror
//! [`integer_type`](crate::dtype). Each is a `Field<T>` over the same native value
//! type as its data type.
//!
//! ```
//! use yggdryl_schema::{DataType, DataTypeId, Field, Int64Field};
//!
//! let field = Int64Field::new("count");
//! assert_eq!(field.name(), "count");
//! assert_eq!(field.dtype().type_id(), DataTypeId::Int64);
//! assert_eq!(field.default(), 0i64);
//!
//! let renamed = field.with_name("total".to_string());
//! assert_eq!(field.name(), "count"); // original untouched
//! assert_eq!(renamed.name(), "total");
//! ```

use crate::dtype::{
    Int128Type, Int16Type, Int256Type, Int32Type, Int64Type, Int8Type, UInt128Type, UInt16Type,
    UInt256Type, UInt32Type, UInt64Type, UInt8Type,
};
use crate::field::{Field, Metadata, PrimitiveField};
use yggdryl_core::{I256, U256};

/// Defines a primitive integer field wrapping the given integer type: a `name`, that
/// fixed `dtype`, and optional [`Metadata`], with the non-mutating `with_*` / `copy`
/// updates and a [`Field`] + [`PrimitiveField`] impl over the native value type.
macro_rules! integer_fields {
    ($($name:ident => $dtype:ident : $type_name:literal : $native:ty),+ $(,)?) => {$(
        #[doc = concat!("A field whose data type is [`", stringify!($dtype), "`](crate::", stringify!($dtype), ").")]
        #[derive(Clone, Debug)]
        pub struct $name {
            name: String,
            dtype: $dtype,
            metadata: Option<Metadata>,
        }

        impl $name {
            #[doc = concat!("A `", $type_name, "` field named `name`, with no metadata.")]
            pub fn new(name: impl Into<String>) -> Self {
                Self { name: name.into(), dtype: $dtype::new(), metadata: None }
            }

            /// The field from its explicit parts.
            pub fn from_parts(name: String, metadata: Option<Metadata>) -> Self {
                Self { name, dtype: $dtype::new(), metadata }
            }

            /// A copy with the given parts overridden; omitted parts come from `self`.
            /// (The data type is fixed and so is not a parameter.)
            pub fn copy(&self, name: Option<String>, metadata: Option<Option<Metadata>>) -> Self {
                Self {
                    name: name.unwrap_or_else(|| self.name.clone()),
                    dtype: self.dtype,
                    metadata: metadata.unwrap_or_else(|| self.metadata.clone()),
                }
            }

            /// A copy renamed to `name`.
            pub fn with_name(&self, name: String) -> Self {
                self.copy(Some(name), None)
            }

            /// A copy carrying `metadata`.
            pub fn with_metadata(&self, metadata: Metadata) -> Self {
                self.copy(None, Some(Some(metadata)))
            }

            /// A copy with the metadata cleared.
            pub fn without_metadata(&self) -> Self {
                self.copy(None, Some(None))
            }
        }

        impl Field<$native> for $name {
            type DType = $dtype;

            fn name(&self) -> &str {
                &self.name
            }

            fn dtype(&self) -> &$dtype {
                &self.dtype
            }

            fn metadata(&self) -> Option<&Metadata> {
                self.metadata.as_ref()
            }
        }

        impl PrimitiveField<$native> for $name {}
    )+};
}

integer_fields! {
    Int8Field => Int8Type : "int8" : i8,
    Int16Field => Int16Type : "int16" : i16,
    Int32Field => Int32Type : "int32" : i32,
    Int64Field => Int64Type : "int64" : i64,
    Int128Field => Int128Type : "int128" : i128,
    Int256Field => Int256Type : "int256" : I256,
    UInt8Field => UInt8Type : "uint8" : u8,
    UInt16Field => UInt16Type : "uint16" : u16,
    UInt32Field => UInt32Type : "uint32" : u32,
    UInt64Field => UInt64Type : "uint64" : u64,
    UInt128Field => UInt128Type : "uint128" : u128,
    UInt256Field => UInt256Type : "uint256" : U256,
}
