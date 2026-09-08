//! An ASCII width crosses the exchange formats as its trimmed text.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, RecordBatch, StringArray};
use yggdryl::arrow::batch_reader;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
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
fn a_cast_into_a_securities_number_holds_the_column_to_the_canonical_spelling() {
    let schema = root([DataType::Utf8.required_field("sid")]);
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
    assert_eq!(
        **literal,
        Expression::Literal({
            let dtype = DataType::FixedAscii(4);
            let value = dtype.scalar(Scalar::from("USD")).unwrap();
            TypedScalar::from_parts(dtype, value).unwrap()
        })
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
fn a_state_sorts_from_the_first_state_to_the_terminal_ones() {
    use yggdryl::types::State;

    // The stored bytes, sorted by nothing but ASCII. This is the whole claim:
    // whatever sorts the column - a Parquet row group's bounds, an external
    // sort, an ORDER BY in something that never heard of this crate - puts
    // every live state before every ended one.
    let mut held: Vec<&str> = yggdryl::AsciiEnum::STATES.to_vec();
    held.sort_unstable();
    assert_eq!(
        held.as_slice(),
        yggdryl::AsciiEnum::STATES,
        "the vocabulary is declared in the order it sorts",
    );

    let ordered = [
        "10PENDING",
        "20NEW",
        "40PARTFILL",
        "60PENDCXL",
        "80FILLED",
        "90CANCELED",
        "95REJECTED",
    ];
    let mut shuffled = [
        "95REJECTED",
        "80FILLED",
        "20NEW",
        "60PENDCXL",
        "10PENDING",
        "90CANCELED",
        "40PARTFILL",
    ];
    shuffled.sort_unstable();
    assert_eq!(shuffled, ordered);

    // The rank is the two leading digits, read as the number they spell.
    for (held, rank) in [
        ("00UNKNOWN", 0),
        ("10PENDING", 10),
        ("40PARTFILL", 40),
        ("80FILLED", 80),
        ("90CANCELED", 90),
        ("95REJECTED", 95),
    ] {
        assert_eq!(State::new(held).unwrap().rank(), Some(rank), "{held}");
    }

    // Every ending is told apart from every other without reading a name.
    for held in [
        "10PENDING",
        "20NEW",
        "40PARTFILL",
        "60PENDCXL",
        "70REPLACED",
    ] {
        assert!(State::new(held).unwrap().is_live(), "{held}");
    }
    assert!(State::new("80FILLED").unwrap().is_done());
    assert!(State::new("90CANCELED").unwrap().is_cancelled());
    assert!(State::new("95REJECTED").unwrap().is_failed());
    for held in ["80FILLED", "90CANCELED", "95REJECTED"] {
        assert!(!State::new(held).unwrap().is_live(), "{held}");
    }

    // The digits between two shipped ranks are placeholders: a state that
    // belongs between them takes one, and the predicates read the band it
    // falls in rather than the exact rank.
    let between = State::new("85ARCHIVED").unwrap();
    assert_eq!(between.rank(), Some(85));
    assert!(between.is_done());
    assert!(!between.is_live());
    assert!(State::new("92HALTED").unwrap().is_cancelled());
    assert!(State::new("97ABORTED").unwrap().is_failed());

    // A value that opens with anything but two digits has no rank, and so is
    // neither live nor ended.
    let unranked = State::new("FILLED").unwrap();
    assert_eq!(unranked.rank(), None);
    assert!(!unranked.is_live());
    assert!(!unranked.is_done());
    assert_eq!(State::new("8FILLED").unwrap().rank(), None);
}

#[test]
fn a_state_answers_a_fix_code_a_fix_name_and_a_scheduler_word_alike() {
    use yggdryl::types::State;

    // One value, four vocabularies: the wire code an ExecutionReport carries,
    // the specification's name for it, the word a scheduler uses, and the
    // short name a FIX bridge logs.
    for (spelling, expected) in [
        ("0", "20NEW"),
        ("1", "40PARTFILL"),
        ("2", "80FILLED"),
        ("8", "95REJECTED"),
        ("F", "40TRADE"),
        ("New", "20NEW"),
        ("PartiallyFilled", "40PARTFILL"),
        ("DoneForDay", "80DONEDAY"),
        ("done_for_day", "80DONEDAY"),
        ("DONE FOR DAY", "80DONEDAY"),
        ("running", "30RUNNING"),
        ("succeeded", "80SUCCESS"),
        ("timed out", "95TIMEOUT"),
        ("failed", "95FAILED"),
        // The short names a FIX bridge logs fold to the same states.
        ("PartFill", "40PARTFILL"),
        ("PartFilled", "40PARTFILL"),
        ("PendNew", "10PENDNEW"),
        ("PendCancel", "60PENDCXL"),
        ("PendReplace", "60PENDRPL"),
        ("DoneDay", "80DONEDAY"),
        ("Cancel", "90CANCELED"),
        ("Reject", "95REJECTED"),
        // A stored value names itself, so resolving one twice is resolving it
        // once.
        ("80FILLED", "80FILLED"),
    ] {
        let held =
            State::from_spelling(spelling).unwrap_or_else(|| panic!("{spelling} names no state"));
        assert_eq!(held.as_str(), expected, "{spelling}");
        assert_eq!(
            State::from_spelling(held.as_str()).unwrap().as_str(),
            expected,
            "{spelling} resolves to itself",
        );
    }

    // A wire code never folds: `A` is PendingNew and `a` is not a code at all.
    assert_eq!(State::from_spelling("A").unwrap().as_str(), "10PENDNEW");
    assert_eq!(State::from_spelling("a"), None);
    assert_eq!(State::from_spelling("whatever"), None);
    assert_eq!(State::from_spelling(""), None);
}

#[test]
fn the_state_and_time_in_force_codes_are_ordinary_datatypes_everywhere_else() {
    use yggdryl::{DataTypeKind, Scalar};

    for (name, dtype, width) in [
        ("state", DataType::State, 10),
        ("timeinforce", DataType::TimeInForce, 8),
    ] {
        // Parsed, displayed and round-tripped by the grammar like any other.
        assert_eq!(DataType::from_str(name).unwrap(), dtype);
        assert_eq!(dtype.to_string(), name);
        assert_eq!(dtype.kind(), DataTypeKind::Ascii);
        assert!(dtype.is_code());
        assert_eq!(dtype.ascii_width(), Some(width));

        // And it crosses Arrow as the fixed width it is, extension name and
        // all, so a column round-trips without becoming plain bytes.
        let field = Field::new(name, dtype.clone(), true);
        let recovered = Field::from_arrow(&field.clone().into_arrow().unwrap()).unwrap();
        assert_eq!(recovered, field);
    }

    // A value wider than the storage is refused by the datatype rather than
    // truncated into something that reads.
    assert!(DataType::State.scalar(Scalar::from("20NEW")).is_ok());
    assert!(DataType::State.scalar(Scalar::from("40PARTFILL")).is_ok());
    assert!(
        DataType::State
            .scalar(Scalar::from("80CALCULATED"))
            .is_err()
    );
    assert!(DataType::TimeInForce.scalar(Scalar::from("0")).is_ok());
}
