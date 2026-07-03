//! The `optional` type: [`OptionalType`], its field [`OptionalField`] and scalar
//! [`Optional`].
//!
//! [`OptionalType`] is the first concrete logical type: a value of
//! the value type, or null, physically stored as the sparse two-variant
//! [`UnionType`](crate::UnionType) between [`NullType`](crate::NullType) and the value type
//! ([`UnionType::optional`](crate::UnionType::optional)). All three carry both trait
//! layers: the raw surface over `OptionalType<D>` and the typed surface wherever the
//! value type has a codec.
//!
//! ```
//! use yggdryl_data::{
//!     DataType, Int64Type, Int64, OptionalField, Optional, OptionalType,
//!     RawDataType, RawField, RawLogical, RawScalar,
//! };
//!
//! // The data type: logically optional, physically the null-or-int64 union.
//! let optional = OptionalType::new(Int64Type);
//! assert_eq!((optional.name(), optional.storage().name()), ("optional", "union"));
//! assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42);
//!
//! // A nullable field of it.
//! let score = OptionalField::<Int64Type>::new("score", true);
//! assert_eq!(score.data_type(), &optional);
//!
//! // A scalar: a value variant, or the null variant.
//! let answer = Optional::new(Int64::new(42));
//! assert_eq!(answer.as_i64().unwrap(), 42);
//! assert!(Optional::<Int64Type, Int64>::null().is_null());
//! ```

mod data_type;
mod field;
mod raw_optional;
mod scalar;
mod typed_optional;

pub use data_type::OptionalType;
pub use field::OptionalField;
pub use raw_optional::RawOptional;
pub use scalar::Optional;
pub use typed_optional::TypedOptional;
