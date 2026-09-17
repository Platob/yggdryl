//! One enum over every structured text format, chosen at run time.
//!
//! `Format` holds this role. It used to be split across `Format` and a second
//! enum, `Structured`, carrying the same four formats under a second set of
//! spellings with the bijection written in both directions.

use yggdryl::text::{Format, Json, TextCodec};
use yggdryl::{MimeType, Url};

#[test]
fn a_name_picks_the_format() {
    for (name, expected) in [
        ("file:///t.json", Format::Json),
        ("file:///t.jsonl.gz", Format::JsonLines),
        ("file:///t.yaml", Format::Yaml),
        ("file:///t.toml", Format::Toml),
    ] {
        assert_eq!(
            Format::from_url(&Url::from_str(name).unwrap()).unwrap(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn a_name_that_is_not_a_text_format_is_reported() {
    let message = Format::from_url(&Url::from_str("file:///t.parquet").unwrap())
        .unwrap_err()
        .to_string();
    assert!(message.contains("expected one of"), "{message}");
    assert!(message.contains("application/json"), "{message}");
}

#[test]
fn a_mime_type_round_trips_through_every_format() {
    for format in Format::ALL {
        assert_eq!(Format::from_mime_type(&format.mime_type()).unwrap(), format);
        assert_eq!(TextCodec::format(&format), format);
    }
    assert_eq!(Format::Json.mime_type(), MimeType::JSON);
    assert_eq!(Format::default(), Format::Json);
}

#[test]
fn a_runtime_format_answers_the_same_calls_as_a_named_one() {
    let expected = Json.from_utf8(r#"{"symbol":"AAPL"}"#).unwrap();
    let text = Format::Json;

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
    assert!(Format::JsonLines.is_multi_document());
}
