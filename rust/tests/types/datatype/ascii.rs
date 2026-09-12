//! US-ASCII is a charset of the one string family: `ascii` is a string whose
//! bytes are US-ASCII, `ascii(n)` bounds it, `fixed_ascii(n)` pads it, and
//! every one of them is `Scalar::String` in memory and Arrow text on the wire.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use yggdryl::arrow::{batch_reader, scalar_array, scalar_value};
use yggdryl::expression::Literal;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::types::{Str, StringLayout, StringParameters};
use yggdryl::{Charset, DataType, DataTypeId, Expression, Field, Scalar, StringEnum, Url};
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
        DataType::fixed_ascii(4).unwrap().required_field("ccy"),
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
        DataType::fixed_ascii(4).unwrap().required_field("a"),
        DataType::fixed_ascii(4).unwrap().required_field("b"),
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
    let schema = root([DataType::fixed_ascii(4).unwrap().required_field("ccy")]);
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
fn a_cast_to_a_bounded_ascii_obeys_the_bound_on_rows() {
    let schema = root([DataType::utf8().required_field("ccy")]);
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

    // The row tier refuses what the column tier refuses, naming the bound.
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
fn a_cast_into_a_securities_number_holds_the_column_to_the_canonical_spelling() {
    let schema = root([DataType::utf8().required_field("sid")]);
    let batch = |values: Vec<&str>| {
        RecordBatch::try_new(
            schema.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(values))],
        )
        .unwrap()
    };
    let bound = "cast(sid as isin) = 'US0378331005'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    // Two numbers closed by their check digits pass, and one matches.
    assert_eq!(
        bound
            .filter(&batch(vec!["US0378331005", "CH0012221716"]))
            .unwrap()
            .num_rows(),
        1
    );
    // The column tier refuses a number its check digit does not close, and
    // one spelled in lower case: a column's bytes are what every reader
    // digests, so a cast lets in the canonical spelling and nothing else.
    for column in [vec!["US0378331005", "US0378331006"], vec!["us0378331005"]] {
        let message = bound.filter(&batch(column)).unwrap_err().to_string();
        assert!(message.contains("canonical spelling"), "{message}");
    }
    // The row tier reads a value as the scalar does, folding the case.
    assert!(
        bound
            .matches(&Scalar::from_sequence([Scalar::from("us0378331005")]))
            .unwrap()
    );
    let message = bound
        .matches(&Scalar::from_sequence([Scalar::from("US0378331006")]))
        .unwrap_err()
        .to_string();
    assert!(message.contains("check digit"), "{message}");
}

#[test]
fn an_ascii_literal_has_a_text_form() {
    let parsed = "ccy = ascii(4) 'USD'".parse::<Expression>().unwrap();
    let Expression::Compare(_, _, literal) = &parsed else {
        panic!("a comparison, got {parsed}");
    };
    // `ascii(4)` is the bounded string, not the padded one.
    assert_eq!(
        **literal,
        Expression::Literal(Literal::new(DataType::from_str("ascii(4)").unwrap(), "USD").unwrap())
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
    let schema = root([DataType::fixed_ascii(4).unwrap().required_field("ccy")]);
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
    assert_eq!(stored.fields()[0].dtype(), &DataType::utf8());
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
    let mut schema = root([DataType::fixed_ascii(4).unwrap().required_field("ccy")]);
    yggdryl::media::iceberg::assign_field_ids(&mut schema, 1).unwrap();
    let json = yggdryl::media::iceberg::schema_into_json(&schema).unwrap();
    let fields = json
        .get_key_str("fields")
        .and_then(Scalar::as_sequence)
        .unwrap();
    assert_eq!(
        fields[0].get_key_str("type").and_then(Scalar::as_str),
        Some("string")
    );
}

#[test]
fn ascii_is_one_charset_of_the_string_family() {
    // The three shapes are one datatype under one charset, and each reads
    // back from its own spelling.
    let plain = DataType::ascii();
    assert_eq!(plain.to_string(), "ascii");
    assert_eq!(DataType::from_str("ascii").unwrap(), plain);
    assert_eq!(DataType::from_str("string(us-ascii)").unwrap(), plain);
    assert_eq!(
        plain.string_parameters(),
        Some(StringParameters::ascii(StringLayout::String))
    );
    assert_eq!(plain.charset(), Some(Charset::Ascii));
    assert_eq!(plain.id(), DataTypeId::String);
    assert!(plain.is_string());
    assert!(!plain.is_code());
    assert_eq!(plain.fixed_byte_width(), None);

    // One number, one reading per layout: `ascii(4)` is a maximum and
    // `fixed_ascii(4)` the width.
    let bounded = DataType::from_str("ascii(4)").unwrap();
    assert_eq!(bounded.to_string(), "ascii(4)");
    assert_eq!(bounded, DataType::from_str("string(us-ascii,4)").unwrap());
    let parameters = bounded.string_parameters().unwrap();
    assert_eq!(parameters.max(), Some(4));
    assert_eq!(parameters.fixed(), None);
    assert_eq!(bounded.fixed_byte_width(), None);
    assert_eq!(bounded.id(), DataTypeId::String);

    let fixed = DataType::fixed_ascii(4).unwrap();
    assert_eq!(fixed.to_string(), "fixed_ascii(4)");
    assert_eq!(DataType::from_str("fixed_ascii(4)").unwrap(), fixed);
    assert_eq!(
        DataType::from_str("fixed_string(us-ascii,4)").unwrap(),
        fixed
    );
    assert_eq!(fixed.string_parameters().unwrap().fixed(), Some(4));
    assert_eq!(fixed.fixed_byte_width(), Some(4));
    assert_eq!(fixed.id(), DataTypeId::FixedString);
    assert_ne!(bounded, fixed);

    // A charset-named spelling takes only a bound; a width is what makes a
    // string fixed; a bound of no bytes is a column of one value.
    assert!(DataType::from_str("ascii(utf-8)").is_err());
    assert!(DataType::from_str("fixed_ascii").is_err());
    assert!(DataType::fixed_ascii(0).is_err());
    assert!(DataType::from_str("ascii(0)").is_err());
}

#[test]
fn an_ascii_value_is_the_string_value_and_carries_no_maximum() {
    // What comes out of an `ascii` column is the crate's one string value,
    // equal to the plain spelling of the same characters.
    let value = DataType::ascii().scalar("USD").unwrap();
    let Scalar::String(held) = &value else {
        panic!("an ascii value is a string, got {value:?}");
    };
    assert_eq!(held.charset(), Charset::Ascii);
    assert_eq!(held.layout(), StringLayout::String);
    assert_eq!(value, Scalar::from("USD"));
    assert_eq!(value.as_str(), Some("USD"));
    assert_eq!(value.id(), DataTypeId::String);
    assert_eq!(value.dtype().unwrap(), DataType::ascii());

    // A maximum is the column's rule: a value read out of `ascii(4)` is an
    // `ascii`, and one that outgrows the column is refused naming the bound.
    let bounded = DataType::from_str("ascii(4)").unwrap();
    assert_eq!(
        bounded.scalar("USD").unwrap().dtype().unwrap(),
        DataType::ascii()
    );
    let refused = bounded.scalar("EURO!").unwrap_err().to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");

    // A fixed width is the value's shape: it is carried, its padding is
    // trimmed on the way in, and it comes back padded on the way out.
    let fixed = DataType::fixed_ascii(4).unwrap();
    let padded = fixed.scalar("USD\0").unwrap();
    assert_eq!(padded.as_str(), Some("USD"));
    assert_eq!(padded.id(), DataTypeId::FixedString);
    assert_eq!(padded.dtype().unwrap(), fixed);
    let Scalar::String(held) = &padded else {
        panic!("a fixed ascii value is a string, got {padded:?}");
    };
    assert_eq!(held.fixed(), Some(4));
    assert_eq!(held.encode().unwrap().as_ref(), b"USD\0");
    assert_eq!(held.encoded_len(), 4);
    assert!(fixed.scalar("EURO!").is_err());
}

#[test]
fn ascii_is_a_repertoire_and_refuses_what_it_never_holds() {
    // A byte above 0x7F and a NUL are refused by name wherever the charset
    // is US-ASCII, from text and from bytes alike.
    for dtype in [
        DataType::ascii(),
        DataType::from_str("ascii(8)").unwrap(),
        DataType::fixed_ascii(8).unwrap(),
    ] {
        let refused = dtype.scalar("caf\u{e9}").unwrap_err().to_string();
        assert!(refused.contains("0xC3"), "{dtype}: {refused}");
        assert!(
            dtype
                .scalar(Scalar::from(vec![0x63_u8, 0x61, 0x66, 0xE9]))
                .is_err(),
            "{dtype}"
        );
        assert!(
            dtype.scalar(Scalar::from(vec![0x80_u8])).is_err(),
            "{dtype}"
        );
        assert_eq!(
            dtype
                .scalar(Scalar::from(b"USD".to_vec()))
                .unwrap()
                .as_str(),
            Some("USD"),
            "{dtype}"
        );
    }
    // Trailing NUL is padding on the fixed layout alone: a variable string
    // holds no NUL at all.
    let refused = DataType::ascii().scalar("USD\0").unwrap_err().to_string();
    assert!(refused.contains("NUL"), "{refused}");
    assert!(
        DataType::from_str("ascii(8)")
            .unwrap()
            .scalar(Scalar::from(b"USD\0".to_vec()))
            .is_err()
    );
    assert_eq!(
        DataType::fixed_ascii(8)
            .unwrap()
            .scalar(Scalar::from(b"USD\0\0\0\0\0".to_vec()))
            .unwrap()
            .as_str(),
        Some("USD")
    );
    // The same rule under the value's own door.
    let ascii = StringParameters::ascii(StringLayout::String);
    assert!(Str::new("caf\u{e9}").try_with_parameters(ascii).is_err());
    assert!(Str::from_bytes(&[0x80], ascii).is_err());
}

#[test]
fn ascii_rides_arrow_text_storage_under_the_string_document() {
    // ASCII bytes are UTF-8, so Arrow is told the truth about the bytes and
    // the charset rides the `yggdryl.string` document beside them.
    let cases: [(DataType, ArrowDataType, &str); 3] = [
        (
            DataType::ascii(),
            ArrowDataType::Utf8,
            r#"{"layout":"string","charset":"us-ascii"}"#,
        ),
        (
            DataType::from_str("ascii(4)").unwrap(),
            ArrowDataType::Utf8,
            r#"{"layout":"string","charset":"us-ascii","max":4}"#,
        ),
        (
            DataType::fixed_ascii(4).unwrap(),
            ArrowDataType::FixedSizeBinary(4),
            r#"{"layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
        ),
    ];
    for (dtype, storage, document) in cases {
        assert_eq!(dtype.clone().into_arrow().unwrap(), storage, "{dtype}");
        let field = dtype.clone().nullable_field("ccy");
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(arrow.data_type(), &storage, "{dtype}");
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            Some("yggdryl.string"),
            "{dtype}"
        );
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_METADATA_KEY)
                .map(String::as_str),
            Some(document),
            "{dtype}"
        );
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{dtype}");

        // A value crosses as itself in both directions.
        let value = dtype.scalar("USD").unwrap();
        let array = scalar_array(&field, &value).unwrap();
        assert_eq!(array.data_type(), &storage, "{dtype}");
        assert_eq!(
            scalar_value(&field, array.as_ref()).unwrap(),
            value,
            "{dtype}"
        );
    }

    // A bare Utf8 column is plain UTF-8, and the retired `yggdryl.ascii`
    // name is nobody's: a field wearing it imports as its storage.
    let plain = arrow_schema::Field::new("ccy", ArrowDataType::Utf8, true);
    assert_eq!(
        Field::from_arrow(&plain).unwrap().dtype(),
        &DataType::utf8()
    );
    let retired = plain.with_metadata(
        [
            (
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "yggdryl.ascii".to_owned(),
            ),
            (EXTENSION_TYPE_METADATA_KEY.to_owned(), String::new()),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        Field::from_arrow(&retired).unwrap().dtype(),
        &DataType::utf8()
    );
}

#[test]
fn a_string_enum_needs_a_fixed_ascii_width_its_members_pack_into() {
    // The members pack into integers through `ascii_packed`, so the
    // vocabulary is accepted on a fixed US-ASCII string of at most sixteen
    // bytes or a code, and refused by name everywhere else.
    let sides = StringEnum::from_logical_name("side").unwrap();
    for accepted in [
        DataType::fixed_ascii(4).unwrap(),
        DataType::fixed_ascii(16).unwrap(),
        DataType::Side,
    ] {
        let field = Field::new("side", accepted.clone(), false)
            .try_with_string_enum(&sides)
            .unwrap_or_else(|error| panic!("{accepted}: {error}"));
        assert_eq!(field.string_enum().unwrap().as_ref(), Some(&sides));
        let recovered = Field::from_arrow(&field.clone().into_arrow().unwrap()).unwrap();
        assert_eq!(recovered, field, "{accepted}");
    }
    for refused in [
        DataType::ascii(),
        DataType::from_str("ascii(4)").unwrap(),
        DataType::fixed_ascii(17).unwrap(),
        DataType::fixed_utf8(4).unwrap(),
        DataType::utf8(),
    ] {
        let message = Field::new("side", refused.clone(), false)
            .try_with_string_enum(&sides)
            .unwrap_err()
            .to_string();
        assert!(
            message.contains(&refused.to_string()),
            "{refused}: {message}"
        );
    }
}
