//! The generic schema field: a name attached to a data type.

mod error;
// The module is named for its one public type, per the one-file-per-type rule.
#[allow(clippy::module_inception)]
mod field;

pub use error::FieldError;
pub use field::{Field, FieldRef};
