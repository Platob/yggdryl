//! Functional tests for the **serie growth + reshape** surface added on top of the typed carriers:
//! capacity-aware building (`with_capacity` + `append`), column concatenation (`extend`),
//! repeated-value fills (`repeat` / `push_repeat`), null-filling (`fill_null` /
//! `fill_null_forward`), boolean compaction (`mask_filter`), order reversal (`reverse`), and sorting
//! (`sort_indices` / `take` / `sort`) — over the numeric [`FixedSerie`] and the byte carriers
//! ([`VarSerie`], [`FixedSizeSerie`]), plus the `fill`-into-`cast_field` wiring.

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::typed::fixedbyte::{Float64, Int32, Int64};
use yggdryl_core::typed::{
    Field, FixedSerie, FixedSizeSerie, FixedUtf8, HeaderField, Scalar, Serie, Utf8, VarSerie,
};

/// A bit mask [`Heap`] built from a keep-flag slice — LSB-first, `1` = keep.
fn mask_of(flags: &[bool]) -> Heap {
    let mut mask = Heap::new();
    for (index, &keep) in flags.iter().enumerate() {
        mask.pwrite_bit(index as u64, keep).unwrap();
    }
    mask
}

// -------------------------------------------------------------------------------------
// with_capacity + append — capacity-aware building reads back
// -------------------------------------------------------------------------------------

#[test]
fn fixed_with_capacity_then_append_reads_back() {
    let mut col = FixedSerie::<Int64>::with_capacity(1000);
    assert_eq!(col.len(), 0);
    col.append(&(0..1000i64).collect::<Vec<_>>());
    assert_eq!(col.len(), 1000);
    assert_eq!(col.get(0), Some(0));
    assert_eq!(col.get(999), Some(999));
    assert_eq!(col.values().len(), 1000);
}

#[test]
fn fixed_append_marks_nullable_range_valid() {
    let mut col = FixedSerie::<Int32>::from_options(&[Some(1), None]);
    col.append(&[7, 8]);
    assert_eq!(col.to_options(), vec![Some(1), None, Some(7), Some(8)]);
}

// -------------------------------------------------------------------------------------
// extend — concatenate another column (values + nulls)
// -------------------------------------------------------------------------------------

#[test]
fn fixed_extend_from_another_serie_with_nulls() {
    let mut left = FixedSerie::<Int32>::from_values(&[1, 2]);
    let right = FixedSerie::<Int32>::from_options(&[Some(3), None, Some(5)]);
    left.extend(&right);
    assert_eq!(left.len(), 5);
    assert_eq!(
        left.to_options(),
        vec![Some(1), Some(2), Some(3), None, Some(5)]
    );
}

#[test]
fn fixed_extend_all_valid_into_nullable() {
    let mut left = FixedSerie::<Int32>::from_options(&[Some(1), None]);
    let right = FixedSerie::<Int32>::from_values(&[3, 4]);
    left.extend(&right);
    assert_eq!(left.to_options(), vec![Some(1), None, Some(3), Some(4)]);
}

// -------------------------------------------------------------------------------------
// repeat / push_repeat — the alloc-constant fill
// -------------------------------------------------------------------------------------

#[test]
fn fixed_repeat_builder_and_push_repeat() {
    let col = FixedSerie::<Int64>::repeat(9, 2048);
    assert_eq!(col.len(), 2048);
    assert!(col.values().iter().all(|&v| v == 9));

    let mut grow = FixedSerie::<Int64>::from_values(&[1, 2]);
    grow.push_repeat(5, 3);
    assert_eq!(grow.values(), vec![1, 2, 5, 5, 5]);
}

// -------------------------------------------------------------------------------------
// fill_null — nulls replaced + becomes non-nullable; null-free is a no-op; forward-fill
// -------------------------------------------------------------------------------------

#[test]
fn fixed_fill_null_replaces_and_drops_validity() {
    let col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None]);
    assert_eq!(col.null_count(), 2);
    let filled = col.fill_null(-1);
    assert_eq!(filled.null_count(), 0); // non-nullable now
    assert_eq!(
        filled.to_options(),
        vec![Some(1), Some(-1), Some(3), Some(-1)]
    );
    // The original is untouched (copy front door).
    assert_eq!(col.null_count(), 2);
}

#[test]
fn fixed_fill_null_on_null_free_is_noop() {
    let col = FixedSerie::<Int32>::from_values(&[1, 2, 3]);
    let filled = col.fill_null(-1);
    assert_eq!(filled.values(), vec![1, 2, 3]);
    assert_eq!(filled.null_count(), 0);
}

#[test]
fn fixed_fill_null_forward_carries_previous() {
    // Leading nulls stay null; interior nulls take the previous non-null value.
    let col = FixedSerie::<Int32>::from_options(&[None, Some(1), None, None, Some(4), None]);
    let filled = col.fill_null_forward();
    assert_eq!(
        filled.to_options(),
        vec![None, Some(1), Some(1), Some(1), Some(4), Some(4)]
    );

    // No leading null → every null closes → the column becomes non-nullable.
    let clean = FixedSerie::<Int32>::from_options(&[Some(7), None, Some(9), None]);
    let filled = clean.fill_null_forward();
    assert_eq!(filled.null_count(), 0);
    assert_eq!(
        filled.to_options(),
        vec![Some(7), Some(7), Some(9), Some(9)]
    );
}

// -------------------------------------------------------------------------------------
// mask_filter — compaction (dense gather, validity preserved)
// -------------------------------------------------------------------------------------

#[test]
fn fixed_mask_filter_compacts_and_keeps_nulls() {
    let col = FixedSerie::<Int32>::from_values(&[10, 20, 30, 40, 50]);
    let mask = mask_of(&[true, false, true, false, true]);
    assert_eq!(col.mask_filter(&mask).values(), vec![10, 30, 50]);

    let nullable = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3), None]);
    let keep = mask_of(&[true, true, false, true]);
    assert_eq!(
        nullable.mask_filter(&keep).to_options(),
        vec![Some(1), None, None]
    );
}

// -------------------------------------------------------------------------------------
// reverse
// -------------------------------------------------------------------------------------

#[test]
fn fixed_reverse_values_and_validity() {
    let col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3)]);
    let reversed = col.reverse();
    assert_eq!(reversed.to_options(), vec![Some(3), None, Some(1)]);

    let mut in_place = FixedSerie::<Int32>::from_values(&[1, 2, 3, 4]);
    in_place.reverse_in_place();
    assert_eq!(in_place.values(), vec![4, 3, 2, 1]);
}

// -------------------------------------------------------------------------------------
// sort — ascending / descending, floats NaN-last, nulls last
// -------------------------------------------------------------------------------------

#[test]
fn fixed_sort_ascending_descending() {
    let col = FixedSerie::<Int32>::from_values(&[30, 10, 20, 10]);
    assert_eq!(col.sort_indices(true), vec![1, 3, 2, 0]); // stable: the two 10s keep order
    assert_eq!(col.sort().values(), vec![10, 10, 20, 30]);
    assert_eq!(
        col.take(&col.sort_indices(false)).values(),
        vec![30, 20, 10, 10]
    );

    let mut in_place = FixedSerie::<Int32>::from_values(&[3, 1, 2]);
    in_place.sort_in_place();
    assert_eq!(in_place.values(), vec![1, 2, 3]);
}

#[test]
fn fixed_sort_floats_nan_last() {
    let col = FixedSerie::<Float64>::from_values(&[3.0, f64::NAN, 1.0, 2.0]);
    let sorted = col.sort();
    let values = sorted.values();
    assert_eq!(&values[..3], &[1.0, 2.0, 3.0]);
    assert!(values[3].is_nan()); // NaN sorts last

    // Descending keeps NaN last as well.
    let desc = col.take(&col.sort_indices(false));
    let values = desc.values();
    assert_eq!(&values[..3], &[3.0, 2.0, 1.0]);
    assert!(values[3].is_nan());
}

#[test]
fn fixed_sort_nulls_last() {
    let col = FixedSerie::<Int32>::from_options(&[Some(3), None, Some(1), Some(2)]);
    assert_eq!(col.sort_indices(true), vec![2, 3, 0, 1]);
    let sorted = col.take(&col.sort_indices(true));
    assert_eq!(sorted.to_options(), vec![Some(1), Some(2), Some(3), None]);
}

// -------------------------------------------------------------------------------------
// cast_field with a fill value satisfies a non-nullable target
// -------------------------------------------------------------------------------------

#[test]
fn cast_field_fill_satisfies_non_nullable_target() {
    let col = FixedSerie::<Int32>::from_options(&[Some(1), None, Some(3)]);
    let target = HeaderField::new(None, DataTypeId::I32, false); // non-nullable

    // Without a fill value the nullable → non-nullable cast still errors.
    assert!(col.cast_field(&target).is_err());

    // With a fill value it succeeds by filling the nulls.
    let filled = col.cast_field_filled(&target, 0).unwrap();
    assert_eq!(filled.null_count(), 0);
    assert_eq!(filled.to_options(), vec![Some(1), Some(0), Some(3)]);
    assert!(!filled.field().nullable());
}

// -------------------------------------------------------------------------------------
// Byte carriers — VarSerie (variable-length utf8)
// -------------------------------------------------------------------------------------

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

#[test]
fn var_with_capacity_append_extend_repeat() {
    let mut col = VarSerie::<Utf8>::with_capacity(4);
    col.append(&strings(&["a", "bb"]));
    assert_eq!(col.len(), 2);

    let other = VarSerie::<Utf8>::from_options(&[Some("ccc".to_string()), None]);
    col.extend(&other);
    assert_eq!(col.get(0), Some("a".to_string()));
    assert_eq!(col.get(2), Some("ccc".to_string()));
    assert_eq!(col.get(3), None); // null carried across the concatenation

    let mut rep = VarSerie::<Utf8>::new();
    rep.push_repeat(&"x".to_string(), 3);
    assert_eq!(rep.values(), strings(&["x", "x", "x"]));
}

#[test]
fn var_mask_filter_reverse_and_lexicographic_sort() {
    let col = VarSerie::<Utf8>::from_values(&strings(&["banana", "apple", "cherry"]));

    let kept = col.mask_filter(&mask_of(&[true, false, true]));
    assert_eq!(kept.values(), strings(&["banana", "cherry"]));

    assert_eq!(
        col.reverse().values(),
        strings(&["cherry", "apple", "banana"])
    );

    // Lexicographic sort (ascending / descending).
    assert_eq!(col.sort().values(), strings(&["apple", "banana", "cherry"]));
    assert_eq!(
        col.take(&col.sort_indices(false)).values(),
        strings(&["cherry", "banana", "apple"])
    );
}

#[test]
fn var_sort_nulls_last() {
    let col = VarSerie::<Utf8>::from_options(&[
        Some("pear".to_string()),
        None,
        Some("apple".to_string()),
    ]);
    let sorted = col.take(&col.sort_indices(true));
    assert_eq!(sorted.get(0), Some("apple".to_string()));
    assert_eq!(sorted.get(1), Some("pear".to_string()));
    assert_eq!(sorted.get(2), None); // null last
}

// -------------------------------------------------------------------------------------
// Byte carriers — FixedSizeSerie (fixed byte width)
// -------------------------------------------------------------------------------------

#[test]
fn fixed_size_with_capacity_append_extend_repeat_reshape() {
    let mut col = FixedSizeSerie::<FixedUtf8>::with_capacity(3, 4);
    col.append(&strings(&["abc", "de"])); // "de" zero-padded to width 3
    assert_eq!(col.len(), 2);

    let other = FixedSizeSerie::<FixedUtf8>::from_values(3, &strings(&["fgh"]));
    col.extend(&other);
    assert_eq!(col.len(), 3);

    let rep = FixedSizeSerie::<FixedUtf8>::repeat(3, &"zz".to_string(), 2);
    assert_eq!(rep.len(), 2);

    // Compaction + reverse over the fixed stride.
    let col = FixedSizeSerie::<FixedUtf8>::from_values(3, &strings(&["aaa", "bbb", "ccc"]));
    let kept = col.mask_filter(&mask_of(&[false, true, true]));
    assert_eq!(kept.len(), 2);
    assert_eq!(kept.get(0), Some("bbb".to_string()));
    assert_eq!(col.reverse().get(0), Some("ccc".to_string()));

    // Lexicographic sort.
    let col = FixedSizeSerie::<FixedUtf8>::from_values(3, &strings(&["ccc", "aaa", "bbb"]));
    let sorted = col.sort();
    assert_eq!(sorted.get(0), Some("aaa".to_string()));
    assert_eq!(sorted.get(2), Some("ccc".to_string()));
}
