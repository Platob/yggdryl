//! The data-model type system: the [`RawDataType`] descriptor and the parameterised
//! [`RawField`] and [`RawScalar`] built on it.
//!
//! These are the abstract base traits; concrete types (`Int32`, `Utf8`, …) and their
//! Arrow-interop machinery land here as the layer grows, one file per type.

mod raw_data_type;
mod raw_field;
mod raw_scalar;

pub use raw_data_type::RawDataType;
pub use raw_field::RawField;
pub use raw_scalar::RawScalar;
