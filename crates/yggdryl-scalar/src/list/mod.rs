//! The `list` scalars: the generic [`Serie`] and the buffer-backed [`Int64Serie`].
//!
//! A list value is a variable-length sequence of one value type — *our array*.
//! [`Serie<D, S>`] is the generic scalar, backed by one zero-copy Arrow child
//! array with per-element scalar accessors; [`Int64Serie`] is the concrete list
//! of `int64`, borrowing the raw Arrow buffers for native `i64` access. The
//! matching [`ListType`](yggdryl_dtype::ListType) data type lives in `yggdryl-dtype`,
//! and its [`ScalarFactory`](crate::ScalarFactory) (`ListType::scalar` /
//! `default_scalar`) builds a [`Serie`].
//!
//! ```
//! use yggdryl_scalar::{Int64Scalar, Scalar, Serie};
//!
//! let numbers = Serie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
//! assert_eq!(numbers.len(), 2);
//! assert_eq!(numbers.get_scalar_at(0), Some(Int64Scalar::new(1)));
//! assert_eq!(
//!     Serie::from_arrow(numbers.to_arrow().as_ref()).unwrap(),
//!     numbers
//! );
//! ```

mod int64_serie;
mod serie;

pub use int64_serie::Int64Serie;
pub use serie::Serie;
