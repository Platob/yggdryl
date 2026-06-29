//! # yggdryl-core
//!
//! The Arrow-centric foundations of **yggdryl**: a hashable, serializable,
//! zero-copy type system that mirrors Apache Arrow's taxonomy.
//!
//! - [`DataType`] is the base trait every type implements, split into the
//!   [`PrimitiveType`] / [`NestedType`] / [`LogicalType`] categories; the
//!   variable-length, binary-backed types share [`BinaryBased`]. Concrete types
//!   ([`Binary`], [`Utf8`]) live one-per-module and the [`AnyType`] enum is the
//!   carrier a field stores.
//! - [`Field`] is a named, nullable, typed column; [`AnyField`] is its
//!   type-erased form. [`PrimitiveField`] / [`NestedField`] / [`LogicalField`]
//!   mirror the type categories.
//! - [`Scalar`] is a single typed cell; [`BinaryScalar`] / [`StringScalar`] keep
//!   their payload in a shared [`Buffer`] for O(1) clones and borrowed access.
//!
//! Every value type round-trips through a canonical string, a component map,
//! bytes and (under the `serde` / `json` features) `serde` / JSON, and derives
//! `Hash` + `Eq`. The crate carries **no** Arrow dependency — converting to
//! `arrow-schema` is the job of `yggdryl-schema`.
//!
//! ```
//! use yggdryl_core::{AnyType, Buffer, DataType, Field, Scalar, StringScalar};
//!
//! let field = Field::new("name", AnyType::from_str("string").unwrap(), false);
//! assert_eq!(field.to_mapping()["type"], "string");
//!
//! let scalar = StringScalar::new("hello");
//! assert_eq!(scalar.as_str(), Some("hello"));
//! assert_eq!(scalar.data_type(), AnyType::from_str("string").unwrap());
//!
//! let shared = Buffer::from_slice(b"hello world");
//! assert_eq!(shared.slice(0..5).as_slice(), b"hello"); // zero-copy
//! ```

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Shared by every submodule via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod buffer;
mod datatype;
mod error;
mod field;
mod mapping;
mod scalar;

pub use buffer::Buffer;
pub use datatype::{
    AnyType, Binary, BinaryBased, DataType, LogicalType, NestedType, PrimitiveType, TypeCategory,
    Utf8,
};
pub use error::{FieldError, ScalarError, TypeError};
pub use field::{AnyField, Field, LogicalField, NestedField, PrimitiveField};
pub use scalar::{BinaryScalar, Scalar, StringScalar};

#[cfg(test)]
mod tests;
