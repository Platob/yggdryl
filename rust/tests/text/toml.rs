use std::io::{Cursor, Read};
use std::str::FromStr;

use yggdryl::text::toml as ytoml;
use yggdryl::{
    DataType, Error, Field, I256, Limits, Scalar, TimeUnit, Timezone, from_toml_scalar,
    from_toml_scalar_with_field, into_toml_scalar,
};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn natural_output_is_accepted_by_the_toml_crate() {
    let value = Scalar::from_record([
        ("active", Scalar::Bool(true)),
        ("id", Scalar::I64(7)),
        ("tags", Scalar::from_sequence([Scalar::from("rust")])),
    ])
    .unwrap();
    let encoded = ytoml::into_utf8(&value).unwrap();

    let foreign: toml::Table = toml::from_str(&encoded).unwrap();
    assert_eq!(foreign["id"].as_integer(), Some(7));
    assert_eq!(ytoml::from_utf8(&encoded).unwrap(), value);
}

#[test]
fn empty_toml_is_an_empty_record() {
    for source in ["", "  \n", "# comment\n"] {
        assert_eq!(
            ytoml::from_utf8(source).unwrap(),
            Scalar::from_record(std::iter::empty::<(&str, Scalar)>()).unwrap()
        );
    }
}

#[test]
fn native_toml_temporals_are_syntax_proven_values() {
    let value =
        ytoml::from_utf8("date = 1979-05-27\ntime = 07:32:00.1\nat = 1970-01-01T00:00:00Z\n")
            .unwrap();
    let record = value.as_record().unwrap();
    assert!(matches!(record["date"], Scalar::Date32(..)));
    assert!(matches!(record["time"], Scalar::Time32(..)));
    assert!(matches!(record["at"], Scalar::DateTime64(..)));
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

    assert_eq!(row[0], Scalar::d256(I256::from_str("1234500").unwrap(), 4));
    assert_eq!(
        row[1],
        Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    assert_eq!(
        row[2],
        Scalar::time32(27_120_100, TimeUnit::Millisecond, Timezone::NAIVE,).unwrap()
    );
    assert_eq!(row[3], Scalar::from(vec![0, 255]));
}

#[test]
fn exact_values_emit_natural_scalars_without_private_tags() {
    let value = Scalar::from_record([
        (
            "amount",
            Scalar::d256(I256::from_str("1234500").unwrap(), 4),
        ),
        (
            "at",
            Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        (
            "clock",
            Scalar::time32(27_120_100, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        ),
        ("payload", Scalar::from(vec![0, 255])),
    ])
    .unwrap();
    let encoded = ytoml::into_utf8(&value).unwrap();
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
    let value = Scalar::time64(1_500_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap();
    let document = Scalar::from_record([("clock", value.clone())]).unwrap();
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
    assert!(Scalar::time32(0, TimeUnit::Second, Timezone::UTC).is_err());
    let invalid =
        Scalar::from_record([("clock", Scalar::Time32(0, TimeUnit::Second, Timezone::UTC))])
            .unwrap();
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
    assert!(ytoml::into_writer_all(std::iter::empty::<&Scalar>(), Vec::new()).is_err());
    assert!(ytoml::into_writer_all([&expected, &expected], Vec::new()).is_err());
    assert!(ytoml::from_utf8_with_limits(source, Limits::new(1, 4, 16, 1)).is_err());
}

#[test]
fn toml_refuses_shapes_its_grammar_cannot_represent() {
    assert!(ytoml::into_bytes(&Scalar::Null).is_err());
    assert!(ytoml::into_bytes(&Scalar::I64(1)).is_err());
    assert!(ytoml::into_bytes(&Scalar::from_record([("missing", Scalar::Null)]).unwrap()).is_err());
    assert!(
        ytoml::into_bytes(&Scalar::from_mapping([(Scalar::I64(1), Scalar::I64(2))]).unwrap())
            .is_err()
    );
}

#[test]
fn the_scalar_entry_points_answer_what_the_explicit_forms_answer() {
    let value = Scalar::from_record([
        ("active", Scalar::Bool(true)),
        ("id", Scalar::I64(7)),
        ("tags", Scalar::from_sequence([Scalar::from("rust")])),
    ])
    .unwrap();
    let encoded = into_toml_scalar(&value).unwrap();
    assert_eq!(encoded, ytoml::into_utf8(&value).unwrap());
    assert_eq!(from_toml_scalar(&encoded).unwrap(), value);
    assert_eq!(ytoml::from_toml_scalar(&encoded).unwrap(), value);

    let text = "id = 7\nname = 'ada'\n";
    let expected = ytoml::from_bytes(text.as_bytes()).unwrap();
    assert_eq!(from_toml_scalar(text).unwrap(), expected);
    let owned_text = String::from(text);
    let owned_bytes = Vec::from(text.as_bytes());
    assert_eq!(from_toml_scalar(owned_text).unwrap(), expected);
    assert_eq!(from_toml_scalar(text.as_bytes()).unwrap(), expected);
    assert_eq!(from_toml_scalar(owned_bytes).unwrap(), expected);

    assert_eq!(
        from_toml_scalar("id =").unwrap_err().to_string(),
        ytoml::from_bytes(b"id =").unwrap_err().to_string()
    );
    assert_eq!(
        into_toml_scalar(&Scalar::I64(1)).unwrap_err().to_string(),
        ytoml::into_utf8(&Scalar::I64(1)).unwrap_err().to_string()
    );
}

#[test]
fn from_toml_scalar_with_field_types_and_orders_as_from_bytes_with_field_does() {
    let input = "clock = '07:32:00.100'\npayload = 'AP8='\nat = 1970-01-01T00:00:00Z\namount = '123.4500'\n";
    let field = typed_row_field();
    let decoded = from_toml_scalar_with_field(input, &field).unwrap();

    assert_eq!(
        decoded,
        ytoml::from_bytes_with_field(input.as_bytes(), &field).unwrap()
    );
    let row = decoded.as_sequence().unwrap();
    assert_eq!(row[0], Scalar::d256(I256::from_str("1234500").unwrap(), 4));
    assert_eq!(
        row[1],
        Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC).unwrap()
    );
    let untyped = from_toml_scalar(input).unwrap();
    assert!(matches!(
        untyped.as_record().unwrap()["amount"],
        Scalar::String(_)
    ));
}

#[test]
fn a_string_naming_an_existing_file_is_toml_content_not_a_path() {
    let path = "Cargo.toml";
    assert!(std::fs::read_to_string(path).unwrap().contains("[package]"));

    let error = from_toml_scalar(path).unwrap_err();
    assert!(
        matches!(error, Error::Codec { format: "toml", .. }),
        "{error}"
    );
    assert!(!error.to_string().contains("[package]"));
    assert_eq!(
        error.to_string(),
        ytoml::from_bytes(path.as_bytes()).unwrap_err().to_string()
    );
}
