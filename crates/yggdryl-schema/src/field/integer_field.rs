//! The primitive integer [`Field`]s — the field-level counterparts of the integer
//! types (`Int8Field`…`UInt64Field`), generated together by one macro to mirror
//! [`integer_type`](crate::dtype).
//!
//! ```
//! use yggdryl_schema::{DataTypeId, Field, Int64Field};
//!
//! let field = Int64Field::new("count");
//! assert_eq!(field.name(), "count");
//! assert_eq!(field.dtype().type_id(), DataTypeId::Int64);
//!
//! let renamed = field.with_name("total".to_string());
//! assert_eq!(field.name(), "count"); // original untouched
//! assert_eq!(renamed.name(), "total");
//! ```

use crate::dtype::{
    DataType, Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type,
    UInt8Type,
};
use crate::field::{Field, Metadata, PrimitiveField};
use crate::nested_fields::NestedFields;

/// Defines a primitive integer field wrapping the given integer type: a `name`, that
/// fixed `dtype`, and optional [`Metadata`], with the non-mutating `with_*` / `copy`
/// updates and a [`Field`] + [`PrimitiveField`] impl.
macro_rules! integer_fields {
    ($($name:ident => $dtype:ident : $type_name:literal),+ $(,)?) => {$(
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

        impl NestedFields for $name {}

        impl Field for $name {
            fn name(&self) -> &str {
                &self.name
            }

            fn dtype(&self) -> &dyn DataType {
                &self.dtype
            }

            fn metadata(&self) -> Option<&Metadata> {
                self.metadata.as_ref()
            }

            fn clone_box(&self) -> Box<dyn Field> {
                Box::new(self.clone())
            }
        }

        impl PrimitiveField for $name {}
    )+};
}

integer_fields! {
    Int8Field => Int8Type : "int8",
    Int16Field => Int16Type : "int16",
    Int32Field => Int32Type : "int32",
    Int64Field => Int64Type : "int64",
    UInt8Field => UInt8Type : "uint8",
    UInt16Field => UInt16Type : "uint16",
    UInt32Field => UInt32Type : "uint32",
    UInt64Field => UInt64Type : "uint64",
}
