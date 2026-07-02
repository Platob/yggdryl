//! The [`Primitive`] category trait: a fixed-width, childless physical type.

use super::RawDataType;

/// A fixed-width primitive type — a childless physical type whose
/// [`byte_width`](RawDataType::byte_width) (or [`bit_width`](RawDataType::bit_width))
/// is always present, laid out directly for zero-copy FFI (integers, floats,
/// boolean).
///
/// It is a marker refining [`RawDataType`]: it carries no methods of its own but lets
/// generic code require "some fixed-width primitive" as a bound.
///
/// ```
/// use yggdryl_data::{Int64, Primitive, RawDataType};
///
/// // Generic over any primitive.
/// fn fixed_byte_width<P: Primitive>(primitive: &P) -> usize {
///     primitive.byte_width().expect("a primitive is fixed-width")
/// }
///
/// assert_eq!(fixed_byte_width(&Int64), 8);
/// ```
pub trait Primitive: RawDataType {}
