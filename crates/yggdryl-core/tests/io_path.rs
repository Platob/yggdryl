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
    let row0 = root.row(0); // Struct([ List(struct{b}[10, 20]) ])

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
    let mut table = StructSerie::from_named(vec![
        ("id", boxed(Serie::from_values(&[1i64, 2, 3]))),
        (
            "name",
            boxed(Utf8Serie::from_strs(&[Some("a"), None, Some("c")])),
        ),
    ])
    .unwrap();

    {
        let view: &dyn AnySerie = &table;
        assert_eq!(view.num_children(), 2);
        assert_eq!(view.child_serie_at(0).unwrap().name(), "id");
        assert_eq!(view.child_serie_at(1).unwrap().name(), "name");
        assert!(view.child_serie_at(2).is_none());
        assert!(view.child_serie_by("id").is_some());
        assert!(view.child_serie_by("missing").is_none());
    }

    // The mutable accessor edits a child in place (length-preserving rename).
    {
        let view: &mut dyn AnySerie = &mut table;
        view.child_serie_at_mut(0).unwrap().set_name("renamed");
        assert!(view.child_serie_at_mut(2).is_none());
    }
    assert_eq!(table.column(0).unwrap().name(), "renamed");
    assert_eq!(table.field(0).unwrap().name(), "renamed");
}

#[test]
fn child_serie_access_on_a_list() {
    let items = Serie::from_values(&[1i32, 2, 3]).named("item");
    let mut list = ListSerie::from_values(items, &[0, 3], None).unwrap();

    {
        let view: &dyn AnySerie = &list;
        assert_eq!(view.num_children(), 1);
        assert!(view.child_serie_at(0).is_some());
        assert!(view.child_serie_at(1).is_none());
        // The single child is addressed by the item column's own name.
        assert!(view.child_serie_by("item").is_some());
        assert!(view.child_serie_by("nope").is_none());
    }
    {
        let view: &mut dyn AnySerie = &mut list;
        assert!(view.child_serie_at_mut(0).is_some());
        assert!(view.child_serie_at_mut(1).is_none());
    }
}

#[test]
fn child_serie_access_on_a_map() {
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
    let values = Serie::from_values(&[1i64, 2]).named("value");
    let mut map = MapSerie::from_entries(keys, values, &[0, 2], None, false).unwrap();

    {
        let view: &dyn AnySerie = &map;
        assert_eq!(view.num_children(), 2);
        assert_eq!(view.child_serie_at(0).unwrap().name(), "key");
        assert_eq!(view.child_serie_at(1).unwrap().name(), "value");
        assert!(view.child_serie_at(2).is_none());
        assert!(view.child_serie_by("key").is_some());
        assert!(view.child_serie_by("value").is_some());
        assert!(view.child_serie_by("nope").is_none());
    }
    {
        let view: &mut dyn AnySerie = &mut map;
        assert!(view.child_serie_at_mut(0).is_some());
        assert!(view.child_serie_at_mut(1).is_some());
        assert!(view.child_serie_at_mut(2).is_none());
    }
}

#[test]
fn child_serie_access_on_a_leaf_is_empty() {
    let leaf = Serie::from_values(&[1i32, 2, 3]);
    let view: &dyn AnySerie = &leaf;
    assert_eq!(view.num_children(), 0);
    assert!(view.child_serie_at(0).is_none());
    assert!(view.child_serie_by("anything").is_none());

    let mut leaf = leaf;
    let view: &mut dyn AnySerie = &mut leaf;
    assert!(view.child_serie_at_mut(0).is_none());
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
