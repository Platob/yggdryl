//! The [`Float64Field`] field.
//!
//! A nullable `float64` column: a name paired with the
//! [`Float64Type`](yggdryl_dtype::Float64Type) data type (native `f64`).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Float64Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Float64Type};
//!
//! let weight = Float64Field::new("weight", false);
//! assert_eq!((weight.name(), weight.data_type().name(), weight.is_nullable()), ("weight", "float64", false));
//! assert_eq!(Float64Field::from_arrow(&weight.to_arrow()).unwrap(), weight);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Float64Type.field("weight", false), weight);
//! ```

// Reuses the field macro shared with the integer family (a float field is the same
// shape: a name paired with the fixed-width float data type).
crate::integer::int_field!(Float64Field, Float64Type, f64, "float64");
