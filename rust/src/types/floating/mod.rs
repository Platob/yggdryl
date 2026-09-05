//! IEEE floating-point datatypes.

mod dtypes;
mod fields;
pub(crate) mod scalars;

pub use fields::*;
pub use scalars::{Float16, Float32, Float64, Floating, FloatingValue};
pub(crate) use scalars::{FloatWidth, canonical_float};
