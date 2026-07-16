//! Path addressing: [`NodePath`] parsing / value-type discipline, the unified child accessors
//! (`child_serie_*` / `child_field_*` / `child_scalar_*`), and the `get_by_path` resolvers on the
//! erased serie, field, and scalar surfaces.

use yggdryl_core::io::fixed::{Field, Serie};
use yggdryl_core::io::nested::{ListSerie, MapSerie, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{
    boxed, AnyField, AnyScalar, AnySerie, DataTypeId, FieldType, NodePath, PathError, PathSegment,
};

// -------------------------------------------------------------------------------------
// NodePath — parsing, canonical render, value-type round-trip.
// -------------------------------------------------------------------------------------

#[test]
fn node_path_parses_every_segment_shape() {
    // Bareword.
    assert_eq!(
        NodePath::parse("field").unwrap().segments(),
        &[PathSegment::name("field")]
    );
    // Dotted and hyphen separators are equivalent.
    assert_eq!(
        NodePath::parse("a.b.c").unwrap(),
        NodePath::parse("a-b-c").unwrap()
    );
    // Bracket-index (all three bracket styles) and chaining onto a name.
    assert_eq!(
        NodePath::parse("[2]").unwrap(),
        NodePath::parse("(2)").unwrap()
    );
    assert_eq!(
        NodePath::parse("(2)").unwrap(),
        NodePath::parse("{2}").unwrap()
    );
    assert_eq!(
        NodePath::parse("a[0].b").unwrap().segments(),
        &[
            PathSegment::name("a"),
            PathSegment::index(0),
            PathSegment::name("b")
        ]
    );
    // Backtick-quoting makes breaking chars literal; a doubled backtick is one literal backtick.
    assert_eq!(
        NodePath::parse("`a.b-c`").unwrap().segments(),
        &[PathSegment::name("a.b-c")]
    );
    assert_eq!(
        NodePath::parse("`x``y`.z").unwrap().segments(),
        &[PathSegment::name("x`y"), PathSegment::name("z")]
    );
}

#[test]
fn node_path_errors_are_guided() {
    assert!(matches!(
        NodePath::parse("a..b").unwrap_err(),
        PathError::EmptySegment { .. }
    ));
    assert!(matches!(
        NodePath::parse("a-").unwrap_err(),
        PathError::TrailingSeparator { .. }
    ));
    assert!(matches!(
        NodePath::parse("`a`b").unwrap_err(),
        PathError::MissingSeparator { .. }
    ));
    assert!(matches!(
        NodePath::parse("a[1").unwrap_err(),
        PathError::UnmatchedBracket { .. }
    ));
    assert!(matches!(
        NodePath::parse("a}").unwrap_err(),
        PathError::UnmatchedClose { .. }
    ));
    let err = NodePath::parse("a[nope]").unwrap_err();
    assert!(matches!(err, PathError::NonIntegerIndex { .. }));
    assert!(err.to_string().contains("nope"));
    assert!(matches!(
        NodePath::parse("`abc").unwrap_err(),
        PathError::UnterminatedQuote { .. }
    ));
}

#[test]
fn node_path_is_a_value_type() {
    use std::collections::HashSet;

    let path = NodePath::parse("outer[3].`in.ner`.leaf").unwrap();
    // Display round-trips through parse.
    assert_eq!(NodePath::parse(&path.to_string()).unwrap(), path);
    // The byte codec is the exact inverse.
    assert_eq!(
        NodePath::deserialize_bytes(&path.serialize_bytes()).unwrap(),
        path
    );
    // Equal paths (dotted vs hyphen) hash equal.
    let set: HashSet<NodePath> = [
        NodePath::parse("a.b").unwrap(),
        NodePath::parse("a-b").unwrap(),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 1);
    // Non-UTF-8 bytes are rejected with a guided error.
    assert!(matches!(
        NodePath::deserialize_bytes(&[0xff]).unwrap_err(),
        PathError::NonUtf8 { .. }
    ));
}

// -------------------------------------------------------------------------------------
// The shared test tree: struct<a: list<struct<{b: i32}>>>.
// -------------------------------------------------------------------------------------

/// The inner `b` column is `[10, 20, 30]`, partitioned by the list into rows `[[e0, e1], [e2]]`.
fn build_tree() -> StructSerie {
    let b = Serie::from_values(&[10i32, 20, 30]).named("b");
    let inner = boxed(StructSerie::from_series(vec![b]).unwrap());
    let list = ListSerie::from_values(inner, &[0, 2, 3], None).unwrap();
    StructSerie::from_named(vec![("a", boxed(list))]).unwrap()
}

// -------------------------------------------------------------------------------------
// get_by_path — symmetric across serie, field, and scalar.
// -------------------------------------------------------------------------------------

#[test]
fn get_by_path_on_a_serie_walks_the_schema_structure() {
    let root = build_tree();
    let root: &dyn AnySerie = &root;

    // "a" -> the list column; "[0]" -> the item child (flattened inner struct); "b" -> the i32 column.
    let b = root.get_by_path("a[0].b").unwrap();
    assert_eq!(b.name(), "b");
    assert_eq!(b.type_id(), DataTypeId::I32);
    assert_eq!(b.len(), 3);

    // The empty path is the identity.
    assert!(root.get_by_path("").unwrap().type_id().is_struct());

    // Guided resolution errors.
    assert!(matches!(
        root.get_by_path("nope").unwrap_err(),
        PathError::NoChildNamed { depth: 0, .. }
    ));
    assert!(matches!(
        root.get_by_path("a[9]").unwrap_err(),
        PathError::ChildIndexOutOfRange { depth: 1, .. }
    ));
}

#[test]
fn get_by_path_on_a_field_mirrors_the_serie() {
    let root = build_tree();
    let field: AnyField = (&root as &dyn AnySerie).field_self();

    let leaf = field.get_by_path("a[0].b").unwrap();
    assert_eq!(leaf.name(), "b");
    assert_eq!(FieldType::type_id(leaf), DataTypeId::I32);
    assert!(!leaf.is_nested());

    assert!(matches!(
        field.get_by_path("a[0].zzz").unwrap_err(),
        PathError::NoChildNamed { depth: 2, .. }
    ));
}

#[test]
fn get_by_path_on_a_scalar_drills_the_data_positionally() {
    let root = build_tree();
    let row0 = root.get(0); // Struct([ List(struct{b}[10, 20]) ])

    // A value's children are its data, so the path is positional: field 0 -> element 0 -> field 0.
    let leaf = row0.get_by_path("[0][0][0]").unwrap();
    assert_eq!(leaf.type_id(), Some(DataTypeId::I32));
    assert_eq!(leaf.bytes(), Some(&10i32.to_le_bytes()[..]));

    // The second element of that list row is b = 20.
    let leaf2 = row0.get_by_path("[0][1][0]").unwrap();
    assert_eq!(leaf2.bytes(), Some(&20i32.to_le_bytes()[..]));

    // An erased struct value is positional (unnamed), so a name segment does not resolve.
    assert!(matches!(
        row0.get_by_path("a").unwrap_err(),
        PathError::NoChildNamed { depth: 0, .. }
    ));
    // An out-of-range element index is a guided error.
    assert!(matches!(
        row0.get_by_path("[0][9]").unwrap_err(),
        PathError::ChildIndexOutOfRange { depth: 1, .. }
    ));
}

#[test]
fn scalar_child_access_resolves_named_leaf_children() {
    // A struct value built with named leaf children resolves by name (and by index).
    let value = AnyScalar::struct_(vec![
        AnyScalar::leaf(
            Field::of("b", DataTypeId::I32, 4, false),
            10i32.to_le_bytes().to_vec(),
        ),
        AnyScalar::leaf(
            Field::of("c", DataTypeId::I32, 4, false),
            20i32.to_le_bytes().to_vec(),
        ),
    ]);
    assert_eq!(value.num_children(), 2);
    assert_eq!(value.child_scalar_by("b"), value.child_scalar_at(0));
    assert_eq!(value.child_scalar_by("c"), value.child_scalar_at(1));
    assert_eq!(
        value.get_by_path("c").unwrap().bytes(),
        Some(&20i32.to_le_bytes()[..])
    );
    assert!(value.child_scalar_by("missing").is_none());

    // A leaf / null value has no children.
    assert_eq!(AnyScalar::null().num_children(), 0);
    assert!(AnyScalar::null().child_scalar_at(0).is_none());
}

// -------------------------------------------------------------------------------------
// Unified child access on the erased serie — struct / list / map override, leaf is empty.
// -------------------------------------------------------------------------------------

#[test]
fn child_serie_access_on_a_struct() {
    let table = StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
        (
            "name",
            boxed(Utf8Serie::from_strs(&[Some("a"), None, Some("c")])),
        ),
    ])
    .unwrap();

    let view: &dyn AnySerie = &table;
    assert_eq!(view.num_children(), 2);
    assert_eq!(view.child_serie_at(0).unwrap().name(), "id");
    assert_eq!(view.child_serie_at(1).unwrap().name(), "name");
    assert!(view.child_serie_at(2).is_none());
    assert!(view.child_serie_by("id").is_some());
    assert!(view.child_serie_by("missing").is_none());

    // COMPILE-FENCE (BUG 2+3): child access is read-only on the public trait. There is no
    // `child_serie_at_mut` — a raw `&mut dyn AnySerie` child could `append_scalar` / `concat` and
    // desync the parent's length invariant, so it is gone. Uncommenting the next line must NOT
    // compile:
    //     let _ = (&mut table as &mut dyn AnySerie).child_serie_at_mut(0);
}

#[test]
fn child_serie_access_on_a_list() {
    let items = Serie::from_values(&[1i32, 2, 3]).named("item");
    let list = ListSerie::from_values(items, &[0, 3], None).unwrap();

    let view: &dyn AnySerie = &list;
    assert_eq!(view.num_children(), 1);
    assert!(view.child_serie_at(0).is_some());
    assert!(view.child_serie_at(1).is_none());
    // The single child is addressed by the item column's own name.
    assert!(view.child_serie_by("item").is_some());
    assert!(view.child_serie_by("nope").is_none());
}

#[test]
fn child_serie_access_on_a_map() {
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
    let values = Serie::from_values(&[1i64, 2]).named("value");
    let map = MapSerie::from_entries(keys, values, &[0, 2], None, false).unwrap();

    let view: &dyn AnySerie = &map;
    assert_eq!(view.num_children(), 2);
    assert_eq!(view.child_serie_at(0).unwrap().name(), "key");
    assert_eq!(view.child_serie_at(1).unwrap().name(), "value");
    assert!(view.child_serie_at(2).is_none());
    assert!(view.child_serie_by("key").is_some());
    assert!(view.child_serie_by("value").is_some());
    assert!(view.child_serie_by("nope").is_none());
}

#[test]
fn child_serie_access_on_a_leaf_is_empty() {
    let leaf = Serie::from_values(&[1i32, 2, 3]);
    let view: &dyn AnySerie = &leaf;
    assert_eq!(view.num_children(), 0);
    assert!(view.child_serie_at(0).is_none());
    assert!(view.child_serie_by("anything").is_none());
}

// -------------------------------------------------------------------------------------
// Unified child access on the erased field.
// -------------------------------------------------------------------------------------

#[test]
fn child_field_access_mirrors_the_serie() {
    let root = build_tree();
    let field: AnyField = (&root as &dyn AnySerie).field_self();

    // struct<a: ...> has one child field "a" (a list).
    assert_eq!(field.num_children(), 1);
    assert_eq!(field.child_field_by("a").unwrap().name(), "a");
    assert!(field.child_field_at(0).unwrap().is_list());
    assert!(field.child_field_at(1).is_none());
    assert!(field.child_field_by("missing").is_none());

    // The list field has one item child; a leaf field has none.
    let list_field = field.child_field_by("a").unwrap();
    assert_eq!(list_field.num_children(), 1);
    let item = list_field.child_field_at(0).unwrap();
    assert!(item.is_struct());
    let leaf = item.child_field_by("b").unwrap();
    assert_eq!(leaf.num_children(), 0);
    assert!(leaf.child_field_at(0).is_none());
}

// -------------------------------------------------------------------------------------
// BUG 2+3: the mutable-child footgun is gone — public mutation stays length-synced.
// -------------------------------------------------------------------------------------

#[test]
fn append_row_grows_every_child_consistently_no_mut_child_footgun() {
    // The public API exposes no `&mut` child that can append; growth goes through `append_row` /
    // `append_null`, which grow ALL children together, keeping the struct's equal-length invariant.
    let mut table = StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[1i64, 2]))),
        ("name", boxed(Utf8Serie::from_strs(&[Some("a"), Some("b")]))),
    ])
    .unwrap();
    assert_eq!(table.len(), 2);

    let row = table.get(0); // reuse row 0's cells as a new row
    table.append_row(row.as_struct().unwrap()).unwrap();
    table.append_null();

    // The struct length and EVERY child column length moved together (no per-child desync).
    assert_eq!(table.len(), 4);
    assert_eq!(table.column(0).unwrap().len(), 4);
    assert_eq!(table.column(1).unwrap().len(), 4);

    // The frame still round-trips (a desynced struct would fail the child-length check on read).
    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back, table);
}

#[test]
fn map_key_stays_non_null_after_append_row() {
    let keys = Utf8Serie::from_strs(&[Some("a")]).named("key");
    let values = Serie::from_values(&[1i64]).named("value");
    let mut map = MapSerie::from_entries(keys, values, &[0, 1], None, false).unwrap();

    // Grow through the parent's `append_row` (the only public grow path) — offsets, entries, and
    // validity stay in lock-step, and the key field stays non-nullable.
    map.append_row(
        boxed(Utf8Serie::from_strs(&[Some("b"), Some("c")])),
        boxed(Serie::from_values(&[2i64, 3])),
    )
    .unwrap();
    assert_eq!(map.len(), 2);
    assert!(!map.key_field().nullable(), "a map key is never null");
    assert_eq!(map.keys().len(), 3);
    assert_eq!(map.values().len(), 3);

    // A null key row is rejected (the invariant is enforced on the public grow path).
    let err = map
        .append_row(
            boxed(Utf8Serie::from_strs(&[None])),
            boxed(Serie::from_values(&[9i64])),
        )
        .unwrap_err();
    assert!(err.to_string().contains("null"), "got {err}");

    // The map still round-trips, key non-null preserved.
    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    assert_eq!(back, map);
    assert!(!back.key_field().nullable());
}

// -------------------------------------------------------------------------------------
// BUG 5: serie and field navigation AGREE on the canonical map "key"/"value" and list "item".
// -------------------------------------------------------------------------------------

#[test]
fn serie_and_field_agree_on_canonical_map_and_list_child_names() {
    // A map whose entry columns are NOT named "key"/"value" (they are "k"/"v"): the canonical
    // fallback must resolve "key"/"value" on BOTH the serie and the field surfaces.
    let keys = Utf8Serie::from_strs(&[Some("x"), Some("y")]).named("k");
    let values = Serie::from_values(&[1i64, 2]).named("v");
    let map = MapSerie::from_entries(keys, values, &[0, 2], None, false).unwrap();
    let map_serie: &dyn AnySerie = &map;
    let map_field: AnyField = map_serie.field_self();

    // "key" resolves to the key child on both sides (via the canonical fallback, since it is "k").
    assert_eq!(map_serie.get_by_path("key").unwrap().name(), "k");
    assert_eq!(map_field.get_by_path("key").unwrap().name(), "k");
    // "value" likewise.
    assert_eq!(map_serie.get_by_path("value").unwrap().name(), "v");
    assert_eq!(map_field.get_by_path("value").unwrap().name(), "v");
    // The actual names still resolve too, and both surfaces agree on a miss.
    assert_eq!(map_field.get_by_path("k").unwrap().name(), "k");
    assert!(map_serie.get_by_path("nope").is_err());
    assert!(map_field.get_by_path("nope").is_err());

    // A list whose item column is NOT named "item" (it is "it"): "item" resolves on both sides.
    let items = Serie::from_values(&[10i32, 20, 30]).named("it");
    let list = ListSerie::from_values(items, &[0, 3], None).unwrap();
    let list_serie: &dyn AnySerie = &list;
    let list_field: AnyField = list_serie.field_self();
    assert_eq!(list_serie.get_by_path("item").unwrap().name(), "it");
    assert_eq!(list_field.get_by_path("item").unwrap().name(), "it");
    assert_eq!(list_field.get_by_path("it").unwrap().name(), "it");
}
