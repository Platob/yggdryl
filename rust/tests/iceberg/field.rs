//! `rust/src/iceberg/field.rs`: the `ICEBERG:` vocabulary a column carries.
//!
//! The `unknown` declaration is the one property that changes what a column
//! is to a table, so its refusal and its clearing are pinned here; the reads
//! the rest of the vocabulary shares are pinned with the documents they come
//! from.

use yggdryl::DataType;

#[test]
fn unknown_is_declared_only_on_a_variant_column_and_clears_by_name() {
    let mut later = DataType::Variant.nullable_field("later");
    later.as_iceberg_mut().set_unknown(true).unwrap();
    assert!(later.as_iceberg().is_unknown());
    assert_eq!(later.get_metadata("ICEBERG:type"), Some("unknown"));

    later.as_iceberg_mut().set_unknown(false).unwrap();
    assert!(!later.as_iceberg().is_unknown());
    assert_eq!(later.get_metadata("ICEBERG:type"), None);
    // Clearing what is not declared changes nothing and does not fail.
    later.as_iceberg_mut().set_unknown(false).unwrap();
    assert_eq!(later, DataType::Variant.nullable_field("later"));

    // Any other column refuses the declaration, naming the key, the datatype
    // and the column, and is left as it was.
    let mut amount = DataType::Int64.nullable_field("amount");
    let message = amount
        .as_iceberg_mut()
        .set_unknown(true)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("ICEBERG:type") && message.contains("int64") && message.contains("amount"),
        "{message}"
    );
    assert_eq!(amount, DataType::Int64.nullable_field("amount"));
}

#[test]
fn only_the_word_unknown_declares_a_column_unknown() {
    let mut payload = DataType::Variant.nullable_field("payload");
    payload.as_iceberg_mut().insert("type", "variant").unwrap();
    assert!(!payload.as_iceberg().is_unknown());
}
