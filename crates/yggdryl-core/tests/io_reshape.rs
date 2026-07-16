//! Phase 8a **reshape** surface (`io::reshape`): the row `filter`, the null `fill_null`, and the
//! logical `to_struct` / `to_list` / `to_map` coercions — on the typed columns and the erased
//! [`AnySerie`]. The claims under test: `filter` keeps exactly the selected rows (value *and*
//! null-ness), on leaf **and** nested columns (whole rows kept/dropped for list/map); `fill_null`
//! replaces nulls and drops the mask (recursing to leaf children for nested columns); and the
//! coercions lift a column into a nested one or return it unchanged when it is already the target /
//! no rule applies. Every result asserts its `len` / `null_count` and a serialize/deserialize
//! round-trip.

use yggdryl_core::io::fixed::temporal::{TimeUnit, Ts64, Tz};
use yggdryl_core::io::fixed::{D128Serie, Field, Scalar, Serie, Ts64Serie, D128};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId, IoError};

/// A leaf `i32` fill value (matching an `i32` column).
fn i32_value(value: i32) -> AnyScalar {
    AnyScalar::leaf(
        Field::of("", DataTypeId::I32, 4, false),
        value.to_le_bytes().to_vec(),
    )
}

// -------------------------------------------------------------------------------------
// filter — leaf columns
// -------------------------------------------------------------------------------------

#[test]
fn filter_keeps_the_selected_rows_and_preserves_their_nulls() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3), Some(4)]);
    let kept = col.filter(&[true, true, false, true]).unwrap();
    assert_eq!(kept.to_options(), [Some(1), None, Some(4)]);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept.null_count(), 1);
    // The result round-trips through its own byte codec.
    assert_eq!(
        Serie::<i32>::deserialize_bytes(&kept.serialize_bytes()).unwrap(),
        kept
    );
}

#[test]
fn filter_all_true_all_false_and_empty() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    // all-true keeps everything (value-equal to the source).
    assert_eq!(col.filter(&[true, true, true]).unwrap(), col);
    // all-false drops everything.
    let none = col.filter(&[false, false, false]).unwrap();
    assert_eq!(none.len(), 0);
    assert_eq!(none.null_count(), 0);
    // an empty column with an empty mask is a no-op.
    assert_eq!(Serie::<i32>::new().filter(&[]).unwrap().len(), 0);
}

#[test]
fn filter_length_mismatch_is_a_guided_error() {
    let col = Serie::from_values(&[1i32, 2, 3]);
    let err = col.filter(&[true, false]).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    let message = err.to_string();
    assert!(
        message.contains("filter mask length 2") && message.contains("column length 3"),
        "message should name the mismatch and the fix: {message}"
    );
}

#[test]
fn filter_on_a_var_column() {
    let col = Utf8Serie::from_strs(&[Some("a"), None, Some("cd"), Some("e")]);
    let kept = col.filter(&[true, true, false, true]).unwrap();
    assert_eq!(kept.to_strs(), [Some("a"), None, Some("e")]);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept.null_count(), 1);
    assert_eq!(
        Utf8Serie::deserialize_bytes(&kept.serialize_bytes()).unwrap(),
        kept
    );
}

#[test]
fn erased_filter_matches_the_typed_filter() {
    let erased = boxed(Serie::from_options(&[Some(1i32), None, Some(3), Some(4)]));
    let kept = erased.filter(&[true, false, true, true]).unwrap();
    let expected = Serie::from_options(&[Some(1i32), Some(3), Some(4)]);
    assert!(kept.eq_any(&expected));
    assert_eq!(kept.len(), 3);
}

// -------------------------------------------------------------------------------------
// filter — nested columns (whole rows kept / dropped)
// -------------------------------------------------------------------------------------

#[test]
fn filter_a_struct_filters_every_column_and_the_row_validity() {
    let ids = boxed(Serie::from_values(&[1i64, 2, 3, 4]));
    let names = boxed(Utf8Serie::from_strs(&[
        Some("a"),
        None,
        Some("c"),
        Some("d"),
    ]));
    let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

    let kept = table.filter(&[true, false, true, false]).unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(
        kept.column(0)
            .unwrap()
            .as_serie::<i64>()
            .unwrap()
            .to_options(),
        [Some(1), Some(3)]
    );
    assert_eq!(
        kept.column(1)
            .unwrap()
            .downcast_ref::<Utf8Serie>()
            .unwrap()
            .to_strs(),
        [Some("a"), Some("c")]
    );
    assert_eq!(
        StructSerie::deserialize_bytes(&kept.serialize_bytes()).unwrap(),
        kept
    );
}

#[test]
fn filter_a_struct_with_null_rows() {
    let col = boxed(Serie::from_values(&[1i32, 2, 3]));
    let field = col.field("a");
    let table =
        StructSerie::from_columns(vec![field], vec![col], Some(&[true, false, true])).unwrap();
    // Keep rows 0 and 1 — row 1 is a null struct row, so the result carries one null.
    let kept = table.filter(&[true, true, false]).unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept.null_count(), 1);
    assert_eq!(
        StructSerie::deserialize_bytes(&kept.serialize_bytes()).unwrap(),
        kept
    );
}

#[test]
fn filter_a_list_keeps_whole_rows() {
    // rows: [10, 20], [30], [40]
    let items = Serie::from_values(&[10i32, 20, 30, 40]).named("item");
    let list = ListSerie::from_values(items, &[0, 2, 3, 4], None).unwrap();
    let kept = list.filter(&[true, false, true]).unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept.get_scalar(0).len(), 2); // [10, 20]
    assert_eq!(kept.get_scalar(1).len(), 1); // [40]
    assert_eq!(
        kept.values().as_serie::<i32>().unwrap().to_options(),
        [Some(10), Some(20), Some(40)]
    );
    assert_eq!(
        ListSerie::deserialize_bytes(&kept.serialize_bytes()).unwrap(),
        kept
    );
}

#[test]
fn filter_a_map_keeps_whole_rows() {
    // rows: {a->1, b->2}, {c->3}
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = Serie::from_values(&[1i64, 2, 3]).named("value");
    let map = MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap();
    let kept = map.filter(&[false, true]).unwrap(); // keep only {c->3}
    assert_eq!(kept.len(), 1);
    assert_eq!(kept.get_scalar(0).len(), 1);
    assert_eq!(kept.keys().value(0), map.keys().value(2));
    assert_eq!(
        MapSerie::deserialize_bytes(&kept.serialize_bytes()).unwrap(),
        kept
    );
}

// -------------------------------------------------------------------------------------
// fill_null
// -------------------------------------------------------------------------------------

#[test]
fn fill_null_replaces_nulls_and_drops_the_mask() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3), None]);
    let filled = col.fill_null(0);
    assert_eq!(filled.to_options(), [Some(1), Some(0), Some(3), Some(0)]);
    assert_eq!(filled.null_count(), 0);
    assert_eq!(
        Serie::<i32>::deserialize_bytes(&filled.serialize_bytes()).unwrap(),
        filled
    );
}

#[test]
fn fill_null_with_no_nulls_clones() {
    let dense = Serie::from_values(&[1i32, 2, 3]);
    assert_eq!(dense.fill_null(9), dense);
}

#[test]
fn scalar_fill_null() {
    assert_eq!(Scalar::<i32>::null().fill_null(7), Scalar::of(7));
    assert_eq!(Scalar::of(1i32).fill_null(7), Scalar::of(1));
}

#[test]
fn erased_fill_null_and_type_mismatch() {
    let col = boxed(Serie::from_options(&[Some(1i32), None, Some(3)]));
    let filled = col.fill_null(&i32_value(0)).unwrap();
    assert_eq!(filled.null_count(), 0);
    assert!(filled.eq_any(&Serie::from_values(&[1i32, 0, 3])));

    // A null fill value is a no-op clone.
    let noop = col.fill_null(&AnyScalar::Null).unwrap();
    assert_eq!(noop.null_count(), 1);

    // A wrong-typed value is a guided error.
    let bad = AnyScalar::leaf(
        Field::of("", DataTypeId::I64, 8, false),
        0i64.to_le_bytes().to_vec(),
    );
    let err = col.fill_null(&bad).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("fill the nulls of a i32 column"));
}

#[test]
fn fill_null_a_leaf_inside_a_struct_fills_only_the_matching_column() {
    // An i32 column (with nulls) and a utf8 column (with nulls); filling with an i32 value fills the
    // i32 column and leaves the utf8 column (a non-matching leaf) untouched — the lenient nested rule.
    let nums = boxed(Serie::from_options(&[Some(1i32), None]));
    let strs = boxed(Utf8Serie::from_strs(&[Some("x"), None]));
    let table = StructSerie::from_named(vec![("n", nums), ("s", strs)]).unwrap();

    let filled = table.fill_null(&i32_value(0)).unwrap();
    assert_eq!(filled.column(0).unwrap().null_count(), 0); // i32 filled
    assert_eq!(
        filled
            .column(0)
            .unwrap()
            .as_serie::<i32>()
            .unwrap()
            .to_options(),
        [Some(1), Some(0)]
    );
    assert_eq!(filled.column(1).unwrap().null_count(), 1); // utf8 left unchanged
    assert_eq!(
        StructSerie::deserialize_bytes(&filled.serialize_bytes()).unwrap(),
        filled
    );
}

#[test]
fn fill_null_a_list_child() {
    // rows: [1, null], [3]
    let items = Serie::from_options(&[Some(1i32), None, Some(3)]).named("item");
    let list = ListSerie::from_values(items, &[0, 2, 3], None).unwrap();
    let filled = list.fill_null(&i32_value(7)).unwrap();
    assert_eq!(filled.len(), 2); // rows preserved
    assert_eq!(
        filled.values().as_serie::<i32>().unwrap().to_options(),
        [Some(1), Some(7), Some(3)]
    );
    assert_eq!(
        ListSerie::deserialize_bytes(&filled.serialize_bytes()).unwrap(),
        filled
    );
}

#[test]
fn fill_null_a_map_fills_only_the_value_child() {
    // one row: {a->1, b->null}
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
    let values = Serie::from_options(&[Some(1i64), None]).named("value");
    let map = MapSerie::from_entries(keys, values, &[0, 2], None, false).unwrap();
    let fill = AnyScalar::leaf(
        Field::of("", DataTypeId::I64, 8, false),
        0i64.to_le_bytes().to_vec(),
    );
    let filled = map.fill_null(&fill).unwrap();
    assert_eq!(
        filled.values().as_serie::<i64>().unwrap().to_options(),
        [Some(1), Some(0)]
    );
    assert_eq!(filled.keys().null_count(), 0); // keys stay non-null
    assert_eq!(
        MapSerie::deserialize_bytes(&filled.serialize_bytes()).unwrap(),
        filled
    );
}

// -------------------------------------------------------------------------------------
// coercions: to_struct / to_list / to_map
// -------------------------------------------------------------------------------------

#[test]
fn to_struct_wraps_a_leaf_in_one_field() {
    let col = boxed(Serie::from_values(&[1i32, 2, 3]));
    let st = col.to_struct("n");
    assert_eq!(st.type_id(), DataTypeId::Struct);
    assert_eq!(st.len(), 3);
    let st_serie = st.as_any().downcast_ref::<StructSerie>().unwrap();
    assert_eq!(st_serie.num_columns(), 1);
    assert_eq!(st_serie.field(0).unwrap().name(), "n");
    assert_eq!(
        StructSerie::deserialize_bytes(&st.serialize_bytes()).unwrap(),
        *st_serie
    );
    // Already a struct -> returned unchanged.
    assert!(st.to_struct("other").eq_any(st.as_ref()));
}

#[test]
fn to_list_lifts_a_leaf_into_singletons() {
    let col = boxed(Serie::from_values(&[1i32, 2, 3]));
    let list = col.to_list();
    assert_eq!(list.type_id(), DataTypeId::List);
    assert_eq!(list.len(), 3);
    let list_serie = list.as_any().downcast_ref::<ListSerie>().unwrap();
    for row in 0..3 {
        assert_eq!(list_serie.get_scalar(row).len(), 1); // every row is a singleton
    }
    assert_eq!(
        ListSerie::deserialize_bytes(&list.serialize_bytes()).unwrap(),
        *list_serie
    );
    // Already a list -> returned unchanged.
    assert!(list.to_list().eq_any(list.as_ref()));
}

#[test]
fn to_map_from_a_two_column_struct() {
    let keys = boxed(Utf8Serie::from_strs(&[Some("a"), Some("b")]));
    let values = boxed(Serie::from_values(&[1i64, 2]));
    let table = boxed(StructSerie::from_named(vec![("k", keys), ("v", values)]).unwrap());
    let map = table.to_map().unwrap();
    assert_eq!(map.type_id(), DataTypeId::Map);
    assert_eq!(map.len(), 2);
    let map_serie = map.as_any().downcast_ref::<MapSerie>().unwrap();
    assert_eq!(
        MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap(),
        *map_serie
    );
    // Already a map -> returned unchanged.
    assert!(map.to_map().unwrap().eq_any(map.as_ref()));
}

#[test]
fn to_map_returns_self_when_no_rule_applies() {
    // A leaf column has no map coercion -> returned unchanged.
    let leaf = boxed(Serie::from_values(&[1i32, 2, 3]));
    let same = leaf.to_map().unwrap();
    assert_eq!(same.type_id(), DataTypeId::I32);
    assert!(same.eq_any(leaf.as_ref()));

    // A 3-column struct has no unambiguous key->value reading -> returned unchanged.
    let three = boxed(
        StructSerie::from_named(vec![
            ("a", boxed(Serie::from_values(&[1i32]))),
            ("b", boxed(Serie::from_values(&[2i32]))),
            ("c", boxed(Serie::from_values(&[3i32]))),
        ])
        .unwrap(),
    );
    assert_eq!(three.to_map().unwrap().type_id(), DataTypeId::Struct);
}

#[test]
fn to_map_errors_on_a_null_key_column() {
    let keys = boxed(Utf8Serie::from_strs(&[Some("a"), None]));
    let values = boxed(Serie::from_values(&[1i64, 2]));
    let table = boxed(StructSerie::from_named(vec![("k", keys), ("v", values)]).unwrap());
    assert!(table.to_map().is_err());
}

#[test]
fn typed_serie_coercion_conveniences() {
    let st = Serie::from_values(&[1i32, 2, 3]).to_struct("n");
    assert_eq!(st.num_columns(), 1);
    assert_eq!(st.field(0).unwrap().name(), "n");

    let list = Serie::from_values(&[1i32, 2, 3]).to_list();
    assert_eq!(list.len(), 3);
    assert_eq!(list.get_scalar(2).len(), 1);
}

// -------------------------------------------------------------------------------------
// Regression — erased decimal / temporal fill_null must not ignore scale / (unit, tz).
// -------------------------------------------------------------------------------------

#[test]
fn erased_decimal_fill_null_rejects_a_mismatched_scale_value() {
    // REGRESSION (FIX 1'+6): a D128 column at scale 2 with a null. A value produced by another
    // column's `value()` carries the source scale in its leaf metadata, so a scale-0 coefficient
    // (which would be mis-read as 1/100th the value) must be rejected with a guided error.
    let col = boxed(
        D128Serie::from_options(
            20,
            2,
            &[
                Some(D128::new(500, 2).unwrap()),
                None,
                Some(D128::new(700, 2).unwrap()),
            ],
        )
        .unwrap(),
    );

    let scale0 = boxed(D128Serie::from_values(20, 0, &[D128::new(5, 0).unwrap()]).unwrap());
    let bad_fill = scale0.value(0); // scale 0 — differs from the column's scale 2
    assert!(col.fill_null(&bad_fill).is_err());

    // A same-scale value() leaf fills correctly (matching-scale path is unchanged).
    let scale2 = boxed(D128Serie::from_values(20, 2, &[D128::new(999, 2).unwrap()]).unwrap());
    let good_fill = scale2.value(0);
    let filled = col.fill_null(&good_fill).unwrap();
    assert_eq!(filled.null_count(), 0);
    assert_eq!(filled.value(1), good_fill); // the once-null cell now holds the (scale-2) fill value
}

#[test]
fn erased_temporal_fill_null_rejects_a_mismatched_unit_value() {
    // REGRESSION (FIX 1'+6): a Ts64[s] column with a null. A Ts64[ms] value (same type_id + width)
    // carries unit=millisecond in its leaf metadata; storing its raw count at the column's second
    // resolution would mis-read the instant, so it must be rejected with a guided error.
    let col = boxed(
        Ts64Serie::from_options(
            TimeUnit::Second,
            Tz::UTC,
            &[
                Some(Ts64::from_epoch(10, TimeUnit::Second, Tz::UTC).unwrap()),
                None,
            ],
        )
        .unwrap(),
    );

    let ms = boxed(
        Ts64Serie::from_values(
            TimeUnit::Millisecond,
            Tz::UTC,
            &[Ts64::from_epoch(20, TimeUnit::Millisecond, Tz::UTC).unwrap()],
        )
        .unwrap(),
    );
    let bad_fill = ms.value(0); // millisecond — differs from the column's second unit
    assert!(col.fill_null(&bad_fill).is_err());

    // A same-unit value() leaf fills correctly.
    let s = boxed(
        Ts64Serie::from_values(
            TimeUnit::Second,
            Tz::UTC,
            &[Ts64::from_epoch(99, TimeUnit::Second, Tz::UTC).unwrap()],
        )
        .unwrap(),
    );
    let good_fill = s.value(0);
    let filled = col.fill_null(&good_fill).unwrap();
    assert_eq!(filled.null_count(), 0);
    assert_eq!(filled.value(1), good_fill);
}
