//! The `list` type: [`List`] and its traits [`RawList`] / [`TypedList`].
//!
//! A list value is a variable-length sequence of one value type. [`List<D>`] is
//! the concrete data type (a [`Nested`](crate::Nested) type whose single child is
//! the nullable `"item"` field), [`RawList`] its untyped surface, and [`TypedList`]
//! the typed layer whenever the value type has a codec. The matching field and
//! scalars (`Serie`, `Int64Serie`) live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64, List, RawDataType, RawList};
//!
//! let list = List::new(Int64);
//! assert_eq!((list.name(), list.arrow_format().as_str()), ("list", "+l"));
//! assert_eq!(list.value_type().name(), "int64");
//! assert_eq!(list.default_value(), Vec::<i64>::new());
//! ```

mod data_type;
mod raw_list;
mod typed_list;

pub use data_type::List;
pub use raw_list::RawList;
pub use typed_list::TypedList;
