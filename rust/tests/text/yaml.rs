use std::io::{Cursor, Read};
use std::str::FromStr;

use saphyr_parser::{Event, EventReceiver, Parser};
use yggdryl::text::yaml;
use yggdryl::{
    DataType, Field, I256, Limits, Scalar, TimeUnit, Timezone, from_yaml_scalar,
    from_yaml_scalar_with_field, into_yaml_scalar,
};

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
        ("active", Scalar::Bool(true)),
        ("id", Scalar::I64(7)),
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
    assert!(matches!(record["amount"], Scalar::String(_)));
    assert!(matches!(record["at"], Scalar::String(_)));
    assert!(matches!(record["payload"], Scalar::String(_)));
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
            Field::new("payload", DataType::Binary, false),
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

    assert_eq!(row[0], Scalar::d256(I256::from_str("1234500").unwrap(), 4));
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
    let invalid = Scalar::Time64(0, TimeUnit::Nanosecond, Timezone::UTC);
    assert!(
        yaml::into_bytes(&invalid)
            .unwrap_err()
            .to_string()
            .contains("DateTime64")
    );
}

#[test]
fn arbitrary_keys_and_standard_custom_tags_have_natural_semantics() {
    let mapping = Scalar::from_mapping([
        (Scalar::from_sequence([Scalar::I64(1)]), Scalar::Bool(true)),
        (Scalar::I64(2), Scalar::from("two")),
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
        Scalar::from_record([("id", Scalar::U64(1))]).unwrap()
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
        ("id", Scalar::I64(7)),
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
    assert_eq!(row[0], Scalar::d256(I256::from_str("1234500").unwrap(), 4));
    assert_eq!(
        row[1],
        Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    let untyped = from_yaml_scalar(input).unwrap();
    assert!(matches!(
        untyped.as_record().unwrap()["amount"],
        Scalar::String(_)
    ));
}

#[test]
fn a_string_naming_an_existing_file_is_a_yaml_string_not_a_path() {
    let path = "Cargo.toml";
    assert!(std::fs::read_to_string(path).unwrap().contains("[package]"));

    let value = from_yaml_scalar(path).unwrap();
    assert_eq!(value, Scalar::from(path));
    assert_eq!(value, yaml::from_bytes(path.as_bytes()).unwrap());
}
