//! The `list` type: [`ListType`] and its traits [`List`] / [`TypedList`].
//!
//! A list value is a variable-length sequence of one value type. [`ListType<D>`] is
//! the concrete data type (a [`Nested`](crate::Nested) type whose single child is
//! the nullable `"item"` field), [`List`] its untyped surface, and [`TypedList`]
//! the typed layer whenever the value type has a codec. The matching field and
//! scalars (`Serie`, `Int64Serie`) live in `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, List, ListType, TypedDataType};
//!
//! let list = ListType::new(Int64Type);
//! assert_eq!((list.name(), list.arrow_format().as_str()), ("list", "+l"));
//! assert_eq!(list.value_type().name(), "int64");
//! assert_eq!(list.default_value(), Vec::<i64>::new());
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod list;
mod typed_list;

pub use data_type::ListType;
pub use list::List;
pub use typed_list::TypedList;
