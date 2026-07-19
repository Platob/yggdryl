//! Functional tests for the [`nested`](yggdryl_core::typed::nested) typed layer — the erased
//! [`Column`] / [`Value`] / [`ColumnField`] keystones and the struct family ([`StructField`] schema,
//! [`StructScalar`] rows, [`StructSerie`] "table"): building a heterogeneous, recursive table, graph
//! discovery (`num_columns` / `column_by_name` / `column_path`), reading rows, **deep in-place
//! mutation** of an inner series, the length-mismatch guided error, nullable rows, the schema
//! descriptors, plus the edges (empty struct, all-null child).

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::IoError;
use yggdryl_core::typed::fixedbyte::{Int32, Int64};
use yggdryl_core::typed::varbyte::Utf8;
use yggdryl_core::typed::{
    Column, ColumnField, FixedSerie, Scalar, Serie, StructField, StructSerie, Value, VarSerie,
};

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
    assert_eq!(null_col.data_type_id(), DataTypeId::Unknown);
    // The real column alongside it still reads through.
    assert_eq!(
        table.row(2).unwrap().get_by_name("id"),
        Some(&Value::Int64(9))
    );
}
