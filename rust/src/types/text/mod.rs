//! UTF-8 text datatypes.

mod dtypes;
mod fields;
mod scalars;

pub use dtypes::TextType;
pub use fields::*;
pub(crate) use scalars::text_from_value;
pub use scalars::{
    LargeUtf8, LargeUtf8Scalar, Text, TextValue, Utf8, Utf8Scalar, Utf8View, Utf8ViewScalar,
};
