//! The `list` type: [`ListType`], its traits [`RawList`] / [`List`], field
//! [`ListField`] and scalar [`ListScalar`].
//!
//! A list value is a variable-length sequence of one value type. [`ListType<D>`]
//! is the concrete data type (a [`Nested`](crate::Nested) type whose single child
//! is the nullable `"item"` field), [`RawList`] its untyped surface, [`List`] the
//! typed layer whenever the value type has a codec, and [`ListScalar`] a single,
//! possibly-null sequence of inner scalars.
//!
//! ```
//! use yggdryl_data::{DataType, Int64, Int64Scalar, ListScalar, ListType, RawDataType, RawScalar};
//!
//! let list = ListType::new(Int64);
//! assert_eq!((list.name(), list.arrow_format().as_str()), ("list", "+l"));
//! assert_eq!(list.default_value(), Vec::<i64>::new());
//!
//! let numbers = ListScalar::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
//! assert_eq!(numbers.value().map(<[Int64Scalar]>::len), Some(2));
//! assert_eq!(
//!     ListScalar::from_arrow(numbers.to_arrow().as_ref()).unwrap(),
//!     numbers
//! );
//! ```

mod data_type;
mod field;
#[allow(clippy::module_inception)]
mod list;
mod raw_list;
mod scalar;

pub use data_type::ListType;
pub use field::ListField;
pub use list::List;
pub use raw_list::RawList;
pub use scalar::ListScalar;
