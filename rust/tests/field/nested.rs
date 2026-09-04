use yggdryl::types::nested;
use yggdryl::{DataType, Field, UnionMode};

use crate::typed::assert_typed_marker;

#[test]
fn nested_markers_cover_every_child_layout() {
    let item = || Field::new("item", DataType::Utf8, true);
    assert_typed_marker::<nested::List>(DataType::list(item()));
    assert_typed_marker::<nested::ListView>(DataType::list_view(item()));
    assert_typed_marker::<nested::FixedSizeList>(DataType::fixed_size_list(item(), 3).unwrap());
    assert_typed_marker::<nested::LargeList>(DataType::large_list(item()));
    assert_typed_marker::<nested::LargeListView>(DataType::large_list_view(item()));
    assert_typed_marker::<nested::Struct>(DataType::from_fields([item()]).unwrap());
    assert_typed_marker::<nested::Union>(DataType::union([(4, item())], UnionMode::Dense).unwrap());
    assert_typed_marker::<nested::Dictionary>(
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
    );
    assert_typed_marker::<nested::Map>(
        DataType::map_of(DataType::Utf8, DataType::Int64, false).unwrap(),
    );
    assert_typed_marker::<nested::RunEndEncoded>(
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap(),
    );
}

/// Item access on a schema node reaches a nested child, never metadata.
///
/// Before this, `field["level"]` was a metadata lookup while
/// `dtype["level"]` was a child, so a caller walking one object graph got
/// two unrelated things from identical syntax. Children win: subscripting a
/// schema node descends the schema.
#[test]
fn subscripting_a_schema_node_reaches_a_nested_child() {
    let line = DataType::from_fields([
        DataType::Float64.required_field("price"),
        DataType::Int64.required_field("qty"),
    ])
    .unwrap()
    .required_field("line");
    let order = DataType::from_fields([DataType::Int64.required_field("id"), line])
        .unwrap()
        .required_field("order");

    // By name and by position, on both `Field` and `DataType`, one answer.
    assert_eq!(order["id"].dtype(), &DataType::Int64);
    assert_eq!(order.dtype()["id"].dtype(), &DataType::Int64);
    assert_eq!(order[0].name(), "id");
    assert_eq!(order.dtype()[1].name(), "line");

    // Chained subscripts are the nesting story - no dotted path form.
    assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);
    assert_eq!(order["line"]["qty"].dtype(), &DataType::Int64);

    // Through a List item and a Map entry, the same way.
    let items = DataType::list(order.clone().with_name("item"));
    assert_eq!(items[0]["id"].dtype(), &DataType::Int64);
    assert_eq!(items["item"]["line"]["price"].dtype(), &DataType::Float64);

    // The non-panicking form stays available and is what the docs point at.
    assert!(order.get_field_by_path("absent").is_none());
    assert!(order.get_field(9).is_none());
}

/// Metadata is not reachable by subscript any more, but is through its view.
#[test]
fn metadata_is_reached_through_its_own_surface_not_a_subscript() {
    let mut field = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    field.insert_metadata("owner", "tests").unwrap();

    // The subscript descends the schema; the metadata key is not a child.
    assert_eq!(field["id"].dtype(), &DataType::Int64);
    assert!(field.get_field_by_path("owner").is_none());

    // The named accessors and the view still answer it.
    assert_eq!(field.get_metadata("owner"), Some("tests"));
    assert_eq!(field.as_metadata().get("owner"), Some("tests"));
}

#[test]
#[should_panic(expected = "is not a child of the field")]
fn subscripting_an_absent_child_panics_with_a_useful_message() {
    let row = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let _ = &row["absent"];
}

#[test]
#[should_panic(expected = "is not a child of the datatype")]
fn subscripting_a_non_nested_datatype_panics_naming_it() {
    let _ = &DataType::Int64["anything"];
}

#[test]
#[should_panic(expected = "so position 5 is out of range")]
fn subscripting_past_the_end_panics_naming_the_arity() {
    let row = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let _ = &row[5];
}

/// Child mutation is named and cache-aware; no `&mut` child escapes it.
#[test]
fn child_mutation_replaces_by_position_and_appends_by_unknown_name() {
    let mut row = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");

    // An unknown name appends - dict-like, and how a schema is built up.
    row.set_field_by_path("venue", DataType::Utf8.nullable_field("venue"))
        .unwrap();
    assert_eq!(row.field_len(), 2);
    assert_eq!(row[1].name(), "venue");

    // A known name replaces in place, keeping its position.
    row.set_field_by_path("id", DataType::Utf8.required_field("id"))
        .unwrap();
    assert_eq!(row.field_len(), 2);
    assert_eq!(row[0].name(), "id");
    assert_eq!(row["id"].dtype(), &DataType::Utf8);

    // A position replaces only, and never grows the node silently.
    row.set_field(1, DataType::LargeUtf8.nullable_field("venue"))
        .unwrap();
    assert_eq!(row["venue"].dtype(), &DataType::LargeUtf8);
    let message = row
        .set_field(7, DataType::Int64.nullable_field("late"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("a child position below 2"), "{message}");
    assert_eq!(row.field_len(), 2, "a refusal leaves the field unchanged");

    // Removal returns the prior child and closes the gap.
    let dropped = row.remove_field_by_path("id").unwrap();
    assert_eq!(dropped.name(), "id");
    assert_eq!(row.field_len(), 1);
    assert_eq!(row[0].name(), "venue");

    // A node with no children to replace says so rather than panicking.
    let mut scalar = DataType::Int64.required_field("price");
    let message = scalar
        .set_field_by_path("child", DataType::Int64.nullable_field("child"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("a struct field"), "{message}");
}

#[test]
fn a_path_resolves_by_name_before_it_decomposes() {
    let row = DataType::from_fields([
        DataType::from_fields([DataType::Float64.required_field("price")])
            .unwrap()
            .required_field("line"),
        DataType::Int64.required_field("a.b"),
    ])
    .unwrap()
    .required_field("row");

    // A route through the graph.
    assert_eq!(row.field_by_path("line.price").unwrap().name(), "price");

    // A child carrying the whole string wins over that route, so a name with a
    // dot in it stays reachable.
    assert_eq!(row.field_by_path("a.b").unwrap().dtype(), &DataType::Int64);
    assert_eq!(row["a.b"].dtype(), &DataType::Int64);

    // `a.b` names a child but carries no `c`, and no other boundary resolves.
    assert!(row.get_field_by_path("a.b.c").is_none());

    // The same string does resolve when the route exists.
    let deep = DataType::from_fields([DataType::from_fields([DataType::Utf8.required_field("c")])
        .unwrap()
        .required_field("a.b")])
    .unwrap()
    .required_field("deep");
    assert_eq!(deep.field_by_path("a.b.c").unwrap().name(), "c");

    // A path naming nothing reports the children that do exist.
    let message = row.field_by_path("missing").unwrap_err().to_string();
    assert!(message.contains("line"), "{message}");
}

#[test]
fn a_list_is_transparent_to_a_dotted_path_when_reading() {
    let item = DataType::from_fields([
        DataType::Float64.required_field("price"),
        DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("party"),
    ])
    .unwrap()
    .required_field("item");
    let orders = DataType::from_fields([DataType::list(item.clone()).nullable_field("orders")])
        .unwrap()
        .required_field("row");

    // The item is a step the path need not spell, and both spellings agree.
    assert_eq!(
        orders.field_by_path("orders.price").unwrap().name(),
        "price"
    );
    assert_eq!(
        orders.field_by_path("orders.item.price").unwrap().name(),
        "price"
    );
    assert_eq!(
        orders.field_by_path("orders.party.id").unwrap().name(),
        "id"
    );
    assert_eq!(orders["orders"]["price"].name(), "price");
    assert_eq!(
        orders.get_field("orders.price"),
        orders.get_field("orders.item.price")
    );

    // The item's own name still wins outright.
    assert_eq!(orders.field_by_path("orders.item").unwrap().name(), "item");

    // A path that resolves through no child reports the path that failed.
    let message = orders
        .field_by_path("orders.quantity")
        .unwrap_err()
        .to_string();
    assert!(message.contains("orders.quantity"), "{message}");
    assert!(orders.get_field_by_path("orders.quantity").is_none());

    // Every list layout reads the same way; a map keeps its entries by name.
    let leaf = DataType::from_fields([DataType::Int64.required_field("value")])
        .unwrap()
        .required_field("item");
    for layout in [
        DataType::list(leaf.clone()),
        DataType::large_list(leaf.clone()),
        DataType::list_view(leaf.clone()),
        DataType::large_list_view(leaf.clone()),
        DataType::fixed_size_list(leaf.clone(), 2).unwrap(),
    ] {
        assert_eq!(
            layout.get_field_by_path("value").map(Field::name),
            Some("value"),
            "{layout}"
        );
        assert_eq!(
            layout.get_field_by_path("item.value").map(Field::name),
            Some("value"),
            "{layout}"
        );
    }
    let map = DataType::map_of(DataType::Utf8, DataType::Int64, false).unwrap();
    assert!(map.get_field_by_path("value").is_none());
    assert_eq!(
        map.get_field_by_path("entries.value").map(Field::name),
        Some("value")
    );

    // A write is not transparent: it addresses the item by its own name, and
    // a list never grows a second child.
    let mut written = orders.clone();
    written
        .set_field_by_path(
            "orders.item.price",
            DataType::Float32.required_field("price"),
        )
        .unwrap();
    assert_eq!(written["orders"]["price"].dtype(), &DataType::Float32);
    assert_eq!(
        written
            .remove_field_by_path("orders.item.party")
            .unwrap()
            .name(),
        "party"
    );
    assert_eq!(written["orders"].dtype().field_len(), 1);
    assert_eq!(written["orders"]["item"].field_len(), 1);
}

#[test]
fn one_key_reaches_a_child_by_position_or_by_path() {
    let row = DataType::from_fields([DataType::from_fields([
        DataType::Float64.required_field("price")
    ])
    .unwrap()
    .required_field("line")])
    .unwrap()
    .required_field("row");

    // The same call, whichever spelling the caller holds.
    assert_eq!(row.field(0).unwrap().name(), "line");
    assert_eq!(row.field("line").unwrap().name(), "line");
    assert_eq!(row.get_field("line.price").unwrap().name(), "price");
    assert!(row.get_field(9).is_none());
    assert!(row.get_field("absent").is_none());

    // `DataType` answers identically, so descending never changes the calls.
    let dtype = row.dtype();
    assert_eq!(dtype.field(0).unwrap().name(), "line");
    assert_eq!(dtype.field_by_path("line.price").unwrap().name(), "price");
}

#[test]
fn a_datatype_replaces_removes_and_keeps_its_layout() {
    // A position replaces through every layout, keeping it.
    let mut list = DataType::list(DataType::Int32.nullable_field("item"));
    list.set_field_at(0, DataType::Int64.nullable_field("item"))
        .unwrap();
    assert_eq!(list, DataType::list(DataType::Int64.nullable_field("item")));

    // Growing or shrinking is a struct's business: a list holds exactly one
    // child, so it refuses rather than quietly becoming a struct.
    let message = list
        .set_field_by_path("extra", DataType::Utf8.nullable_field("extra"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("a struct field"), "{message}");
    assert!(list.remove_field_at(0).is_err());
    assert_eq!(list, DataType::list(DataType::Int64.nullable_field("item")));

    // A struct grows by an unresolved name and shrinks by either key.
    let mut row = DataType::from_fields([DataType::Int64.required_field("id")]).unwrap();
    row.set_field("venue", DataType::Utf8.nullable_field("venue"))
        .unwrap();
    assert_eq!(row.field_len(), 2);
    assert_eq!(row.remove_field("venue").unwrap().name(), "venue");
    assert_eq!(row.remove_field(0).unwrap().name(), "id");
    assert_eq!(row.field_len(), 0);
}

#[test]
fn setting_by_path_reaches_a_nested_child() {
    let mut row = DataType::from_fields([DataType::from_fields([
        DataType::Int32.required_field("price")
    ])
    .unwrap()
    .required_field("line")])
    .unwrap()
    .required_field("row");

    row.set_field_by_path("line.price", DataType::Float64.required_field("price"))
        .unwrap();
    assert_eq!(row["line"]["price"].dtype(), &DataType::Float64);

    // Removing reaches the same child, and the parent keeps its own identity.
    assert_eq!(row.remove_field("line.price").unwrap().name(), "price");
    assert_eq!(row["line"].field_len(), 0);
    assert_eq!(row.field_len(), 1);
}

#[test]
fn two_datatypes_meet_at_the_one_that_holds_both() {
    let up = |left: &DataType, right: &DataType| left.merge_with(right, true).unwrap();
    let down = |left: &DataType, right: &DataType| left.merge_with(right, false).unwrap();

    // Width resolves in the direction asked for, and only in that direction.
    assert_eq!(up(&DataType::Int32, &DataType::Int64), DataType::Int64);
    assert_eq!(down(&DataType::Int32, &DataType::Int64), DataType::Int32);
    assert_eq!(
        up(&DataType::Float32, &DataType::Float64),
        DataType::Float64
    );
    assert_eq!(down(&DataType::Utf8, &DataType::LargeUtf8), DataType::Utf8);

    // A null column has no shape, so it takes the other's in either position
    // and in either direction.
    assert_eq!(up(&DataType::Null, &DataType::Utf8), DataType::Utf8);
    assert_eq!(down(&DataType::Int64, &DataType::Null), DataType::Int64);

    // Bytes hold every other encoding, and text holds all but bytes.
    assert_eq!(up(&DataType::Utf8, &DataType::Binary), DataType::Binary);
    assert_eq!(up(&DataType::Int64, &DataType::Binary), DataType::Binary);
    assert_eq!(up(&DataType::Int64, &DataType::Utf8), DataType::Utf8);
    assert_eq!(up(&DataType::Boolean, &DataType::Utf8), DataType::Utf8);
    assert_eq!(up(&DataType::Date32, &DataType::Utf8), DataType::Utf8);

    // A pair with no meeting point that is not a re-encoding is refused, and
    // the refusal names both sides.
    let refused = DataType::Boolean
        .merge_with(&DataType::Int64, true)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("boolean") || refused.contains("int64"),
        "{refused}"
    );
    assert!(
        DataType::Float64
            .merge_with(&DataType::decimal128(10, 2).unwrap(), true)
            .is_err(),
        "an exact number and an approximate one have no honest meeting point"
    );
}

#[test]
fn merging_structs_takes_the_union_of_their_fields() {
    let left = DataType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::Utf8.required_field("venue"),
    ])
    .unwrap();
    let right = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])
    .unwrap();

    let merged = left.merge_with(&right, true).unwrap();
    assert_eq!(merged.field_len(), 3);

    // A name both carry merges, and stays required because both require it.
    assert_eq!(merged["id"].dtype(), &DataType::Int64);
    assert!(!merged["id"].is_nullable());

    // A name only one side carries becomes nullable: the rows the other side
    // described do not have it.
    assert!(merged["venue"].is_nullable());
    assert!(merged["price"].is_nullable());

    // Order is the receiver's, with additions appended, so a merge never
    // reorders columns a caller already depends on.
    assert_eq!(merged[0].name(), "id");
    assert_eq!(merged[1].name(), "venue");
    assert_eq!(merged[2].name(), "price");

    // Merging a schema with itself changes nothing.
    assert_eq!(merged.merge_with(&merged, true).unwrap(), merged);
}

#[test]
fn merging_reaches_into_every_nested_layout() {
    // Lists merge their item.
    assert_eq!(
        DataType::list(DataType::Int32.nullable_field("item"))
            .merge_with(
                &DataType::list(DataType::Int64.nullable_field("item")),
                true
            )
            .unwrap(),
        DataType::list(DataType::Int64.nullable_field("item")),
    );

    // Maps merge through their entries.
    assert_eq!(
        DataType::map_of(DataType::Utf8, DataType::Int32, true)
            .unwrap()
            .merge_with(
                &DataType::map_of(DataType::Utf8, DataType::Int64, true).unwrap(),
                true,
            )
            .unwrap(),
        DataType::map_of(DataType::Utf8, DataType::Int64, true).unwrap(),
    );

    // And the recursion goes all the way down.
    let deep = |inner: DataType| {
        DataType::from_fields([DataType::from_fields([inner.required_field("n")])
            .unwrap()
            .required_field("in")])
        .unwrap()
    };
    let merged = deep(DataType::Int32)
        .merge_with(&deep(DataType::Int64), true)
        .unwrap();
    assert_eq!(merged["in"]["n"].dtype(), &DataType::Int64);
}

#[test]
fn merging_fields_unions_metadata_and_keeps_the_receivers_value() {
    let mut left = Field::new("price", DataType::Int32, false);
    left.set_metadata([("owner", "left"), ("only_left", "1")])
        .unwrap();
    let mut right = Field::new("price", DataType::Int64, true);
    right
        .set_metadata([("owner", "right"), ("only_right", "2")])
        .unwrap();

    let merged = left.merge_with(&right, true).unwrap();
    assert_eq!(merged.dtype(), &DataType::Int64);

    // Either side being nullable carries over: a value absent from one of two
    // sources is absent from their union.
    assert!(merged.is_nullable());

    // Every key arrives, and the receiver wins the one they disagree on.
    assert_eq!(merged.get_metadata("owner"), Some("left"));
    assert_eq!(merged.get_metadata("only_left"), Some("1"));
    assert_eq!(merged.get_metadata("only_right"), Some("2"));

    // Merging a field with itself changes nothing.
    assert_eq!(merged.merge_with(&merged, true).unwrap(), merged);
}

#[test]
fn unnesting_flattens_structs_to_leaves_named_by_their_path() {
    let row = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::from_fields([
            DataType::Float64.required_field("px"),
            DataType::from_fields([DataType::Utf8.required_field("ccy")])
                .unwrap()
                .required_field("meta"),
        ])
        .unwrap()
        .nullable_field("line"),
        DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
    ])
    .unwrap()
    .required_field("row");

    let leaves = row.unnest_fields();
    let names: Vec<&str> = leaves.iter().map(Field::name).collect();

    // Structs flatten all the way down; a list is a leaf, not its item.
    assert_eq!(names, ["id", "line.px", "line.meta.ccy", "levels"]);

    // A leaf under a nullable ancestor is nullable, because a null parent
    // leaves it with no value to carry.
    assert!(!leaves[0].is_nullable());
    assert!(leaves[1].is_nullable(), "px is required, but line is not");
    assert!(leaves[2].is_nullable());

    // Every name it answers is one the path accessor resolves, so a flattened
    // column list and the tree it came from address children the same way.
    for leaf in &leaves {
        assert!(
            row.get_field_by_path(leaf.name()).is_some(),
            "{:?} must resolve",
            leaf.name()
        );
    }

    // A node with no children answers nothing rather than failing.
    assert!(DataType::Int64.unnest_fields().is_empty());
}

#[test]
fn exploding_replaces_each_collection_with_what_it_holds() {
    let row = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
        DataType::map_of(DataType::Utf8, DataType::Int64, true)
            .unwrap()
            .nullable_field("tags"),
        DataType::dictionary(DataType::Int16, DataType::Utf8)
            .unwrap()
            .required_field("codes"),
    ])
    .unwrap();

    let exploded = row.explode_fields();

    // Same columns, same order: one row's worth of an expanded table.
    assert_eq!(exploded.len(), row.field_len());
    assert_eq!(
        exploded.iter().map(Field::name).collect::<Vec<_>>(),
        ["id", "levels", "tags", "codes"],
    );

    assert_eq!(exploded[0].dtype(), &DataType::Int64, "not a collection");
    assert_eq!(
        exploded[1].dtype(),
        &DataType::Float64,
        "a list answers its item"
    );
    assert!(
        exploded[2].dtype().as_fields().is_some(),
        "a map answers its entries"
    );
    assert_eq!(
        exploded[3].dtype(),
        &DataType::Utf8,
        "a dictionary answers its value"
    );

    // The column keeps its name and is nullable when the collection or its
    // element is: an absent list yields no element.
    assert!(exploded[1].is_nullable());

    // One level only, so the depth is the caller's decision.
    let deep = DataType::from_fields([DataType::list(
        DataType::list(DataType::Int64.nullable_field("item")).nullable_field("item"),
    )
    .nullable_field("deep")])
    .unwrap();
    let once = DataType::from_fields(deep.explode_fields()).unwrap();
    assert!(matches!(once.explode_fields()[0].dtype(), DataType::Int64));
}
