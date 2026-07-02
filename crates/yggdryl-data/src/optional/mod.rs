//! The `optional` type: [`Optional`], its field [`OptionalField`] and scalar
//! [`OptionalScalar`].
//!
//! [`Optional`] is the first concrete [`Logical`](crate::Logical) type: a value of
//! the value type, or null, physically stored as the sparse two-variant
//! [`Union`](crate::Union) between [`Null`](crate::Null) and the value type
//! ([`Union::optional`](crate::Union::optional)). All three carry both trait
//! layers: the raw surface over `Optional<D>` and the typed surface wherever the
//! value type has a codec.
//!
//! ```
//! use yggdryl_data::{
//!     DataType, Int64, Int64Scalar, Logical, Optional, OptionalField, OptionalScalar,
//!     RawDataType, RawField, RawScalar,
//! };
//!
//! // The data type: logically optional, physically the null-or-int64 union.
//! let optional = Optional::new(Int64);
//! assert_eq!((optional.name(), optional.storage().name()), ("optional", "union"));
//! assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42);
//!
//! // A nullable field of it.
//! let score = OptionalField::<Int64>::new("score", true);
//! assert_eq!(score.data_type(), &optional);
//!
//! // A scalar: a value variant, or the null variant.
//! let answer = OptionalScalar::new(Int64Scalar::new(42));
//! assert_eq!(answer.as_i64(), Some(42));
//! assert!(OptionalScalar::<Int64, Int64Scalar>::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Optional;
pub use field::OptionalField;
pub use scalar::OptionalScalar;
