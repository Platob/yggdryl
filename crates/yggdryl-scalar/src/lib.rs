//! # yggdryl-scalar
//!
//! The scalar container layer of yggdryl: one typed value, owned by this
//! crate, laid out per the Apache Arrow columnar spec.
//!
//! A [`Scalar<T>`] pairs a [`DataType`](yggdryl_schema::DataType) with one
//! element's value bytes held in an `arrow-buffer` [`Buffer`]
//! (`None` = null). The [`ScalarType`] subtrait ties each schema type to its
//! one-element layout, so every construction path validates and invalid
//! scalars are unrepresentable. Buffers are refcounted: extracting a scalar
//! from a larger container slices, never copies.
//!
//! ```
//! use yggdryl_scalar::Scalar;
//! use yggdryl_schema::Int32;
//!
//! let seven = Scalar::from_native(Int32, 7);
//! assert_eq!(seven.as_native(), Some(7));
//! assert!(Scalar::null(Int32).is_null());
//! ```
//!
//! [`Buffer`]: arrow_buffer::Buffer

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

mod binary_scalar_type;
mod error;
mod scalar;
mod scalar_type;
mod string_scalar_type;

pub use binary_scalar_type::BinaryScalarType;
pub use error::ScalarError;
pub use scalar::Scalar;
pub use scalar_type::ScalarType;
pub use string_scalar_type::StringScalarType;
