//! [`PrimitiveType`] — the fixed-width primitive category of [`DataType`].

// The core's runtime dtype tag, aliased to free the name `PrimitiveType` for this
// crate's canonical trait. The trait is the typing API every higher layer reaches for;
// the core enum stays the low-level FFI tag the converter is keyed on, and the two
// interoperate through [`primitive_tag`](PrimitiveType::primitive_tag) (and each
// concrete type's `from_primitive_tag`).
use yggdryl_core::PrimitiveType as PrimitiveTag;

use crate::DataType;

/// A fixed-width primitive data type — the category of the ten native numerics
/// (`Int8` … `Float64`) plus `Boolean`.
///
/// This trait is the **canonical** primitive-typing API. It maps to the low-level
/// [`yggdryl_core::PrimitiveType`] runtime tag (aliased `PrimitiveTag` in this module)
/// through [`primitive_tag`](PrimitiveType::primitive_tag): the ten numerics return
/// their tag, `Boolean` returns `None` (it is bit-packed and sits outside the core
/// enum's ten numeric tags).
///
/// ```
/// use yggdryl_dtype::{I64Type, PrimitiveType};
///
/// assert_eq!(I64Type::new().primitive_tag(), Some(yggdryl_core::PrimitiveType::I64));
/// // Round-trip through the core tag.
/// assert_eq!(I64Type::from_primitive_tag(yggdryl_core::PrimitiveType::I64), Some(I64Type::new()));
/// ```
pub trait PrimitiveType: DataType {
    /// The equivalent [`yggdryl_core::PrimitiveType`] runtime tag, or `None` for
    /// `Boolean` (bit-packed, outside the core enum's ten numeric tags).
    fn primitive_tag(&self) -> Option<PrimitiveTag>;
}
