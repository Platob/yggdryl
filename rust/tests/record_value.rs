//! A record is a typed row: its datatype and one value per field, in order.

use yggdryl::{DataType, Value};

fn trade_type() -> DataType {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .unwrap()
}

fn trade(id: i64, venue: Option<&str>) -> Value {
    Value::record(
        trade_type(),
        [
            Value::I64(id),
            venue.map_or(Value::Null, |venue| Value::String(venue.into())),
        ],
    )
    .unwrap()
}

#[test]
fn a_record_knows_its_type_and_refuses_the_wrong_shape() {
    let record = trade(7, Some("XNAS"));
    let (data_type, values) = record.as_record().expect("a record");
    assert_eq!(data_type, &trade_type());
    assert_eq!(values.len(), 2);
    assert_eq!(record.kind(), "record");

    // Inference is what the record already carries.
    assert_eq!(record.data_type().unwrap(), trade_type());

    // A value count that disagrees with the fields is an error, not a guess.
    assert!(Value::record(trade_type(), [Value::I64(1)]).is_err());
    // A scalar datatype types no record.
    assert!(Value::record(DataType::Int64, [Value::I64(1)]).is_err());
}

#[test]
fn every_text_format_spells_a_record_as_its_named_mapping() {
    let record = trade(7, Some("XNAS"));

    let json = String::from_utf8(yggdryl::json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(json, "{\"id\":7,\"venue\":\"XNAS\"}");

    let yaml = String::from_utf8(yggdryl::yaml::to_vec(&record).unwrap()).unwrap();
    assert!(yaml.contains("id: 7"), "unexpected yaml: {yaml}");
    assert!(yaml.contains("venue: XNAS"), "unexpected yaml: {yaml}");

    // TOML quotes keys canonically, so the table spells the same names.
    let toml = String::from_utf8(yggdryl::toml::to_vec(&record).unwrap()).unwrap();
    assert!(toml.contains("\"id\" = 7"), "unexpected toml: {toml}");
    assert!(
        toml.contains("\"venue\" = \"XNAS\""),
        "unexpected toml: {toml}"
    );

    // The mapping spelling reads back as the mapping it is: the names survive,
    // the datatype is what the schemaless wire drops.
    let reread = yggdryl::json::from_str(&json).unwrap();
    assert_eq!(reread, record.record_to_mapping());
}

#[test]
fn records_order_hash_and_compare_by_type_then_values() {
    use std::collections::HashSet;

    let left = trade(1, Some("XNAS"));
    let same = trade(1, Some("XNAS"));
    let other = trade(2, Some("XNAS"));

    assert_eq!(left, same);
    assert!(left < other);

    // The hash reads canonical content only, so the key is stable.
    #[allow(clippy::mutable_key_type)]
    let mut seen = HashSet::new();
    assert!(seen.insert(left));
    assert!(!seen.insert(same));
    assert!(seen.insert(other));
}

#[cfg(feature = "arrow")]
mod arrow_bridge {
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, Value, arrow};

    fn batch() -> RecordBatch {
        let schema = super::trade_type()
            .required_field("row")
            .to_arrow_schema()
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
    fn a_batch_reads_as_a_sequence_of_records_and_serializes_everywhere() {
        let value = arrow::batch_to_value(&batch()).unwrap();
        let rows = value.as_sequence().expect("a sequence of rows");
        assert_eq!(rows.len(), 2);
        let (data_type, first) = rows[0].as_record().expect("a record row");
        assert_eq!(data_type.field_len(), 2);
        assert_eq!(first[0], Value::I64(1));
        // A null slot is the null value, exactly as the value model spells it.
        assert_eq!(rows[1].as_record().unwrap().1[1], Value::Null);

        let json = String::from_utf8(yggdryl::json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            json,
            "[{\"id\":1,\"venue\":\"XNAS\"},{\"id\":2,\"venue\":null}]"
        );
        // YAML and TOML take the same value through their ordinary entry
        // points; TOML has no top-level array, so the rows wrap in a table.
        assert!(yggdryl::yaml::to_vec(&value).is_ok());
        let document = Value::from_mapping([(Value::from("rows"), value)]).unwrap();
        assert!(yggdryl::toml::to_vec(&document).is_ok());
    }

    #[test]
    fn an_array_reads_as_the_sequence_its_rows_spell() {
        let field = DataType::Int64.nullable_field("id");
        let array = Int64Array::from(vec![Some(5_i64), None]);
        let value = arrow::array_to_value(&field, &array).unwrap();
        assert_eq!(value, Value::from_sequence([Value::I64(5), Value::Null]));
    }
}

#[test]
fn the_structural_wire_round_trips_a_record_with_its_type() {
    // The typed pairing serializes values through serde; a record must come
    // back a record, type and all, through the same wire.
    let record = trade(9, None);
    let encoded = serde_json::to_string(&record).unwrap();
    assert!(encoded.contains("\"type\":\"record\""), "wire: {encoded}");
    let decoded: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, record);
}
