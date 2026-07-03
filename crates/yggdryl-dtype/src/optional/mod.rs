//! The `optional` type: [`OptionalType`] and its traits [`Optional`] / [`TypedOptional`].
//!
//! [`OptionalType<D>`] is the first concrete logical type: a value of the value
//! type, or null, physically stored as the sparse two-variant
//! [`UnionType`](crate::UnionType) between [`NullType`](crate::NullType) and the
//! value type ([`UnionType::optional`](crate::UnionType::optional)). [`Optional`] is
//! its untyped surface and [`TypedOptional`] the typed layer wherever the value type
//! has a codec. The matching field and scalar live in `yggdryl-field` and
//! `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, Logical, OptionalType, TypedDataType};
//!
//! // Logically optional, physically the null-or-int64 union.
//! let optional = OptionalType::new(Int64Type);
//! assert_eq!((optional.name(), optional.storage().name()), ("optional", "union"));
//! assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42);
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod optional;
mod typed_optional;

pub use data_type::OptionalType;
pub use optional::Optional;
pub use typed_optional::TypedOptional;
