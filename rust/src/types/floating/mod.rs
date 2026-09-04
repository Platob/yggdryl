//! IEEE floating-point datatypes.

mod dtypes;
mod fields;
mod scalars;

pub use fields::*;
pub(crate) use scalars::{FloatWidth, canonical_float};
