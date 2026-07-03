//! The `union` type: [`UnionType`] and its traits [`Union`] / [`TypedUnion`].
//!
//! A union value is exactly one of several child types, discriminated by a type id.
//! [`UnionType`] carries the Arrow `(type id, child field)` pairs and mode
//! losslessly; [`UnionType::optional`] names the two-variant union between
//! [`NullType`](crate::NullType) and a value type — the storage of the logical
//! [`OptionalType`](crate::OptionalType) type (see the [`optional`](crate::optional)
//! module). The matching field lives in `yggdryl-field`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, Nested, UnionType};
//!
//! let union = UnionType::optional(&Int64Type);
//! assert_eq!((union.name(), union.child_count()), ("union", 2));
//! assert_eq!(union.arrow_format(), "+us:0,1");
//!
//! // The Arrow round trip is lossless for any union.
//! assert_eq!(UnionType::from_arrow(&union.to_arrow()).unwrap(), union);
//! ```

mod data_type;
mod typed_union;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod union;

pub use data_type::UnionType;
pub use typed_union::TypedUnion;
pub use union::Union;
