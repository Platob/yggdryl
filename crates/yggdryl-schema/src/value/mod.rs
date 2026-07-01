//! The dynamic value layer: [`Any`] (a value of any type) and [`Struct`] (a struct
//! value — an array of [`Any`]). These are the native value types the dynamic
//! [`AnyType`](crate::AnyType) / [`StructType`](crate::StructType) describe.

mod any;
mod struct_value;

pub use any::Any;
pub use struct_value::Struct;
