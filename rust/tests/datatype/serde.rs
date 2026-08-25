use yggdryl::DataType;

#[test]
fn structural_json_uses_tagged_objects_and_rejects_bad_shapes() {
    let data_type = DataType::from_str("struct<id:bigint,tags:array<string>>").unwrap();
    let data_type_json = data_type.clone().into_json().unwrap();
    let data_type_value: serde_json::Value = serde_json::from_str(&data_type_json).unwrap();
    assert_eq!(data_type_value["type"], "struct");
    assert!(data_type_value["fields"].is_array());
    assert_eq!(data_type_value["fields"][0]["data_type"]["type"], "int64");
    let serde_json = serde_json::to_string(&data_type).unwrap();
    assert_eq!(
        serde_json::from_str::<DataType>(&serde_json).unwrap(),
        data_type
    );

    for malformed_or_invalid in [
        "{",
        r#"{"type":"not_a_datatype"}"#,
        r#"{"type":"int64","unknown":true}"#,
        r#"{"type":"timestamp","unit":"year_month"}"#,
        r#"{"type":"interval","unit":"second"}"#,
        r#"{"type":"decimal128","precision":0,"scale":0}"#,
        r#"{"type":"fixed_size_binary","width":-1}"#,
    ] {
        assert!(
            DataType::from_json(malformed_or_invalid).is_err(),
            "{malformed_or_invalid}"
        );
    }
}
