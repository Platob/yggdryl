//! `rust/src/avro/arrow.rs`: the schema projection no caller can reach.
//!
//! `schema_json_from_field` is the crate-private rendering of a `Field` as an
//! Avro schema document, which every container write goes through. Everything
//! a caller can observe lives in `tests/media/avro.rs`.

use yggdryl::DataType;
use yggdryl::StructType;
use yggdryl::internals::avro_arrow::schema_json_from_field;

#[test]
fn an_ascii_column_is_an_avro_string() {
    let root = StructType::from_fields([
        DataType::fixed_ascii(4).unwrap().required_field("ccy"),
        DataType::fixed_ascii(16).unwrap().nullable_field("code"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let schema = schema_json_from_field(&root).unwrap();
    let fields = schema
        .get_key_str("fields")
        .and_then(yggdryl::Scalar::as_sequence)
        .unwrap();
    assert_eq!(
        fields[0]
            .get_key_str("type")
            .and_then(yggdryl::Scalar::as_str),
        Some("string")
    );
    let optional = fields[1]
        .get_key_str("type")
        .and_then(yggdryl::Scalar::as_sequence)
        .unwrap();
    assert!(
        optional
            .iter()
            .any(|branch| branch.as_str() == Some("string")),
        "{optional:?}"
    );
}

#[test]
fn a_string_in_another_charset_is_not_an_avro_string() {
    // Avro's string is UTF-8; bytes in another charset are not, so the
    // column is refused by name rather than written as mojibake.
    let latin = DataType::cp1252();
    let root = StructType::from_fields([latin.required_field("label")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let message = schema_json_from_field(&root).unwrap_err().to_string();
    assert!(
        message.contains("expected a datatype Avro can spell"),
        "{message}"
    );
    assert!(message.contains("cp1252"), "{message}");
}

#[test]
fn a_code_column_is_an_avro_string_and_a_code_key_is_spellable() {
    // Avro has no fixed-width text, so a code spells `string` with no
    // logical type - the contrast with a UUID, which annotates `uuid`.
    // Every registered code, read from the one listing: this used to name
    // four of them, and `side`, `state` and `timeinforce` were refused as
    // unspellable by a column spelling that had drifted
    // behind the family.
    let mut fields: Vec<_> = DataType::CODES
        .iter()
        .map(|(name, dtype, _)| dtype.clone().required_field(*name))
        .collect();
    // A map key gate that nothing else in the tree exercises for a
    // non-Utf8 key.
    fields.push(
        DataType::map_of(DataType::MicCode, DataType::Int64, true)
            .unwrap()
            .required_field("by_venue"),
    );
    let codes = DataType::CODES.len();
    let root = DataType::from(StructType::from_fields(fields).unwrap()).required_field("row");
    let schema = schema_json_from_field(&root).unwrap();
    let fields = schema
        .get_key_str("fields")
        .and_then(yggdryl::Scalar::as_sequence)
        .unwrap();

    for ((name, ..), field) in DataType::CODES.iter().zip(fields.iter()) {
        assert_eq!(
            field.get_key_str("type").and_then(yggdryl::Scalar::as_str),
            Some("string"),
            "{name}"
        );
    }
    assert_eq!(
        fields[codes]
            .get_key_str("type")
            .and_then(|value| value.get_key_str("type"))
            .and_then(|value| value.as_str().map(str::to_owned)),
        Some("map".to_owned())
    );
}
