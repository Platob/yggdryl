//! The **nested** typed layer (`io::nested`): the erased [`Column`] / [`ColumnField`] carriers, the
//! [`StructField`] schema (↔ Arrow `Field` *and* `Schema`), and [`StructSerie`] (↔ `StructArray` and
//! `RecordBatch`). Structural tests run always; the Arrow interop tests are gated on the `arrow`
//! feature. Recursion (struct-of-struct), nullability, and byte-exact round-trips are the focus.

use yggdryl_core::io::fixed::{Field, PrimitiveType, Serie};
use yggdryl_core::io::nested::{Column, ColumnField, StructField, StructSerie, Value};
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{DataTypeId, FieldType};

// -------------------------------------------------------------------------------------
// StructField — the centralized schema
// -------------------------------------------------------------------------------------

fn person_schema() -> StructField {
    StructField::new(
        "person",
        vec![
            ColumnField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false)),
            ColumnField::leaf(Field::new("age", &PrimitiveType::<i32>::new(), true)),
        ],
        true,
    )
}

#[test]
fn struct_field_is_a_schema() {
    let schema = person_schema();
    assert_eq!(schema.name(), "person");
    assert_eq!(schema.type_name(), "struct");
    assert_eq!(FieldType::type_id(&schema), DataTypeId::Struct);
    assert!(schema.is_struct() && schema.nullable());
    assert_eq!(schema.num_fields(), 2);
    assert_eq!(schema.field(0).unwrap().name(), "id");
    assert_eq!(schema.index_of("age"), Some(1));
    assert!(schema.field_named("missing").is_none());

    // A value type: equal by content, and usable as a map key.
    assert_eq!(person_schema(), schema);
    use std::collections::HashSet;
    let set: HashSet<StructField> = [person_schema(), schema.clone()].into_iter().collect();
    assert_eq!(set.len(), 1);
}

#[test]
fn struct_field_with_builders() {
    let base = StructField::new("s", vec![], false);
    let built = base
        .with_field(ColumnField::leaf(Field::new(
            "x",
            &PrimitiveType::<f64>::new(),
            false,
        )))
        .with_nullable(true)
        .with_metadata_entry("origin", "test");
    assert_eq!(built.num_fields(), 1);
    assert!(built.nullable());
    assert_eq!(built.metadata().get("origin"), Some("test"));
    // The original is untouched (immutable updates).
    assert_eq!(base.num_fields(), 0);
}

// -------------------------------------------------------------------------------------
// Column — erasing typed columns, field inference, row access
// -------------------------------------------------------------------------------------

#[test]
fn column_erases_every_leaf_family() {
    use yggdryl_core::io::fixed::{D128Serie, FixedBinarySerie, NullSerie, D128};
    use yggdryl_core::io::var::BinarySerie;

    let fixed = Column::from(Serie::from_values(&[1i32, 2, 3]));
    assert_eq!(fixed.type_id(), DataTypeId::I32);
    assert_eq!(fixed.len(), 3);

    let utf8 = Column::from(Utf8Serie::from_strs(&[Some("a"), None]));
    assert_eq!(utf8.type_id(), DataTypeId::Utf8);
    assert!(utf8.has_nulls());

    let binary = Column::from(BinarySerie::from_byte_values(&[Some(&b"x"[..])]).unwrap());
    assert_eq!(binary.type_id(), DataTypeId::Binary);

    let decimal =
        Column::from(D128Serie::from_values(20, 2, &[D128::new(12345, 2).unwrap()]).unwrap());
    assert_eq!(decimal.type_id(), DataTypeId::D128);

    let fsb = Column::from(FixedBinarySerie::from_values(2, &[Some(&b"ab"[..])]).unwrap());
    assert_eq!(fsb.type_id(), DataTypeId::FixedBinary);

    let null = Column::from(NullSerie::with_len(4));
    assert_eq!(null.type_id(), DataTypeId::Null);
    assert_eq!(null.len(), 4);
    assert_eq!(null.null_count(), 4);
}

#[test]
fn column_field_infers_nullability_and_names() {
    let dense = Column::from(Serie::from_values(&[1i64, 2]));
    let field = dense.field("id");
    assert_eq!(field.name(), "id");
    assert_eq!(field.type_id(), DataTypeId::I64);
    assert!(!field.nullable()); // no nulls -> non-nullable

    let nullable = Column::from(Serie::from_options(&[Some(1i64), None]));
    assert!(nullable.field("id").nullable());
}

// -------------------------------------------------------------------------------------
// StructSerie — build, row access, serialize round-trip
// -------------------------------------------------------------------------------------

fn sample_table() -> StructSerie {
    let ids = Column::from(Serie::from_values(&[1i64, 2, 3]));
    let ages = Column::from(Serie::from_options(&[Some(30i32), None, Some(41)]));
    let names = Column::from(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
    StructSerie::from_named(vec![("id", ids), ("age", ages), ("name", names)]).unwrap()
}

#[test]
fn struct_serie_builds_from_named_columns() {
    let table = sample_table();
    assert_eq!(table.len(), 3);
    assert_eq!(table.num_columns(), 3);
    assert_eq!(table.field(2).unwrap().name(), "name");
    assert_eq!(
        table.column_named("age").unwrap().type_id(),
        DataTypeId::I32
    );
    assert!(!table.has_nulls()); // no top-level (struct) nulls

    // Mismatched lengths are a guided error.
    let short = Column::from(Serie::from_values(&[1i64]));
    let long = Column::from(Serie::from_values(&[1i64, 2]));
    let err = StructSerie::from_named(vec![("a", short), ("b", long)]).unwrap_err();
    assert!(err.to_string().contains("length"));
}

#[test]
fn struct_serie_row_access() {
    let table = sample_table();
    let Value::Struct(row) = table.get_row(0) else {
        panic!("expected a struct row");
    };
    assert!(!row.is_null());
    assert_eq!(row.num_fields(), 3);
    // The `name` cell of row 0 is the utf8 "ann".
    let name = row.value_named("name").unwrap();
    assert_eq!(name.bytes(), Some(&b"ann"[..]));
    // Row 2's `name` is null (the utf8 column has a null there).
    let Value::Struct(row2) = table.get_row(2) else {
        panic!()
    };
    assert!(row2.value_named("name").unwrap().is_null());
    // Out of range -> null.
    assert!(table.get_row(9).is_null());
}

#[test]
fn struct_serie_serialize_round_trip() {
    let table = sample_table();
    let bytes = table.serialize_bytes();
    let back = StructSerie::deserialize_bytes(&bytes).unwrap();
    assert_eq!(back, table);
}

#[test]
fn struct_serie_nullable_rows_round_trip() {
    let ids = Column::from(Serie::from_values(&[1i64, 2, 3]));
    let names = Column::from(Utf8Serie::from_strs(&[Some("a"), Some("b"), Some("c")]));
    let fields = vec![ids.field("id"), names.field("name")];
    // Row 1 is a null struct.
    let table =
        StructSerie::from_columns(fields, vec![ids, names], Some(&[true, false, true])).unwrap();
    assert_eq!(table.null_count(), 1);
    assert!(table.get_row(1).is_null());
    assert!(!table.get_row(0).is_null());
    // Round-trips through the byte codec (validity preserved).
    let back = StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap();
    assert_eq!(back, table);
}

#[test]
fn struct_of_struct_serialize_round_trip() {
    // Recursive: a struct column whose child is itself a struct column.
    let xs = Column::from(Serie::from_values(&[1i32, 2]));
    let ys = Column::from(Serie::from_values(&[3i32, 4]));
    let points = StructSerie::from_named(vec![("x", xs), ("y", ys)]).unwrap();
    let labels = Column::from(Utf8Serie::from_strs(&[Some("p"), Some("q")]));
    let outer =
        StructSerie::from_named(vec![("point", Column::from(points)), ("label", labels)]).unwrap();
    assert_eq!(outer.len(), 2);
    assert_eq!(outer.column(0).unwrap().type_id(), DataTypeId::Struct);
    let back = StructSerie::deserialize_bytes(&outer.serialize_bytes()).unwrap();
    assert_eq!(back, outer);
}

#[test]
fn empty_struct_serie() {
    let schema = person_schema();
    let empty = StructSerie::empty(&schema);
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.num_columns(), 2);
    assert_eq!(
        StructSerie::deserialize_bytes(&empty.serialize_bytes()).unwrap(),
        empty
    );
}

// -------------------------------------------------------------------------------------
// Arrow interop (feature `arrow`)
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
mod arrow {
    use super::*;
    use arrow_array::Array;

    #[test]
    fn struct_field_maps_to_arrow_field_and_schema() {
        let schema = person_schema();
        // As an Arrow Field of Struct type.
        let field = schema.to_arrow_field();
        assert_eq!(field.name(), "person");
        assert!(field.is_nullable());
        let arrow_schema::DataType::Struct(children) = field.data_type() else {
            panic!("expected a Struct data type");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name(), "id");
        assert_eq!(children[0].data_type(), &arrow_schema::DataType::Int64);
        // Round-trips back exactly.
        assert_eq!(StructField::from_arrow_field(&field), Some(schema.clone()));

        // As a top-level Arrow Schema (children become the schema's fields).
        let arrow_schema = schema.to_arrow_schema();
        assert_eq!(arrow_schema.fields().len(), 2);
        assert_eq!(arrow_schema.field(1).name(), "age");
        // from_arrow_schema yields an unnamed, non-null struct of the same children.
        let recovered = StructField::from_arrow_schema(&arrow_schema).unwrap();
        assert_eq!(recovered.num_fields(), 2);
        assert_eq!(recovered.name(), "");
    }

    #[test]
    fn struct_serie_to_from_struct_array() {
        let table = sample_table();
        let field = table.to_field("person").to_arrow_field();
        let array = table.to_arrow_array();
        assert_eq!(array.len(), 3);
        assert_eq!(array.num_columns(), 3);
        // Column 2 (name) is a StringArray with a null at index 2.
        let names = array
            .column(2)
            .as_any()
            .downcast_ref::<arrow_array::StringArray>()
            .unwrap();
        assert_eq!(names.value(0), "ann");
        assert!(names.is_null(2));
        // Round-trip back to a StructSerie, byte-exact.
        let back = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, table);
    }

    #[test]
    fn struct_serie_to_from_record_batch() {
        let table = sample_table();
        let batch = table.to_record_batch().unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(batch.schema().field(0).name(), "id");
        // Round-trips back, byte-exact.
        let back = StructSerie::from_record_batch(&batch).unwrap();
        assert_eq!(back, table);
    }

    #[test]
    fn nullable_struct_has_no_record_batch() {
        let ids = Column::from(Serie::from_values(&[1i64, 2]));
        let table =
            StructSerie::from_columns(vec![ids.field("id")], vec![ids], Some(&[true, false]))
                .unwrap();
        // A struct with null rows can be a StructArray...
        let array = table.to_arrow_array();
        assert_eq!(array.null_count(), 1);
        // ...but not a RecordBatch (no top-level validity).
        let err = table.to_record_batch().unwrap_err();
        assert!(err.to_string().contains("RecordBatch"));
    }

    #[test]
    fn struct_of_struct_arrow_round_trip() {
        let xs = Column::from(Serie::from_options(&[Some(1i32), None]));
        let ys = Column::from(Serie::from_values(&[3i32, 4]));
        let points = StructSerie::from_named(vec![("x", xs), ("y", ys)]).unwrap();
        let outer = StructSerie::from_named(vec![("point", Column::from(points))]).unwrap();
        let field = outer.to_field("outer").to_arrow_field();
        let array = outer.to_arrow_array();
        // The child column is itself a StructArray.
        assert!(array
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .is_some());
        let back = StructSerie::from_arrow_array(&array, &field).unwrap();
        assert_eq!(back, outer);
    }

    #[test]
    fn decimal_and_fixedsize_children_round_trip_via_record_batch() {
        use yggdryl_core::io::fixed::{D128Serie, FixedBinarySerie, D128};

        let amounts = Column::from(
            D128Serie::from_values(
                20,
                4,
                &[D128::new(105000, 4).unwrap(), D128::new(2, 4).unwrap()],
            )
            .unwrap(),
        );
        let codes = Column::from(
            FixedBinarySerie::from_values(3, &[Some(&b"USD"[..]), Some(&b"EUR"[..])]).unwrap(),
        );
        let table = StructSerie::from_named(vec![("amt", amounts), ("code", codes)]).unwrap();

        let batch = table.to_record_batch().unwrap();
        // The Arrow column types are the precise ones (Decimal128, FixedSizeBinary) — each child
        // delegates to its own `Serie`'s zero-copy converter.
        assert!(matches!(
            batch.schema().field(0).data_type(),
            arrow_schema::DataType::Decimal128(20, 4)
        ));
        assert_eq!(
            batch.schema().field(1).data_type(),
            &arrow_schema::DataType::FixedSizeBinary(3)
        );
        let back = StructSerie::from_record_batch(&batch).unwrap();
        assert_eq!(back, table);
    }

    #[test]
    fn from_externally_built_record_batch() {
        // A RecordBatch built by Arrow itself imports into a StructSerie.
        use std::sync::Arc;
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("n", arrow_schema::DataType::Int32, false),
            arrow_schema::Field::new("s", arrow_schema::DataType::Utf8, true),
        ]));
        let n = Arc::new(arrow_array::Int32Array::from(vec![10, 20, 30]));
        let s = Arc::new(arrow_array::StringArray::from(vec![
            Some("x"),
            None,
            Some("z"),
        ]));
        let batch = arrow_array::RecordBatch::try_new(schema, vec![n, s]).unwrap();
        let table = StructSerie::from_record_batch(&batch).unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(table.column_named("n").unwrap().type_id(), DataTypeId::I32);
        // And back out to an equal batch.
        let round = table.to_record_batch().unwrap();
        assert_eq!(round.num_rows(), 3);
    }

    #[test]
    fn unsupported_arrow_type_is_a_guided_error() {
        use std::sync::Arc;
        // A Boolean column is not modeled by this crate.
        let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
            "flag",
            arrow_schema::DataType::Boolean,
            false,
        )]));
        let flags = Arc::new(arrow_array::BooleanArray::from(vec![true, false]));
        let batch = arrow_array::RecordBatch::try_new(schema, vec![flags]).unwrap();
        let err = StructSerie::from_record_batch(&batch).unwrap_err();
        assert!(err.to_string().contains("not a yggdryl-modeled"));
    }
}
