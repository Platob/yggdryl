//! UTF-8 text datatypes.

mod dtypes;
mod fields;
mod scalars;

pub use fields::*;
pub use scalars::{LargeUtf8Scalar, Utf8Scalar, Utf8ViewScalar};
