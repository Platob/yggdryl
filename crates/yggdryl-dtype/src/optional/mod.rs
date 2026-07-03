//! The `optional` type: [`Optional`] and its traits [`RawOptional`] / [`TypedOptional`].
//!
//! [`Optional<D>`] is the first concrete logical type: a value of the value type,
//! or null, physically stored as the sparse two-variant [`Union`](crate::Union)
//! between [`Null`](crate::Null) and the value type
//! ([`Union::optional`](crate::Union::optional)). [`RawOptional`] is its untyped
//! surface and [`TypedOptional`] the typed layer wherever the value type has a
//! codec. The matching field and scalar live in `yggdryl-field` and
//! `yggdryl-scalar`.
//!
//! ```
//! use yggdryl_dtype::{DataType, Int64, Optional, RawDataType, RawLogical};
//!
//! // Logically optional, physically the null-or-int64 union.
//! let optional = Optional::new(Int64);
//! assert_eq!((optional.name(), optional.storage().name()), ("optional", "union"));
//! assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42);
//! ```

mod data_type;
mod raw_optional;
mod typed_optional;

pub use data_type::Optional;
pub use raw_optional::RawOptional;
pub use typed_optional::TypedOptional;
