//! The **list** nested family (`io::nested::list`) built on the root erased primitives: the
//! [`ListField`] schema (↔ an Arrow `List` `Field`) and [`ListSerie`] (↔ Arrow `ListArray`). A list
//! column is `i32` offsets over one flattened child column, so the tests exercise offset validation,
//! null-vs-empty rows, recursive list-of-struct / struct-of-list round-trips, and — under `arrow` —
//! the `ListArray` interop including a sliced import.

use yggdryl_core::io::fixed::{Field, PrimitiveType, Serie};
use yggdryl_core::io::nested::{ListField, ListSerie, ListType, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{AnyField, AnyScalar, AnySerie, DataType, DataTypeId, FieldType};

// -------------------------------------------------------------------------------------
// ListType / ListField — the descriptor and centralized schema
// -------------------------------------------------------------------------------------

fn i32_item(nullable: bool) -> AnyField {
    AnyField::leaf(Field::new("item", &PrimitiveType::<i32>::new(), nullable))
}

#[test]
fn list_type_and_field_describe_the_shape() {
    let dt = ListType::new(i32_item(true));
    assert_eq!(dt.name(), "list");
    assert_eq!(dt.type_id(), DataTypeId::List);
    assert!(dt.is_list() && dt.is_nested());
    assert_eq!(dt.item().name(), "item");

    let schema = ListField::new("scores", i32_item(true), true);
    assert_eq!(schema.name(), "scores");
    assert_eq!(schema.type_name(), "list");
    assert_eq!(FieldType::type_id(&schema), DataTypeId::List);
    assert!(schema.is_list() && schema.nullable());
    assert_eq!(schema.item().name(), "item");
    assert_eq!(schema.data_type(), dt);

    // A value type: equal by content, usable as a map key.
    use std::collections::HashSet;
    let set: HashSet<ListField> = [
        ListField::new("scores", i32_item(true), true),
        schema.clone(),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 1);

    // Round-trips through AnyField.
    let any: AnyField = schema.clone().into();
    assert_eq!(ListField::from_any_field(any), Some(schema));
    // A non-list AnyField is rejected.
    assert!(ListField::from_any_field(i32_item(false)).is_none());
}

#[test]
fn list_field_with_builders() {
    let base = ListField::new("xs", i32_item(false), false);
    let built = base
        .with_name("ys")
        .with_nullable(true)
        .with_item(AnyField::leaf(Field::new(
            "item",
            &PrimitiveType::<f64>::new(),
            true,
        )))
        .with_metadata_entry("origin", "test");
    assert_eq!(built.name(), "ys");
    assert!(built.nullable());
    assert_eq!(built.item().type_id(), DataTypeId::F64);
    assert_eq!(built.metadata().get("origin"), Some("test"));
    // Immutable updates: the base is untouched.
    assert_eq!(base.name(), "xs");
    assert_eq!(base.item().type_id(), DataTypeId::I32);
    assert_eq!(base.copy(), base);
}

// -------------------------------------------------------------------------------------
// ListSerie — build, offsets, row access, serialize round-trip (no arrow)
// -------------------------------------------------------------------------------------

/// 4 rows over the flat child [1, 2, 3]: [1, 2], [] (empty), null, [3].
fn sample_list() -> ListSerie {
    let items = Serie::from_values(&[1i32, 2, 3]).named("item");
    ListSerie::from_values(items, &[0, 2, 2, 2, 3], Some(&[true, true, false, true])).unwrap()
}

#[test]
fn list_serie_builds_and_reports_shape() {
    let list = sample_list();
    assert_eq!(list.len(), 4);
    assert_eq!(list.null_count(), 1);
    assert!(list.has_nulls());
    assert_eq!(list.offsets(), &[0, 2, 2, 2, 3]);
    assert_eq!(list.item_field().name(), "item");
    assert_eq!(list.item_field().type_id(), DataTypeId::I32);
    assert_eq!(list.values().len(), 3); // the flattened child
    assert_eq!(list.value_range(0), Some((0, 2)));
    assert_eq!(list.value_range(3), Some((2, 3)));
    assert_eq!(list.value_range(9), None);
}

#[test]
fn list_serie_row_access_distinguishes_null_from_empty() {
    let list = sample_list();
    // Row 0 = [1, 2] (present).
    assert!(matches!(list.row(0), AnyScalar::List(_)));
    let r0 = list.row_scalar(0);
    assert!(!r0.is_null());
    assert_eq!(r0.len(), 2);
    assert_eq!(r0.items().as_serie::<i32>().unwrap().get(0), Some(1));
    assert_eq!(r0.items().as_serie::<i32>().unwrap().get(1), Some(2));

    // Row 1 = [] — present but empty (NOT null).
    assert!(matches!(list.row(1), AnyScalar::List(_)));
    let r1 = list.row_scalar(1);
    assert!(!r1.is_null());
    assert!(r1.is_empty());
    assert_eq!(r1.len(), 0);

    // Row 2 = null.
    assert!(list.row(2).is_null());
    assert!(list.row_scalar(2).is_null());

    // Row 3 = [3].
    assert_eq!(list.row_scalar(3).len(), 1);

    // Out of range -> null.
    assert!(list.row(9).is_null());
    assert!(list.row_scalar(9).is_null());
}

#[test]
fn list_serie_serialize_round_trip() {
    let list = sample_list();
    let back = ListSerie::deserialize_bytes(&list.serialize_bytes()).unwrap();
    assert_eq!(back, list);
}

#[test]
fn list_serie_slice_windows_rows_and_child() {
    // 4 rows over flat child [1..=6]: [1,2],[3],[4,5],[6] with offsets [0,2,3,5,6].
    let items = Serie::from_values(&[1i32, 2, 3, 4, 5, 6]).named("item");
    let list = ListSerie::from_values(items, &[0, 2, 3, 5, 6], None).unwrap();
    let middle = list.slice(1, 2); // rows [3] and [4, 5]
    assert_eq!(middle.len(), 2);
    assert_eq!(middle.offsets(), &[0, 1, 3]); // rebased to 0
    assert_eq!(middle.values().len(), 3); // child windowed to [3, 4, 5]
    let expected = ListSerie::from_values(
        Serie::from_values(&[3i32, 4, 5]).named("item"),
        &[0, 1, 3],
        None,
    )
    .unwrap();
    assert_eq!(middle, expected);
    // Clamping: out-of-range / overlong requests never panic.
    assert_eq!(list.slice(3, 100).len(), 1);
    assert_eq!(list.slice(9, 1).len(), 0);
}

#[test]
fn empty_list_serie_round_trips() {
    let schema = ListField::new("xs", i32_item(true), true);
    let empty = ListSerie::empty(&schema);
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.offsets(), &[0]);
    assert_eq!(empty.values().len(), 0);
    assert_eq!(
        ListSerie::deserialize_bytes(&empty.serialize_bytes()).unwrap(),
        empty
    );
}

#[test]
fn bad_offsets_are_guided_errors() {
    let items = || Serie::from_values(&[1i32, 2, 3]).named("item");
    // First offset must be 0.
    let err = ListSerie::from_values(items(), &[1, 2, 3], None).unwrap_err();
    assert!(err.to_string().contains("first offset must be 0"), "{err}");
    // Non-decreasing.
    let err = ListSerie::from_values(items(), &[0, 2, 1, 3], None).unwrap_err();
    assert!(err.to_string().contains("non-decreasing"), "{err}");
    // Last offset must equal the child length (3), not 2.
    let err = ListSerie::from_values(items(), &[0, 1, 2], None).unwrap_err();
    assert!(
        err.to_string()
            .contains("must equal the flattened child length"),
        "{err}"
    );
    // An empty offsets slice is rejected.
    let err = ListSerie::from_values(items(), &[], None).unwrap_err();
    assert!(err.to_string().contains("at least one offset"), "{err}");
}

// -------------------------------------------------------------------------------------
// Recursion: list-of-struct and struct-of-list (byte codec)
// -------------------------------------------------------------------------------------

#[test]
fn list_of_struct_serialize_round_trip() {
    // The flat child is a struct column {x: i32, y: utf8} of 3 rows; the list groups it into 2 rows.
    let inner = StructSerie::from_series(vec![
        Serie::from_values(&[1i32, 2, 3]).named("x"),
        Utf8Serie::from_strs(&[Some("a"), None, Some("c")]).named("y"),
    ])
    .unwrap();
    let list = ListSerie::from_values(inner.named("item"), &[0, 2, 3], None).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list.item_field().type_id(), DataTypeId::Struct);
    assert_eq!(list.values().type_id(), DataTypeId::Struct);
    let back = ListSerie::deserialize_bytes(&list.serialize_bytes()).unwrap();
    assert_eq!(back, list);
}

#[test]
fn struct_of_list_via_from_series_serialize_round_trip() {
    // A struct built from `.named` columns, one of which is itself a list column.
    let scores = ListSerie::from_values(
        Serie::from_values(&[1i32, 2, 3, 4]).named("item"),
        &[0, 2, 4],
        None,
    )
    .unwrap();
    let ids = Serie::from_values(&[10i64, 20]);
    let table = StructSerie::from_series(vec![ids.named("id"), scores.named("scores")]).unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(
        table.column_named("scores").unwrap().type_id(),
        DataTypeId::List
    );
    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back, table);
}

#[test]
fn list_of_list_serialize_round_trip() {
    // A list whose element is itself a list<i32> — the recursion nests through the central dispatch.
    let leaves = Serie::from_values(&[1i32, 2, 3, 4, 5]).named("item");
    let inner = ListSerie::from_values(leaves, &[0, 2, 3, 5], None).unwrap(); // 3 inner lists
    let outer = ListSerie::from_values(inner.named("item"), &[0, 1, 3], None).unwrap(); // 2 outer rows
    assert_eq!(outer.len(), 2);
    assert_eq!(outer.item_field().type_id(), DataTypeId::List);
    let back = ListSerie::deserialize_bytes(&outer.serialize_bytes()).unwrap();
    assert_eq!(back, outer);
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`)
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
mod arrow {
    use super::*;
    use arrow_array::Array;

    #[test]
    fn list_i32_arrow_round_trip() {
        // 3 rows: [1, null], [], [3, 4] over flat child [1, null, 3, 4].
        let items = Serie::from_options(&[Some(1i32), None, Some(3), Some(4)]).named("item");
        let list = ListSerie::from_values(items, &[0, 2, 2, 4], None).unwrap();
        let field = list.to_field("scores").to_arrow_field();
        assert!(matches!(field.data_type(), arrow_schema::DataType::List(_)));
        let array = list.to_arrow_array().unwrap();
        assert_eq!(array.len(), 3);
        let back = ListSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn nullable_list_rows_arrow_round_trip() {
        let list = sample_list(); // has a null row and an empty row
        let field = list.to_field("l").to_arrow_field();
        let array = list.to_arrow_array().unwrap();
        assert_eq!(array.null_count(), 1);
        let back = ListSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn list_of_struct_arrow_round_trip() {
        let inner = StructSerie::from_series(vec![
            Serie::from_options(&[Some(1i32), None, Some(3)]).named("x"),
            Serie::from_values(&[4i32, 5, 6]).named("y"),
        ])
        .unwrap();
        let list = ListSerie::from_values(inner.named("item"), &[0, 2, 3], None).unwrap();
        let field = list.to_field("rows").to_arrow_field();
        let array = list.to_arrow_array().unwrap();
        assert!(array
            .values()
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .is_some());
        let back = ListSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn sliced_list_import_reads_logical_window() {
        // Build a 4-row list, export, slice [1, 3), import -> equals the same 2 rows built fresh.
        let items = Serie::from_values(&[1i32, 2, 3, 4, 5, 6]).named("item");
        let list = ListSerie::from_values(items, &[0, 2, 3, 5, 6], None).unwrap(); // [1,2],[3],[4,5],[6]
        let field = list.to_field("l").to_arrow_field();
        let array = list.to_arrow_array().unwrap();
        let sliced = Array::slice(&array, 1, 2); // logical rows [3], [4, 5]
        let back = ListSerie::from_arrow_array(sliced.as_ref(), &field).unwrap();
        let expected = ListSerie::from_values(
            Serie::from_values(&[3i32, 4, 5]).named("item"),
            &[0, 1, 3],
            None,
        )
        .unwrap();
        assert_eq!(back, expected);
    }

    #[test]
    fn list_as_a_struct_child_via_arrow() {
        // A list column nested inside a struct, exported and re-imported through the StructArray path.
        let scores = ListSerie::from_values(
            Serie::from_values(&[1i32, 2, 3]).named("item"),
            &[0, 2, 3],
            None,
        )
        .unwrap();
        let ids = Serie::from_values(&[10i64, 20]);
        let table =
            StructSerie::from_series(vec![ids.named("id"), scores.named("scores")]).unwrap();
        let field = table.to_field("row").to_arrow_field();
        let array = table.to_arrow_array().unwrap();
        let back = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, table);
    }

    #[test]
    fn list_from_externally_built_arrow_array() {
        // A ListArray built directly with arrow-rs imports to an equal ListSerie.
        use std::sync::Arc;
        let values = Arc::new(arrow_array::Int32Array::from(vec![1, 2, 3, 4]));
        let offsets =
            arrow_buffer::OffsetBuffer::new(arrow_buffer::ScalarBuffer::from(vec![0i32, 2, 2, 4]));
        let item_field = Arc::new(arrow_schema::Field::new(
            "item",
            arrow_schema::DataType::Int32,
            false,
        ));
        let array = arrow_array::ListArray::new(item_field, offsets, values, None);
        let field = arrow_schema::Field::new("l", array.data_type().clone(), false);
        let back = ListSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back.row_scalar(0).len(), 2);
        assert_eq!(back.row_scalar(1).len(), 0);
        assert_eq!(back.row_scalar(2).len(), 2);
    }
}
