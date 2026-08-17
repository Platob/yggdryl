//! One enum over every structured text format, chosen at runtime.

use super::Text;
use crate::io::{Buffer, IOBase};
use crate::text::{Format, Json, TextCodec};
use crate::{MimeType, Url, Value};

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

fn value() -> Value {
    Json.loads(r#"{"symbol":"AAPL"}"#).unwrap()
}

#[test]
fn a_name_picks_the_format() {
    assert_eq!(
        Text::for_url(&Url::from_str("file:///t.json").unwrap()).unwrap(),
        Text::Json
    );
    assert_eq!(
        Text::for_url(&Url::from_str("file:///t.jsonl.gz").unwrap()).unwrap(),
        Text::Jsonl
    );
    assert_eq!(
        Text::for_url(&Url::from_str("file:///t.yaml").unwrap()).unwrap(),
        Text::Yaml
    );
    assert_eq!(
        Text::for_url(&Url::from_str("file:///t.toml").unwrap()).unwrap(),
        Text::Toml
    );
}

#[test]
fn a_name_that_is_not_a_text_format_is_reported() {
    let message = Text::for_url(&Url::from_str("file:///t.parquet").unwrap())
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected one of"), "{message}");
    assert!(message.contains("application/json"), "{message}");
}

#[test]
fn the_enum_and_the_format_types_agree() {
    for format in Format::ALL {
        let text = Text::from_format(format);
        assert_eq!(text.format(), format);
        assert_eq!(Text::for_mime_type(&format.mime_type()).unwrap(), text);
    }

    // A concrete format type converts into the enum without restating anything.
    assert_eq!(Text::from(Json), Text::Json);
    assert_eq!(Text::Json.mime_type(), MimeType::JSON);
}

#[test]
fn an_inferred_round_trip_applies_the_format_and_the_coding() {
    let expected = value();

    for name in ["quote.json", "quote.json.gz", "quote.yaml", "quote.toml"] {
        let mut target = handle(name);
        Text::dump_inferred(&mut target, &expected)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = Text::load_inferred(&target).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(actual, expected, "{name}");
    }

    // The compressed handle really is compressed.
    let mut coded = handle("quote.json.gz");
    Text::dump_inferred(&mut coded, &expected).unwrap();
    assert_eq!(coded.read_range(0, 2).unwrap(), [0x1F, 0x8B]);
}

#[test]
fn the_enum_answers_the_same_calls_as_a_named_format() {
    let expected = value();
    let text = Text::Json;

    assert_eq!(
        text.loads(&text.dumps(&expected).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        text.load_slice(&text.dump_vec(&expected).unwrap()).unwrap(),
        expected
    );
    assert!(!text.is_multi_document());
    assert!(Text::Jsonl.is_multi_document());
}
