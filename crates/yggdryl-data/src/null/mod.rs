//! The `null` type: [`Null`], its field [`NullField`] and scalar [`NullScalar`].
//!
//! [`Null`] is Apache Arrow's `Null` — the type whose every value is null. It carries
//! no storage (no byte width, no codec) and exists so schemas, unions (see
//! [`UnionType`](crate::UnionType)) and scalars can name "always null" as a first-class type.
//!
//! ```
//! use yggdryl_data::{Null, NullField, NullScalar, RawDataType, RawField, RawScalar};
//!
//! assert_eq!(Null.name(), "null");
//! assert_eq!(Null.arrow_format(), "n");
//! assert_eq!(Null.byte_width(), None); // no storage
//!
//! let gap = NullField::new("gap", true);
//! assert_eq!((gap.name(), gap.data_type().name()), ("gap", "null"));
//!
//! // Every null scalar is null; it holds no value.
//! assert!(NullScalar::new().is_null());
//! assert_eq!(NullScalar::new().value(), None);
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::Null;
pub use field::NullField;
pub use scalar::NullScalar;
