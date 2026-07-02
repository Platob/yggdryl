//! Variable-size UTF-8 string data types.

mod large_utf8;
mod utf8;

pub use large_utf8::LargeUtf8Type;
pub use utf8::Utf8Type;
