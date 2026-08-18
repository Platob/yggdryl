use yggdryl::field::nested;
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
/// `data_type["level"]` was a child, so a caller walking one object graph got
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
    assert_eq!(order["id"].data_type(), &DataType::Int64);
    assert_eq!(order.data_type()["id"].data_type(), &DataType::Int64);
    assert_eq!(order[0].name(), "id");
    assert_eq!(order.data_type()[1].name(), "line");

    // Chained subscripts are the nesting story - no dotted path form.
    assert_eq!(order["line"]["price"].data_type(), &DataType::Float64);
    assert_eq!(order["line"]["qty"].data_type(), &DataType::Int64);

    // Through a List item and a Map entry, the same way.
    let items = DataType::list(order.clone().with_name("item"));
    assert_eq!(items[0]["id"].data_type(), &DataType::Int64);
    assert_eq!(
        items["item"]["line"]["price"].data_type(),
        &DataType::Float64
    );

    // The non-panicking form stays available and is what the docs point at.
    assert!(order.get_field_by_name("absent").is_none());
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
    assert_eq!(field["id"].data_type(), &DataType::Int64);
    assert!(field.get_field_by_name("owner").is_none());

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
    row.set_field_by_name("venue", DataType::Utf8.nullable_field("venue"))
        .unwrap();
    assert_eq!(row.field_len(), 2);
    assert_eq!(row[1].name(), "venue");

    // A known name replaces in place, keeping its position.
    row.set_field_by_name("id", DataType::Utf8.required_field("id"))
        .unwrap();
    assert_eq!(row.field_len(), 2);
    assert_eq!(row[0].name(), "id");
    assert_eq!(row["id"].data_type(), &DataType::Utf8);

    // A position replaces only, and never grows the node silently.
    row.set_field(1, DataType::LargeUtf8.nullable_field("venue"))
        .unwrap();
    assert_eq!(row["venue"].data_type(), &DataType::LargeUtf8);
    let message = row
        .set_field(7, DataType::Int64.nullable_field("late"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("a child position below 2"), "{message}");
    assert_eq!(row.field_len(), 2, "a refusal leaves the field unchanged");

    // Removal returns the prior child and closes the gap.
    let dropped = row.remove_field_by_name("id").unwrap();
    assert_eq!(dropped.name(), "id");
    assert_eq!(row.field_len(), 1);
    assert_eq!(row[0].name(), "venue");

    // A node with no children to replace says so rather than panicking.
    let mut scalar = DataType::Int64.required_field("price");
    let message = scalar
        .set_field_by_name("child", DataType::Int64.nullable_field("child"))
        .unwrap_err()
        .to_string();
    assert!(message.contains("a struct field"), "{message}");
}
