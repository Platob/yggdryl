//! An ASCII width crosses the exchange formats as its trimmed text.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray};
use yggdryl::arrow::batch_reader;
use yggdryl::generic::RecordOptions;
use yggdryl::io::{Buffer, IOBase, IOMedia};
use yggdryl::{DataType, Expression, Field, Scalar, TypedScalar, Url};

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
        DataType::Ascii32.required_field("ccy"),
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
        DataType::Ascii32.required_field("a"),
        DataType::Ascii32.required_field("b"),
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
    let schema = root([DataType::Ascii32.required_field("ccy")]);
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

    let bound = "cast(ccy as ascii32) = 'USD'"
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
    let refused = "cast('EURO!' as ascii32) = ccy"
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
    let parsed = "ccy = ascii32 'USD'".parse::<Expression>().unwrap();
    let Expression::Compare(_, _, literal) = &parsed else {
        panic!("a comparison, got {parsed}");
    };
    assert_eq!(
        **literal,
        Expression::Literal(
            TypedScalar::from_parts(DataType::Ascii32, Scalar::from("USD")).unwrap()
        )
    );
    // The literal prints in its own width and re-parses; a registered name
    // spells the literal of the width it registers over, which for ISO 4217's
    // three letters is `ascii24`.
    assert_eq!(parsed.to_string(), "ccy = ascii32 'USD'");
    assert_eq!(parsed.to_string().parse::<Expression>().unwrap(), parsed);
    assert_eq!(
        "ccy = currency 'USD'".parse::<Expression>().unwrap(),
        "ccy = ascii24 'USD'".parse::<Expression>().unwrap()
    );

    let message = "ccy = ascii32 'EURO!'"
        .parse::<Expression>()
        .unwrap_err()
        .to_string();
    assert!(message.contains("at most 4 bytes"), "{message}");
}

#[test]
fn an_ascii_column_round_trips_through_avro_as_text() {
    let schema = root([DataType::Ascii32.required_field("ccy")]);
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
    let mut schema = root([DataType::Ascii32.required_field("ccy")]);
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

/// The dictionary array an `AsciiDictionary` encodes, and the vocabulary it
/// reads back.
mod dictionary {
    use std::sync::Arc;

    use arrow_array::types::{Int32Type, Int64Type};
    use arrow_array::{Array, DictionaryArray, FixedSizeBinaryArray, Int32Array, StringArray};
    use yggdryl::{AsciiDictionary, DataType, Scalar};

    fn encoded(array: &dyn Array) -> &DictionaryArray<Int32Type> {
        array
            .as_any()
            .downcast_ref::<DictionaryArray<Int32Type>>()
            .expect("an int32-keyed dictionary")
    }

    fn vocabulary(array: &DictionaryArray<Int32Type>) -> &FixedSizeBinaryArray {
        array
            .values()
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .expect("padded ASCII storage")
    }

    #[test]
    fn a_second_column_continues_the_codes_of_the_first() {
        let mut currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        let first = currencies
            .into_arrow_array([Some("USD"), Some("EUR"), Some("USD")])
            .unwrap();
        let first = encoded(first.as_ref());
        assert_eq!(first.keys().values(), &[0, 1, 0]);
        assert_eq!(vocabulary(first).len(), 2);

        // The vocabulary grows to the union: a known value keeps its code and
        // an unseen one takes the next.
        let second = currencies
            .into_arrow_array([Some("JPY"), Some("EUR"), Some("GBP")])
            .unwrap();
        let second = encoded(second.as_ref());
        assert_eq!(second.keys().values(), &[2, 1, 3]);
        assert_eq!(currencies.as_values(), ["USD", "EUR", "JPY", "GBP"]);
        assert_eq!(vocabulary(second).len(), 4);

        // The first array keeps the vocabulary it was built with.
        assert_eq!(vocabulary(first).len(), 2);
    }

    #[test]
    fn a_refused_column_registers_nothing() {
        let mut currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        currencies.into_arrow_array([Some("USD")]).unwrap();

        // The mutation fails atomically, so the values registered before the
        // refusal are gone and the next code is the one it was.
        let refused = currencies
            .into_arrow_array([Some("EUR"), Some("JPY"), Some("EURO!")])
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        assert_eq!(currencies.as_values(), ["USD"]);
        assert_eq!(currencies.get_code("EUR"), None);
        assert_eq!(currencies.push("JPY").unwrap(), 1);
    }

    #[test]
    fn a_repeated_vocabulary_value_is_refused_naming_both_positions() {
        // Arrow does not require a dictionary's values to be unique, and a
        // repeat would give one code away and shift every later one.
        for vocabulary in [
            vec![&b"USD\0"[..], &b"USD\0"[..], &b"EUR\0"[..]],
            vec![&b"AB\0\0"[..], &b"AB\0\0"[..], &b"CD\0\0"[..]],
        ] {
            let values = FixedSizeBinaryArray::try_from_iter(vocabulary.into_iter()).unwrap();
            let array = DictionaryArray::<Int32Type>::try_new(
                Int32Array::from(vec![2, 0]),
                Arc::new(values),
            )
            .unwrap();
            let refused = AsciiDictionary::from_arrow_array(&array)
                .unwrap_err()
                .to_string();
            assert!(
                refused.contains("a vocabulary with no repeated value"),
                "{refused}"
            );
            assert!(refused.contains("at 0 and 1"), "{refused}");
        }
    }

    #[test]
    fn a_null_vocabulary_slot_is_refused_naming_its_position() {
        let values = FixedSizeBinaryArray::try_from_sparse_iter_with_size(
            [Some(b"USD\0".as_slice()), None].into_iter(),
            4,
        )
        .unwrap();
        let array =
            DictionaryArray::<Int32Type>::try_new(Int32Array::from(vec![0]), Arc::new(values))
                .unwrap();
        let refused = AsciiDictionary::from_arrow_array(&array)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a null vocabulary value at 1"),
            "{refused}"
        );
    }

    #[test]
    fn a_null_is_a_null_key_and_the_values_are_padded_storage() {
        let mut currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        let array = currencies
            .into_arrow_array([Some("USD"), None, Some("EU"), Some("USD")])
            .unwrap();
        let array = encoded(array.as_ref());
        assert_eq!(array.len(), 4);
        assert_eq!(
            array.keys().iter().collect::<Vec<_>>(),
            [Some(0), None, Some(1), Some(0)]
        );
        assert_eq!(array.null_count(), 1);

        // Storage pads to the width, and the scalar boundary reads it trimmed.
        let stored = vocabulary(array);
        assert_eq!(stored.value(0), b"USD\0");
        assert_eq!(stored.value(1), b"EU\0\0");
        let field = DataType::Ascii32.required_field("ccy");
        assert_eq!(
            yggdryl::arrow::array_to_value(&field, stored).unwrap(),
            Scalar::from_sequence([Scalar::from("USD"), Scalar::from("EU")])
        );
    }

    #[test]
    fn an_int64_key_encodes_and_reads_back_under_its_own_key_type() {
        let mut wide = AsciiDictionary::new(DataType::Ascii64)
            .unwrap()
            .with_key(DataType::Int64)
            .unwrap();
        let array = wide
            .into_arrow_array([Some("SETTLED"), None, Some("PENDING"), Some("SETTLED")])
            .unwrap();
        let keyed = array
            .as_any()
            .downcast_ref::<DictionaryArray<Int64Type>>()
            .expect("an int64-keyed dictionary");
        assert_eq!(
            keyed.keys().iter().collect::<Vec<_>>(),
            [Some(0), None, Some(1), Some(0)]
        );

        let read = AsciiDictionary::from_arrow_array(array.as_ref()).unwrap();
        assert_eq!(read, wide);
        assert_eq!(read.key(), &DataType::Int64);
        assert_eq!(read.values_dtype(), &DataType::Ascii64);
    }

    #[test]
    fn the_vocabulary_round_trips_through_the_arrow_array() {
        let mut currencies = AsciiDictionary::new(DataType::Ascii32).unwrap();
        let array = currencies
            .into_arrow_array([Some("USD"), Some("EUR"), None, Some("JPY")])
            .unwrap();
        let read = AsciiDictionary::from_arrow_array(array.as_ref()).unwrap();
        assert_eq!(read, currencies);
        assert_eq!(read.as_values(), ["USD", "EUR", "JPY"]);
        assert_eq!(read.get_code("JPY"), Some(2));
        assert_eq!(read.get(0), Some("USD"));

        // A wider vocabulary keeps its own width.
        let mut wide = AsciiDictionary::new(DataType::Ascii128).unwrap();
        let array = wide.into_arrow_array([Some("SIXTEEN-BYTE-ID")]).unwrap();
        let read = AsciiDictionary::from_arrow_array(array.as_ref()).unwrap();
        assert_eq!(read.values_dtype(), &DataType::Ascii128);
        assert_eq!(read.get(0), Some("SIXTEEN-BYTE-ID"));
    }

    #[test]
    fn another_layout_is_refused_by_name() {
        let text = StringArray::from(vec!["USD", "EUR"]);
        let refused = AsciiDictionary::from_arrow_array(&text)
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("a dictionary array of int32 or int64 keys over an ASCII width"),
            "{refused}"
        );
        assert!(refused.contains("got Utf8"), "{refused}");

        // A dictionary whose values are not an ASCII width is refused too.
        let utf8: DictionaryArray<Int32Type> = vec!["USD", "EUR", "USD"].into_iter().collect();
        let refused = AsciiDictionary::from_arrow_array(&utf8)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("over an ASCII width"), "{refused}");
        assert!(refused.contains("Dictionary"), "{refused}");
    }
}
