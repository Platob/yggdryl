//! The `optional` type: [`OptionalType`] and its traits [`Optional`] / [`TypedOptional`].
//!
//! [`OptionalType`] is the first concrete logical type: a value of some value type,
//! or null, physically stored as the sparse two-variant
//! [`UnionType`](crate::UnionType) between [`NullType`](crate::NullType) and the
//! value type ([`UnionType::optional`](crate::UnionType::optional)). It is the
//! concrete, *dynamic* type — carrying its value type only as the union's Arrow field
//! — with [`Optional`] its untyped surface. [`TypedOptionalType<D>`] is the
//! statically-typed optional of a value type `D` (adding [`TypedOptional`] and the
//! byte codec), erasing back to [`OptionalType`] with
//! [`erase`](TypedOptionalType::erase). The matching field and scalar live in
//! `yggdryl-field` and `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64Type, Logical, TypedDataType, TypedOptional, TypedOptionalType};
//!
//! // Logically optional, physically the null-or-int64 union.
//! let optional = TypedOptionalType::new(Int64Type);
//! assert_eq!((optional.name(), optional.storage().name()), ("optional", "union"));
//! assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42);
//! ```

mod data_type;
#[allow(clippy::module_inception)] // the base-trait module shares the family's bare name
mod optional;
mod typed_optional;
mod typed_optional_type;

pub use data_type::OptionalType;
pub use optional::Optional;
pub use typed_optional::TypedOptional;
pub use typed_optional_type::TypedOptionalType;
