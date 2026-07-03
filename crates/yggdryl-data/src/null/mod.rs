//! The `null` type: [`NullType`], its field [`NullField`] and scalar [`Null`].
//!
//! [`NullType`] is Apache Arrow's `NullType` — the type whose every value is null. It carries
//! no storage (no byte width, no codec) and exists so schemas, unions (see
//! [`UnionType`](crate::UnionType)) and scalars can name "always null" as a first-class type.
//!
//! ```
//! use yggdryl_data::{NullType, NullField, Null, RawDataType, RawField, RawScalar};
//!
//! assert_eq!(NullType.name(), "null");
//! assert_eq!(NullType.arrow_format(), "n");
//! assert_eq!(NullType.byte_width(), None); // no storage
//!
//! let gap = NullField::new("gap", true);
//! assert_eq!((gap.name(), gap.data_type().name()), ("gap", "null"));
//!
//! // Every null scalar is null; it holds no value.
//! assert!(Null::new().is_null());
//! assert_eq!(Null::new().value(), None);
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::NullType;
pub use field::NullField;
pub use scalar::Null;
