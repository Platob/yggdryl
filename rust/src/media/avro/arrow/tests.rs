//! The Avro schema projection an integration test cannot reach.
//!
//! `schema_json_from_field` is the crate-private rendering of a `Field` as an
//! Avro schema document, which every container write goes through. Everything
//! a caller can observe lives in `tests/media/avro.rs`.

use crate::DataType;

#[test]
fn an_ascii_column_is_an_avro_string() {
    let root = DataType::from_fields([
        DataType::fixed_ascii(4).unwrap().required_field("ccy"),
        DataType::fixed_ascii(16).unwrap().nullable_field("code"),
    ])
    .unwrap()
    .required_field("row");
    let schema = super::schema_json_from_field(&root).unwrap();
    let fields = schema
        .get_key_str("fields")
        .and_then(crate::Scalar::as_sequence)
        .unwrap();
    assert_eq!(
        fields[0]
            .get_key_str("type")
            .and_then(crate::Scalar::as_str),
        Some("string")
    );
    let optional = fields[1]
        .get_key_str("type")
        .and_then(crate::Scalar::as_sequence)
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
    let latin = DataType::string(crate::Charset::Cp1252).unwrap();
    let root = DataType::from_fields([latin.required_field("label")])
        .unwrap()
        .required_field("row");
    let message = super::schema_json_from_field(&root)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("expected a datatype Avro can spell"),
        "{message}"
    );
    assert!(message.contains("string(windows-1252)"), "{message}");
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
        DataType::map_of(DataType::Mic, DataType::Int64, true)
            .unwrap()
            .required_field("by_venue"),
    );
    let codes = DataType::CODES.len();
    let root = DataType::from_fields(fields).unwrap().required_field("row");
    let schema = super::schema_json_from_field(&root).unwrap();
    let fields = schema
        .get_key_str("fields")
        .and_then(crate::Scalar::as_sequence)
        .unwrap();

    for ((name, ..), field) in DataType::CODES.iter().zip(fields.iter()) {
        assert_eq!(
            field.get_key_str("type").and_then(crate::Scalar::as_str),
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
