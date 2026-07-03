//! The [`UInt32Field`] field.
//!
//! A nullable `uint32` column: a name paired with the
//! [`UInt32Type`](yggdryl_dtype::UInt32Type) data type (native `u32`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, UInt32Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, UInt32Type};
//!
//! let id = UInt32Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint32", false));
//! assert_eq!(UInt32Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(UInt32Type.field("id", false), id);
//! ```

crate::integer::int_field!(UInt32Field, UInt32Type, u32, "uint32");
