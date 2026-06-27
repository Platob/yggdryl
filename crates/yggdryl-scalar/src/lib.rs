//! # yggdryl-scalar
//!
//! An **atomic scalar value** for yggdryl: a single, type-erased value that knows its
//! own [`DataType`] and round-trips losslessly to and from an Apache **Arrow** scalar.
//! It is the value-level companion to [`yggdryl-schema`](yggdryl_schema)'s type-level
//! [`DataType`] / [`Field`] and to [`yggdryl-serie`]'s columnar `Serie` — where a
//! `Serie` is a *column* of values, a [`Scalar`] is *one* value.
//!
//! Every variant carries the **full** type information of its
//! [`DataType`](yggdryl_schema::DataType): an [`Int`](Scalar::Int) keeps its width and
//! signedness, a [`Decimal`](Scalar::Decimal) its precision / scale / storage width, a
//! [`Timestamp`](Scalar::Timestamp) its [`TimeUnit`](yggdryl_core::TimeUnit) and optional
//! [`Timezone`](yggdryl_core::Timezone), so [`data_type`](Scalar::data_type) reconstructs
//! the exact logical type. Like the schema layer, the model is **parameterised, not
//! combinatorial** — one [`Int`](Scalar::Int) variant covers every width.
//!
//! ## Arrow scalar conversion
//!
//! The headline capability is total conversion with Arrow:
//!
//! - [`to_array`](Scalar::to_array) renders the value as a **length-1
//!   [`ArrayRef`](arrow_array::ArrayRef)** of its [`DataType`]'s Arrow type;
//! - [`to_arrow_scalar`](Scalar::to_arrow_scalar) wraps that in an
//!   [`arrow_array::Scalar`] — the broadcast marker Arrow's compute kernels accept;
//! - [`Scalar::from_array`] reads any Arrow array cell back into a [`Scalar`], and
//!   [`Scalar::from_arrow_scalar`] reads an [`arrow_array::Scalar`].
//!
//! ## Serialization
//!
//! As with every yggdryl value type, a [`Scalar`] round-trips through a canonical
//! [string](Scalar::to_str) (`42::int64`, `'hi'::utf8`, `null::int64`), a
//! [component map](Scalar::to_mapping), [bytes](Scalar::to_bytes) (lossless Arrow IPC,
//! the canonical interchange form) and — under the `serde` / `json` features — JSON,
//! and is [`Hash`] + [`Eq`] so it can key a map or set (floats hash by their canonical
//! bit pattern; see [`F64`]).
//!
//! ```
//! use yggdryl_scalar::{Scalar, DataType};
//!
//! let value = Scalar::int(42, 64, true);
//! assert_eq!(value.data_type(), DataType::int(64, true));
//! assert_eq!(value.to_str(), "42::int64");
//!
//! // Round-trip through a length-1 Arrow array.
//! let array = value.to_array().unwrap();
//! assert_eq!(array.len(), 1);
//! assert_eq!(Scalar::from_array(array.as_ref(), 0).unwrap(), value);
//! ```

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime cost).
/// Shared by every submodule via `pub(crate) use log_event`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod arrow;
mod bytes;
mod error;
mod scalar;

#[cfg(test)]
mod tests;

pub use bytes::from_bytes;
pub use error::{ScalarError, ScalarResult};
pub use scalar::{Interval, Scalar, F64};

// Re-export the vocabulary a scalar is built on, so dependents resolve everything
// through `yggdryl_scalar::` (mirroring how `yggdryl-serie` re-exports its deps).
pub use arrow_buffer::i256;
pub use yggdryl_core::{Charset, Date, DateTime, Duration, Time, TimeUnit, Timezone};
pub use yggdryl_schema::{DataType, Field};

// Re-export the Arrow array crate so callers can name `ArrayRef` / `arrow_array::Scalar`
// without taking their own pinned Arrow dependency.
pub use arrow_array;
