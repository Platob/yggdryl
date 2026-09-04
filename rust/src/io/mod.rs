//! Transitional byte handles and Arrow row routing.
//!
//! The concrete handle and row-routing implementations remain here until their
//! owning `holder` and `media` layers move in the following phases.

#[cfg(feature = "arrow")]
pub(crate) mod merge;
#[cfg(feature = "arrow")]
pub mod partition;

/// How a directory name spells a value that is not there.
pub const NULL_PARTITION: &str = "null";
