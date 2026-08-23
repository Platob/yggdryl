//! One enum over every structured text format, chosen at runtime.

use super::Text;
use crate::text::{Format, Json, TextCodec};
use crate::{MimeType, Url};

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
fn the_enum_answers_the_same_calls_as_a_named_format() {
    let expected = Json.from_utf8(r#"{"symbol":"AAPL"}"#).unwrap();
    let text = Text::Json;

    assert_eq!(
        text.from_utf8(&text.into_utf8(&expected).unwrap()).unwrap(),
        expected
    );
    assert_eq!(
        text.from_bytes(&text.into_bytes(&expected).unwrap())
            .unwrap(),
        expected
    );
    assert!(!text.is_multi_document());
    assert!(Text::Jsonl.is_multi_document());
}
