//! The `union` type: [`Union`] and its traits [`RawUnion`] / [`TypedUnion`].
//!
//! A union value is exactly one of several child types, discriminated by a type id.
//! [`Union`] carries the Arrow `(type id, child field)` pairs and mode losslessly;
//! [`Union::optional`] names the two-variant union between [`Null`](crate::Null)
//! and a value type — the storage of the logical [`Optional`](crate::Optional)
//! type (see the [`optional`](crate::optional) module). The matching field lives
//! in `yggdryl-field`.
//!
//! ```
//! use yggdryl_dtype::{Int64, RawDataType, RawNested, Union};
//!
//! let union = Union::optional(&Int64);
//! assert_eq!((union.name(), union.child_count()), ("union", 2));
//! assert_eq!(union.arrow_format(), "+us:0,1");
//!
//! // The Arrow round trip is lossless for any union.
//! assert_eq!(Union::from_arrow(&union.to_arrow()).unwrap(), union);
//! ```

mod data_type;
mod raw_union;
mod typed_union;

pub use data_type::Union;
pub use raw_union::RawUnion;
pub use typed_union::TypedUnion;
