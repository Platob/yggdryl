//! The [`UInt64Field`] field.
//!
//! A nullable `uint64` column: a name paired with the
//! [`UInt64Type`](yggdryl_dtype::UInt64Type) data type (native `u64`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, UInt64Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, UInt64Type};
//!
//! let id = UInt64Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint64", false));
//! assert_eq!(UInt64Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(UInt64Type.field("id", false), id);
//! ```

crate::integer::int_field!(UInt64Field, UInt64Type, u64, "uint64");
