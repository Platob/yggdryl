//! The `uint16` integer type: [`UInt16Type`], its nullable field [`UInt16Field`] and scalar
//! [`UInt16`].
//!
//! [`UInt16Type`] is Apache Arrow's `UInt16Type` — a fixed-width
//! [`Primitive`](crate::Primitive) whose native Rust type is `u16`, stored
//! little-endian in two bytes, Arrow C Data Interface format `"S"`.
//!
//! ```
//! use yggdryl_data::{DataType, UInt16Type, UInt16Field, UInt16, RawDataType, RawField, RawScalar};
//!
//! // The data type describes itself and round-trips its native value through bytes.
//! assert_eq!(UInt16Type.name(), "uint16");
//! assert_eq!(UInt16Type.arrow_format(), "S");
//! assert_eq!(UInt16Type.byte_width(), Some(2));
//! let bytes = UInt16Type.native_to_bytes(&42);
//! assert_eq!(UInt16Type.native_from_bytes(&bytes).unwrap(), 42);
//!
//! // A nullable field pairs a name with the type.
//! let id = UInt16Field::new("id", false);
//! assert_eq!((id.name(), id.data_type().name(), id.is_nullable()), ("id", "uint16", false));
//!
//! // A scalar holds a single value, or null.
//! assert_eq!(UInt16::new(42).value(), Some(&42));
//! assert!(UInt16::null().is_null());
//! ```

mod data_type;
mod field;
mod scalar;

pub use data_type::UInt16Type;
pub use field::UInt16Field;
pub use scalar::UInt16;
