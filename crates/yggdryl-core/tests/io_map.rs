//! The **map** nested family (`io::nested::map`) built on the root erased primitives: the
//! [`MapField`] schema (↔ an Arrow `Map` `Field`) and [`MapSerie`] (↔ Arrow `MapArray`). A map column
//! is the optimized alias of `List<Struct<{key non-null, value}>>`, so the tests exercise offset
//! validation, non-null keys, null-vs-empty rows, per-row key lookup (`get_value`), `keys_sorted`,
//! recursive map-of-list / list-of-map / map-of-struct / struct-of-map / map-of-map round-trips, and —
//! under `arrow` — the `MapArray` interop including a sliced import and an externally-built array.

use yggdryl_core::io::fixed::{Field, PrimitiveType, Serie};
use yggdryl_core::io::nested::{ListSerie, MapField, MapSerie, MapType, StructSerie};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{AnyField, AnyScalar, AnySerie, DataType, DataTypeId, FieldType};

// -------------------------------------------------------------------------------------
// MapType / MapField — the descriptor and centralized schema
// -------------------------------------------------------------------------------------

fn utf8_key(nullable: bool) -> AnyField {
    AnyField::leaf(Field::new("key", &PrimitiveType::<i32>::new(), nullable))
}

fn i64_value(nullable: bool) -> AnyField {
    AnyField::leaf(Field::new("value", &PrimitiveType::<i64>::new(), nullable))
}

#[test]
fn map_type_and_field_describe_the_shape() {
    let dt = MapType::new(utf8_key(false), i64_value(true), false);
    assert_eq!(dt.name(), "map");
    assert_eq!(dt.type_id(), DataTypeId::Map);
    assert!(dt.is_map() && dt.is_nested());
    assert_eq!(dt.key().name(), "key");
    assert_eq!(dt.value().name(), "value");
    assert!(!dt.keys_sorted());

    let schema = MapField::new("counts", utf8_key(false), i64_value(true), true, false);
    assert_eq!(schema.name(), "counts");
    assert_eq!(schema.type_name(), "map");
    assert_eq!(FieldType::type_id(&schema), DataTypeId::Map);
    assert!(schema.is_map() && schema.nullable());
    assert_eq!(schema.key().name(), "key");
    assert_eq!(schema.value().name(), "value");
    assert!(!schema.keys_sorted());
    assert_eq!(schema.data_type(), dt);

    // A value type: equal by content, usable as a map key.
    use std::collections::HashSet;
    let set: HashSet<MapField> = [
        MapField::new("counts", utf8_key(false), i64_value(true), true, false),
        schema.clone(),
    ]
    .into_iter()
    .collect();
    assert_eq!(set.len(), 1);

    // Round-trips through AnyField.
    let any: AnyField = schema.clone().into();
    assert_eq!(MapField::from_any_field(any), Some(schema));
    // A non-map AnyField is rejected.
    assert!(MapField::from_any_field(utf8_key(false)).is_none());
}

#[test]
fn map_field_with_builders() {
    let base = MapField::new("m", utf8_key(false), i64_value(true), false, false);
    let built = base
        .with_name("n")
        .with_nullable(true)
        .with_keys_sorted(true)
        .with_metadata_entry("origin", "test");
    assert_eq!(built.name(), "n");
    assert!(built.nullable());
    assert!(built.keys_sorted());
    assert_eq!(built.metadata().get("origin"), Some("test"));
    // Immutable updates: the base is untouched.
    assert_eq!(base.name(), "m");
    assert!(!base.nullable());
    assert!(!base.keys_sorted());
    assert_eq!(base.copy(), base);
}

// -------------------------------------------------------------------------------------
// MapSerie — build, offsets, row access, key lookup, serialize round-trip (no arrow)
// -------------------------------------------------------------------------------------

/// 4 rows over 3 flat entries {"a"->1, "b"->2, "c"->3}: {"a"->1, "b"->2}, {} (empty), null, {"c"->3}.
fn sample_map() -> MapSerie {
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = Serie::from_values(&[1i64, 2, 3]).named("value");
    MapSerie::from_entries(
        keys,
        values,
        &[0, 2, 2, 2, 3],
        Some(&[true, true, false, true]),
        false,
    )
    .unwrap()
}

#[test]
fn map_serie_builds_and_reports_shape() {
    let map = sample_map();
    assert_eq!(map.len(), 4);
    assert_eq!(map.null_count(), 1);
    assert!(map.has_nulls());
    assert_eq!(map.offsets(), &[0, 2, 2, 2, 3]);
    assert_eq!(map.key_field().name(), "key");
    assert_eq!(map.key_field().type_id(), DataTypeId::Utf8);
    assert_eq!(map.value_field().name(), "value");
    assert_eq!(map.value_field().type_id(), DataTypeId::I64);
    assert_eq!(map.keys().len(), 3); // the flattened keys
    assert_eq!(map.values().len(), 3); // the flattened values
    assert_eq!(map.entries().num_columns(), 2);
    assert_eq!(map.value_range(0), Some((0, 2)));
    assert_eq!(map.value_range(3), Some((2, 3)));
    assert_eq!(map.value_range(9), None);
    assert!(!map.keys_sorted());
    // The inferred key field is non-nullable (a map key is never null).
    assert!(!map.key_field().nullable());
}

#[test]
fn map_serie_row_access_distinguishes_null_from_empty() {
    let map = sample_map();
    // Row 0 = {"a"->1, "b"->2} (present).
    assert!(matches!(map.row(0), AnyScalar::Map { .. }));
    let r0 = map.row_scalar(0);
    assert!(!r0.is_null());
    assert_eq!(r0.len(), 2);
    assert_eq!(r0.entries().len(), 2);

    // Row 1 = {} — present but empty (NOT null).
    assert!(matches!(map.row(1), AnyScalar::Map { .. }));
    let r1 = map.row_scalar(1);
    assert!(!r1.is_null());
    assert!(r1.is_empty());
    assert_eq!(r1.len(), 0);

    // Row 2 = null.
    assert!(map.row(2).is_null());
    assert!(map.row_scalar(2).is_null());

    // Row 3 = {"c"->3}.
    assert_eq!(map.row_scalar(3).len(), 1);

    // Out of range -> null.
    assert!(map.row(9).is_null());
    assert!(map.row_scalar(9).is_null());
}

#[test]
fn map_serie_get_value_scans_the_row() {
    let map = sample_map();
    // Row 0 maps "a"->1 and "b"->2.
    let key_a = map.keys().value(0); // "a"
    let key_b = map.keys().value(1); // "b"
    let key_c = map.keys().value(2); // "c"
    assert_eq!(map.get_value(0, &key_a), Some(map.values().value(0))); // -> 1
    assert_eq!(map.get_value(0, &key_b), Some(map.values().value(1))); // -> 2
                                                                       // "c" is not in row 0.
    assert_eq!(map.get_value(0, &key_c), None);
    // Row 3 maps "c"->3.
    assert_eq!(map.get_value(3, &key_c), Some(map.values().value(2)));
    // A null row and the empty row yield None for any key.
    assert_eq!(map.get_value(2, &key_a), None); // null row
    assert_eq!(map.get_value(1, &key_a), None); // empty row
                                                // Out of range -> None.
    assert_eq!(map.get_value(9, &key_a), None);
}

#[test]
fn map_serie_serialize_round_trip() {
    let map = sample_map();
    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    assert_eq!(back, map);
}

#[test]
fn keys_sorted_round_trips_through_bytes() {
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
    let values = Serie::from_values(&[1i64, 2]).named("value");
    let sorted = MapSerie::from_entries(keys, values, &[0, 2], None, true).unwrap();
    assert!(sorted.keys_sorted());
    let back = MapSerie::deserialize_bytes(&sorted.serialize_bytes()).unwrap();
    assert!(back.keys_sorted());
    assert_eq!(back, sorted);
}

#[test]
fn map_serie_slice_windows_rows_and_entries() {
    // 4 rows over 6 flat entries: {a->1,b->2},{c->3},{d->4,e->5},{f->6} with offsets [0,2,3,5,6].
    let keys = Utf8Serie::from_strs(&[
        Some("a"),
        Some("b"),
        Some("c"),
        Some("d"),
        Some("e"),
        Some("f"),
    ])
    .named("key");
    let values = Serie::from_values(&[1i64, 2, 3, 4, 5, 6]).named("value");
    let map = MapSerie::from_entries(keys, values, &[0, 2, 3, 5, 6], None, false).unwrap();
    let middle = map.slice(1, 2); // rows {c->3} and {d->4, e->5}
    assert_eq!(middle.len(), 2);
    assert_eq!(middle.offsets(), &[0, 1, 3]); // rebased to 0
    assert_eq!(middle.keys().len(), 3); // entries windowed to [c, d, e]
    let expected = MapSerie::from_entries(
        Utf8Serie::from_strs(&[Some("c"), Some("d"), Some("e")]).named("key"),
        Serie::from_values(&[3i64, 4, 5]).named("value"),
        &[0, 1, 3],
        None,
        false,
    )
    .unwrap();
    assert_eq!(middle, expected);
    // Clamping: out-of-range / overlong requests never panic.
    assert_eq!(map.slice(3, 100).len(), 1);
    assert_eq!(map.slice(9, 1).len(), 0);
}

#[test]
fn empty_map_serie_round_trips() {
    let schema = MapField::new("m", utf8_key(false), i64_value(true), true, false);
    let empty = MapSerie::empty(&schema);
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.offsets(), &[0]);
    assert_eq!(empty.keys().len(), 0);
    assert_eq!(empty.values().len(), 0);
    assert_eq!(
        MapSerie::deserialize_bytes(&empty.serialize_bytes()).unwrap(),
        empty
    );
}

#[test]
fn null_keys_are_rejected() {
    // A key column carrying a null is rejected — a map key is never null.
    let keys = Utf8Serie::from_strs(&[Some("a"), None]).named("key");
    let values = Serie::from_values(&[1i64, 2]).named("value");
    let err = MapSerie::from_entries(keys, values, &[0, 2], None, false).unwrap_err();
    assert!(err.to_string().contains("must not contain nulls"), "{err}");
}

#[test]
fn bad_offsets_are_guided_errors() {
    let keys = || Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
    let values = || Serie::from_values(&[1i64, 2, 3]).named("value");
    // First offset must be 0.
    let err = MapSerie::from_entries(keys(), values(), &[1, 2, 3], None, false).unwrap_err();
    assert!(err.to_string().contains("first offset must be 0"), "{err}");
    // Non-decreasing.
    let err = MapSerie::from_entries(keys(), values(), &[0, 2, 1, 3], None, false).unwrap_err();
    assert!(err.to_string().contains("non-decreasing"), "{err}");
    // Last offset must equal the entries length (3), not 2.
    let err = MapSerie::from_entries(keys(), values(), &[0, 1, 2], None, false).unwrap_err();
    assert!(
        err.to_string()
            .contains("must equal the flattened entries length"),
        "{err}"
    );
    // An empty offsets slice is rejected.
    let err = MapSerie::from_entries(keys(), values(), &[], None, false).unwrap_err();
    assert!(err.to_string().contains("at least one offset"), "{err}");
}

// -------------------------------------------------------------------------------------
// Recursion: map-of-list, list-of-map, map-of-struct, struct-of-map, map-of-map (byte codec)
// -------------------------------------------------------------------------------------

/// A flat map {utf8 -> i64} of `entry_count` entries over one row, for embedding as a child.
fn one_row_map(entry_count: usize) -> MapSerie {
    let keys: Vec<Option<&str>> = ["a", "b", "c", "d"][..entry_count]
        .iter()
        .map(|s| Some(*s))
        .collect();
    let vals: Vec<i64> = (0..entry_count as i64).collect();
    let keys = Utf8Serie::from_strs(&keys).named("key");
    let values = Serie::from_values(&vals).named("value");
    MapSerie::from_entries(keys, values, &[0, entry_count as i32], None, false).unwrap()
}

#[test]
fn map_of_list_serialize_round_trip() {
    // A map whose VALUE is a list<i32>: {"a" -> [1,2], "b" -> [3]}.
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
    let inner = ListSerie::from_values(
        Serie::from_values(&[1i32, 2, 3]).named("item"),
        &[0, 2, 3],
        None,
    )
    .unwrap();
    let map = MapSerie::from_entries(keys, inner.named("value"), &[0, 2], None, false).unwrap();
    assert_eq!(map.value_field().type_id(), DataTypeId::List);
    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    assert_eq!(back, map);
}

#[test]
fn list_of_map_serialize_round_trip() {
    // A list whose element is a map<utf8, i64> — the recursion nests through the central dispatch.
    let inner = one_row_map(3); // one map row with 3 entries
    let outer = ListSerie::from_values(inner.named("item"), &[0, 1], None).unwrap();
    assert_eq!(outer.item_field().type_id(), DataTypeId::Map);
    let back = ListSerie::deserialize_bytes(&outer.serialize_bytes()).unwrap();
    assert_eq!(back, outer);
}

#[test]
fn map_of_struct_serialize_round_trip() {
    // A map whose VALUE is a struct {x: i32, y: utf8}: {"a" -> {1,"p"}, "b" -> {2,"q"}}.
    let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
    let inner = StructSerie::from_series(vec![
        Serie::from_values(&[1i32, 2]).named("x"),
        Utf8Serie::from_strs(&[Some("p"), Some("q")]).named("y"),
    ])
    .unwrap();
    let map = MapSerie::from_entries(keys, inner.named("value"), &[0, 2], None, false).unwrap();
    assert_eq!(map.value_field().type_id(), DataTypeId::Struct);
    let back = MapSerie::deserialize_bytes(&map.serialize_bytes()).unwrap();
    assert_eq!(back, map);
}

#[test]
fn struct_of_map_serialize_round_trip() {
    // A struct built from `.named` columns, one of which is itself a map column.
    let counts = one_row_map(2);
    let ids = Serie::from_values(&[10i64]);
    let table = StructSerie::from_series(vec![ids.named("id"), counts.named("counts")]).unwrap();
    assert_eq!(
        table.column_named("counts").unwrap().type_id(),
        DataTypeId::Map
    );
    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back, table);
}

#[test]
fn map_of_map_serialize_round_trip() {
    // A map whose VALUE is itself a map<utf8, i64>: {"outer" -> {"a"->1, "b"->2}}.
    let inner = one_row_map(2);
    let keys = Utf8Serie::from_strs(&[Some("outer")]).named("key");
    let outer = MapSerie::from_entries(keys, inner.named("value"), &[0, 1], None, false).unwrap();
    assert_eq!(outer.value_field().type_id(), DataTypeId::Map);
    let back = MapSerie::deserialize_bytes(&outer.serialize_bytes()).unwrap();
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
    fn map_utf8_i64_arrow_round_trip() {
        // 3 rows: {"a"->1, "b"->null}, {}, {"c"->3} over flat entries.
        let keys = Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]).named("key");
        let values = Serie::from_options(&[Some(1i64), None, Some(3)]).named("value");
        let map = MapSerie::from_entries(keys, values, &[0, 2, 2, 3], None, false).unwrap();
        let field = map.to_field("counts").to_arrow_field();
        assert!(matches!(
            field.data_type(),
            arrow_schema::DataType::Map(_, false)
        ));
        let array = map.to_arrow_array().unwrap();
        assert_eq!(array.len(), 3);
        let back = MapSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, map);
    }

    #[test]
    fn nullable_map_rows_arrow_round_trip() {
        let map = sample_map(); // has a null row and an empty row
        let field = map.to_field("m").to_arrow_field();
        let array = map.to_arrow_array().unwrap();
        assert_eq!(array.null_count(), 1);
        let back = MapSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, map);
    }

    #[test]
    fn keys_sorted_survives_arrow_round_trip() {
        let keys = Utf8Serie::from_strs(&[Some("a"), Some("b")]).named("key");
        let values = Serie::from_values(&[1i64, 2]).named("value");
        let map = MapSerie::from_entries(keys, values, &[0, 2], None, true).unwrap();
        let field = map.to_field("m").to_arrow_field();
        assert!(matches!(
            field.data_type(),
            arrow_schema::DataType::Map(_, true)
        ));
        let array = map.to_arrow_array().unwrap();
        let back = MapSerie::from_arrow_array(&array, &field).unwrap();
        assert!(back.keys_sorted());
        assert_eq!(back, map);
    }

    #[test]
    fn sliced_map_import_reads_logical_window() {
        // Build a 4-row map, export, slice [1, 3), import -> equals the same 2 rows built fresh.
        let keys = Utf8Serie::from_strs(&[
            Some("a"),
            Some("b"),
            Some("c"),
            Some("d"),
            Some("e"),
            Some("f"),
        ])
        .named("key");
        let values = Serie::from_values(&[1i64, 2, 3, 4, 5, 6]).named("value");
        let map = MapSerie::from_entries(keys, values, &[0, 2, 3, 5, 6], None, false).unwrap();
        let field = map.to_field("m").to_arrow_field();
        let array = map.to_arrow_array().unwrap();
        let sliced = Array::slice(&array, 1, 2); // logical rows {c->3}, {d->4, e->5}
        let back = MapSerie::from_arrow_array(sliced.as_ref(), &field).unwrap();
        let expected = MapSerie::from_entries(
            Utf8Serie::from_strs(&[Some("c"), Some("d"), Some("e")]).named("key"),
            Serie::from_values(&[3i64, 4, 5]).named("value"),
            &[0, 1, 3],
            None,
            false,
        )
        .unwrap();
        assert_eq!(back, expected);
    }

    #[test]
    fn map_as_a_struct_child_via_arrow() {
        // A map column nested inside a struct, exported and re-imported through the StructArray path.
        let counts = one_row_map(2);
        let ids = Serie::from_values(&[10i64]);
        let table =
            StructSerie::from_series(vec![ids.named("id"), counts.named("counts")]).unwrap();
        let field = table.to_field("row").to_arrow_field();
        let array = table.to_arrow_array().unwrap();
        let back = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, table);
    }

    #[test]
    fn map_as_a_record_batch_column() {
        // A map column carried as a RecordBatch column (a struct with no top-level nulls).
        let counts = one_row_map(3);
        let ids = Serie::from_values(&[7i64]);
        let table =
            StructSerie::from_series(vec![ids.named("id"), counts.named("counts")]).unwrap();
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(batch.num_rows(), 1);
        let back = StructSerie::from_record_batch(&batch).unwrap();
        assert_eq!(back, table);
    }

    #[test]
    fn map_from_externally_built_arrow_array() {
        // A MapArray built directly with arrow-rs imports to an equal MapSerie.
        use std::sync::Arc;
        let keys = arrow_array::StringArray::from(vec!["a", "b", "c"]);
        let values = arrow_array::Int64Array::from(vec![1i64, 2, 3]);
        let key_field = Arc::new(arrow_schema::Field::new(
            "key",
            arrow_schema::DataType::Utf8,
            false,
        ));
        let value_field = Arc::new(arrow_schema::Field::new(
            "value",
            arrow_schema::DataType::Int64,
            true,
        ));
        let entries = arrow_array::StructArray::from(vec![
            (key_field, Arc::new(keys) as arrow_array::ArrayRef),
            (value_field, Arc::new(values) as arrow_array::ArrayRef),
        ]);
        let entries_field = Arc::new(arrow_schema::Field::new(
            "entries",
            entries.data_type().clone(),
            false,
        ));
        let offsets =
            arrow_buffer::OffsetBuffer::new(arrow_buffer::ScalarBuffer::from(vec![0i32, 2, 2, 3]));
        let array = arrow_array::MapArray::new(entries_field, offsets, entries, None, false);
        let field = arrow_schema::Field::new("m", array.data_type().clone(), false);
        let back = MapSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back.len(), 3);
        assert_eq!(back.row_scalar(0).len(), 2);
        assert_eq!(back.row_scalar(1).len(), 0);
        assert_eq!(back.row_scalar(2).len(), 1);
        // The imported key field is non-nullable.
        assert!(!back.key_field().nullable());
    }

    #[test]
    fn nullable_key_map_import_is_forced_non_null_and_nests_without_panic() {
        // A foreign MapArray whose entries KEY field is declared nullable=true (violating Arrow's
        // "a map key is never null" invariant). Import must force the key field non-null, and then
        // nesting the map as a list element and exporting must NOT panic (the field descriptor and
        // the map array's key nullability now agree — the invariant is restored on import).
        use std::sync::Arc;
        let keys = arrow_array::StringArray::from(vec!["a", "b", "c"]);
        let values = arrow_array::Int64Array::from(vec![1i64, 2, 3]);
        let key_field = Arc::new(arrow_schema::Field::new(
            "key",
            arrow_schema::DataType::Utf8,
            true, // hostile: a nullable key
        ));
        let value_field = Arc::new(arrow_schema::Field::new(
            "value",
            arrow_schema::DataType::Int64,
            true,
        ));
        let entries = arrow_array::StructArray::from(vec![
            (key_field, Arc::new(keys) as arrow_array::ArrayRef),
            (value_field, Arc::new(values) as arrow_array::ArrayRef),
        ]);
        let entries_field = Arc::new(arrow_schema::Field::new(
            "entries",
            entries.data_type().clone(),
            false,
        ));
        let offsets =
            arrow_buffer::OffsetBuffer::new(arrow_buffer::ScalarBuffer::from(vec![0i32, 2, 3]));
        let array = arrow_array::MapArray::new(entries_field, offsets, entries, None, false);
        let field = arrow_schema::Field::new("m", array.data_type().clone(), false);

        let map = MapSerie::from_arrow_array(&array, &field).unwrap();
        // The invariant is enforced on import.
        assert!(!map.key_field().nullable());

        // Nest the map as a single list row and export — must not panic.
        let rows = map.len() as i32;
        let list = ListSerie::from_values(map.named("m"), &[0, rows], None).unwrap();
        let list_array = list.to_arrow_array().unwrap();
        assert_eq!(list_array.len(), 1);
    }

    #[test]
    fn empty_map_with_nullable_key_schema_nests_without_panic() {
        // A MapField whose key field is declared nullable=true; the invariant is enforced so the
        // empty map's stored key field is non-null, and nesting it in a list exports without panic.
        let schema = MapField::new("m", utf8_key(true), i64_value(true), false, false);
        let empty = MapSerie::empty(&schema);
        assert!(!empty.key_field().nullable());
        let list = ListSerie::from_values(empty.named("m"), &[0], None).unwrap();
        let list_array = list.to_arrow_array().unwrap();
        assert_eq!(list_array.len(), 0);
    }
}
