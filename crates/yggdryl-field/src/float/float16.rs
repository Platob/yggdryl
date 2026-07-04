//! The [`Float16Field`] field.
//!
//! A nullable `float16` column: a name paired with the
//! [`Float16Type`](yggdryl_dtype::Float16Type) data type (native [`half::f16`](yggdryl_dtype::half::f16)).
//!
//! ```
//! use yggdryl_field::{Field, FieldFactory, Float16Field};
//! use yggdryl_field::yggdryl_dtype::{DataType, Float16Type};
//!
//! let weight = Float16Field::new("weight", false);
//! assert_eq!((weight.name(), weight.data_type().name(), weight.is_nullable()), ("weight", "float16", false));
//! assert_eq!(Float16Field::from_arrow(&weight.to_arrow()).unwrap(), weight);
//!
//! // The data type is the factory: it builds the same field.
//! assert_eq!(Float16Type.field("weight", false), weight);
//! ```

// Reuses the field macro shared with the integer family (a float field is the same
// shape: a name paired with the fixed-width float data type).
crate::integer::int_field!(Float16Field, Float16Type, half::f16, "float16");
