//! [`NumericSerie`] — the numeric-analytics **capability** over a column, the seam the stats /
//! time-series layer builds on.
//!
//! It is a *capability sub-trait* (the pattern of [`ArrowNative`](crate::io::fixed::ArrowNative)):
//! implemented only for a column whose element is [`NumericCast`] — i.e. every fixed numeric
//! [`Serie<T>`] (the 17 integers + 3 floats). A non-numeric column (utf8, binary, nested) simply
//! does not have it, so the reductions are *statically* available exactly where they are meaningful,
//! with no runtime "is this numeric?" check on the hot path. Everything folds over the column's
//! allocation-free [`iter`](Serie::iter) / [`iter_valid`](Serie::iter_valid), through the common
//! `f64` bridge ([`NumericCast::to_f64`]) so one implementation covers native *and* wide integers
//! *and* floats.
//!
//! DESIGN: this is the **foundational** reduction set (count / sum / mean / min / max) — the base a
//! richer numeric/time-series layer (variance, quantiles, rolling windows, resampling) extends. It
//! deliberately stops there; the heavier statistics are not modeled yet. `NaN` follows IEEE through
//! `sum`/`mean` (a `NaN` element propagates) but is **skipped** by `min`/`max` (via [`f64::min`] /
//! [`f64::max`], which return the non-`NaN` operand) — documented per method.

use crate::io::fixed::Serie;
use crate::io::NumericCast;

/// The numeric-analytics reductions available on any numeric column. See the [module
/// docs](self) for the design (capability sub-trait, `f64` bridge, `NaN` handling).
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::NumericSerie;
///
/// let col = Serie::from_options(&[Some(1i32), None, Some(2), Some(6)]);
/// assert_eq!(col.valid_count(), 3);
/// assert_eq!(col.sum_f64(), 9.0);
/// assert_eq!(col.mean_f64(), Some(3.0));
/// assert_eq!(col.min_f64(), Some(1.0));
/// assert_eq!(col.max_f64(), Some(6.0));
/// ```
pub trait NumericSerie {
    /// The number of **present** (non-null) elements — the denominator of the reductions.
    fn valid_count(&self) -> usize;

    /// The sum of the present elements as `f64` (`0.0` over an empty / all-null column). A `NaN`
    /// element propagates (IEEE).
    fn sum_f64(&self) -> f64;

    /// The minimum present element as `f64`, or `None` if there is none. `NaN` elements are
    /// **skipped** (via [`f64::min`]); `None` only when every element is null / the column is empty.
    fn min_f64(&self) -> Option<f64>;

    /// The maximum present element as `f64`, or `None` if there is none. `NaN` elements are
    /// **skipped** (via [`f64::max`]).
    fn max_f64(&self) -> Option<f64>;

    /// The arithmetic mean of the present elements, or `None` if there is none. A `NaN` element
    /// propagates through the sum.
    fn mean_f64(&self) -> Option<f64> {
        match self.valid_count() {
            0 => None,
            n => Some(self.sum_f64() / n as f64),
        }
    }

    /// The present elements as `f64`, in order — the materialized (allocating) form of
    /// [`iter_valid`](Serie::iter_valid) mapped through the `f64` bridge.
    fn to_f64_values(&self) -> Vec<f64>;

    /// Every element as `Option<f64>`, in order (a null → `None`) — the materialized form of
    /// [`iter`](Serie::iter) mapped through the `f64` bridge.
    fn to_f64_options(&self) -> Vec<Option<f64>>;
}

// DESIGN: one blanket impl over `NumericCast` (which has `NativeType` as a supertrait, so it also
// bounds `Serie<T>`) covers every numeric column with no per-type code — native and wide integers
// and floats alike fold through `to_f64`. Non-numeric series never match, so utf8/binary/nested get
// no (meaningless) reductions.
impl<T: NumericCast> NumericSerie for Serie<T> {
    fn valid_count(&self) -> usize {
        self.len() - self.null_count()
    }

    // DESIGN — auto-vectorization (see the CLAUDE.md rule): a reduction folds over the column's
    // **contiguous value slice** ([`values`](Serie::values)). For a **no-null** column that slice
    // *is* the present values, so we fold the whole `&[T]` — a plain `map(..).sum()` / `reduce(..)`
    // LLVM auto-vectorizes. A column **with nulls** keeps the null-aware
    // [`iter_valid`](Serie::iter_valid) fold (its placeholders must be skipped); the fold order is
    // identical either way, so the result is byte-identical to the previous implementation.

    fn sum_f64(&self) -> f64 {
        if self.has_nulls() {
            return self.iter_valid().map(NumericCast::to_f64).sum();
        }
        self.values().iter().map(|&v| v.to_f64()).sum()
    }

    fn min_f64(&self) -> Option<f64> {
        if self.has_nulls() {
            return self.iter_valid().map(NumericCast::to_f64).reduce(f64::min);
        }
        self.values().iter().map(|&v| v.to_f64()).reduce(f64::min)
    }

    fn max_f64(&self) -> Option<f64> {
        if self.has_nulls() {
            return self.iter_valid().map(NumericCast::to_f64).reduce(f64::max);
        }
        self.values().iter().map(|&v| v.to_f64()).reduce(f64::max)
    }

    fn to_f64_values(&self) -> Vec<f64> {
        self.iter_valid().map(NumericCast::to_f64).collect()
    }

    fn to_f64_options(&self) -> Vec<Option<f64>> {
        self.iter()
            .map(|value| value.map(NumericCast::to_f64))
            .collect()
    }
}
