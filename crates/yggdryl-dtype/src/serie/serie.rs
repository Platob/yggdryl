//! The [`Serie`] base trait: the untyped surface of a serie data type.

use crate::{DataType, Nested};

/// The untyped surface every serie data type carries: a variable-length sequence of
/// one value type, exposing that value type.
///
/// It refines [`Nested`] (the single child is the item field) and names the value
/// data type as the associated [`ValueType`](Serie::ValueType) so the concrete type
/// is preserved for zero-cost access, mirroring `yggdryl-field`'s `Field` and
/// `yggdryl-scalar`'s `Scalar`. A value type with a codec also gets the typed
/// [`TypedSerie`](crate::TypedSerie) layer.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Serie, SerieType, Nested};
///
/// let serie = SerieType::new(Int64Type);
/// assert_eq!(serie.value_type().name(), "int64");
/// assert_eq!(serie.child_count(), 1);
/// ```
pub trait Serie: Nested {
    /// The value type this serie sequences.
    type ValueType: DataType;

    /// The value type this serie sequences.
    fn value_type(&self) -> &Self::ValueType;
}
