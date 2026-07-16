//! **Phase 9a — erased child-column mutation + range set.** The two binding-facing surfaces:
//!
//! * [`AnySerie::set_child_at`] / [`AnySerie::set_child_by`] replace (struct: add-or-replace) one
//!   child column of a nested struct / list / map in place. The struct schema is *derived* from its
//!   columns, so the change is reflected by [`AnySerie::field`] and survives serialize / deserialize.
//! * [`AnySerie::set_slice`] overwrites a contiguous, length-preserving range of a leaf column, taking
//!   a same-type source through the typed `set_range` fast path and any other through a per-cell
//!   `set_cell` fallback.

use yggdryl_core::io::fixed::{D128Serie, Field, Serie, D128};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar, AnySerie, DataTypeId, IoError};

// ---- cell constructors, exactly as each leaf `value(i)` builds them (empty name/metadata) ---------

fn i64_cell(value: i64) -> AnyScalar {
    AnyScalar::leaf(
        Field::of("", DataTypeId::I64, 8, false),
        value.to_le_bytes().to_vec(),
    )
}

fn i32_cell(value: i32) -> AnyScalar {
    AnyScalar::leaf(
        Field::of("", DataTypeId::I32, 4, false),
        value.to_le_bytes().to_vec(),
    )
}

fn utf8_cell(value: &str) -> AnyScalar {
    AnyScalar::leaf(
        Field::of("", DataTypeId::Utf8, 4, false),
        value.as_bytes().to_vec(),
    )
}

fn a_struct() -> Box<dyn AnySerie> {
    boxed(
        StructSerie::from_named(vec![
            ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
            (
                "name",
                boxed(Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")])),
            ),
        ])
        .unwrap(),
    )
}

fn a_list() -> Box<dyn AnySerie> {
    // 3 rows over the flat child [10,20,30,40]: [[10,20],[30],[40]].
    let items = Serie::from_values(&[10i32, 20, 30, 40]).named("item");
    boxed(ListSerie::from_values(items, &[0, 2, 3, 4], None).unwrap())
}

fn a_map() -> Box<dyn AnySerie> {
    // 2 rows over 3 entries: {"a"->1,"b"->2}, {"c"->3}.
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = Serie::from_values(&[1i64, 2, 3]).named("value");
    boxed(MapSerie::from_entries(keys, values, &[0, 2, 3], None, false).unwrap())
}

// =====================================================================================
// set_child_at
// =====================================================================================

#[test]
fn set_child_at_struct_replaces_a_column_and_the_derived_field_reflects_it() {
    let mut table = a_struct();

    // Replace col 0 (`id`, i64) with an i32 column of the same row count.
    table
        .set_child_at(0, boxed(Serie::from_values(&[10i32, 20, 30])).as_ref())
        .unwrap();

    // field() is derived from the columns: col 0 keeps its NAME ("id") but is now typed I32.
    let field = table.field("t");
    assert_eq!(field.child_field_at(0).unwrap().name(), "id");
    assert_eq!(field.child_field_at(0).unwrap().type_id(), DataTypeId::I32);
    // The data changed; the other column is untouched; the row count is unchanged.
    assert_eq!(table.get_by_path("id").unwrap().value(0), i32_cell(10));
    assert_eq!(table.get_by_path("name").unwrap().value(0), utf8_cell("a"));
    assert_eq!(table.num_children(), 2);
    assert_eq!(table.len(), 3);

    // The whole struct still round-trips, with the new i32 column in place.
    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back.field(0).unwrap().type_id(), DataTypeId::I32);
    assert_eq!(back.field(0).unwrap().name(), "id");
    assert_eq!(back.column(0).unwrap().value(2), i32_cell(30));
}

#[test]
fn set_child_at_struct_length_mismatch_is_a_guided_error() {
    let mut table = a_struct();
    let err = table
        .set_child_at(0, boxed(Serie::from_values(&[10i32, 20])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    let text = err.to_string();
    assert!(text.contains("length 2"), "{text}");
    assert!(text.contains("struct length is 3"), "{text}");
    // The failed set left the struct unchanged.
    assert_eq!(table.get_by_path("id").unwrap().value(0), i64_cell(1));
}

#[test]
fn set_child_at_struct_index_out_of_bounds_is_a_guided_error() {
    let mut table = a_struct();
    let err = table
        .set_child_at(5, boxed(Serie::from_values(&[1i32, 2, 3])).as_ref())
        .unwrap_err();
    assert!(matches!(
        err,
        IoError::IndexOutOfBounds { index: 5, len: 2 }
    ));
}

#[test]
fn set_child_at_on_a_leaf_is_a_guided_error() {
    let mut leaf = boxed(Serie::from_values(&[1i64, 2, 3]));
    let err = leaf
        .set_child_at(0, boxed(Serie::from_values(&[9i64, 9, 9])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("leaf"), "{err}");
}

#[test]
fn set_child_at_list_index_0_replaces_the_item_child() {
    let mut list = a_list();

    // The flattened length is 4; replace the i32 item child with an i64 one of length 4.
    list.set_child_at(0, boxed(Serie::from_values(&[1i64, 2, 3, 4])).as_ref())
        .unwrap();

    // The derived item field reflects the new type; the row count is unchanged.
    assert_eq!(
        list.field("l").child_field_at(0).unwrap().type_id(),
        DataTypeId::I64
    );
    assert_eq!(list.get_by_path("item").unwrap().value(3), i64_cell(4));
    assert_eq!(list.len(), 3);

    // Round-trips with the swapped item child.
    let back = ListSerie::deserialize_bytes(&list.serialize_bytes()).unwrap();
    assert_eq!(back.values().value(0), i64_cell(1));
    assert_eq!(back.len(), 3);
}

#[test]
fn set_child_at_list_wrong_flattened_len_is_a_guided_error() {
    let mut list = a_list();
    let err = list
        .set_child_at(0, boxed(Serie::from_values(&[1i32, 2])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("flattened length 4"), "{err}");
}

#[test]
fn set_child_at_list_nonzero_index_is_a_guided_error() {
    let mut list = a_list();
    let err = list
        .set_child_at(1, boxed(Serie::from_values(&[1i32, 2, 3, 4])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("index 0"), "{err}");
}

#[test]
fn set_child_at_map_replaces_keys_and_values() {
    let mut map = a_map();

    // Index 1 replaces the values column (i64 -> i32, length == 3 entries).
    map.set_child_at(1, boxed(Serie::from_values(&[9i32, 8, 7])).as_ref())
        .unwrap();
    assert_eq!(map.get_by_path("value").unwrap().value(0), i32_cell(9));
    assert_eq!(
        map.field("m").child_field_at(1).unwrap().type_id(),
        DataTypeId::I32
    );

    // Index 0 replaces the keys column (still non-null, length == 3).
    map.set_child_at(
        0,
        boxed(Utf8Serie::from_strs(&[Some("x"), Some("y"), Some("z")])).as_ref(),
    )
    .unwrap();
    assert_eq!(map.get_by_path("key").unwrap().value(0), utf8_cell("x"));
    assert_eq!(map.len(), 2);

    // Round-trips with both children swapped, key stays non-null.
    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    assert_eq!(back.keys().value(2), utf8_cell("z"));
    assert_eq!(back.values().value(0), i32_cell(9));
    assert_eq!(back.keys().null_count(), 0);
}

#[test]
fn set_child_at_map_key_must_stay_non_null() {
    let mut map = a_map();
    let bad = Utf8Serie::from_strs(&[Some("a"), None, Some("c")]); // a null key
    let err = map.set_child_at(0, boxed(bad).as_ref()).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("null key"), "{err}");
    // The key column is untouched (still the original, non-null).
    assert_eq!(map.get_by_path("key").unwrap().value(0), utf8_cell("a"));
}

#[test]
fn set_child_at_map_wrong_len_and_bad_index_are_guided_errors() {
    let mut map = a_map();
    // Wrong length for the values column.
    let err = map
        .set_child_at(1, boxed(Serie::from_values(&[1i64, 2])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("length 2"), "{err}");
    // Index past the two children.
    let err = map
        .set_child_at(2, boxed(Serie::from_values(&[1i64, 2, 3])).as_ref())
        .unwrap_err();
    assert!(err.to_string().contains("two child"), "{err}");
}

// =====================================================================================
// set_child_by
// =====================================================================================

#[test]
fn set_child_by_struct_is_dict_like_add_or_replace() {
    let mut table = boxed(
        StructSerie::from_named(vec![("id", boxed(Serie::from_values(&[1i64, 2])))]).unwrap(),
    );

    // No column named "score" -> ADD it.
    table
        .set_child_by("score", boxed(Serie::from_values(&[9i32, 8])).as_ref())
        .unwrap();
    assert_eq!(table.num_children(), 2);
    assert_eq!(table.field("t").child_field_at(1).unwrap().name(), "score");
    assert_eq!(table.get_by_path("score").unwrap().value(0), i32_cell(9));

    // Column "id" exists -> REPLACE it (no new column added).
    table
        .set_child_by("id", boxed(Serie::from_values(&[100i64, 200])).as_ref())
        .unwrap();
    assert_eq!(table.num_children(), 2);
    assert_eq!(table.get_by_path("id").unwrap().value(0), i64_cell(100));

    // Adding a wrong-length column is a guided error (the row count must match).
    let err = table
        .set_child_by("bad", boxed(Serie::from_values(&[1i32, 2, 3])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));

    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back.num_columns(), 2);
    assert_eq!(back.column_named("score").unwrap().value(1), i32_cell(8));
}

#[test]
fn set_child_by_on_a_field_less_struct_adopts_the_child_len() {
    let mut empty = boxed(StructSerie::from_series(vec![]).unwrap());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.num_children(), 0);

    empty
        .set_child_by("a", boxed(Serie::from_values(&[1i64, 2, 3, 4, 5])).as_ref())
        .unwrap();
    assert_eq!(empty.len(), 5); // the 0-column struct adopts the child's length
    assert_eq!(empty.num_children(), 1);
    assert_eq!(empty.get_by_path("a").unwrap().value(4), i64_cell(5));
}

#[test]
fn set_child_by_map_routes_key_and_value() {
    let mut map = a_map();

    map.set_child_by("value", boxed(Serie::from_values(&[5i64, 6, 7])).as_ref())
        .unwrap();
    assert_eq!(map.get_by_path("value").unwrap().value(2), i64_cell(7));

    map.set_child_by(
        "key",
        boxed(Utf8Serie::from_strs(&[Some("p"), Some("q"), Some("r")])).as_ref(),
    )
    .unwrap();
    assert_eq!(map.get_by_path("key").unwrap().value(1), utf8_cell("q"));

    // An unknown child name is a guided error naming the two children.
    let err = map
        .set_child_by("nope", boxed(Serie::from_values(&[1i64, 2, 3])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    let text = err.to_string();
    assert!(text.contains("key") && text.contains("value"), "{text}");

    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    assert_eq!(back.values().value(0), i64_cell(5));
    assert_eq!(back.keys().value(0), utf8_cell("p"));
}

#[test]
fn set_child_by_list_routes_item() {
    let mut list = a_list();
    list.set_child_by("item", boxed(Serie::from_values(&[7i32, 7, 7, 7])).as_ref())
        .unwrap();
    assert_eq!(list.get_by_path("item").unwrap().value(0), i32_cell(7));

    let err = list
        .set_child_by("nope", boxed(Serie::from_values(&[1i32, 2, 3, 4])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("item"), "{err}");
}

#[test]
fn set_child_by_on_a_leaf_is_a_guided_error() {
    let mut leaf = boxed(Serie::from_values(&[1i64]));
    let err = leaf
        .set_child_by("x", boxed(Serie::from_values(&[9i64])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("leaf"), "{err}");
}

// =====================================================================================
// set_slice
// =====================================================================================

#[test]
fn set_slice_overwrites_a_middle_range_of_an_i64_column() {
    // Fast path: source is the SAME concrete Serie<i64>.
    let mut col = boxed(Serie::from_values(&[0i64, 0, 0, 0, 0]));
    let patch = boxed(Serie::from_options(&[Some(7i64), None]));
    col.set_slice(1, patch.as_ref()).unwrap();

    assert_eq!(
        col.as_serie::<i64>().unwrap().to_options(),
        [Some(0), Some(7), None, Some(0), Some(0)]
    );
    assert_eq!(col.len(), 5); // length preserved

    // The leaf column round-trips with the overwritten range in place.
    let back =
        Serie::<i64>::deserialize_bytes(&col.as_serie::<i64>().unwrap().serialize_bytes()).unwrap();
    assert_eq!(
        back.to_options(),
        [Some(0), Some(7), None, Some(0), Some(0)]
    );
}

#[test]
fn set_slice_overwrites_a_middle_range_of_a_utf8_column() {
    // Fast path via ByteSerie::set_range (an offset splice, changing element lengths).
    let mut text = boxed(Utf8Serie::from_strs(&[
        Some("aa"),
        Some("bb"),
        Some("cc"),
        Some("dd"),
    ]));
    let patch = boxed(Utf8Serie::from_strs(&[Some("longer"), None]));
    text.set_slice(1, patch.as_ref()).unwrap();

    assert_eq!(text.value(0), utf8_cell("aa"));
    assert_eq!(text.value(1), utf8_cell("longer"));
    assert!(text.value(2).is_null());
    assert_eq!(text.value(3), utf8_cell("dd"));
    assert_eq!(text.len(), 4);
}

#[test]
fn set_slice_falls_back_to_per_cell_for_a_non_fast_path_leaf() {
    // A decimal column is not in the fast-path set, so set_slice uses the per-cell set_cell loop;
    // a same-type, same-scale source is written correctly.
    let mut dec = boxed(
        D128Serie::from_values(
            20,
            2,
            &[
                D128::new(100, 2).unwrap(),
                D128::new(200, 2).unwrap(),
                D128::new(300, 2).unwrap(),
                D128::new(400, 2).unwrap(),
            ],
        )
        .unwrap(),
    );
    let patch = boxed(
        D128Serie::from_values(
            20,
            2,
            &[D128::new(999, 2).unwrap(), D128::new(888, 2).unwrap()],
        )
        .unwrap(),
    );
    let expect_1 = patch.value(0);
    let expect_2 = patch.value(1);
    dec.set_slice(1, patch.as_ref()).unwrap();

    assert_eq!(dec.value(1), expect_1);
    assert_eq!(dec.value(2), expect_2);
    assert_eq!(
        dec.value(0),
        boxed(D128Serie::from_values(20, 2, &[D128::new(100, 2).unwrap()]).unwrap()).value(0)
    );
    assert_eq!(dec.len(), 4);
}

#[test]
fn set_slice_out_of_bounds_is_a_guided_error_and_leaves_the_column_unchanged() {
    let mut col = boxed(Serie::from_values(&[0i64, 1, 2, 3, 4]));
    // offset 4 + source len 2 = 6 > 5.
    let err = col
        .set_slice(4, boxed(Serie::from_values(&[8i64, 9])).as_ref())
        .unwrap_err();
    assert!(matches!(
        err,
        IoError::IndexOutOfBounds { index: 6, len: 5 }
    ));
    // Untouched.
    assert_eq!(
        col.as_serie::<i64>().unwrap().to_options(),
        [Some(0), Some(1), Some(2), Some(3), Some(4)]
    );
}

#[test]
fn set_slice_cross_type_source_gives_set_cells_guided_error() {
    // A different concrete type is not fast-pathed; the per-cell set_cell rejects it with its guided
    // type-mismatch error (no silent numeric cast).
    let mut col = boxed(Serie::from_values(&[0i64, 0, 0]));
    let err = col
        .set_slice(0, boxed(Serie::from_values(&[1i32, 2, 3])).as_ref())
        .unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(
        err.to_string().contains("must match the leaf column"),
        "{err}"
    );
}

#[test]
fn set_slice_on_a_nested_column_is_a_guided_error() {
    let mut table = a_struct();
    let src =
        boxed(StructSerie::from_named(vec![("id", boxed(Serie::from_values(&[9i64])))]).unwrap());
    let err = table.set_slice(0, src.as_ref()).unwrap_err();
    assert!(matches!(err, IoError::Unsupported { .. }));
    assert!(err.to_string().contains("nested"), "{err}");
    // The struct is unchanged.
    assert_eq!(table.get_by_path("id").unwrap().value(0), i64_cell(1));
}

#[test]
fn set_slice_of_an_empty_source_is_a_no_op() {
    let mut col = boxed(Serie::from_values(&[1i64, 2, 3]));
    col.set_slice(3, boxed(Serie::<i64>::from_values(&[])).as_ref())
        .unwrap(); // offset == len, empty source
    assert_eq!(col.len(), 3);
    assert_eq!(col.value(0), i64_cell(1));
}
