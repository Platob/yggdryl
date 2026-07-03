//! The `list` type: [`ListType`], its traits [`RawList`] / [`TypedList`], field
//! [`ListField`] and scalar [`Serie`].
//!
//! A list value is a variable-length sequence of one value type. [`ListType<D>`]
//! is the concrete data type (a [`Nested`](crate::Nested) type whose single child
//! is the nullable `"item"` field), [`RawList`] its untyped surface, [`TypedList`] the
//! typed layer whenever the value type has a codec, and [`Serie`] a single,
//! possibly-null sequence — *our array*, backed by one zero-copy Arrow child
//! array with per-element scalar accessors ([`Int64Serie`] is the concrete list
//! of `int64`, borrowing the raw Arrow buffers for native `i64` access).
//!
//! ```
//! use yggdryl_data::{DataType, Int64Type, Int64, Serie, ListType, RawDataType, RawScalar};
//!
//! let list = ListType::new(Int64Type);
//! assert_eq!((list.name(), list.arrow_format().as_str()), ("list", "+l"));
//! assert_eq!(list.default_value(), Vec::<i64>::new());
//!
//! let numbers = Serie::new(vec![Int64::new(1), Int64::new(2)]);
//! assert_eq!(numbers.len(), 2);
//! assert_eq!(numbers.get_scalar_at(0), Some(Int64::new(1)));
//! assert_eq!(
//!     Serie::from_arrow(numbers.to_arrow().as_ref()).unwrap(),
//!     numbers
//! );
//! ```

mod data_type;
mod field;
mod int64_serie;
mod raw_list;
mod scalar;
mod typed_list;

pub use data_type::ListType;
pub use field::ListField;
pub use int64_serie::Int64Serie;
pub use raw_list::RawList;
pub use scalar::Serie;
pub use typed_list::TypedList;
