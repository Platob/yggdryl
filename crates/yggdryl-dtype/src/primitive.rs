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
/// use yggdryl_dtype::{Int64, Primitive, RawDataType};
///
/// // Every primitive has a fixed *bit* width — bytes for most, a single bit for a
/// // boolean (whose `byte_width` is `None`), so bit width is the shared invariant.
/// fn fixed_bit_width<P: Primitive>(primitive: &P) -> usize {
///     primitive.bit_width().expect("a primitive has a fixed bit width")
/// }
///
/// assert_eq!(fixed_bit_width(&Int64), 64);
/// ```
pub trait Primitive: RawDataType {}
