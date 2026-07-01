//! The [`PrimitiveType`] category marker.

use crate::data_type::DataType;

/// Marks the primitive data types — scalars with neither an inner type nor child
/// fields (e.g. [`BinaryType`](crate::BinaryType), a boolean, an integer). It adds
/// no behaviour beyond [`DataType`]; it exists so a bound can require "a primitive
/// type".
///
/// ```
/// use yggdryl_schema::{BinaryType, PrimitiveType};
///
/// fn takes_primitive<T: PrimitiveType>(_t: &T) {}
/// takes_primitive(&BinaryType::new());
/// ```
pub trait PrimitiveType: DataType {}
