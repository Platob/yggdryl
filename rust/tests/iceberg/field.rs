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
