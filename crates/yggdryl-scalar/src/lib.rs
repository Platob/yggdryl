//! # yggdryl-scalar
//!
//! The Apache Arrow-centralized scalar layer for yggdryl, built on top of
//! `yggdryl-dtype` and `yggdryl-core`. It defines the **scalars** of the model —
//! single, possibly-null values of a data type — the third of the three data
//! layers (`yggdryl-dtype`, `yggdryl-field`, `yggdryl-scalar`), each concern its
//! own crate, so the concrete types share one bare name across the layers (a
//! `yggdryl_scalar::Int64` holds one value of the `yggdryl_dtype::Int64` type).
//!
//! The layer is four traits, each re-exported at the crate root:
//!
//! - The **untyped base** [`RawScalar`] — the FFI-facing value: its data type,
//!   nullness, the native value, and the `as_*` accessors reading it as any
//!   exactly-representable Rust target.
//! - The **typed** [`Scalar`], whose value is the native Rust type `T`.
//! - [`FromScalar`] — the native Rust targets readable out of any scalar, behind
//!   the generic accessors such as [`Serie::get_at`].
//! - [`DefaultScalar`] — the scalar a `yggdryl_dtype::DataType<T>` defaults to
//!   (this crate builds on the data types, never the other way around, so the
//!   default *scalar* of a type lives here rather than on `DataType`).
//!
//! Concrete scalars live in per-family modules mirroring `yggdryl-dtype` — the
//! [`integer`] module holds every signed and unsigned integer, the [`binary`]
//! module the byte sequence (doubling as a `yggdryl-core` positioned-IO
//! resource), the [`null`] module the always-null scalar, the [`optional`] module
//! the null-or-value variant, and the [`list`], [`map`] and [`struct`](r#struct)
//! modules the nested values (the union, dynamic at runtime, has no scalar). Add
//! more following the rules in `CLAUDE.md`.
//!
//! Every scalar converts to and from its Apache Arrow equivalent (`to_arrow` /
//! `from_arrow`): a one-element [`arrow_array`] array — Arrow's own scalar
//! representation. The `arrow-array`, `arrow-buffer` and `arrow-schema` subset
//! crates, the `yggdryl-dtype` layer and `yggdryl-core` are re-exported so
//! downstream code uses the exact versions this crate was built against.

/// The Apache Arrow array layer (`arrow-array`), re-exported so downstream code and
/// the `to_arrow` / `from_arrow` surface share one version.
pub use arrow_array;
/// The Apache Arrow buffer layer (`arrow-buffer`), re-exported so downstream code
/// can build the zero-copy buffers the array scalars (such as [`Int64Serie`])
/// borrow.
pub use arrow_buffer;
/// The Apache Arrow schema layer (`arrow-schema`), re-exported so downstream code
/// and the data types' Arrow surface share one version.
pub use arrow_schema;
/// The yggdryl foundation layer (`yggdryl-core`), re-exported so downstream code
/// reaches the positioned-IO surface the [`Binary`] value plugs into
/// (`RawIOBase`, `ByteBuffer`, the cursor / slice adapters) at the exact version
/// this crate was built against.
pub use yggdryl_core;
/// The yggdryl data-type layer (`yggdryl-dtype`), re-exported so downstream code
/// reaches the data types (and [`DataError`](yggdryl_dtype::DataError)) at the
/// exact version this crate was built against.
pub use yggdryl_dtype;

mod default_scalar;
mod from_scalar;
mod raw_scalar;
mod scalar;

pub use default_scalar::DefaultScalar;
pub use from_scalar::FromScalar;
pub use raw_scalar::RawScalar;
pub use scalar::Scalar;

pub mod binary;
pub mod integer;
pub mod list;
pub mod map;
pub mod null;
pub mod optional;
pub mod r#struct;

pub use binary::Binary;
pub use list::{Int64Serie, Serie};
pub use map::Map;
pub use null::Null;
pub use optional::Optional;
pub use r#struct::Struct;

pub use integer::{Int16, Int32, Int64, Int8, UInt16, UInt32, UInt64, UInt8};
