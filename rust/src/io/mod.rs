//! Transitional byte handles and Arrow row routing.
//!
//! The concrete handle and row-routing implementations remain here until their
//! owning `holder`, `coding`, and `media` layers move in the following phases.

mod buffer;
mod coding;
#[cfg(feature = "arrow")]
pub(crate) mod merge;
#[cfg(feature = "arrow")]
pub mod partition;

pub use buffer::Buffer;
pub use coding::Coded;

/// How a directory name spells a value that is not there.
pub const NULL_PARTITION: &str = "null";
