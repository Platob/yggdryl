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

#[test]
fn an_identifier_reads_as_every_integer_reads_and_a_refusal_names_its_key() {
    let mut root = DataType::Int64.required_field("root");
    for (name, text) in [
        ("schema-id", " 3 "),
        ("spec-id", "+4"),
        ("partition-source-id", "-5"),
    ] {
        root.as_iceberg_mut().insert(name, text).unwrap();
    }
    assert_eq!(root.as_iceberg().schema_id().unwrap(), Some(3));
    assert_eq!(root.as_iceberg().spec_id().unwrap(), Some(4));
    assert_eq!(root.as_iceberg().partition_source_id().unwrap(), Some(-5));

    // Text no integer spells, and one past thirty-two bits, is refused naming
    // the key and the text.
    for text in ["1.0", "three", "2147483648"] {
        root.as_iceberg_mut().insert("schema-id", text).unwrap();
        let message = root.as_iceberg().schema_id().unwrap_err().to_string();
        assert!(message.contains("schema-id"), "{text}: {message}");
        assert!(message.contains(text), "{text}: {message}");
    }
}

#[test]
fn the_identifier_columns_read_each_member_as_an_integer_and_skip_the_blanks() {
    let mut root = DataType::Int64.required_field("root");
    assert!(root.as_iceberg().identifier_field_ids().unwrap().is_empty());
    root.as_iceberg_mut()
        .insert("identifier-field-ids", " 1 , +2,,3 ")
        .unwrap();
    assert_eq!(root.as_iceberg().identifier_field_ids().unwrap(), [1, 2, 3]);

    for text in ["1,x", "1.5", "2147483648"] {
        root.as_iceberg_mut()
            .insert("identifier-field-ids", text)
            .unwrap();
        let message = root
            .as_iceberg()
            .identifier_field_ids()
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("identifier-field-ids"),
            "{text}: {message}"
        );
        assert!(message.contains(text), "{text}: {message}");
    }
}

/// A decimal default travels in the spec's single-value form, its fraction
/// digits stating the column's scale - the one decimal text written at its
/// scale, because a reader that checks it, as Java's does, refuses any
/// other - and reads back as the value it was.
#[test]
fn a_decimal_default_states_the_column_scale_in_its_text() {
    use yggdryl::{Decimal, Scalar};

    let mut price = DataType::decimal128(10, 2).unwrap().nullable_field("price");
    price
        .as_iceberg_mut()
        .set_initial_default(&Scalar::decimal128(15, 1))
        .unwrap();
    price
        .as_iceberg_mut()
        .set_write_default(&Scalar::decimal128(7, 0))
        .unwrap();
    assert_eq!(
        price.get_metadata("ICEBERG:initial-default"),
        Some("\"1.50\"")
    );
    assert_eq!(
        price.get_metadata("ICEBERG:write-default"),
        Some("\"7.00\"")
    );
    let default = price.as_iceberg().initial_default().unwrap().unwrap();
    assert_eq!(price.scalar(default).unwrap(), Scalar::decimal128(150, 2));

    // The fixed leaf rides `decimal(38, 18)` and states all eighteen places.
    let mut fixed = DataType::Decimal.nullable_field("px");
    let half = Scalar::Decimal("0.5".parse::<Decimal>().unwrap());
    fixed.as_iceberg_mut().set_initial_default(&half).unwrap();
    assert_eq!(
        fixed.get_metadata("ICEBERG:initial-default"),
        Some("\"0.500000000000000000\"")
    );

    // A value the column cannot hold is refused, and leaves it as it was.
    let mut narrow = DataType::decimal128(4, 2).unwrap().nullable_field("narrow");
    assert!(
        narrow
            .as_iceberg_mut()
            .set_initial_default(&Scalar::decimal128(1_005, 3))
            .is_err()
    );
    assert_eq!(narrow.get_metadata("ICEBERG:initial-default"), None);
}
