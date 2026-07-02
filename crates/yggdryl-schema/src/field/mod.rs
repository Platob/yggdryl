//! The schema field: the abstract `Field` base and its generic
//! implementation.

mod error;
// The module is named for its abstract base trait, per the
// one-file-per-type rule.
#[allow(clippy::module_inception)]
mod field;
mod typed_field;

pub use error::FieldError;
pub use field::Field;
pub use typed_field::{TypedField, TypedFieldRef};
