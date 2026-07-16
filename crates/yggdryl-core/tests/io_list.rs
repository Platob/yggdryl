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
// Recursion: nested-element rows, deeper slices, all-null / all-empty, empty-vs-null
// -------------------------------------------------------------------------------------

/// A 3-row `list<list<i32>>` over 5 inner lists over the flat leaves `[1..=6]`:
/// inner = `[1,2],[3],[],[4,5],[6]` (offsets `[0,2,3,3,5,6]`); outer rows group those
/// `[[1,2],[3]]`, `[[]]`, `[[4,5],[6]]` (offsets `[0,2,3,5]`).
fn list_of_list() -> ListSerie {
    let leaves = Serie::from_values(&[1i32, 2, 3, 4, 5, 6]).named("item");
    let inner = ListSerie::from_values(leaves, &[0, 2, 3, 3, 5, 6], None).unwrap();
    ListSerie::from_values(inner.named("item"), &[0, 2, 3, 5], None).unwrap()
}

#[test]
fn row_access_on_a_list_of_list_yields_a_list_column() {
    // A `list<list<i32>>` row is an `AnyScalar::List` whose items are *themselves* a list column.
    let outer = list_of_list();
    assert_eq!(outer.item_field().type_id(), DataTypeId::List);

    let row0 = outer.row(0);
    let items = row0.as_list().expect("row 0 is a present list");
    assert_eq!(items.type_id(), DataTypeId::List); // the elements are a list column
    assert_eq!(items.len(), 2); // [[1,2],[3]]
    assert!(matches!(items.value(0), AnyScalar::List(_)));

    // `row_scalar` sees the same recursive shape.
    let r0 = outer.row_scalar(0);
    assert_eq!(r0.len(), 2);
    assert_eq!(r0.items().type_id(), DataTypeId::List);
    // Row 1 is the single empty inner list `[[]]`: one element, itself an empty list.
    let r1 = outer.row_scalar(1);
    assert_eq!(r1.len(), 1);
    assert_eq!(r1.items().value(0).as_list().unwrap().len(), 0);
}

#[test]
fn slice_of_a_list_of_list_equals_the_logical_window() {
    let outer = list_of_list();
    // Row 1 alone is `[[]]` — a list containing one empty list.
    let middle = outer.slice(1, 1);
    assert_eq!(middle.len(), 1);
    let expected = ListSerie::from_values(
        ListSerie::from_values(Serie::<i32>::from_values(&[]).named("item"), &[0, 0], None)
            .unwrap()
            .named("item"),
        &[0, 1],
        None,
    )
    .unwrap();
    assert_eq!(middle, expected);
    // Rows [1, 3): `[[]]`, `[[4,5],[6]]`.
    let tail = outer.slice(1, 2);
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.values().len(), 3); // inner windowed to [], [4,5], [6]
                                        // A round-trip of the slice stays byte-exact.
    assert_eq!(
        ListSerie::deserialize_bytes(&tail.serialize_bytes()).unwrap(),
        tail
    );
}

#[test]
fn all_null_and_all_empty_list_columns_byte_round_trip() {
    // Every row null: an empty child, offsets all 0, a present mask of all-false.
    let all_null = ListSerie::from_values(
        Serie::<i32>::from_values(&[]).named("item"),
        &[0, 0, 0, 0],
        Some(&[false, false, false]),
    )
    .unwrap();
    assert_eq!(all_null.len(), 3);
    assert_eq!(all_null.null_count(), 3);
    assert!(all_null.row(0).is_null() && all_null.row(2).is_null());
    assert_eq!(
        ListSerie::deserialize_bytes(&all_null.serialize_bytes()).unwrap(),
        all_null
    );

    // Every row a present zero-length list: same empty child, no nulls.
    let all_empty = ListSerie::from_values(
        Serie::<i32>::from_values(&[]).named("item"),
        &[0, 0, 0, 0],
        None,
    )
    .unwrap();
    assert_eq!(all_empty.len(), 3);
    assert_eq!(all_empty.null_count(), 0);
    assert!(!all_empty.row(0).is_null());
    assert!(all_empty.row_scalar(0).is_empty());
    assert_eq!(
        ListSerie::deserialize_bytes(&all_empty.serialize_bytes()).unwrap(),
        all_empty
    );

    // An all-null column and an all-empty column of the same length are NOT equal.
    assert_ne!(all_null, all_empty);
}

#[test]
fn empty_vs_null_nested_list_element_are_distinct() {
    // Inner list column: row 0 = empty (present), row 1 = null.
    let inner = ListSerie::from_values(
        Serie::<i32>::from_values(&[]).named("item"),
        &[0, 0, 0],
        Some(&[true, false]),
    )
    .unwrap();
    // Outer wraps each inner row as its own single-element outer row.
    let outer = ListSerie::from_values(inner.named("item"), &[0, 1, 2], None).unwrap();
    let empty_elem = outer.row_scalar(0); // holds one *empty* inner list
    let null_elem = outer.row_scalar(1); // holds one *null* inner list
    assert_eq!(empty_elem.len(), 1);
    assert_eq!(null_elem.len(), 1);
    // The empty-element row and the null-element row are distinct values.
    assert_ne!(empty_elem, null_elem);
    assert_eq!(
        ListSerie::deserialize_bytes(&outer.serialize_bytes()).unwrap(),
        outer
    );
}

#[test]
fn list_deserialize_truncation_and_corruption_are_guided_errors_not_panics() {
    let list = list_of_list();
    let bytes = list.serialize_bytes();
    // Every truncation either errors or decodes to something *other* than the full value.
    for cut in 0..bytes.len() {
        if let Ok(partial) = ListSerie::deserialize_bytes(&bytes[..cut]) {
            assert_ne!(
                partial, list,
                "truncation at {cut} wrongly decoded the full value"
            );
        }
    }
    // Garbage and empty input never panic.
    assert!(ListSerie::deserialize_bytes(&[0xff; 48]).is_err());
    assert!(ListSerie::deserialize_bytes(&[]).is_err());
    // Corrupting a byte inside the schema region errors (or at least diverges), never panics.
    let schema_len = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let mut corrupt = bytes.clone();
    corrupt[8] ^= 0xff; // the schema's leading tag byte
    if let Ok(decoded) = ListSerie::deserialize_bytes(&corrupt) {
        assert_ne!(decoded, list);
    }
    assert!(schema_len > 0);
}

#[test]
fn list_hostile_offsets_are_guided_errors() {
    let items = || Serie::from_values(&[1i32, 2, 3]).named("item");
    // A leading negative offset is caught as "first offset must be 0".
    let err = ListSerie::from_values(items(), &[-1, 2, 3], None).unwrap_err();
    assert!(err.to_string().contains("first offset must be 0"), "{err}");
    // A negative offset mid-run trips the non-decreasing guard.
    let err = ListSerie::from_values(items(), &[0, -1, 3], None).unwrap_err();
    assert!(err.to_string().contains("non-decreasing"), "{err}");
    // A last offset past the child length is rejected (would over-read the child on a rebase).
    let err = ListSerie::from_values(items(), &[0, 2, 99], None).unwrap_err();
    assert!(
        err.to_string()
            .contains("must equal the flattened child length"),
        "{err}"
    );
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`)
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
mod arrow {
    use super::*;
    use arrow_array::Array;
    use yggdryl_core::io::nested::MapSerie;

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

    /// Exports and re-imports a nested list column through the Arrow `ListArray` path, asserting
    /// byte-exact identity — the recursion nests the child through the central dispatch.
    fn arrow_round_trip(list: &ListSerie) {
        let field = list.to_field("l").to_arrow_field();
        let array = list.to_arrow_array().unwrap();
        let back = ListSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(&back, list, "list Arrow round-trip differed");
    }

    #[test]
    fn list_of_list_arrow_round_trip() {
        arrow_round_trip(&list_of_list());
    }

    #[test]
    fn list_of_map_arrow_round_trip() {
        // A list whose element is a map<utf8, i64>: two outer rows over three inner map rows.
        let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
        let values = Serie::from_options(&[Some(1i64), None, Some(3)]).named("value");
        let inner = MapSerie::from_entries(keys, values, &[0, 2, 2, 3], None, false).unwrap();
        let outer = ListSerie::from_values(inner.named("item"), &[0, 2, 3], None).unwrap();
        assert_eq!(outer.item_field().type_id(), DataTypeId::Map);
        arrow_round_trip(&outer);
    }

    #[test]
    fn all_null_and_all_empty_list_arrow_round_trip() {
        let all_null = ListSerie::from_values(
            Serie::<i32>::from_values(&[]).named("item"),
            &[0, 0, 0, 0],
            Some(&[false, false, false]),
        )
        .unwrap();
        let array = all_null.to_arrow_array().unwrap();
        assert_eq!(array.null_count(), 3);
        arrow_round_trip(&all_null);

        let all_empty = ListSerie::from_values(
            Serie::<i32>::from_values(&[]).named("item"),
            &[0, 0, 0, 0],
            None,
        )
        .unwrap();
        assert_eq!(all_empty.to_arrow_array().unwrap().null_count(), 0);
        arrow_round_trip(&all_empty);
    }

    #[test]
    fn sliced_list_of_struct_import_reads_the_logical_window() {
        // A multi-level (list-of-struct) Arrow array sliced at offset > 0 imports as the logical
        // window: offsets rebased, the struct child windowed to exactly the sliced rows' elements.
        let inner = StructSerie::from_series(vec![
            Serie::from_values(&[1i32, 2, 3, 4, 5]).named("x"),
            Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c"), Some("d"), Some("e")])
                .named("y"),
        ])
        .unwrap();
        // 4 rows over the 5 struct elements: [e0,e1],[e2],[e3,e4],[] (offsets [0,2,3,5,5]).
        let list = ListSerie::from_values(inner.named("item"), &[0, 2, 3, 5, 5], None).unwrap();
        let field = list.to_field("l").to_arrow_field();
        let array = list.to_arrow_array().unwrap();
        let sliced = Array::slice(&array, 1, 2); // logical rows [{x:3,y:c}], [{x:4,y:d},{x:5,y:e}]
        let back = ListSerie::from_arrow_array(sliced.as_ref(), &field).unwrap();
        let expected = ListSerie::from_values(
            StructSerie::from_series(vec![
                Serie::from_values(&[3i32, 4, 5]).named("x"),
                Utf8Serie::from_strs(&[Some("c"), Some("d"), Some("e")]).named("y"),
            ])
            .unwrap()
            .named("item"),
            &[0, 1, 3],
            None,
        )
        .unwrap();
        assert_eq!(back, expected);
    }
}
