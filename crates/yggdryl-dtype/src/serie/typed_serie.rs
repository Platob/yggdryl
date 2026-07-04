//! The typed [`TypedSerie`] trait: a [`Serie`](super::Serie) whose value type has a
//! codec.

use super::Serie;
use crate::TypedDataType;

/// A [`Serie`](super::Serie) whose value type is a typed [`TypedDataType<T>`] — the
/// list's values have native Rust representation `Vec<T>`.
///
/// It names the concrete value type as the associated
/// [`ValueType`](TypedSerie::ValueType) (a [`TypedDataType<T>`]) so it is preserved
/// for zero-cost access, mirroring `yggdryl-field`'s `Field` and `yggdryl-scalar`'s
/// `Scalar`. It also carries the [`TypedDataType<Vec<T>>`] surface itself: the codec
/// concatenates and splits the value type's per-element bytes, and the default is the
/// empty serie. The untyped [`Serie`](super::Serie) is implemented by both the
/// dynamic [`SerieType`](crate::SerieType) and the typed
/// [`TypedSerieType<D>`](crate::TypedSerieType); this typed layer is only the latter.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, TypedDataType, TypedSerie, TypedSerieType};
///
/// fn default_of<T, L: TypedSerie<T>>(serie: &L) -> Vec<T> {
///     serie.default_value() // the empty serie
/// }
///
/// let serie = TypedSerieType::new(Int64Type);
/// assert_eq!(serie.value_type().name(), "int64");
/// assert_eq!(default_of(&serie), Vec::<i64>::new());
/// ```
pub trait TypedSerie<T>: Serie + TypedDataType<Vec<T>> {
    /// The value type this serie sequences.
    type ValueType: TypedDataType<T>;

    /// The value type this serie sequences.
    fn value_type(&self) -> &Self::ValueType;
}
