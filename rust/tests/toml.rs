use std::io::{Cursor, Read};
use std::str::FromStr;

use yggdryl::toml as ytoml;
use yggdryl::{DataType, Field, I256, Limits, TimeUnit, Timezone, Value};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn natural_output_is_accepted_by_the_toml_crate() {
    let value = Value::from_record([
        ("active", Value::Bool(true)),
        ("id", Value::I64(7)),
        ("tags", Value::from_sequence([Value::from("rust")])),
    ])
    .unwrap();
    let encoded = ytoml::into_utf8(&value).unwrap();

    assert!(!encoded.contains("$yggdryl"));
    let foreign: toml::Table = toml::from_str(&encoded).unwrap();
    assert_eq!(foreign["id"].as_integer(), Some(7));
    assert_eq!(ytoml::from_utf8(&encoded).unwrap(), value);
}

#[test]
fn empty_toml_is_an_empty_record() {
    for source in ["", "  \n", "# comment\n"] {
        assert_eq!(
            ytoml::from_utf8(source).unwrap(),
            Value::from_record(std::iter::empty::<(&str, Value)>()).unwrap()
        );
    }
}

#[test]
fn native_toml_temporals_are_syntax_proven_values() {
    let value =
        ytoml::from_utf8("date = 1979-05-27\ntime = 07:32:00.1\nat = 1970-01-01T00:00:00Z\n")
            .unwrap();
    let record = value.as_record().unwrap();
    assert!(matches!(record["date"], Value::Date32(..)));
    assert!(matches!(record["time"], Value::Time32(..)));
    assert!(matches!(record["at"], Value::DateTime64(..)));
}

fn typed_row_field() -> Field {
    Field::new(
        "row",
        DataType::from_fields([
            Field::new("amount", DataType::decimal256(76, 4).unwrap(), false),
            Field::new(
                "at",
                DataType::Timestamp(TimeUnit::Second, Some(Timezone::UTC)),
                false,
            ),
            Field::new(
                "clock",
                DataType::time32(TimeUnit::Millisecond).unwrap(),
                false,
            ),
            Field::new("payload", DataType::Binary, false),
        ])
        .unwrap(),
        false,
    )
}

#[test]
fn a_field_restores_exact_types_from_natural_toml() {
    let input = "amount = '123.4500'\nat = 1970-01-01T00:00:00Z\nclock = '07:32:00.100'\npayload = 'AP8='\n";
    let decoded = ytoml::from_utf8_with_field(input, &typed_row_field()).unwrap();
    let row = decoded.as_sequence().unwrap();

    assert_eq!(row[0], Value::d256(I256::from_str("1234500").unwrap(), 4));
    assert_eq!(
        row[1],
        Value::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    assert_eq!(
        row[2],
        Value::time32(27_120_100, TimeUnit::Millisecond, Timezone::NAIVE,).unwrap()
    );
    assert_eq!(row[3], Value::from(vec![0, 255]));
}

#[test]
fn exact_values_emit_natural_scalars_without_private_tags() {
    let value = Value::from_record([
        ("amount", Value::d256(I256::from_str("1234500").unwrap(), 4)),
        (
            "at",
            Value::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        (
            "clock",
            Value::time32(27_120_100, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        ),
        ("payload", Value::from(vec![0, 255])),
    ])
    .unwrap();
    let encoded = ytoml::into_utf8(&value).unwrap();
    assert!(!encoded.contains("$yggdryl"));
    let _: toml::Table = toml::from_str(&encoded).unwrap();

    let typed = ytoml::from_utf8_with_field(&encoded, &typed_row_field()).unwrap();
    assert_eq!(
        typed.as_sequence().unwrap()[0],
        value.as_record().unwrap()["amount"]
    );
}

#[test]
fn time_of_day_is_naive_and_zoned_text_is_refused() {
    let field = Field::new(
        "clock",
        DataType::time64(TimeUnit::Nanosecond).unwrap(),
        false,
    );
    let value = Value::time64(1_500_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap();
    let document = Value::from_record([("clock", value.clone())]).unwrap();
    let encoded = ytoml::into_utf8(&document).unwrap();
    let row_field = Field::new("row", DataType::from_fields([field]).unwrap(), false);
    assert_eq!(
        ytoml::from_utf8_with_field(&encoded, &row_field)
            .unwrap()
            .as_sequence()
            .unwrap()[0],
        value
    );

    assert!(
        ytoml::from_utf8_with_field("clock = '00:00:00+02:00'\n", &row_field)
            .unwrap_err()
            .to_string()
            .contains("DateTime64")
    );
    assert!(Value::time32(0, TimeUnit::Second, Timezone::UTC).is_err());
    let invalid =
        Value::from_record([("clock", Value::Time32(0, TimeUnit::Second, Timezone::UTC))]).unwrap();
    assert!(
        ytoml::into_bytes(&invalid)
            .unwrap_err()
            .to_string()
            .contains("DateTime64")
    );
}

#[test]
fn readers_single_document_rules_and_limits_are_explicit() {
    let source = "label = 'café'\nvalue = 1\n";
    let expected = ytoml::from_utf8(source).unwrap();
    assert_eq!(
        ytoml::from_reader(OneByte(Cursor::new(source.as_bytes()))).unwrap(),
        expected
    );

    let mut output = Vec::new();
    ytoml::into_writer_all([&expected], &mut output).unwrap();
    assert_eq!(
        ytoml::from_bytes_all(&output).unwrap(),
        std::slice::from_ref(&expected)
    );
    assert!(ytoml::into_writer_all(std::iter::empty::<&Value>(), Vec::new()).is_err());
    assert!(ytoml::into_writer_all([&expected, &expected], Vec::new()).is_err());
    assert!(ytoml::from_utf8_with_limits(source, Limits::new(1, 4, 16, 1)).is_err());
}

#[test]
fn toml_refuses_shapes_its_grammar_cannot_represent() {
    assert!(ytoml::into_bytes(&Value::Null).is_err());
    assert!(ytoml::into_bytes(&Value::I64(1)).is_err());
    assert!(ytoml::into_bytes(&Value::from_record([("missing", Value::Null)]).unwrap()).is_err());
    assert!(
        ytoml::into_bytes(&Value::from_mapping([(Value::I64(1), Value::I64(2))]).unwrap()).is_err()
    );
}
