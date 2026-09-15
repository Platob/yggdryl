use std::io::{Cursor, Read};
use std::str::FromStr;

use saphyr_parser::{Event, EventReceiver, Parser};
use yggdryl::text::yaml;
use yggdryl::{
    DataType, DataTypeId, Field, Limits, Scalar, TimeUnit, Timezone, from_yaml_scalar,
    from_yaml_scalar_with_field, i256, into_yaml_scalar,
};

#[test]
fn field_directed_yaml_restores_exact_decimal_and_interval_leaves() {
    let decimal = DataType::decimal64(18, 2).unwrap();
    let value = decimal.scalar(Scalar::from(125)).unwrap();
    let encoded = into_yaml_scalar(&value).unwrap();
    let decoded = from_yaml_scalar_with_field(&encoded, &decimal.required_field("value")).unwrap();
    assert_eq!(decoded.id(), DataTypeId::Decimal64);

    let interval = DataType::Interval(TimeUnit::DayTime);
    let value = interval
        .scalar(Scalar::from_sequence([Scalar::from(2), Scalar::from(3)]))
        .unwrap();
    let encoded = into_yaml_scalar(&value).unwrap();
    assert!(encoded.contains("[2, 3]"), "{encoded}");
    let decoded =
        from_yaml_scalar_with_field(&encoded, &interval.clone().required_field("value")).unwrap();
    assert_eq!(decoded.id(), DataTypeId::Interval);
    assert_eq!(decoded.dtype().unwrap(), interval);
}

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

struct Sink;

impl<'input> EventReceiver<'input> for Sink {
    fn on_event(&mut self, _event: Event<'input>) {}
}

#[test]
fn natural_output_is_accepted_by_an_independent_yaml_parser() {
    let value = Scalar::from_record([
        ("active", Scalar::from(true)),
        ("id", Scalar::from(7)),
        ("tags", Scalar::from_sequence([Scalar::from("rust")])),
    ])
    .unwrap();
    let encoded = yaml::into_utf8(&value).unwrap();

    Parser::new_from_str(&encoded)
        .load(&mut Sink, false)
        .unwrap();
    assert_eq!(yaml::from_utf8(&encoded).unwrap(), value);
}

#[test]
fn yaml_standard_binary_is_not_a_private_envelope() {
    let value = Scalar::from_record([("payload", Scalar::from(vec![0, 255]))]).unwrap();
    let encoded = yaml::into_utf8(&value).unwrap();
    assert!(encoded.contains("!!binary"), "{encoded}");
    assert_eq!(yaml::from_utf8(&encoded).unwrap(), value);
}

#[test]
fn untyped_yaml_preserves_only_syntax_proven_types() {
    let value =
        yaml::from_utf8("amount: '123.4500'\nat: '1970-01-01T00:00:00Z'\npayload: 'AP8='\n")
            .unwrap();
    let record = value.as_record().unwrap();
    assert!(record["amount"].as_str().is_some());
    assert!(record["at"].as_str().is_some());
    assert!(record["payload"].as_str().is_some());
}

fn typed_row_field() -> Field {
    Field::new(
        "row",
        DataType::from_fields([
            Field::new("amount", DataType::decimal256(76, 4).unwrap(), false),
            Field::new(
                "at",
                DataType::DateTime64 {
                    unit: TimeUnit::Second,
                    timezone: Timezone::UTC,
                },
                false,
            ),
            Field::new(
                "clock",
                DataType::time64(TimeUnit::Nanosecond).unwrap(),
                false,
            ),
            Field::new("payload", DataType::binary(), false),
        ])
        .unwrap(),
        false,
    )
}

#[test]
fn a_field_restores_exact_types_from_natural_yaml() {
    let input =
        "payload: AP8=\nclock: '00:00:01.5'\namount: '123.4500'\nat: '1970-01-01T00:00:00Z'\n";
    let decoded = yaml::from_utf8_with_field(input, &typed_row_field()).unwrap();
    let row = decoded.as_sequence().unwrap();

    assert_eq!(row[0], Scalar::d256(i256::from_str("1234500").unwrap(), 4));
    assert_eq!(
        row[1],
        Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    assert_eq!(
        row[2],
        Scalar::time64(1_500_000_000, TimeUnit::Nanosecond, Timezone::NAIVE,).unwrap()
    );
    assert_eq!(row[3], Scalar::from(vec![0, 255]));
}

#[test]
fn time_of_day_is_naive_and_zoned_text_is_refused() {
    let field = Field::new(
        "clock",
        DataType::time32(TimeUnit::Millisecond).unwrap(),
        false,
    );
    let value = Scalar::time32(1_500, TimeUnit::Millisecond, Timezone::NAIVE).unwrap();
    let encoded = yaml::into_utf8(&value).unwrap();
    assert_eq!(yaml::from_utf8_with_field(&encoded, &field).unwrap(), value);

    assert!(
        yaml::from_utf8_with_field("'00:00:00+02:00'", &field)
            .unwrap_err()
            .to_string()
            .contains("DateTime64")
    );
    assert!(Scalar::time64(0, TimeUnit::Nanosecond, Timezone::UTC).is_err());
}

#[test]
fn arbitrary_keys_and_standard_custom_tags_have_natural_semantics() {
    let mapping = Scalar::from_mapping([
        (Scalar::from_sequence([Scalar::from(1)]), Scalar::from(true)),
        (Scalar::from(2), Scalar::from("two")),
    ])
    .unwrap();
    assert_eq!(
        yaml::from_bytes(&yaml::into_bytes(&mapping).unwrap()).unwrap(),
        mapping
    );

    assert_eq!(
        yaml::from_utf8("!vendor/value text\n").unwrap(),
        Scalar::from("text")
    );
    assert_eq!(
        yaml::from_utf8("!vendor/value {id: 1}\n").unwrap(),
        Scalar::from_record([("id", Scalar::from(1))]).unwrap()
    );
}

#[test]
fn readers_documents_and_limits_are_bounded() {
    let source = "label: café\nvalue: 1\n";
    let expected = yaml::from_utf8(source).unwrap();
    assert_eq!(
        yaml::from_reader(OneByte(Cursor::new(source.as_bytes()))).unwrap(),
        expected
    );

    let documents = yaml::from_utf8_all("one\n---\ntwo\n").unwrap();
    assert_eq!(documents, [Scalar::from("one"), Scalar::from("two")]);
    assert!(yaml::from_utf8_all_with_limits("one\n---\ntwo\n", Limits::new(8, 64, 16, 1)).is_err());
    assert!(yaml::from_utf8_with_limits("[[[0]]]", Limits::new(2, 64, 16, 1)).is_err());
}

#[test]
fn nonfinite_yaml_floats_use_the_core_schema_spelling() {
    for value in [Scalar::from(f64::NAN), Scalar::from(f64::INFINITY)] {
        let encoded = yaml::into_utf8(&value).unwrap();
        let decoded = yaml::from_utf8(&encoded).unwrap();
        assert!(decoded.as_f64().unwrap().is_nan() == value.as_f64().unwrap().is_nan());
    }
}

#[test]
fn the_scalar_entry_points_answer_what_the_explicit_forms_answer() {
    let value = Scalar::from_record([
        ("id", Scalar::from(7)),
        ("name", Scalar::from("ada")),
        ("tags", Scalar::from_sequence([Scalar::from("rust")])),
    ])
    .unwrap();
    let encoded = into_yaml_scalar(&value).unwrap();
    assert_eq!(encoded, yaml::into_utf8(&value).unwrap());
    assert_eq!(from_yaml_scalar(&encoded).unwrap(), value);
    assert_eq!(yaml::from_yaml_scalar(&encoded).unwrap(), value);

    let text = "id: 7\nname: ada\n";
    let expected = yaml::from_bytes(text.as_bytes()).unwrap();
    assert_eq!(from_yaml_scalar(text).unwrap(), expected);
    let owned_text = String::from(text);
    let owned_bytes = Vec::from(text.as_bytes());
    assert_eq!(from_yaml_scalar(owned_text).unwrap(), expected);
    assert_eq!(from_yaml_scalar(text.as_bytes()).unwrap(), expected);
    assert_eq!(from_yaml_scalar(owned_bytes).unwrap(), expected);

    assert_eq!(
        from_yaml_scalar("id: [").unwrap_err().to_string(),
        yaml::from_bytes(b"id: [").unwrap_err().to_string()
    );
}

#[test]
fn from_yaml_scalar_with_field_types_and_orders_as_from_bytes_with_field_does() {
    let input =
        "clock: '00:00:01.5'\npayload: AP8=\nat: '1970-01-01T00:00:00Z'\namount: '123.4500'\n";
    let field = typed_row_field();
    let decoded = from_yaml_scalar_with_field(input, &field).unwrap();

    assert_eq!(
        decoded,
        yaml::from_bytes_with_field(input.as_bytes(), &field).unwrap()
    );
    let row = decoded.as_sequence().unwrap();
    assert_eq!(row[0], Scalar::d256(i256::from_str("1234500").unwrap(), 4));
    assert_eq!(
        row[1],
        Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    let untyped = from_yaml_scalar(input).unwrap();
    assert!(untyped.as_record().unwrap()["amount"].as_str().is_some());
}

#[test]
fn a_string_naming_an_existing_file_is_a_yaml_string_not_a_path() {
    let path = "Cargo.toml";
    assert!(std::fs::read_to_string(path).unwrap().contains("[package]"));

    let value = from_yaml_scalar(path).unwrap();
    assert_eq!(value, Scalar::from(path));
    assert_eq!(value, yaml::from_bytes(path.as_bytes()).unwrap());
}

#[test]
fn a_plain_merge_key_is_refused_and_every_other_spelling_of_it_is_text() {
    // `<<` in key position is the one place the spelling matters: the merge it
    // asks for is not performed, so it is refused by name rather than read as
    // a field called `<<`.
    let error = yaml::from_utf8("a: 1\n<<: {b: 2}\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("YAML merge keys are not supported"),
        "{error}"
    );
    let error = yaml::from_utf8("base: &b {a: 1}\nchild: {<<: *b, c: 2}\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("YAML merge keys are not supported"),
        "{error}"
    );

    // Anywhere else the same two characters are the string they look like.
    for (document, expected) in [
        ("k: <<\n", Scalar::from("<<")),
        ("a: &m <<\nb: *m\n", Scalar::from("<<")),
    ] {
        let value = yaml::from_utf8(document).unwrap();
        assert_eq!(
            value.get_key_str("k").or(value.get_key_str("b")),
            Some(&expected),
            "{document}"
        );
    }
    assert_eq!(yaml::from_utf8("<<\n").unwrap(), Scalar::from("<<"));
    assert_eq!(
        yaml::from_utf8("- <<\n").unwrap(),
        Scalar::from_sequence([Scalar::from("<<")])
    );

    // A quoted or tagged `<<` was never a merge request, so it stays a key.
    for document in ["'<<': 1\n", "!!str << : 1\n"] {
        let value = yaml::from_utf8(document).unwrap();
        assert_eq!(
            value.get_key_str("<<").and_then(Scalar::as_i128),
            Some(1),
            "{document}"
        );
    }

    // An alias replays the spelling as well as the value, so an anchored `<<`
    // reaching key position through an explicit key is refused exactly as the
    // text `<<` written there would be. saphyr accepts an alias as an explicit
    // key, so this path is reachable and the flag has to survive the anchor.
    let error = yaml::from_utf8("a: &m <<\n? *m\n: 2\n")
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "invalid yaml data at byte 16: YAML merge keys are not supported"
    );
}

#[test]
fn a_duplicate_key_is_refused_at_the_byte_the_second_one_was_written_at() {
    // The entry index a value-level refusal carries is only useful once it is
    // restated as the offset every other error in this codec reports.
    for (document, position, reason) in [
        ("a: 1\na: 2\n", 5, "record contains a duplicate field name"),
        (
            "alpha: 1\nbeta: 2\nalpha: 3\n",
            17,
            "record contains a duplicate field name",
        ),
        (
            "outer:\n  x: 1\n  x: 2\n",
            16,
            "record contains a duplicate field name",
        ),
        (
            "x: &m {a: 1, a: 2}\ny: *m\n",
            13,
            "record contains a duplicate field name",
        ),
        ("{1: a, 1: b}\n", 7, "mapping contains a duplicate key"),
        // Multibyte keys count bytes, and the offset is stream-absolute.
        (
            "café: 1\ncafé: 2\n",
            9,
            "record contains a duplicate field name",
        ),
        (
            "a: 1\n---\nb: 1\nb: 2\n",
            14,
            "record contains a duplicate field name",
        ),
    ] {
        let error = yaml::from_utf8(document).unwrap_err().to_string();
        assert_eq!(
            error,
            format!("invalid yaml data at byte {position}: {reason}"),
            "{document:?}"
        );
    }
}

#[test]
fn a_tag_the_value_model_cannot_carry_is_erased_and_its_payload_kept() {
    // A non-core tag is an annotation: there is no carrier for it, so what
    // survives is the value underneath, unchanged.
    assert_eq!(
        yaml::from_utf8("!vendor/value text\n").unwrap(),
        Scalar::from("text")
    );
    let value = yaml::from_utf8("!vendor/value {id: 1}\n").unwrap();
    assert_eq!(value.get_key_str("id").and_then(Scalar::as_i128), Some(1));

    // A core tag resolves the scalar rather than annotating it, and one that
    // disagrees with the container it sits on is a refusal.
    assert_eq!(yaml::from_utf8("!!null foo\n").unwrap(), Scalar::Null);
    assert_eq!(yaml::from_utf8("!!str 12\n").unwrap(), Scalar::from("12"));
    assert_eq!(
        yaml::from_utf8("!!timestamp 2020-01-01\n").unwrap(),
        Scalar::from("2020-01-01")
    );
    assert_eq!(
        yaml::from_utf8("!!seq [1]\n").unwrap(),
        Scalar::from_sequence([Scalar::from(1_u64)])
    );
    for document in ["!!seq {a: 1}\n", "!!map [1]\n", "!!set {a}\n"] {
        let error = yaml::from_utf8(document).unwrap_err().to_string();
        assert!(
            error.contains("YAML core tag does not match container"),
            "{document:?} {error}"
        );
    }
}

#[test]
fn plain_resolution_answers_only_what_the_spelling_proves() {
    for (document, expected) in [
        ("nUll", Scalar::Null),
        ("~", Scalar::Null),
        ("yes", Scalar::from(true)),
        ("Off", Scalar::from(false)),
        // Single letters are not booleans here, whatever YAML 1.1 allowed.
        ("y", Scalar::from("y")),
        ("0x1F", Scalar::from(31_u64)),
        ("0o17", Scalar::from(15_u64)),
        ("0b101", Scalar::from(5_u64)),
        ("1_000", Scalar::from(1000_u64)),
        // A leading zero is decimal, never octal.
        ("018", Scalar::from(18_u64)),
        // The sign decides signedness even when the magnitude is zero.
        ("-0", Scalar::from(0_i64)),
        ("0", Scalar::from(0_u64)),
        (".5", Scalar::from(0.5)),
        ("1e3", Scalar::from(1000.0)),
        // Rust's float spellings are not YAML's.
        ("inf", Scalar::from("inf")),
        ("nan", Scalar::from("nan")),
        // Underscores must sit between digits, and a broken one is just text.
        ("1__0", Scalar::from("1__0")),
        ("1.0_", Scalar::from("1.0_")),
        // Neither timestamps nor sexagesimals are resolved.
        ("2020-01-01", Scalar::from("2020-01-01")),
        ("12:30", Scalar::from("12:30")),
    ] {
        assert_eq!(yaml::from_utf8(document).unwrap(), expected, "{document:?}");
    }

    // A malformed radix prefix is a refusal, while a malformed bare spelling
    // is simply not a number.
    for document in ["0x", "0xg", "0o8", "0b", "0b102"] {
        let error = yaml::from_utf8(document).unwrap_err().to_string();
        assert!(
            error.contains("invalid YAML integer"),
            "{document:?} {error}"
        );
    }
}

#[test]
fn an_alias_is_charged_what_writing_the_value_out_again_would_have_cost() {
    // The budgets bound what an expansion produces, not what the source text
    // spells, so an anchor reused four times is counted four times.
    let document = "a: &x [1, 2, 3]\nb: *x\nc: *x\nd: *x\n";
    assert!(yaml::from_utf8_with_limits(document, Limits::new(64, 4096, 12, 4)).is_err());
    assert!(yaml::from_utf8_with_limits(document, Limits::new(64, 4096, 64, 4)).is_ok());

    // A record key costs one node and one level, exactly as a mapping key
    // scalar does, so an anchored record is charged for its names.
    let document = "a: &x {p: 1, q: 2}\nb: *x\n";
    assert!(yaml::from_utf8_with_limits(document, Limits::new(64, 4096, 8, 4)).is_err());
    assert!(yaml::from_utf8_with_limits(document, Limits::new(64, 4096, 64, 4)).is_ok());

    // An alias naming nothing is refused before any budget is consulted.
    let error = yaml::from_utf8("b: *missing\n").unwrap_err().to_string();
    assert!(error.contains("unknown anchor"), "{error}");
}

#[test]
fn an_explicit_but_empty_document_is_null_and_two_roots_are_refused() {
    assert_eq!(yaml::from_utf8("---\n").unwrap(), Scalar::Null);
    assert!(yaml::from_utf8("").is_err());
    let error = yaml::from_utf8("a: 1\nb\n").unwrap_err().to_string();
    assert!(!error.is_empty(), "{error}");
}

#[test]
fn a_duplicate_is_found_when_its_own_mapping_closes_and_not_before() {
    // A nested mapping seals while the rest of the document is still being
    // read, so its duplicate is refused ahead of a fault further on.
    let error = yaml::from_utf8_with_limits(
        "x: {a: 1, a: 2}\ny: [1,2,3,4,5,6,7,8]\n",
        Limits::new(64, 4096, 8, 4),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        error,
        "invalid yaml data at byte 10: record contains a duplicate field name"
    );

    // The root mapping closes last, so a fault before the end of the document
    // is still what the reader stops on.
    let error = yaml::from_utf8("a: 1\na: 2\nb: [\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("did not find expected node content"),
        "{error}"
    );
}

#[test]
fn a_tag_costs_the_budget_nothing_once_the_wrapper_is_gone() {
    // The tag is an annotation with no carrier, so an alias to a tagged
    // scalar is charged for the scalar and not for the annotation.
    let document = "a: &x !v 1\nb: *x\nc: *x\nd: *x\n";
    assert!(yaml::from_utf8_with_limits(document, Limits::new(64, 4096, 10, 4)).is_ok());
    // The parser's own accounting still bounds it, and says where it stopped.
    let error = yaml::from_utf8_with_limits(document, Limits::new(64, 4096, 9, 4))
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "invalid yaml data at byte 26: decoded node limit exceeded"
    );
}
