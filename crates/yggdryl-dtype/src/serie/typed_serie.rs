//! The typed [`TypedSerie`] trait: a [`Serie`](super::Serie) whose value type has a
//! codec.

use super::Serie;
use crate::TypedDataType;

/// A [`Serie`](super::Serie) whose value type is a typed [`TypedDataType<T>`] — the
/// list's values have native Rust representation `Vec<T>`.
///
/// The concrete value type is [`Serie`](super::Serie)'s associated
/// [`ValueType`](super::Serie::ValueType), here refined to a
/// [`TypedDataType<T>`]; `value_type` is inherited from [`Serie`](super::Serie) and
/// returns it. It also carries the [`TypedDataType<Vec<T>>`] surface itself: the
/// codec concatenates and splits the value type's per-element bytes, and the default
/// is the empty serie.
///
/// ```
/// use yggdryl_dtype::{Int64Type, SerieType, TypedDataType, TypedSerie};
///
/// fn default_of<T, L: TypedSerie<T>>(serie: &L) -> Vec<T> {
///     serie.default_value() // the empty serie
/// }
///
/// let serie = SerieType::new(Int64Type);
/// assert_eq!(default_of(&serie), Vec::<i64>::new());
/// ```
pub trait TypedSerie<T>: Serie<ValueType: TypedDataType<T>> + TypedDataType<Vec<T>> {}
