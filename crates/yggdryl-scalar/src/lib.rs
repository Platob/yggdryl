//! # yggdryl-scalar
//!
//! The value + dynamic-schema layer for yggdryl. A **scalar** is a value: a native
//! Rust primitive, the dynamic [`Any`], or a nested [`Struct`] (an array of `Any`).
//! The generic [`Scalar`] trait unifies them — every scalar reports its
//! [`type_id`](Scalar::type_id), whether it [`is_null`](Scalar::is_null), and promotes
//! to [`Any`](Scalar::to_any) — so nested structures hold and build from any scalar
//! uniformly.
//!
//! - [`Any`] is the dynamic value (primitives + the recursive [`Struct`]), with atomic
//!   accessors (`as_i32`, …). [`Struct`] is an array of `Any`, built from a collection
//!   of scalars via [`Struct::from_scalars`].
//! - The schema that describes the values — [`AnyType`] / [`AnyField`] and the nested
//!   [`StructType`] / [`StructField`] — is generic over these scalar values (an Arrow
//!   schema is a `StructField`) and carries the recursive Arrow round-trip built on the
//!   `yggdryl-schema` crate's [`ArrowSchema`] node.
//!
//! ```
//! use yggdryl_scalar::{Any, Scalar, Struct};
//!
//! // Build a row from native scalars; each promotes to an `Any` child.
//! let row = Struct::from_scalars([1i32, 2i32]);
//! assert_eq!(row.len(), 2);
//! assert_eq!(row.to_any(), Any::Struct(row.clone()));
//! ```

mod any_field;
mod any_scalar;
mod any_type;
mod arrow;
mod scalar;
mod struct_field;
mod struct_scalar;
mod struct_type;

pub use any_field::AnyField;
pub use any_scalar::Any;
pub use any_type::AnyType;
pub use arrow::ArrowSchemaExt;
pub use scalar::Scalar;
pub use struct_field::StructField;
pub use struct_scalar::Struct;
pub use struct_type::StructType;
// Re-export the schema vocabulary the scalar layer builds on, so this crate is a
// one-stop for the full value + schema surface.
pub use yggdryl_schema::{
    ArrowArray, ArrowError, ArrowSchema, DataType, DataTypeId, Field, Metadata, PrimitiveField,
    PrimitiveType,
};
// The 256-bit native value types live in the core crate; re-export for convenience.
pub use yggdryl_core::{I256, U256};
