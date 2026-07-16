//! The analytics **seams**: the zero-copy / allocation-free iteration on a column
//! ([`Serie::values`] / [`Serie::iter`] / [`Serie::iter_valid`], and the var-family
//! [`ByteSerie::iter_bytes`] / [`iter_valid_bytes`]) and the [`NumericSerie`] reduction capability
//! (count / sum / mean / min / max) they feed. This is the base the stats / time-series layer
//! extends; the tests pin its null-, empty-, wide-integer-, and `NaN`-handling contracts.

use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::NumericSerie;

// -------------------------------------------------------------------------------------
// Iteration seams — values() is zero-copy raw; iter()/iter_valid() are null-aware.
// -------------------------------------------------------------------------------------

#[test]
fn values_is_the_raw_contiguous_slice() {
    let col = Serie::from_values(&[10i32, 20, 30]);
    assert_eq!(col.values(), &[10, 20, 30]);
    // Under a null, the slot holds a placeholder (default), but the SLICE still has `len` entries.
    let with_null = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(with_null.values().len(), 3);
    assert_eq!(with_null.values()[0], 1);
    assert_eq!(with_null.values()[2], 3);
    assert_eq!(with_null.values()[1], i32::default()); // the placeholder under the null
}

#[test]
fn iter_is_null_aware_and_iter_valid_drops_nulls() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3), None]);
    assert_eq!(
        col.iter().collect::<Vec<_>>(),
        [Some(1), None, Some(3), None]
    );
    assert_eq!(col.iter_valid().collect::<Vec<_>>(), [1, 3]);
    // A fully-present column: iter mirrors the values, iter_valid mirrors it too.
    let dense = Serie::from_values(&[5i32, 6, 7]);
    assert_eq!(dense.iter().flatten().collect::<Vec<_>>(), [5, 6, 7]);
    assert_eq!(dense.iter_valid().collect::<Vec<_>>(), [5, 6, 7]);
    // Empty column: both iterators are empty.
    let empty = Serie::<i32>::from_values(&[]);
    assert_eq!(empty.iter().count(), 0);
    assert_eq!(empty.iter_valid().count(), 0);
}

#[test]
fn byte_serie_iter_bytes_is_zero_copy_and_null_aware() {
    let col = Utf8Serie::from_strs(&[Some("a"), None, Some("cc")]);
    let seen: Vec<Option<&[u8]>> = col.iter_bytes().collect();
    assert_eq!(seen, [Some(&b"a"[..]), None, Some(&b"cc"[..])]);
    let valid: Vec<&[u8]> = col.iter_valid_bytes().collect();
    assert_eq!(valid, [&b"a"[..], &b"cc"[..]]);
}

// -------------------------------------------------------------------------------------
// NumericSerie reductions — count / sum / mean / min / max.
// -------------------------------------------------------------------------------------

#[test]
fn reductions_over_a_column_with_nulls() {
    let col = Serie::from_options(&[Some(1i32), None, Some(2), Some(6)]);
    assert_eq!(col.valid_count(), 3);
    assert_eq!(col.sum_f64(), 9.0);
    assert_eq!(col.mean_f64(), Some(3.0));
    assert_eq!(col.min_f64(), Some(1.0));
    assert_eq!(col.max_f64(), Some(6.0));
    assert_eq!(col.to_f64_values(), [1.0, 2.0, 6.0]);
    assert_eq!(
        col.to_f64_options(),
        [Some(1.0), None, Some(2.0), Some(6.0)]
    );
}

#[test]
fn reductions_over_empty_and_all_null_columns() {
    for col in [
        Serie::<i32>::from_values(&[]),
        Serie::from_options(&[None, None]),
    ] {
        assert_eq!(col.valid_count(), 0);
        assert_eq!(col.sum_f64(), 0.0); // the empty sum is 0, never NaN
        assert_eq!(col.mean_f64(), None); // no denominator
        assert_eq!(col.min_f64(), None);
        assert_eq!(col.max_f64(), None);
        assert!(col.to_f64_values().is_empty());
    }
}

#[test]
fn reductions_on_wide_and_unsigned_integers() {
    // Wide integers fold through the same f64 bridge (lossy at the very top, exact here).
    let wide = Serie::from_values(&[10i128, 20, 30]);
    assert_eq!(wide.sum_f64(), 60.0);
    assert_eq!(wide.mean_f64(), Some(20.0));
    let unsigned = Serie::from_values(&[1u64, 2, 3, 4]);
    assert_eq!(unsigned.max_f64(), Some(4.0));
    assert_eq!(unsigned.min_f64(), Some(1.0));
}

#[test]
fn float_reductions_skip_nan_in_min_max_but_propagate_in_sum() {
    let col = Serie::from_values(&[1.0f64, f64::NAN, 3.0]);
    // min/max skip NaN (f64::min / f64::max return the non-NaN operand).
    assert_eq!(col.min_f64(), Some(1.0));
    assert_eq!(col.max_f64(), Some(3.0));
    // sum/mean propagate NaN (IEEE).
    assert!(col.sum_f64().is_nan());
    assert!(col.mean_f64().unwrap().is_nan());
    // A single-NaN column: min/max reduce to NaN (nothing non-NaN to prefer).
    let all_nan = Serie::from_values(&[f64::NAN]);
    assert!(all_nan.min_f64().unwrap().is_nan());
}

#[test]
fn mean_is_sum_over_valid_count() {
    let col = Serie::from_options(&[Some(2.0f64), Some(4.0), None, Some(9.0)]);
    assert_eq!(col.valid_count(), 3);
    assert_eq!(col.sum_f64(), 15.0);
    assert_eq!(col.mean_f64(), Some(5.0));
}
