//! The [`UInt16Field`] field.
//!
//! A nullable `uint16` column: a name paired with the
//! [`UInt16Type`](yggdryl_dtype::UInt16Type) data type (native `u16`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, UInt16Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, UInt16Type};
//!
//! let id = UInt16Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint16", false));
//! assert_eq!(UInt16Field::from_arrow(&id.to_arrow()).unwrap(), id);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(UInt16Type.field("id", false), id);
//! ```

crate::integer::int_field!(UInt16Field, UInt16Type, u16, "uint16");
