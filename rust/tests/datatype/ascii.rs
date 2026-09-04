//! An ASCII width crosses the exchange formats as its trimmed text.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray};
use yggdryl::arrow::batch_reader;
use yggdryl::generic::RecordOptions;
use yggdryl::holder::Buffer;
use yggdryl::{DataType, Expression, Field, Scalar, TypedScalar, Url};
use yggdryl::{IOBase, IOMedia};

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new("row", DataType::from_fields(fields).unwrap(), false)
}

/// Four-byte storage: the padded codes the writer would have stored.
fn currencies(codes: &[&[u8; 4]]) -> ArrayRef {
    Arc::new(
        FixedSizeBinaryArray::try_from_sparse_iter_with_size(
            codes.iter().map(|code| Some(code.as_slice())),
            4,
        )
        .unwrap(),
    )
}

#[test]
fn a_filter_over_an_ascii_column_binds_and_evaluates() {
    let schema = root([
        DataType::FixedAscii(4).required_field("ccy"),
        DataType::Int64.required_field("qty"),
    ]);
    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema().unwrap(),
        vec![
            currencies(&[b"USD\0", b"EUR\0", b"USD\0"]),
            Arc::new(Int64Array::from(vec![1, 2, 3])),
        ],
    )
    .unwrap();

    // The column meets the literal at utf8, and the cast trims the padding.
    let bound = "ccy = 'USD'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let kept = bound.filter(&batch).unwrap();
    assert_eq!(kept.num_rows(), 2);
    let quantities = kept
        .column(1)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(quantities.values(), &[1, 3]);

    // The row tier reads the same trimmed text.
    let row = Scalar::from_sequence([Scalar::from("USD"), Scalar::from(1_i64)]);
    assert!(bound.matches(&row).unwrap());
    let row = Scalar::from_sequence([Scalar::from("EUR"), Scalar::from(2_i64)]);
    assert!(!bound.matches(&row).unwrap());
}

#[test]
fn two_ascii_columns_compare_at_both_tiers() {
    let schema = root([
        DataType::FixedAscii(4).required_field("a"),
        DataType::FixedAscii(4).required_field("b"),
    ]);
    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema().unwrap(),
        vec![
            currencies(&[b"USD\0", b"EUR\0"]),
            currencies(&[b"USD\0", b"USD\0"]),
        ],
    )
    .unwrap();

    // Two ASCII operands meet at their own width, so the row tier compares
    // the trimmed text the same way the column tier compares storage.
    let equal = "a = b"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(equal.filter(&batch).unwrap().num_rows(), 1);
    assert!(
        equal
            .matches(&Scalar::from_sequence([
                Scalar::from("USD"),
                Scalar::from("USD")
            ]))
            .unwrap()
    );
    assert!(
        !equal
            .matches(&Scalar::from_sequence([
                Scalar::from("USD"),
                Scalar::from("EUR")
            ]))
            .unwrap()
    );
    let before = "a < b"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(before.filter(&batch).unwrap().num_rows(), 1);
    assert!(
        before
            .matches(&Scalar::from_sequence([
                Scalar::from("EUR"),
                Scalar::from("USD")
            ]))
            .unwrap()
    );
}

#[test]
fn string_functions_read_an_ascii_column_as_text() {
    let schema = root([DataType::FixedAscii(4).required_field("ccy")]);
    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema().unwrap(),
        vec![currencies(&[b"USD\0", b"EUR\0"])],
    )
    .unwrap();
    let usd = Scalar::from_sequence([Scalar::from("USD")]);

    for (text, kept) in [
        ("upper(ccy) = 'USD'", 1),
        ("lower(ccy) = 'usd'", 1),
        ("length(ccy) = 3", 2),
        ("starts_with(ccy, 'U')", 1),
        ("concat(ccy, 'X') = 'USDX'", 1),
    ] {
        let bound = text.parse::<Expression>().unwrap().bind(&schema).unwrap();
        assert_eq!(bound.filter(&batch).unwrap().num_rows(), kept, "{text}");
        assert!(bound.matches(&usd).unwrap(), "{text}");
    }
}

#[test]
fn a_cast_to_an_ascii_width_obeys_the_width_rule_on_rows() {
    let schema = root([DataType::Utf8.required_field("ccy")]);
    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema().unwrap(),
        vec![Arc::new(StringArray::from(vec!["USD", "EUR"]))],
    )
    .unwrap();

    let bound = "cast(ccy as ascii(4)) = 'USD'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(bound.filter(&batch).unwrap().num_rows(), 1);
    assert!(
        bound
            .matches(&Scalar::from_sequence([Scalar::from("USD")]))
            .unwrap()
    );

    // The row tier refuses what the column tier refuses, naming the width.
    let message = bound
        .matches(&Scalar::from_sequence([Scalar::from("EURO!")]))
        .unwrap_err()
        .to_string();
    assert!(message.contains("at most 4 bytes"), "{message}");
    let refused = "cast('EURO!' as ascii(4)) = ccy"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let message = refused.filter(&batch).unwrap_err().to_string();
    assert!(message.contains("at most 4 bytes"), "{message}");
    let message = refused
        .matches(&Scalar::from_sequence([Scalar::from("USD")]))
        .unwrap_err()
        .to_string();
    assert!(message.contains("at most 4 bytes"), "{message}");
}

#[test]
fn an_ascii_literal_has_a_text_form() {
    let parsed = "ccy = ascii(4) 'USD'".parse::<Expression>().unwrap();
    let Expression::Compare(_, _, literal) = &parsed else {
        panic!("a comparison, got {parsed}");
    };
    assert_eq!(
        **literal,
        Expression::Literal(
            TypedScalar::from_parts(DataType::FixedAscii(4), Scalar::from("USD")).unwrap()
        )
    );
    // The literal prints in its own datatype and re-parses; a registered code
    // spells a literal of its own, which is not the literal of the width that
    // happens to hold the same bytes.
    assert_eq!(parsed.to_string(), "ccy = ascii(4) 'USD'");
    assert_eq!(parsed.to_string().parse::<Expression>().unwrap(), parsed);
    let currency = "ccy = currency 'USD'".parse::<Expression>().unwrap();
    assert_eq!(currency.to_string(), "ccy = currency 'USD'");
    assert_eq!(
        currency.to_string().parse::<Expression>().unwrap(),
        currency
    );
    assert_ne!(
        currency,
        "ccy = ascii(3) 'USD'".parse::<Expression>().unwrap()
    );
    let refused = "ccy = country 'USD'"
        .parse::<Expression>()
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 2 bytes"), "{refused}");

    let message = "ccy = ascii(4) 'EURO!'"
        .parse::<Expression>()
        .unwrap_err()
        .to_string();
    assert!(message.contains("at most 4 bytes"), "{message}");
}

#[test]
fn an_ascii_column_round_trips_through_avro_as_text() {
    let schema = root([DataType::FixedAscii(4).required_field("ccy")]);
    let batch = RecordBatch::try_new(
        schema.into_arrow_schema().unwrap(),
        vec![currencies(&[b"USD\0", b"EU\0\0"])],
    )
    .unwrap();
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///ccy.avro").unwrap().media_type());
    let options = RecordOptions::for_media_type(handle.media_type()).unwrap();
    handle
        .overwrite_arrow_reader(batch_reader(batch.schema(), [batch]), &options)
        .unwrap();

    // Avro has no fixed-width text, so the column is a string and every
    // reader sees the trimmed code rather than the padded storage.
    let stored = handle.read_arrow_field(&options).unwrap();
    assert_eq!(stored.fields()[0].dtype(), &DataType::Utf8);
    let read: Vec<RecordBatch> = handle
        .read_arrow_reader(&options)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let ccy = read[0]
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(ccy.value(0), "USD");
    assert_eq!(ccy.value(1), "EU");
}

#[cfg(feature = "iceberg")]
#[test]
fn an_ascii_column_is_an_iceberg_string() {
    let mut schema = root([DataType::FixedAscii(4).required_field("ccy")]);
    yggdryl::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let json = yggdryl::iceberg::schema_into_json(&schema).unwrap();
    let fields = json
        .get_key_str("fields")
        .and_then(Scalar::as_sequence)
        .unwrap();
    assert_eq!(
        fields[0].get_key_str("type").and_then(Scalar::as_str),
        Some("string")
    );
}
