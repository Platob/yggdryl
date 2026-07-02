//! The `union` type: [`UnionType`] and its field [`UnionField`].
//!
//! A union value is exactly one of several child types, discriminated by a type id.
//! [`UnionType`] carries the Arrow `(type id, child field)` pairs and mode losslessly;
//! [`UnionType::optional`] names the two-variant union between [`Null`](crate::Null)
//! and a value type — the storage of the logical
//! [`OptionalType`](crate::OptionalType) type (see the [`optional`](crate::optional)
//! module).
//!
//! ```
//! use yggdryl_data::{Int64, RawDataType, RawField, RawNested, UnionField, UnionType};
//!
//! let union = UnionType::optional(&Int64);
//! assert_eq!((union.name(), union.child_count()), ("union", 2));
//! assert_eq!(union.arrow_format(), "+us:0,1");
//!
//! let field = UnionField::new("value", union.clone(), true);
//! assert_eq!(field.data_type(), &union);
//!
//! // The Arrow round trip is lossless for any union.
//! assert_eq!(UnionType::from_arrow(&union.to_arrow()).unwrap(), union);
//! ```

mod data_type;
mod field;
mod raw_union;
#[allow(clippy::module_inception)]
mod union;

pub use data_type::UnionType;
pub use field::UnionField;
pub use raw_union::RawUnion;
pub use union::Union;
