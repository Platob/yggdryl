//! The **safe deep-cell setter**: [`AnySerie::set_by_path`] / [`AnySerie::set_at`] overwrite a single
//! leaf cell addressed by a path (or pure coordinates), in place and **length-preservingly** — the
//! mutable, overwrite-only counterpart of `get_by_path`. Every hop is an overwrite of an existing
//! cell, so a set can never change a column length and therefore never desync a struct's equal-length
//! or a list / map's `offsets[last] == child len` invariant. The round-trips (serialize/deserialize
//! succeeds, len unchanged) are the executable proof it stays consistent.

use yggdryl_core::io::fixed::{D128Serie, Field, Serie, D128};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId, IoError};

/// An `i32` leaf cell value, exactly as `Serie::<i32>::value` builds one (empty name/metadata), so it
/// compares equal to what `get_by_path(...).value(i)` reads back.
fn i32_cell(value: i32) -> AnyScalar {
    AnyScalar::leaf(
        Field::of("", DataTypeId::I32, 4, false),
        value.to_le_bytes().to_vec(),
    )
}

/// The task's suggested tree: `struct<a: i32, b: list<i32>>`, 3 rows.
/// `a = [1, 2, 3]`; `b` flattens `[10, 20, 30, 40]` into rows `[[10, 20], [30], [40]]`.
fn build_root() -> Box<dyn AnySerie> {
    let a = Serie::from_values(&[1i32, 2, 3]).named("a");
    let items = Serie::from_values(&[10i32, 20, 30, 40]).named("item");
    let list = ListSerie::from_values(items, &[0, 2, 3, 4], None).unwrap();
    boxed(StructSerie::from_named(vec![("a", a), ("b", boxed(list))]).unwrap())
}

// -------------------------------------------------------------------------------------
// The happy path — overwrite a leaf cell, read it back, prove the length is untouched.
// -------------------------------------------------------------------------------------

#[test]
fn set_by_path_overwrites_a_shallow_leaf_cell() {
    let mut root = build_root();

    // Overwrite row 1 of child `a` (a leaf i32 column): "a" navigates to the column, "[1]" is the cell.
    root.set_by_path("a[1]", &i32_cell(99)).unwrap();

    // get_by_path reads the leaf column back; the cell changed, its neighbours did not, len unchanged.
    let a = root.get_by_path("a").unwrap();
    assert_eq!(a.value(1), i32_cell(99));
    assert_eq!(a.value(0), i32_cell(1));
    assert_eq!(a.value(2), i32_cell(3));
    assert_eq!(a.len(), 3);
    assert_eq!(root.len(), 3);
}

#[test]
fn set_by_path_overwrites_a_deep_leaf_cell_through_a_list() {
    let mut root = build_root();

    // "b" -> the list column, "[0]" -> its flattened item child (schema), "[2]" -> cell 2 of that leaf.
    root.set_by_path("b[0][2]", &i32_cell(999)).unwrap();

    let item = root.get_by_path("b[0]").unwrap();
    assert_eq!(item.value(2), i32_cell(999));
    assert_eq!(item.len(), 4); // the flattened child length is unchanged
    assert_eq!(root.len(), 3);

    // The whole tree still round-trips — deserialize succeeds, so nothing desynced.
    let back = StructSerie::deserialize_bytes(&root.serialize_bytes()).unwrap();
    assert_eq!(back.len(), 3);
    assert_eq!(
        (&back as &dyn AnySerie)
            .get_by_path("b[0]")
            .unwrap()
            .value(2),
        i32_cell(999)
    );
}

#[test]
fn set_by_path_can_set_a_null_cell_leniently() {
    let mut root = build_root();
    assert_eq!(root.get_by_path("a").unwrap().null_count(), 0);

    // A null AnyScalar clears the cell (lenient nullability), still length-preserving.
    root.set_by_path("a[0]", &AnyScalar::Null).unwrap();

    let a = root.get_by_path("a").unwrap();
    assert!(a.value(0).is_null());
    assert_eq!(a.null_count(), 1);
    assert_eq!(a.len(), 3);
    // Round-trips with the new null in place.
    let back = StructSerie::deserialize_bytes(&root.serialize_bytes()).unwrap();
    assert!((&back as &dyn AnySerie)
        .get_by_path("a")
        .unwrap()
        .value(0)
        .is_null());
}

#[test]
fn set_at_overwrites_by_pure_coordinates() {
    let mut root = build_root();

    // Column 0 (`a`), cell 1.
    root.set_at(&[0, 1], &i32_cell(77)).unwrap();
    assert_eq!(root.get_by_path("a").unwrap().value(1), i32_cell(77));

    // Deep: column 1 (`b`) -> item child 0 -> cell 3 of the flattened items.
    root.set_at(&[1, 0, 3], &i32_cell(444)).unwrap();
    assert_eq!(root.get_by_path("b[0]").unwrap().value(3), i32_cell(444));
    assert_eq!(root.len(), 3);
}

// -------------------------------------------------------------------------------------
// A bare leaf column at the root — the "container path" is empty, "[i]" is the cell.
// Exercises each leaf family's `set_cell` (native, var, decimal).
// -------------------------------------------------------------------------------------

#[test]
fn set_by_path_on_a_bare_leaf_column_sets_the_cell() {
    // Native i32.
    let mut col = boxed(Serie::from_values(&[1i32, 2, 3]));
    col.set_by_path("[1]", &i32_cell(20)).unwrap();
    assert_eq!(col.value(1), i32_cell(20));
    assert_eq!(col.len(), 3);

    // Variable-length utf8 (a re-length overwrite of the slot, still no row-count change).
    let mut text = boxed(Utf8Serie::from_strs(&[Some("a"), Some("bb"), Some("ccc")]));
    let hello = AnyScalar::leaf(Field::of("", DataTypeId::Utf8, 4, false), b"hello".to_vec());
    text.set_by_path("[0]", &hello).unwrap();
    assert_eq!(text.value(0), hello);
    assert_eq!(text.len(), 3);

    // Decimal — copy row 2's coefficient into row 0 (matching scale, so `set_coeff_bytes` applies).
    let mut dec = boxed(
        D128Serie::from_values(
            20,
            2,
            &[
                D128::new(100, 2).unwrap(),
                D128::new(200, 2).unwrap(),
                D128::new(300, 2).unwrap(),
            ],
        )
        .unwrap(),
    );
    let three = dec.value(2);
    dec.set_by_path("[0]", &three).unwrap();
    assert_eq!(dec.value(0), three);
    assert_eq!(dec.len(), 3);
}

// -------------------------------------------------------------------------------------
// A map value leaf, reached through the map's `value` child.
// -------------------------------------------------------------------------------------

#[test]
fn set_by_path_reaches_a_map_value_leaf() {
    // Two rows over 3 entries: {"a"->1, "b"->2}, {"c"->3}; values are i32.
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = Serie::from_values(&[1i32, 2, 3]).named("value");
    let mut map = boxed(MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap());

    // "value" -> the flattened value child (leaf i32), "[1]" -> entry 1 of it.
    map.set_by_path("value[1]", &i32_cell(222)).unwrap();
    assert_eq!(map.get_by_path("value").unwrap().value(1), i32_cell(222));
    assert_eq!(map.len(), 2);

    // Keys are untouched and it still round-trips.
    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    let back = &back as &dyn AnySerie;
    assert_eq!(back.get_by_path("value").unwrap().value(1), i32_cell(222));
    assert_eq!(
        back.get_by_path("key").unwrap().value(0),
        map.get_by_path("key").unwrap().value(0)
    );
}

// -------------------------------------------------------------------------------------
// Guided errors — every failure names how to fix it, and never desyncs the column.
// -------------------------------------------------------------------------------------

#[test]
fn set_by_path_empty_path_is_a_guided_error() {
    let mut root = build_root();
    let err = root.set_by_path("", &i32_cell(0)).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("leaf cell"));
    assert_eq!(root.len(), 3); // untouched
}

#[test]
fn set_by_path_bad_path_string_surfaces_the_parse_error() {
    let mut root = build_root();
    let err = root.set_by_path("a..b[0]", &i32_cell(0)).unwrap_err();
    // The guided NodePath parse message passes through unchanged.
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("empty path segment"));
}

#[test]
fn set_by_path_unknown_child_is_a_guided_error() {
    let mut root = build_root();
    let err = root.set_by_path("nope[0]", &i32_cell(0)).unwrap_err();
    assert!(err.to_string().contains("no child named"));
    assert_eq!(root.len(), 3);
}

#[test]
fn set_by_path_interior_index_out_of_range_is_a_guided_error() {
    let mut root = build_root();
    // `b` is a list with a single (index-0) item child; navigating "[9]" into it is out of range.
    let err = root.set_by_path("b[9][0]", &i32_cell(0)).unwrap_err();
    assert!(err.to_string().contains("out of range"));
}

#[test]
fn set_by_path_final_name_segment_is_a_guided_error() {
    let mut root = build_root();
    // A cell is addressed by index; a trailing name addresses a child column, not a cell.
    let err = root.set_by_path("a", &i32_cell(0)).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("must be a cell index"));
}

#[test]
fn set_by_path_to_a_non_leaf_container_is_a_guided_error() {
    let mut root = build_root();
    // "b[0]" addresses the list column `b`'s cell 0 — a whole list row, not a leaf cell.
    let err = root.set_by_path("b[0]", &i32_cell(0)).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("LEAF cell"));
    assert_eq!(root.len(), 3);
}

#[test]
fn set_by_path_cell_index_out_of_range_is_a_guided_error() {
    let mut root = build_root();
    let err = root.set_by_path("a[9]", &i32_cell(0)).unwrap_err();
    assert!(matches!(
        err,
        IoError::IndexOutOfBounds { index: 9, len: 3 }
    ));
    assert_eq!(root.len(), 3);
}

#[test]
fn set_by_path_wrong_value_type_is_a_guided_error() {
    let mut root = build_root();
    // A utf8 value into an i32 leaf cell.
    let wrong = AnyScalar::leaf(Field::of("", DataTypeId::Utf8, 4, false), b"x".to_vec());
    let err = root.set_by_path("a[1]", &wrong).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("must match the leaf column"));
    // The mismatched set left the column unchanged.
    assert_eq!(root.get_by_path("a").unwrap().value(1), i32_cell(2));
}
