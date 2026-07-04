//! # yggdryl-scalar
//!
//! The Apache Arrow-centralized scalar layer for yggdryl, built on top of
//! `yggdryl-dtype` and `yggdryl-core`. It defines the **scalars** of the model —
//! single, possibly-null values of a data type — the third of the three data
//! layers (`yggdryl-dtype`, `yggdryl-field`, `yggdryl-scalar`), each concern its
//! own crate, so the concrete types share one naming convention across the layers
//! (a `yggdryl_scalar::Int64Scalar` holds one value of the `yggdryl_dtype::Int64Type`
//! type).
//!
//! The layer is four traits, each re-exported at the crate root:
//!
//! - The **untyped base** [`Scalar`] — the FFI-facing value: its data type,
//!   nullness, the native value, and the `as_*` accessors reading it as any
//!   exactly-representable Rust target.
//! - The **typed** [`TypedScalar`], generic over the data type `DT`, its value
//!   type `T`, and the concrete Apache Arrow array types it produces —
//!   `ArrowScalar` (the [`to_arrow_scalar`](Scalar::to_arrow_scalar) form) and
//!   `ArrowArray` (the [`to_arrow_array`](Scalar::to_arrow_array) form, defaulting
//!   to `ArrowScalar`).
//! - [`FromScalar`] — the native Rust targets readable out of any scalar, behind
//!   the generic accessors such as [`Serie::get_at`].
//! - [`ScalarFactory`] — a typed data type builds its scalar
//!   ([`Int64Type.scalar(42)`](ScalarFactory::scalar) → [`Int64Scalar`], plus
//!   [`null_scalar`](ScalarFactory::null_scalar) / [`default_scalar`](ScalarFactory::default_scalar)).
//!
//! Concrete scalars live in per-family modules mirroring `yggdryl-dtype` — the
//! [`integer`] module holds every signed and unsigned integer, the [`binary`]
//! module the byte sequence (doubling as a `yggdryl-core` positioned-IO
//! resource), the [`null`] module the always-null scalar, the [`optional`] module
//! the null-or-value variant, and the [`serie`], [`map`] and [`struct`](r#struct)
//! modules the nested values (the union, dynamic at runtime, has no scalar). Add
//! more following the rules in `CLAUDE.md`.
//!
//! Every scalar converts to and from its Apache Arrow equivalent (`to_arrow_scalar` /
//! `from_arrow`): a one-element [`arrow_array`] array — Arrow's own scalar
//! representation. The `arrow-array`, `arrow-buffer` and `arrow-schema` subset
//! crates, the `yggdryl-dtype` layer and `yggdryl-core` are re-exported so
//! downstream code uses the exact versions this crate was built against.

/// The Apache Arrow array layer (`arrow-array`), re-exported so downstream code and
/// the `to_arrow_scalar` / `from_arrow` surface share one version.
pub use arrow_array;
/// The Apache Arrow buffer layer (`arrow-buffer`), re-exported so downstream code
/// can build the zero-copy buffers the array scalars (such as [`Int64Serie`])
/// borrow.
pub use arrow_buffer;
/// The Apache Arrow schema layer (`arrow-schema`), re-exported so downstream code
/// and the data types' Arrow surface share one version.
pub use arrow_schema;
/// The yggdryl foundation layer (`yggdryl-core`), re-exported so downstream code
/// reaches the positioned-IO surface the [`BinaryScalar`] value plugs into
/// (`RawIOBase`, `ByteBuffer`, the cursor / slice adapters) at the exact version
/// this crate was built against.
pub use yggdryl_core;
/// The yggdryl data-type layer (`yggdryl-dtype`), re-exported so downstream code
/// reaches the data types (and [`DataError`](yggdryl_dtype::DataError)) at the
/// exact version this crate was built against.
pub use yggdryl_dtype;

mod from_scalar;
mod scalar;
mod scalar_factory;
mod typed_scalar;

pub use from_scalar::FromScalar;
pub use scalar::Scalar;
pub use scalar_factory::ScalarFactory;
pub use typed_scalar::TypedScalar;

pub mod binary;
pub mod integer;
pub mod map;
pub mod null;
pub mod optional;
pub mod serie;
pub mod r#struct;

pub use binary::BinaryScalar;
pub use map::MapScalar;
pub use null::NullScalar;
pub use optional::OptionalScalar;
pub use r#struct::StructScalar;
pub use serie::{
    Int16Serie, Int32Serie, Int64Serie, Int8Serie, Serie, UInt16Serie, UInt32Serie, UInt64Serie,
    UInt8Serie,
};

pub use integer::{
    Int16Scalar, Int32Scalar, Int64Scalar, Int8Scalar, UInt16Scalar, UInt32Scalar, UInt64Scalar,
    UInt8Scalar,
};
