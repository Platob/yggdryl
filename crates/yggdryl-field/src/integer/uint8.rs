//! The [`UInt8Field`] field.
//!
//! A nullable `uint8` column: a name paired with the
//! [`UInt8Type`](yggdryl_dtype::UInt8Type) data type (native `u8`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, UInt8Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, UInt8Type};
//!
//! let id = UInt8Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint8", false));
//! assert_eq!(UInt8Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(UInt8Type.field("id", false), id);
//! ```

crate::integer::int_field!(UInt8Field, UInt8Type, u8, "uint8");
