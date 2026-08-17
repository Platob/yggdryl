//! Every format answers the same four pairs of questions.

use super::{Json, Jsonl, TextCodec, Toml, Yaml};
use crate::io::{Buffer, IOBase};
use crate::text::Value;
use crate::{MimeType, Url};

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

fn value() -> Value {
    Json.loads(r#"{"symbol":"AAPL","price":1.5}"#).unwrap()
}

#[test]
fn a_value_round_trips_through_text_bytes_and_readers() {
    let expected = value();

    for (label, text) in [
        ("json", Json.dumps(&expected).unwrap()),
        ("toml", Toml.dumps(&expected).unwrap()),
        ("yaml", Yaml.dumps(&expected).unwrap()),
    ] {
        let parsed = match label {
            "json" => Json.loads(&text).unwrap(),
            "toml" => Toml.loads(&text).unwrap(),
            _ => Yaml.loads(&text).unwrap(),
        };
        assert_eq!(parsed, expected, "{label}");
    }

    // Bytes and readers are the same operation on a different carrier.
    let bytes = Json.dump_vec(&expected).unwrap();
    assert_eq!(Json.load_slice(&bytes).unwrap(), expected);
    assert_eq!(Json.read(bytes.as_slice()).unwrap(), expected);
}

#[test]
fn a_handle_round_trips_and_applies_its_own_coding() {
    let expected = value();

    let mut plain = handle("quote.json");
    Json.dump(&mut plain, &expected).unwrap();
    assert_eq!(Json.load(&plain).unwrap(), expected);
    assert_eq!(&plain.read_range(0, 1).unwrap(), b"{");

    let mut compressed = handle("quote.json.gz");
    Json.dump(&mut compressed, &expected).unwrap();
    assert_eq!(Json.load(&compressed).unwrap(), expected);
    // The bytes really are compressed, and reading decompresses them.
    assert_eq!(compressed.read_range(0, 2).unwrap(), [0x1F, 0x8B]);
    assert!(compressed.size() != plain.size());
}

#[test]
fn newline_delimited_json_holds_one_value_per_line() {
    let values = vec![
        Json.loads(r#"{"id":1}"#).unwrap(),
        Json.loads(r#"{"id":2}"#).unwrap(),
    ];

    let text = Jsonl.dumps_all_to_string(&values);
    assert_eq!(text.lines().count(), 2);
    assert_eq!(Jsonl.loads_all(&text).unwrap(), values);

    let mut lines = handle("rows.jsonl");
    Jsonl.dump_all(&mut lines, &values).unwrap();
    assert_eq!(Jsonl.load_all(&lines).unwrap(), values);
    assert!(Jsonl.is_multi_document());
    assert!(!Json.is_multi_document());
}

#[test]
fn each_format_reports_the_media_type_it_writes() {
    assert_eq!(Json.mime_type(), MimeType::JSON);
    assert_eq!(Jsonl.mime_type(), MimeType::JSON_LINES);
    assert_eq!(Yaml.mime_type(), MimeType::YAML);
    assert_eq!(Toml.mime_type(), MimeType::TOML);
}

#[test]
fn a_missing_resource_is_empty_rather_than_an_error() {
    // Reading nothing is not a value, so the parser says so; the read itself
    // does not fail.
    let absent = handle("absent.json");
    assert_eq!(absent.size(), 0);
    assert!(Json.load(&absent).is_err());

    // A multi-document format simply has no documents.
    let absent = handle("absent.jsonl");
    assert!(Jsonl.load_all(&absent).unwrap().is_empty());
}

/// A shorthand the newline-delimited test uses.
trait DumpAllToString {
    fn dumps_all_to_string(&self, values: &[Value]) -> String;
}

impl<T: TextCodec> DumpAllToString for T {
    fn dumps_all_to_string(&self, values: &[Value]) -> String {
        String::from_utf8(self.dump_vec_all(values).unwrap()).unwrap()
    }
}
