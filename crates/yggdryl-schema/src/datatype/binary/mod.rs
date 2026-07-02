//! Opaque binary data types.

// The module is named for its plainest type, per the one-file-per-type rule.
#[allow(clippy::module_inception)]
mod binary;
mod fixed_size_binary;
mod large_binary;

pub use binary::BinaryType;
pub use fixed_size_binary::FixedSizeBinaryType;
pub use large_binary::LargeBinaryType;
