//! The `union` type: [`Union`], its field [`UnionField`], and the
//! [`OptionalScalar`] built on the null-or-value union.
//!
//! A union value is exactly one of several child types, discriminated by a type id.
//! [`Union`] carries the Arrow `(type id, child field)` pairs and mode losslessly;
//! [`Union::optional`] names the two-variant union between [`Null`](crate::Null)
//! and a value type, and [`OptionalScalar`] is the scalar of that shape — an inner
//! scalar or the null variant, with all value access redirected to the inner
//! scalar.
//!
//! ```
//! use yggdryl_data::{Int64, Int64Scalar, Nested, OptionalScalar, RawDataType, RawScalar, Union};
//!
//! let union = Union::optional(&Int64);
//! assert_eq!((union.name(), union.child_count()), ("union", 2));
//!
//! let answer = OptionalScalar::new(Int64Scalar::new(42));
//! assert_eq!(answer.as_i64(), Some(42));
//! assert_eq!(answer.data_type(), &union);
//!
//! // The Arrow round trip preserves the variant.
//! let missing: OptionalScalar<Int64, Int64Scalar> = OptionalScalar::null();
//! let back: OptionalScalar<Int64, Int64Scalar> =
//!     OptionalScalar::from_arrow(missing.to_arrow().as_ref()).unwrap();
//! assert!(back.is_null());
//! ```

mod data_type;
mod field;
mod optional_scalar;

pub use data_type::Union;
pub use field::UnionField;
pub use optional_scalar::OptionalScalar;
