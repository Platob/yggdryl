//! # yggdryl-core
//!
//! The Arrow-centric foundations of **yggdryl**: a hashable, serializable,
//! zero-copy type system that mirrors Apache Arrow's taxonomy.
//!
//! - [`DataType`] is the base trait every type implements, split into the
//!   [`PrimitiveType`] / [`NestedType`] / [`LogicalType`] categories; the
//!   variable-length, binary-backed types share [`BinaryBased`]. Concrete type
//!   descriptors ([`BinaryType`], [`Utf8`]) live one-per-module and the
//!   [`AnyType`] enum is the carrier a field stores.
//! - [`Field`] is a named, nullable, typed column; [`AnyField`] is its
//!   type-erased form. [`PrimitiveField`] / [`NestedField`] / [`LogicalField`]
//!   mirror the type categories.
//! - [`Scalar`] is the value abstraction; [`Binary`] is the in-memory binary
//!   buffer — a growable, shared-allocation value that also implements [`Io`]
//!   (O(1) clone, zero-copy reads) and round-trips through a binary frame, a
//!   component map and JSON.
//! - [`Io`] centralises byte access (`pread`/`pwrite`, `size`, `tell`/`seek`,
//!   capacity/resize); reads hand back zero-copy [`Binary`] views.
//!
//! Every value type round-trips through a canonical string, a component map,
//! bytes and (under the `serde` / `json` features) `serde` / JSON, and derives
//! `Hash` + `Eq`. The crate carries **no** Arrow dependency — converting to
//! `arrow-schema` is the job of `yggdryl-schema`.
//!
//! ```
//! use yggdryl_core::{AnyType, Binary, DataType, Field, Io, Scalar};
//!
//! let field = Field::new("name", AnyType::from_str("binary").unwrap(), false);
//! assert_eq!(field.to_mapping()["type"], "binary");
//!
//! let mut buf = Binary::from_bytes(b"hello world");
//! assert_eq!(buf.read(5).unwrap().as_slice(), b"hello"); // zero-copy view
//! assert_eq!(buf.size(), 11);
//! assert_eq!(buf.data_type(), AnyType::from_str("binary").unwrap());
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
mod io;
mod mapping;
mod scalar;

pub use buffer::Buffer;
pub use datatype::{
    AnyType, BinaryBased, BinaryType, DataType, LogicalType, NestedType, PrimitiveType,
    TypeCategory, Utf8,
};
pub use error::{FieldError, IoError, ScalarError, TypeError};
pub use field::{AnyField, Field, LogicalField, NestedField, PrimitiveField};
pub use io::{Io, Whence};
pub use scalar::{Binary, Scalar};

#[cfg(test)]
mod tests;
