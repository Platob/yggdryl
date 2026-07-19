//! Functional tests for the [`nested`](yggdryl_core::typed::nested) typed layer — the erased
//! [`Column`] / [`Value`] / [`ColumnField`] keystones and the struct family ([`StructField`] schema,
//! [`StructScalar`] rows, [`StructSerie`] "table"): building a heterogeneous, recursive table, graph
//! discovery (`num_columns` / `column_by_name` / `column_path`), reading rows, **deep in-place
//! mutation** of an inner series, the length-mismatch guided error, nullable rows, the schema
//! descriptors, plus the edges (empty struct, all-null child).

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::{Heap, IOBase, IoError};
use yggdryl_core::typed::fixedbyte::{Int32, Int64};
use yggdryl_core::typed::varbyte::Utf8;
use yggdryl_core::typed::{
    Column, ColumnField, FixedSerie, ListSerie, MapSerie, Scalar, Serie, StructField, StructSerie,
    Value, VarSerie,
};

/// Builds an `i32`-offsets [`Heap`] from `offsets` (little-endian, `offsets[0]` first) — the shared
/// helper for the list / map `from_offsets` tests.
fn offsets_heap(offsets: &[i32]) -> Heap {
    let mut heap = Heap::new();
    for (index, &value) in offsets.iter().enumerate() {
        heap.pwrite_i32(index as u64 * 4, value).unwrap();
    }
    heap
}

/// A validity [`Heap`] from `bits` (`true` = valid), LSB-first — for the nullable list / map tests.
fn validity_heap(bits: &[bool]) -> Heap {
    let mut heap = Heap::new();
    for (index, &valid) in bits.iter().enumerate() {
        heap.pwrite_bit(index as u64, valid).unwrap();
    }
    heap
}

/// Builds the reference table used across the tests: an `Int64` `id`, a `Utf8` `name`, and a nested
/// `Struct` `address` (a `Utf8` `city` + an `Int32` `zip`) — three columns of length 3.
fn sample_table() -> StructSerie {
    let id = FixedSerie::<Int64>::from_values(&[10, 20, 30]).with_name("id");
    let name =
        VarSerie::<Utf8>::from_values(&["ada".into(), "bo".into(), "cy".into()]).with_name("name");

    let city = VarSerie::<Utf8>::from_values(&["paris".into(), "rome".into(), "oslo".into()])
        .with_name("city");
    let zip = FixedSerie::<Int32>::from_values(&[75001, 100, 3]).with_name("zip");
    let address = StructSerie::from_columns(vec![Column::from(city), Column::from(zip)])
        .unwrap()
        .with_name("address");

    StructSerie::from_columns(vec![
        Column::from(id),
        Column::from(name),
        Column::from(address),
    ])
    .unwrap()
    .with_name("people")
}

#[test]
fn build_and_shape() {
    let table = sample_table();
    assert_eq!(table.num_columns(), 3);
    assert_eq!(table.len(), 3);
    assert!(!table.is_empty());
    assert_eq!(table.data_type_id(), DataTypeId::Struct);
    assert_eq!(table.name(), Some("people"));
    // The columns are addressable positionally and by name.
    assert_eq!(table.column(0).unwrap().name(), Some("id"));
    assert!(table.column_by_name("missing").is_none());
}

#[test]
fn column_by_name_reads_values() {
    let table = sample_table();
    let name = table.column_by_name("name").expect("name column");
    assert_eq!(name.len(), 3);
    assert_eq!(name.data_type_id(), DataTypeId::Utf8);
    assert_eq!(name.get(0), Value::Utf8("ada".into()));
    assert_eq!(name.get(1), Value::Utf8("bo".into()));
    assert_eq!(name.get(2), Value::Utf8("cy".into()));
    // Out of range is a null value, never a panic.
    assert_eq!(name.get(9), Value::Null);
}

#[test]
fn row_reads_by_index_and_name() {
    let table = sample_table();
    let row = table.row(1).expect("row 1");
    assert_eq!(row.len(), 3);
    assert!(row.is_valid());
    // By name (any column) and by index (the `name` column is index 1).
    assert_eq!(row.get_by_name("id"), Some(&Value::Int64(20)));
    assert_eq!(row.get(1), Some(&Value::Utf8("bo".into())));
    // The nested address row comes back as a `Value::Row`.
    match row.get_by_name("address") {
        Some(Value::Row(address)) => {
            assert_eq!(
                address.get_by_name("city"),
                Some(&Value::Utf8("rome".into()))
            );
            assert_eq!(address.get_by_name("zip"), Some(&Value::Int32(100)));
        }
        other => panic!("expected a nested address row, got {other:?}"),
    }
    // Out of range is `None`.
    assert!(table.row(3).is_none());
}

#[test]
fn column_path_reaches_inner_column() {
    let table = sample_table();
    let city = table.column_path("address.city").expect("address.city");
    assert_eq!(city.name(), Some("city"));
    assert_eq!(city.data_type_id(), DataTypeId::Utf8);
    assert_eq!(city.get(2), Value::Utf8("oslo".into()));
    // A dotted path whose head is not a struct, or a missing segment, yields None.
    assert!(table.column_path("name.nope").is_none());
    assert!(table.column_path("address.zipcode").is_none());
}

#[test]
fn deep_mutation_of_a_top_level_column() {
    let mut table = sample_table();
    // Recover the concrete `FixedSerie<Int64>` from the erased `&mut Column` and edit it in place.
    let column = table.column_by_name_mut("id").expect("id column");
    match column {
        Column::Int64(serie) => serie.set(0, 999).unwrap(),
        other => panic!("expected an Int64 column, got {:?}", other.data_type_id()),
    }
    // The edit is visible on a subsequent row read — no copy, the backing series was mutated.
    let row = table.row(0).expect("row 0");
    assert_eq!(row.get_by_name("id"), Some(&Value::Int64(999)));
}

#[test]
fn deep_mutation_through_a_nested_path() {
    let mut table = sample_table();
    // `column_path_mut` descends into the nested struct and hands back the inner `&mut Column`.
    let inner = table.column_path_mut("address.zip").expect("address.zip");
    match inner {
        Column::Int32(serie) => serie.set(1, 42).unwrap(),
        other => panic!("expected an Int32 column, got {:?}", other.data_type_id()),
    }
    match table.row(1).unwrap().get_by_name("address") {
        Some(Value::Row(address)) => {
            assert_eq!(address.get_by_name("zip"), Some(&Value::Int32(42)));
        }
        other => panic!("expected a nested address row, got {other:?}"),
    }
}

#[test]
fn from_columns_length_mismatch_is_guided() {
    let short = FixedSerie::<Int64>::from_values(&[1, 2]).with_name("a");
    let long = FixedSerie::<Int64>::from_values(&[1, 2, 3]).with_name("b");
    // `unwrap_err` would need `StructSerie: Debug`; match the `Ok`/`Err` instead (a `Column` erases
    // over carriers that are not all `Debug`, so the struct is intentionally not `Debug`).
    let err = match StructSerie::from_columns(vec![Column::from(short), Column::from(long)]) {
        Ok(_) => panic!("expected a length-mismatch error"),
        Err(err) => err,
    };
    assert!(matches!(err, IoError::TypedCast { .. }));
    // The message names the offending child, both lengths, and the fix.
    let text = err.to_string();
    assert!(text.contains("child column 1"), "message: {text}");
    assert!(text.contains('2') && text.contains('3'), "message: {text}");
    assert!(
        text.contains("share the struct's length"),
        "message: {text}"
    );
}

#[test]
fn nullable_struct_row() {
    let id = FixedSerie::<Int64>::from_values(&[1, 2]).with_name("id");
    let mut table = StructSerie::from_columns(vec![Column::from(id)]).unwrap();
    assert_eq!(table.null_count(), 0);

    table.push_null(); // appends a third row, marked null at the struct level
    assert_eq!(table.len(), 3);
    assert_eq!(table.null_count(), 1);
    assert!(table.is_valid(0));
    assert!(!table.is_valid(2));

    // The null row surfaces as `None` from `is_valid` and as a null-flagged `StructScalar`.
    let row = table.row(2).expect("row 2 exists");
    assert!(row.is_null());
    // Its (grown) child slot is a null value.
    assert_eq!(table.column_by_name("id").unwrap().get(2), Value::Null);
}

#[test]
fn struct_field_schema() {
    let table = sample_table();
    let field: StructField = table.field();
    assert_eq!(field.name(), Some("people"));
    assert_eq!(field.num_fields(), 3);
    assert_eq!(field.names(), vec!["id", "name", "address"]);
    assert!(!field.nullable());

    // field_by_name reaches the nested struct field, whose own schema lists its children.
    match field.field_by_name("address") {
        Some(ColumnField::Struct(address)) => {
            assert_eq!(address.names(), vec!["city", "zip"]);
            assert_eq!(address.data_type_id(), DataTypeId::Struct);
        }
        other => panic!("expected a nested struct field, got {other:?}"),
    }

    // Value-typed identity: an equal schema compares equal and hashes equal.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let same = sample_table().field();
    assert_eq!(field, same);
    let mut h1 = DefaultHasher::new();
    let mut h2 = DefaultHasher::new();
    field.hash(&mut h1);
    same.hash(&mut h2);
    assert_eq!(h1.finish(), h2.finish());
    // A schema with a different name is a different key.
    let renamed = field.clone().with_name("humans");
    assert_ne!(field, renamed);
}

#[test]
fn column_field_children_of_a_nested_field() {
    let table = sample_table();
    let field = table.field();
    // The top-level `address` field's ColumnField descriptor exposes its two leaf children.
    let address = field.field_by_name("address").expect("address field");
    assert_eq!(address.name(), Some("address"));
    assert_eq!(address.data_type_id(), DataTypeId::Struct);
    let children = address.children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name(), Some("city"));
    assert_eq!(children[0].data_type_id(), DataTypeId::Utf8);
    assert_eq!(children[1].name(), Some("zip"));
    assert_eq!(children[1].data_type_id(), DataTypeId::I32);
    // A leaf ColumnField has no children.
    assert!(children[0].children().is_empty());
}

#[test]
fn serie_children_graph_edge() {
    // A struct series exposes its columns through the `Serie::children` graph method; a leaf series
    // has none.
    let table = sample_table();
    assert_eq!(Serie::children(&table).len(), 3);

    let leaf = FixedSerie::<Int64>::from_values(&[1, 2, 3]);
    assert!(Serie::children(&leaf).is_empty());
}

#[test]
fn empty_struct_edge() {
    // A struct with no columns is empty and length 0; a name-only `new` starts the same way.
    let empty = StructSerie::from_columns(vec![]).unwrap();
    assert_eq!(empty.num_columns(), 0);
    assert_eq!(empty.len(), 0);
    assert!(empty.is_empty());
    assert_eq!(empty.null_count(), 0);
    assert!(empty.row(0).is_none());

    let built = StructSerie::new("s");
    assert_eq!(built.num_columns(), 0);
    assert_eq!(built.name(), Some("s"));
}

#[test]
fn all_null_child_column() {
    // A bufferless Null column of n nulls composes like any other child.
    let id = FixedSerie::<Int64>::from_values(&[7, 8, 9]).with_name("id");
    let blanks = Column::null(3); // 3 nulls, no buffer
    let table = StructSerie::from_columns(vec![Column::from(id), blanks]).unwrap();
    assert_eq!(table.num_columns(), 2);
    assert_eq!(table.len(), 3);

    let null_col = table.column(1).expect("null column");
    assert_eq!(null_col.len(), 3);
    assert_eq!(null_col.null_count(), 3);
    assert!(null_col.is_null(0));
    assert_eq!(null_col.get(0), Value::Null);
    // A bufferless Null column reports the typed all-null dtype (distinct from Unknown / raw bytes).
    assert_eq!(null_col.data_type_id(), DataTypeId::Null);
    // The real column alongside it still reads through.
    assert_eq!(
        table.row(2).unwrap().get_by_name("id"),
        Some(&Value::Int64(9))
    );
}

// ---- list family ------------------------------------------------------------------------

/// A `ListSerie` over an `Int64` child with offsets `[0, 2, 2, 5]` — three lists `[1, 2]`, `[]`,
/// `[3, 4, 5]` over the flattened child `[1, 2, 3, 4, 5]`.
fn sample_list() -> ListSerie {
    let child = FixedSerie::<Int64>::from_values(&[1, 2, 3, 4, 5]).with_name("item");
    ListSerie::from_offsets(
        "nums",
        offsets_heap(&[0, 2, 2, 5]),
        Column::from(child),
        None,
        3,
    )
}

#[test]
fn list_shape_and_reads() {
    let list = sample_list();
    assert_eq!(list.len(), 3);
    assert!(!list.is_empty());
    assert_eq!(list.data_type_id(), DataTypeId::List);
    assert_eq!(list.name(), Some("nums"));

    // The [start, end) child ranges.
    assert_eq!(list.list_at(0), Some((0, 2)));
    assert_eq!(list.list_at(1), Some((2, 2))); // the empty middle list
    assert_eq!(list.list_at(2), Some((2, 5)));
    assert_eq!(list.list_at(3), None);

    // `get` materializes each sub-list as a `Value::List(ListScalar)`.
    match list.get(0) {
        Value::List(scalar) => {
            assert_eq!(scalar.len(), 2);
            assert_eq!(scalar.get(0), Some(&Value::Int64(1)));
            assert_eq!(scalar.get(1), Some(&Value::Int64(2)));
        }
        other => panic!("expected a list element, got {other:?}"),
    }
    match list.get(2) {
        Value::List(scalar) => {
            assert_eq!(
                scalar.values(),
                &[Value::Int64(3), Value::Int64(4), Value::Int64(5)]
            );
        }
        other => panic!("expected a list element, got {other:?}"),
    }
    // The empty (but non-null) middle list is a `Value::List` of length 0, not `Value::Null`.
    match list.get(1) {
        Value::List(scalar) => assert!(scalar.is_empty() && !scalar.is_null()),
        other => panic!("expected an empty list element, got {other:?}"),
    }
}

#[test]
fn list_null_element() {
    // Offsets `[0, 2, 2, 5]` with the middle list marked null via the validity buffer.
    let child = FixedSerie::<Int64>::from_values(&[1, 2, 3, 4, 5]);
    let list = ListSerie::from_offsets(
        "nums",
        offsets_heap(&[0, 2, 2, 5]),
        Column::from(child),
        Some(validity_heap(&[true, false, true])),
        3,
    );
    assert_eq!(list.null_count(), 1);
    assert!(list.is_valid(0));
    assert!(!list.is_valid(1));
    // A null list surfaces as `Value::Null` from the erased `get`.
    assert_eq!(list.get(1), Value::Null);
    // Its lower-level `list` scalar still reports the null flag.
    assert!(list.list(1).unwrap().is_null());
    assert!(list.list(0).unwrap().is_valid());
}

#[test]
fn list_values_reads_flat_child() {
    let list = sample_list();
    // `values()` is the flattened child column, read directly (all five elements).
    let flat = list.values();
    assert_eq!(flat.len(), 5);
    assert_eq!(flat.data_type_id(), DataTypeId::I64);
    assert_eq!(flat.get(0), Value::Int64(1));
    assert_eq!(flat.get(4), Value::Int64(5));
    // The `Serie::children` graph edge is the one flattened child.
    assert_eq!(Serie::children(&list).len(), 1);
}

#[test]
fn list_deep_mutation_via_values_mut() {
    let mut list = sample_list();
    // Recover the concrete `FixedSerie<Int64>` from the erased `&mut Column` and edit in place.
    match list.values_mut() {
        Column::Int64(serie) => serie.set(0, 99).unwrap(),
        other => panic!("expected an Int64 child, got {:?}", other.data_type_id()),
    }
    // The edit is visible on a subsequent `get` — no copy, the backing child was mutated.
    match list.get(0) {
        Value::List(scalar) => assert_eq!(scalar.get(0), Some(&Value::Int64(99))),
        other => panic!("expected a list element, got {other:?}"),
    }
}

#[test]
fn list_push_builds_offsets() {
    // Build the same three lists by pushing spans over a pre-loaded child.
    let child = FixedSerie::<Int64>::from_values(&[1, 2, 3, 4, 5]);
    let mut list = ListSerie::new("nums", Column::from(child));
    assert_eq!(list.len(), 0);
    assert!(list.is_empty());
    list.push(2); // [1, 2]
    list.push(0); // []
    list.push(3); // [3, 4, 5]
    assert_eq!(list.len(), 3);
    assert_eq!(list.list_at(2), Some((2, 5)));
    list.push_null(); // an empty, null span appended at the tail
    assert_eq!(list.len(), 4);
    assert_eq!(list.null_count(), 1);
    assert_eq!(list.get(3), Value::Null);
}

#[test]
fn list_field_schema() {
    let list = sample_list();
    let field = list.field();
    assert_eq!(field.name(), Some("nums"));
    assert_eq!(field.data_type_id(), DataTypeId::List);
    assert!(!field.nullable());
    // The item field describes the child element type.
    assert_eq!(field.item().data_type_id(), DataTypeId::I64);

    // The list's ColumnField exposes exactly one child (the item), and hashes/compares by value.
    let column_field = ColumnField::List(field.clone());
    assert_eq!(column_field.data_type_id(), DataTypeId::List);
    let children = column_field.children();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].data_type_id(), DataTypeId::I64);

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let same = sample_list().field();
    assert_eq!(field, same);
    let mut h1 = DefaultHasher::new();
    let mut h2 = DefaultHasher::new();
    field.hash(&mut h1);
    same.hash(&mut h2);
    assert_eq!(h1.finish(), h2.finish());
}

// ---- map family -------------------------------------------------------------------------

/// A `MapSerie` with a `Utf8` key + `Int32` value and offsets `[0, 2, 3]` — two maps
/// `{"a": 1, "b": 2}` and `{"c": 3}` over the flattened entries.
fn sample_map() -> MapSerie {
    let keys =
        VarSerie::<Utf8>::from_values(&["a".into(), "b".into(), "c".into()]).with_name("key");
    let vals = FixedSerie::<Int32>::from_values(&[1, 2, 3]).with_name("value");
    MapSerie::from_offsets(
        "m",
        offsets_heap(&[0, 2, 3]),
        Column::from(keys),
        Column::from(vals),
        None,
        2,
    )
    .unwrap()
}

#[test]
fn map_shape_and_reads() {
    let map = sample_map();
    assert_eq!(map.len(), 2);
    assert!(!map.is_empty());
    assert_eq!(map.data_type_id(), DataTypeId::Map);
    assert_eq!(map.name(), Some("m"));

    assert_eq!(map.map_at(0), Some((0, 2)));
    assert_eq!(map.map_at(1), Some((2, 3)));
    assert_eq!(map.map_at(2), None);

    match map.get(0) {
        Value::Map(scalar) => {
            assert_eq!(scalar.len(), 2);
            assert_eq!(scalar.get_key(0), Some(&Value::Utf8("a".into())));
            assert_eq!(scalar.get_value(0), Some(&Value::Int32(1)));
            // Lookup by key.
            assert_eq!(
                scalar.get_by_key(&Value::Utf8("b".into())),
                Some(&Value::Int32(2))
            );
            assert_eq!(scalar.get_by_key(&Value::Utf8("z".into())), None);
        }
        other => panic!("expected a map element, got {other:?}"),
    }
    match map.get(1) {
        Value::Map(scalar) => {
            assert_eq!(
                scalar.get_by_key(&Value::Utf8("c".into())),
                Some(&Value::Int32(3))
            )
        }
        other => panic!("expected a map element, got {other:?}"),
    }
}

#[test]
fn map_keys_and_values_columns() {
    let map = sample_map();
    // `keys()` / `values()` are the entries' two flattened columns.
    assert_eq!(map.keys().len(), 3);
    assert_eq!(map.keys().data_type_id(), DataTypeId::Utf8);
    assert_eq!(map.values().data_type_id(), DataTypeId::I32);
    assert_eq!(map.keys().get(2), Value::Utf8("c".into()));
    assert_eq!(map.values().get(2), Value::Int32(3));
    // `entries()` is the two-column struct; `Serie::children` yields both columns.
    assert_eq!(map.entries().num_columns(), 2);
    assert_eq!(Serie::children(&map).len(), 2);
}

#[test]
fn map_keys_sorted_flag() {
    let map = sample_map();
    assert!(!map.keys_sorted());
    let sorted = sample_map().with_keys_sorted(true);
    assert!(sorted.keys_sorted());
    // The flag carries onto the schema.
    assert!(sorted.field().keys_sorted());
    assert!(!map.field().keys_sorted());
}

#[test]
fn map_nullable_keys_is_guided_error() {
    // A nullable key column (built from options with a None) is refused with a guided error.
    let keys = VarSerie::<Utf8>::from_options(&[Some("a".into()), None]);
    let vals = FixedSerie::<Int32>::from_values(&[1, 2]);
    let err = match MapSerie::new("m", Column::from(keys), Column::from(vals)) {
        Ok(_) => panic!("expected a nullable-keys error"),
        Err(err) => err,
    };
    assert!(matches!(err, IoError::TypedCast { .. }));
    let text = err.to_string();
    assert!(
        text.contains("key column must be non-nullable"),
        "message: {text}"
    );
    assert!(text.contains("cannot be null"), "message: {text}");
}

#[test]
fn map_null_element_and_push() {
    // Build two maps by pushing spans, then a null map at the tail.
    let keys = VarSerie::<Utf8>::from_values(&["a".into(), "b".into(), "c".into()]);
    let vals = FixedSerie::<Int32>::from_values(&[1, 2, 3]);
    let mut map = MapSerie::new("m", Column::from(keys), Column::from(vals)).unwrap();
    map.push(2); // {"a": 1, "b": 2}
    map.push(1); // {"c": 3}
    map.push_null(); // a null map
    assert_eq!(map.len(), 3);
    assert_eq!(map.null_count(), 1);
    assert!(!map.is_valid(2));
    assert_eq!(map.get(2), Value::Null);
    assert!(map.map(2).unwrap().is_null());
}

#[test]
fn map_deep_mutation_via_values_mut() {
    let mut map = sample_map();
    // Edit the flattened value column in place through the erased `&mut Column`.
    match map.values_mut() {
        Column::Int32(serie) => serie.set(0, 42).unwrap(),
        other => panic!(
            "expected an Int32 value column, got {:?}",
            other.data_type_id()
        ),
    }
    match map.get(0) {
        Value::Map(scalar) => assert_eq!(
            scalar.get_by_key(&Value::Utf8("a".into())),
            Some(&Value::Int32(42))
        ),
        other => panic!("expected a map element, got {other:?}"),
    }
}

// ---- nested combos ----------------------------------------------------------------------

#[test]
fn struct_of_list_recurses_and_deep_mutates() {
    // A StructSerie whose one column is a ListSerie — `column_path` / `children` recurse into it,
    // and a deep mutation reaches the flattened leaf child.
    let child = FixedSerie::<Int64>::from_values(&[1, 2, 3, 4, 5]);
    let list = ListSerie::from_offsets(
        "nums",
        offsets_heap(&[0, 2, 2, 5]),
        Column::from(child),
        None,
        3,
    );
    let id = FixedSerie::<Int64>::from_values(&[10, 20, 30]).with_name("id");
    let mut table = StructSerie::from_columns(vec![Column::from(id), Column::from(list)])
        .unwrap()
        .with_name("t");
    assert_eq!(table.num_columns(), 2);
    assert_eq!(table.len(), 3);

    // The list column is addressable by name and reports its nested field.
    let field = table.field();
    match field.field_by_name("nums") {
        Some(ColumnField::List(list_field)) => {
            assert_eq!(list_field.item().data_type_id(), DataTypeId::I64);
        }
        other => panic!("expected a nested list field, got {other:?}"),
    }

    // `column_by_name` reaches the list; reading row 2's list value.
    match table.column_by_name("nums").unwrap().get(2) {
        Value::List(scalar) => assert_eq!(scalar.len(), 3),
        other => panic!("expected a list element, got {other:?}"),
    }

    // Deep mutation: recover the ListSerie, then its flattened Int64 child, and edit the leaf.
    match table.column_by_name_mut("nums").unwrap() {
        Column::List(list) => match list.values_mut() {
            Column::Int64(serie) => serie.set(0, 77).unwrap(),
            other => panic!("expected an Int64 leaf, got {:?}", other.data_type_id()),
        },
        other => panic!("expected a list column, got {:?}", other.data_type_id()),
    }
    match table.column_by_name("nums").unwrap().get(0) {
        Value::List(scalar) => assert_eq!(scalar.get(0), Some(&Value::Int64(77))),
        other => panic!("expected a list element, got {other:?}"),
    }
}

#[test]
fn list_of_map_nested_value() {
    // A ListSerie whose flattened child is a MapSerie — reading a list element yields lists of maps.
    let keys = VarSerie::<Utf8>::from_values(&["a".into(), "b".into(), "c".into()]);
    let vals = FixedSerie::<Int32>::from_values(&[1, 2, 3]);
    // Three maps: {"a":1}, {"b":2}, {"c":3}.
    let inner_map = MapSerie::from_offsets(
        "m",
        offsets_heap(&[0, 1, 2, 3]),
        Column::from(keys),
        Column::from(vals),
        None,
        3,
    )
    .unwrap();
    // Two lists over the three maps: [map0, map1] and [map2].
    let list = ListSerie::from_offsets(
        "maps",
        offsets_heap(&[0, 2, 3]),
        Column::from(inner_map),
        None,
        2,
    );
    assert_eq!(list.len(), 2);
    // `children` recurses into the flattened map child.
    assert_eq!(Serie::children(&list).len(), 1);
    assert_eq!(list.values().data_type_id(), DataTypeId::Map);

    // List 0 holds two map elements.
    match list.get(0) {
        Value::List(scalar) => {
            assert_eq!(scalar.len(), 2);
            match scalar.get(1) {
                Some(Value::Map(inner)) => assert_eq!(
                    inner.get_by_key(&Value::Utf8("b".into())),
                    Some(&Value::Int32(2))
                ),
                other => panic!("expected an inner map, got {other:?}"),
            }
        }
        other => panic!("expected a list element, got {other:?}"),
    }
}
