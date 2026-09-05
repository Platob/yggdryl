//! UTF-8 text datatypes.

mod dtypes;
mod fields;
mod scalars;

pub use fields::*;
pub use scalars::{LargeUtf8Scalar, TextValue, Utf8Scalar, Utf8ViewScalar};
