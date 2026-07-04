//! The [`Float32Field`] field.
//!
//! A nullable `float32` column: a name paired with the
//! [`Float32Type`](yggdryl_dtype::Float32Type) data type (native `f32`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Float32Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Float32Type};
//!
//! let weight = Float32Field::new("weight", false);
//! assert_eq!((weight.name(), weight.data_type().name(), weight.is_nullable()), ("weight", "float32", false));
//! assert_eq!(Float32Field::from_arrow(&weight.to_arrow()).unwrap(), weight);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Float32Type.field("weight", false), weight);
//! ```

// Reuses the field macro shared with the integer family (a float field is the same
// shape: a name paired with the fixed-width float data type).
crate::integer::int_field!(Float32Field, Float32Type, f32, "float32");
