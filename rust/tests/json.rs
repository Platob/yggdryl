use std::io::{Cursor, Read};
use std::str::FromStr;

use yggdryl::text::{self, Format, Formatting, Limits};
use yggdryl::{DataType, Error, Field, I256, Scalar, TimeUnit, Timezone, json};

struct OneByte<R>(R);

impl<R: Read> Read for OneByte<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let length = output.len().min(1);
        self.0.read(&mut output[..length])
    }
}

#[test]
fn natural_output_is_an_ordinary_json_document() {
    let value = Scalar::from_record([
        ("active", Scalar::Bool(true)),
        ("id", Scalar::I64(7)),
        ("tags", Scalar::from_sequence([Scalar::from("rust")])),
    ])
    .unwrap();
    let encoded = json::into_utf8(&value).unwrap();

    assert_eq!(encoded, r#"{"active":true,"id":7,"tags":["rust"]}"#);
    let foreign: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(foreign["id"], 7);
    assert_eq!(json::from_utf8(&encoded).unwrap(), value);
}

#[test]
fn untyped_reads_return_only_what_json_proves() {
    let value =
        json::from_utf8(r#"{"amount":"123.4500","at":"1970-01-01T00:00:00Z","payload":"AP8="}"#)
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
fn a_field_restores_exact_natural_types_and_record_order() {
    let input = r#"{
        "payload":"AP8=",
        "clock":"07:32:00.100",
        "amount":"123.4500",
        "at":"1970-01-01T00:00:00Z"
    }"#;
    let decoded = json::from_utf8_with_field(input, &typed_row_field()).unwrap();
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
fn time_of_day_is_naive_and_zoned_text_is_refused() {
    for (field, value) in [
        (
            Field::new(
                "clock",
                DataType::time32(TimeUnit::Millisecond).unwrap(),
                false,
            ),
            Scalar::time32(1_500, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
        ),
        (
            Field::new(
                "clock",
                DataType::time64(TimeUnit::Nanosecond).unwrap(),
                false,
            ),
            Scalar::time64(1_500_000_000, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        ),
    ] {
        let encoded = json::into_utf8(&value).unwrap();
        assert_eq!(json::from_utf8_with_field(&encoded, &field).unwrap(), value);
    }

    let field = Field::new("clock", DataType::time32(TimeUnit::Second).unwrap(), false);
    assert!(
        json::from_utf8_with_field("\"00:00:00+02:00\"", &field)
            .unwrap_err()
            .to_string()
            .contains("DateTime64")
    );
    assert!(Scalar::time32(0, TimeUnit::Second, Timezone::UTC).is_err());
    let invalid = Scalar::Time32(0, TimeUnit::Second, Timezone::UTC);
    assert!(
        json::into_bytes(&invalid)
            .unwrap_err()
            .to_string()
            .contains("DateTime64")
    );
}

#[test]
fn bytes_readers_writers_and_streams_are_equivalent() {
    let source = br#"{"label":"caf\u00e9","value":1}"#;
    let expected = json::from_bytes(source).unwrap();
    assert_eq!(
        json::from_reader(OneByte(Cursor::new(source))).unwrap(),
        expected
    );

    let mut output = Vec::new();
    json::into_writer(&expected, &mut output).unwrap();
    assert_eq!(json::from_bytes(&output).unwrap(), expected);

    let values = json::from_bytes_all(b"1 true {\"id\":2}").unwrap();
    assert_eq!(values.len(), 3);
    let mut reader = Cursor::new(b"1 2 3".as_slice());
    assert_eq!(
        json::from_reader_iter(&mut reader)
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap(),
        [Scalar::U64(1), Scalar::U64(2), Scalar::U64(3)]
    );
}

#[test]
fn json_lines_are_strict_and_dispatch_as_one_sequence() {
    let values = json::from_lines_utf8("1\r\n\n{\"x\":2}\n").unwrap();
    assert_eq!(values.len(), 2);
    assert!(json::from_lines_utf8("1 2\n").is_err());
    assert_eq!(
        text::from_utf8("1\n2\n", Format::JsonLines).unwrap(),
        Scalar::from_sequence([Scalar::U64(1), Scalar::U64(2)])
    );
}

#[test]
fn limits_and_invalid_inputs_fail_at_the_boundary() {
    assert!(json::from_bytes_with_limits(b"[[[0]]]", Limits::new(2, 64, 16, 1)).is_err());
    assert!(json::from_bytes_with_limits(b"[0,1,2]", Limits::new(3, 64, 3, 1)).is_err());
    assert!(json::from_bytes(b"1 2").is_err());
    assert!(json::from_bytes(b"\xff").is_err());
    assert!(json::into_bytes(&Scalar::from(f64::NAN)).is_err());
}

#[test]
fn formatting_changes_layout_not_meaning() {
    let value = Scalar::from_record([("id", Scalar::I64(1))]).unwrap();
    let pretty = json::into_utf8_with_formatting(&value, Formatting::indented(2)).unwrap();
    assert!(pretty.contains("\n  \"id\": 1\n"));
    assert_eq!(json::from_utf8(&pretty).unwrap(), value);
}

#[test]
fn codec_errors_keep_the_format_and_byte_position() {
    match json::from_utf8("{\"ok\":1, bad}").unwrap_err() {
        Error::Codec {
            format, position, ..
        } => {
            assert_eq!(format, "json");
            assert!(position > 0);
        }
        error => panic!("unexpected error: {error}"),
    }
}
