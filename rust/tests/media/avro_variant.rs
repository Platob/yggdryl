//! Resolved Avro Variant records retain their two raw buffers.

use yggdryl::IOBase;
use yggdryl::avro::{self, Schema};
use yggdryl::holder::Buffer;
use yggdryl::{MediaType, MimeType, Scalar, Variant};

fn buffer() -> Buffer {
    let mut buffer = Buffer::new();
    buffer.set_media_type(MediaType::new(MimeType::AVRO));
    buffer
}

fn variant() -> Variant {
    Variant::encode(
        &Scalar::from_struct([
            ("name", Scalar::from("Ada")),
            ("active", Scalar::from(true)),
        ])
        .unwrap(),
    )
    .unwrap()
}

fn variant_record(annotation: bool) -> String {
    let annotation = annotation
        .then_some(",\"logicalType\":\"variant\"")
        .unwrap_or("");
    format!(
        r#"{{"type":"record","name":"variant_value"{annotation},"fields":[{{"name":"metadata","type":"bytes"}},{{"name":"value","type":"bytes"}}]}}"#
    )
}

fn schema_with_one_variant(annotation: bool) -> String {
    format!(
        r#"{{"type":"record","name":"row","fields":[{{"name":"v","type":{}}}]}}"#,
        variant_record(annotation),
    )
}

fn resolved(writer: &str, reader: &str, rows: &[Scalar]) -> Vec<Scalar> {
    let writer_json = yggdryl::json::from_utf8(writer).unwrap();
    let reader = reader.parse::<Schema>().unwrap();
    let mut handle = buffer();
    avro::write_container(&mut handle, &writer_json, &[], rows).unwrap();
    avro::read_container_resolved(&handle, &reader)
        .unwrap()
        .rows
}

#[test]
fn an_annotated_reader_resolves_a_variant_with_its_exact_buffers() {
    let held = variant();
    let rows = resolved(
        &schema_with_one_variant(true),
        &schema_with_one_variant(true),
        &[Scalar::from_struct([("v", Scalar::Variant(held.clone()))]).unwrap()],
    );
    assert_eq!(
        rows[0].get_key_str("v"),
        Some(&Scalar::Variant(held)),
        "the reader annotation owns Variant interpretation"
    );
}

#[test]
fn an_unannotated_reader_keeps_a_variant_record_as_an_ordinary_struct() {
    let held = variant();
    let rows = resolved(
        &schema_with_one_variant(true),
        &schema_with_one_variant(false),
        &[Scalar::from_struct([("v", Scalar::Variant(held.clone()))]).unwrap()],
    );
    let record = rows[0]
        .get_key_str("v")
        .and_then(Scalar::as_struct)
        .unwrap();
    assert_eq!(
        record.get("metadata").and_then(Scalar::as_bytes),
        Some(held.metadata())
    );
    assert_eq!(
        record.get("value").and_then(Scalar::as_bytes),
        Some(held.value())
    );
}

#[test]
fn a_variant_through_a_named_reference_and_union_resolves_as_a_variant() {
    let held = variant();
    let schema = format!(
        r#"{{"type":"record","name":"row","fields":[
            {{"name":"first","type":{}}},
            {{"name":"by_ref","type":"variant_value"}},
            {{"name":"by_union","type":["null","variant_value"],"default":null}}
        ]}}"#,
        variant_record(true),
    );
    let row = Scalar::from_struct([
        ("first", Scalar::Variant(held.clone())),
        ("by_ref", Scalar::Variant(held.clone())),
        ("by_union", Scalar::Variant(held.clone())),
    ])
    .unwrap();
    let rows = resolved(&schema, &schema, &[row]);
    for name in ["first", "by_ref", "by_union"] {
        assert_eq!(
            rows[0].get_key_str(name),
            Some(&Scalar::Variant(held.clone())),
            "{name}"
        );
    }
}

#[test]
fn a_missing_annotated_variant_field_uses_its_variant_default() {
    let held = variant();
    let byte_string = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("\\u{byte:04x}"))
            .collect::<String>()
    };
    let writer = r#"{"type":"record","name":"row","fields":[{"name":"id","type":"long"}]}"#;
    let reader = format!(
        r#"{{"type":"record","name":"row","fields":[
            {{"name":"id","type":"long"}},
            {{"name":"v","type":{},"default":{{"metadata":"{}","value":"{}"}}}}
        ]}}"#,
        variant_record(true),
        byte_string(held.metadata()),
        byte_string(held.value()),
    );
    let rows = resolved(
        writer,
        &reader,
        &[Scalar::from_struct([("id", Scalar::from(7_i64))]).unwrap()],
    );
    assert_eq!(rows[0].get_key_str("v"), Some(&Scalar::Variant(held)));
}
