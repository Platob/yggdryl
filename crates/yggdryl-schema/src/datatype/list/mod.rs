//! Variable-size list data types.

mod large_list;
// The module is named for its plainest type, per the one-file-per-type rule.
#[allow(clippy::module_inception)]
mod list;

pub use large_list::LargeListType;
pub use list::ListType;
