//! # yggdryl-scalar
//!
//! The **atomic value layer** for yggdryl — the value-level companion to
//! [`yggdryl-schema`](yggdryl_schema)'s type-level [`DataType`] / [`Field`] and to
//! [`yggdryl-serie`]'s columnar `Serie`. Where a `Serie` is a *column* of values, a
//! scalar is *one* value.
//!
//! It is organised like the `Serie` layer: the object-safe [`Scalar`] **trait** is
//! implemented by a per-type **concrete** scalar — [`IntScalar`], [`VarcharScalar`],
//! [`DateScalar`], [`StructScalar`], [`ListScalar`], [`MapScalar`], … — and the boxed
//! handle is [`ScalarRef`] (`Arc<dyn Scalar>`). Each concrete is a thin typed view over
//! the shared [`ScalarValue`] engine, the tagged representation that owns the Arrow /
//! serialization logic and carries the **full** type information of its
//! [`DataType`](yggdryl_schema::DataType) (an integer keeps its width and signedness, a
//! decimal its precision / scale / storage width, a timestamp its
//! [`TimeUnit`](yggdryl_core::TimeUnit) and optional [`Timezone`](yggdryl_core::Timezone))
//! — so [`data_type`](Scalar::data_type) reconstructs the exact logical type. Like the
//! schema layer, the model is **parameterised, not combinatorial**.
//!
//! ## Arrow scalar conversion
//!
//! - [`to_array`](Scalar::to_array) renders the value as a **length-1
//!   [`ArrayRef`](arrow_array::ArrayRef)** of its [`DataType`]'s Arrow type;
//! - [`to_arrow_scalar`](Scalar::to_arrow_scalar) wraps that in an
//!   [`arrow_array::Scalar`] broadcast marker;
//! - [`ScalarValue::scalar_at`] reads any Arrow array cell back into the right concrete
//!   scalar (and [`ScalarValue::from_array`] into the tagged value).
//!
//! ## Serialization
//!
//! As with every yggdryl value type, a scalar round-trips through a canonical
//! [string](Scalar::to_str) (`42::int64`, `'hi'::utf8`, `null::int64`), a
//! [component map](Scalar::to_mapping), [bytes](Scalar::to_bytes) (lossless Arrow IPC,
//! the canonical interchange form) and — under the `serde` / `json` features — JSON, and
//! the [`ScalarValue`] is [`Hash`] + [`Eq`] so it can key a map or set (floats hash by
//! their canonical bit pattern; see [`F64`]).
//!
//! ```
//! use yggdryl_scalar::{DataType, IntScalar, Scalar, ScalarValue};
//!
//! let value = IntScalar::new(42, 64, true); // a concrete per-type scalar
//! assert_eq!(*value.data_type(), DataType::int(64, true));
//! assert_eq!(value.to_str(), "42::int64");
//!
//! // Round-trip through a length-1 Arrow array, recovering the right concrete scalar.
//! let array = value.to_array().unwrap();
//! let back = ScalarValue::scalar_at(array.as_ref(), 0).unwrap();
//! assert_eq!(back.to_str(), "42::int64");
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
mod concrete;
mod error;
mod scalar;
mod value;

#[cfg(test)]
mod tests;

pub use bytes::from_bytes;
pub use concrete::{
    BinaryScalar, BooleanScalar, BsonScalar, DateScalar, DecimalScalar, DurationScalar,
    FloatScalar, IntScalar, IntervalScalar, JsonScalar, ListScalar, MapScalar, NullScalar,
    StructScalar, TimeScalar, TimestampScalar, TimezoneScalar, VarcharScalar,
};
pub use error::{ScalarError, ScalarResult};
pub use scalar::{from_value, Scalar, ScalarRef, TypedScalar};
pub use value::{Interval, ScalarValue, F64};

// Re-export the vocabulary a scalar is built on, so dependents resolve everything
// through `yggdryl_scalar::` (mirroring how `yggdryl-serie` re-exports its deps).
pub use arrow_buffer::i256;
pub use yggdryl_core::{Charset, Date, DateTime, Duration, Time, TimeUnit, Timezone};
pub use yggdryl_schema::{DataType, Field};

// Re-export the Arrow array crate so callers can name `ArrayRef` / `arrow_array::Scalar`
// without taking their own pinned Arrow dependency.
pub use arrow_array;
