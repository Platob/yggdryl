//! Rows are schema-ordered sequences; structured-text objects are records.

use yggdryl::{DataType, Expression, Value};

fn trade(id: i64, venue: Option<&str>) -> Value {
    Value::from_sequence([
        Value::I64(id),
        venue.map_or(Value::Null, |venue| Value::String(venue.into())),
    ])
}

#[test]
fn rows_are_sequences_and_objects_are_records() {
    let row = trade(7, Some("XNAS"));
    assert_eq!(row.kind(), "sequence");
    assert_eq!(
        row.as_sequence(),
        Some([Value::I64(7), Value::from("XNAS")].as_slice())
    );

    let object =
        Value::from_record([("id", Value::I64(7)), ("venue", Value::from("XNAS"))]).unwrap();
    let json = String::from_utf8(yggdryl::json::into_bytes(&object).unwrap()).unwrap();
    assert_eq!(json, "{\"id\":7,\"venue\":\"XNAS\"}");
    assert_eq!(yggdryl::json::from_utf8(&json).unwrap(), object);
}

#[test]
fn the_removed_typed_row_wire_has_no_compatibility_alias() {
    let removed = r#"{"type":"record","value":["struct<id: int64>",[1]]}"#;
    assert!(serde_json::from_str::<Value>(removed).is_err());
}

#[test]
fn struct_expressions_evaluate_to_schema_ordered_sequences() {
    let schema = DataType::from_fields([DataType::Int64.required_field("source")])
        .unwrap()
        .required_field("row");
    let bound = "struct(1 as id, 'XNAS' as venue)"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let source = Value::from_sequence([Value::I64(0)]);
    let expected = Value::from_sequence([Value::I64(1), Value::from("XNAS")]);
    assert_eq!(bound.eval(&source).unwrap(), expected);

    let printed = bound.expression().to_string();
    assert!(printed.contains("struct("), "{printed}");
    let reparsed = printed
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(reparsed.eval(&source).unwrap(), expected);
}

#[cfg(feature = "arrow")]
mod arrow_bridge {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, Value, arrow};

    fn batch() -> RecordBatch {
        let schema = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("venue"),
        ])
        .unwrap()
        .required_field("row")
        .into_arrow_schema()
        .unwrap();
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1_i64, 2])),
                Arc::new(StringArray::from(vec![Some("XNAS"), None])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_batch_reads_as_schema_ordered_row_sequences() {
        let value = arrow::batch_to_value(&batch()).unwrap();
        let rows = value.as_sequence().expect("a sequence of rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].as_sequence(),
            Some([Value::I64(1), Value::from("XNAS")].as_slice())
        );
        assert_eq!(rows[1].as_sequence().unwrap()[1], Value::Null);

        let json = String::from_utf8(yggdryl::json::into_bytes(&value).unwrap()).unwrap();
        assert_eq!(json, "[[1,\"XNAS\"],[2,null]]");
    }

    #[test]
    fn an_array_reads_as_the_sequence_its_values_spell() {
        let field = DataType::Int64.nullable_field("id");
        let array = Int64Array::from(vec![Some(5_i64), None]);
        let value = arrow::array_to_value(&field, &array).unwrap();
        assert_eq!(value, Value::from_sequence([Value::I64(5), Value::Null]));
    }
}
