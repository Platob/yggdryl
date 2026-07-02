//! # yggdryl-array
//!
//! The array container layer of yggdryl: typed columns, owned by this crate,
//! laid out exactly per the Apache Arrow columnar spec.
//!
//! The abstract [`Array`] base defines what every array is — a data type, a
//! length and an optional validity bitmap — and [`PrimitiveArray<T>`] is the
//! fixed-width implementation over an `arrow-buffer`
//! [`ScalarBuffer`](arrow_buffer::ScalarBuffer) of natives plus an
//! [`NullBuffer`](arrow_buffer::NullBuffer) validity bitmap. Buffers are
//! refcounted: slicing an array or extracting one element as a
//! [`Scalar`](yggdryl_scalar::Scalar) never copies.
//!
//! ```
//! use yggdryl_array::{Array, PrimitiveArray};
//! use yggdryl_schema::Int32Type;
//!
//! let column = PrimitiveArray::from_options(Int32Type, vec![Some(1), None, Some(3)]);
//! assert_eq!(column.len(), 3);
//! assert_eq!(column.null_count(), 1);
//! assert_eq!(column.value(0), Some(1));
//! assert_eq!(column.value(1), None);
//! ```

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays free of the dependency by default and pays no
/// runtime cost). Reached from submodules via `crate::log_event!`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod array;
mod arrays;
mod error;
mod primitive_array;

pub use array::Array;
pub use arrays::{
    Date32Array, Date64Array, Decimal128Array, Decimal256Array, DurationArray, Float32Array,
    Float64Array, Int16Array, Int32Array, Int64Array, Int8Array, Time32Array, Time64Array,
    TimestampArray, UInt16Array, UInt32Array, UInt64Array, UInt8Array,
};
pub use error::ArrayError;
pub use primitive_array::PrimitiveArray;
