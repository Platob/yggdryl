//! IEEE floating-point datatypes.

mod dtypes;
mod fields;
pub(crate) mod scalars;

pub use fields::*;
pub(crate) use scalars::{FloatWidth, canonical_float};
