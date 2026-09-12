//! What a row reads as under its Struct field, cell by cell.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::{DataType, Field, FieldRecord, FieldScalar, Scalar};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn schema() -> Field {
    DataType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::utf8(), true),
        Field::new("price", DataType::decimal128(10, 2).unwrap(), true),
    ])
    .unwrap()
    .required_field("row")
}

fn row() -> Scalar {
    Scalar::from_sequence([
        Scalar::from(7_i64),
        Scalar::from("AAPL"),
        Scalar::d128(150, 2),
    ])
}

#[test]
fn a_row_pairs_every_cell_with_its_child() {
    let schema = schema();
    let record = FieldRecord::new(&schema, row()).unwrap();
    assert_eq!(record.len(), 3);
    assert!(!record.is_empty());
    assert!(std::ptr::eq(record.field(), &schema));
    for (index, (cell, child)) in record.iter().zip(schema.fields()).enumerate() {
        assert!(std::ptr::eq(cell.field(), child), "cell {index}");
        assert_eq!(cell.name(), child.name());
    }
    assert_eq!(
        record.names().collect::<Vec<_>>(),
        ["id", "symbol", "price"]
    );
    assert_eq!(record[0].as_i64(), Some(7));
    assert_eq!(record["symbol"].as_str(), Some("AAPL"));
    assert_eq!(record.as_str("symbol"), Some("AAPL"));
    assert_eq!(record.as_str(0), None);
    assert_eq!(record.as_str("absent"), None);
    assert_eq!(
        record.get(2).and_then(FieldScalar::as_decimal),
        Some((crate::I256::from_i128(150), 2))
    );
    assert_eq!(record.get("price"), record.get_by_index(2));
    assert!(record.get(3).is_none());
    assert!(record.get_by_name("volume").is_none());
}

#[test]
fn a_name_resolves_exactly_as_the_field_resolves_it() {
    let schema = DataType::from_fields([
        Field::new("Symbol", DataType::utf8(), false),
        Field::new("symbol", DataType::utf8(), false),
        DataType::from_fields([Field::new("px", DataType::Float64, false)])
            .unwrap()
            .required_field("leg"),
    ])
    .unwrap()
    .required_field("row");
    let record = FieldRecord::new(
        &schema,
        Scalar::from_sequence([
            Scalar::from("upper"),
            Scalar::from("lower"),
            Scalar::from_sequence([Scalar::from(1.5_f64)]),
        ]),
    )
    .unwrap();
    // The field's own child lookup owns the rule: every key answers the same
    // child on both sides, and a folded name misses on both.
    for key in [
        "symbol", "Symbol", "SYMBOL", "leg", "LEG", "px", "leg.px", "absent",
    ] {
        assert_eq!(
            record.get_by_name(key).map(FieldScalar::name),
            schema
                .index_of(key)
                .map(|index| schema.fields()[index].name()),
            "{key}"
        );
    }
    assert_eq!(record.as_str("symbol"), Some("lower"));
    assert_eq!(record.as_str("Symbol"), Some("upper"));
    assert!(record.get("SYMBOL").is_none());
    // A dotted path names a descendant the field walks to, never a cell.
    assert_eq!(schema.get_field("leg.px").map(Field::name), Some("px"));
    assert!(record.get("leg.px").is_none());
    assert_eq!(record["leg"].get(0).and_then(Scalar::as_f64), Some(1.5));
}

#[test]
fn a_named_record_and_an_ordered_sequence_read_alike() {
    let schema = schema();
    let named = Scalar::from_record([
        ("price", Scalar::d128(150, 2)),
        ("symbol", Scalar::from("AAPL")),
        ("id", Scalar::from(7)),
    ])
    .unwrap();
    let from_record = FieldRecord::new(&schema, named).unwrap();
    let from_sequence = FieldRecord::new(&schema, row()).unwrap();
    assert_eq!(from_record, from_sequence);
    assert_eq!(hash_of(&from_record), hash_of(&from_sequence));
    // The cells are what the field stores: the id was narrowed on the way in.
    assert_eq!(from_record[0].value(), &Scalar::from(7_i64));
    assert_eq!(from_record.clone().into_scalar(), row());
    assert_eq!(Scalar::from(from_record.clone()), row());
    assert_eq!(
        from_record.into_values(),
        row().as_sequence().unwrap().to_vec()
    );
}

#[test]
fn the_root_must_be_a_required_struct_and_the_row_must_fit_it() {
    let nullable = schema().with_nullable(true);
    let refused = FieldRecord::new(&nullable, row()).unwrap_err().to_string();
    assert!(refused.contains("non-null struct root"), "{refused}");

    let leaf = Field::new("id", DataType::Int64, false);
    let refused = FieldRecord::new(&leaf, row()).unwrap_err().to_string();
    assert!(refused.contains("struct root"), "{refused}");

    let schema = schema();
    let short = Scalar::from_sequence([Scalar::from(7_i64)]);
    let refused = FieldRecord::new(&schema, short).unwrap_err().to_string();
    assert!(refused.contains("3"), "{refused}");

    let wrong = Scalar::from_sequence([Scalar::from("seven"), Scalar::Null, Scalar::Null]);
    let refused = FieldRecord::new(&schema, wrong).unwrap_err().to_string();
    assert!(refused.contains("id"), "{refused}");

    let missing = Scalar::from_sequence([Scalar::Null, Scalar::Null, Scalar::Null]);
    assert!(
        FieldRecord::new(&schema, missing).is_err(),
        "id is required"
    );

    let extra =
        Scalar::from_record([("id", Scalar::from(1)), ("volume", Scalar::from(1))]).unwrap();
    assert!(FieldRecord::new(&schema, extra).is_err());
}

#[test]
fn an_empty_struct_reads_an_empty_row() {
    let schema = DataType::from_fields([]).unwrap().required_field("row");
    let record = FieldRecord::new(&schema, Scalar::from_sequence([])).unwrap();
    assert!(record.is_empty());
    assert_eq!(record.names().count(), 0);
    assert_eq!(record.to_string(), "{}");
    assert_eq!(record.into_scalar(), Scalar::from_sequence([]));
}

#[test]
fn a_row_iterates_by_value_and_by_reference() {
    let schema = schema();
    let record = FieldRecord::new(&schema, row()).unwrap();
    let names: Vec<&str> = (&record).into_iter().map(FieldScalar::name).collect();
    assert_eq!(names, ["id", "symbol", "price"]);
    let values: Vec<Scalar> = record.into_iter().map(FieldScalar::into_value).collect();
    assert_eq!(values, row().as_sequence().unwrap());
}

#[test]
fn rows_compare_by_datatype_and_cells_and_never_by_the_root_around_them() {
    let first = schema();
    let renamed = schema().with_name("trade");
    let left = FieldRecord::new(&first, row()).unwrap();
    let right = FieldRecord::new(&renamed, row()).unwrap();
    assert_eq!(left, right);
    assert_eq!(hash_of(&left), hash_of(&right));

    let other = FieldRecord::new(
        &first,
        Scalar::from_sequence([Scalar::from(8_i64), Scalar::Null, Scalar::Null]),
    )
    .unwrap();
    assert_ne!(left, other);

    let widened = DataType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::large_utf8(), true),
        Field::new("price", DataType::decimal128(10, 2).unwrap(), true),
    ])
    .unwrap()
    .required_field("row");
    let wide = FieldRecord::new(&widened, row()).unwrap();
    assert_ne!(left, wide, "another datatype is another row");
}

#[test]
fn a_row_displays_its_named_cells() {
    let schema = schema();
    let record = FieldRecord::new(&schema, row()).unwrap();
    assert_eq!(record.to_string(), "{id=7, symbol=AAPL, price=1.50}");
    let absent = FieldRecord::new(
        &schema,
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null, Scalar::Null]),
    )
    .unwrap();
    assert_eq!(absent.to_string(), "{id=1, symbol=null, price=null}");
    let debug = format!("{record:?}");
    assert!(debug.starts_with("FieldRecord {"), "{debug}");
}

#[test]
fn a_row_serializes_as_its_field_and_cells() {
    let schema = schema();
    let record = FieldRecord::new(&schema, row()).unwrap();
    let encoded: serde_json::Value = serde_json::to_value(&record).unwrap();
    assert_eq!(encoded["field"], serde_json::to_value(&schema).unwrap());
    assert_eq!(encoded["values"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        encoded["values"][1]["value"],
        serde_json::to_value(Scalar::from("AAPL")).unwrap()
    );
}

#[test]
#[should_panic(expected = "is not a child of the field")]
fn subscripting_an_absent_name_panics_like_a_field_does() {
    let schema = schema();
    let record = FieldRecord::new(&schema, row()).unwrap();
    let _ = &record["volume"];
}

#[test]
#[should_panic(expected = "out of range")]
fn subscripting_a_position_past_the_row_panics_like_a_field_does() {
    let schema = schema();
    let record = FieldRecord::new(&schema, row()).unwrap();
    let _ = &record[3];
}

#[cfg(feature = "arrow")]
mod arrow {
    use super::{DataType, Field, FieldRecord, Scalar, row, schema};

    #[test]
    fn a_row_round_trips_through_a_one_row_batch() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        let batch = record.clone().into_arrow_batch().unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 3);
        let decoded = FieldRecord::from_arrow_batch(&schema, &batch, 0).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.into_scalar(), row());
    }

    #[test]
    fn a_batch_row_is_read_under_the_field_the_batch_was_written_under() {
        let schema = schema();
        let rows = Scalar::from_sequence([
            row(),
            Scalar::from_sequence([Scalar::from(8_i64), Scalar::Null, Scalar::d128(1, 2)]),
        ]);
        let batch = crate::arrow::batch_from_value(&schema, &rows).unwrap();
        let second = FieldRecord::from_arrow_batch(&schema, &batch, 1).unwrap();
        assert_eq!(second["id"].as_i64(), Some(8));
        assert!(second["symbol"].is_null());
        assert_eq!(second.as_str("symbol"), None);
        // A physically spelled reading is held canonically: the decimal is
        // at the field's scale, so it is the value the row was built from.
        assert_eq!(second["price"].value(), &Scalar::d128(1, 2));

        let past = FieldRecord::from_arrow_batch(&schema, &batch, 2)
            .unwrap_err()
            .to_string();
        assert!(past.contains("row 2"), "{past}");

        let narrower = DataType::from_fields([Field::new("id", DataType::Int64, false)])
            .unwrap()
            .required_field("row");
        let refused = FieldRecord::from_arrow_batch(&narrower, &batch, 0)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("columns"), "{refused}");

        let nullable = schema.clone().with_nullable(true);
        assert!(FieldRecord::from_arrow_batch(&nullable, &batch, 0).is_err());
    }
}
