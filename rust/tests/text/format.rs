//! `rust/src/text/format.rs`: the format a name, an extension or a media
//! type infers, and the runtime form that answers the same calls as a named
//! one.

use std::path::Path;

use yggdryl::{Format, Scalar};
use yggdryl::{json, text, toml};

#[test]
fn format_names_and_extensions_are_inferred() {
    assert_eq!(Format::from_str("application/json").unwrap(), Format::Json);
    assert_eq!(
        Format::from_extension(".NDJSON").unwrap(),
        Format::JsonLines
    );
    assert_eq!(
        Format::from_path(Path::new("events.jsonl")).unwrap(),
        Format::JsonLines
    );
    assert_eq!(
        Format::from_path(Path::new("schema.YML")).unwrap(),
        Format::Yaml
    );
    assert_eq!(Format::from_str("application/toml").unwrap(), Format::Toml);
    assert_eq!(Format::from_extension(".TOML").unwrap(), Format::Toml);
    assert_eq!(
        Format::from_path(Path::new("pyproject.toml")).unwrap(),
        Format::Toml
    );
    assert!(Format::from_path(Path::new("no-extension")).is_err());
}

#[test]
fn format_serde_uses_stable_tokens() {
    let encoded = serde_json::to_vec(&Format::JsonLines).unwrap();
    assert_eq!(encoded, br#""json_lines""#);
    assert_eq!(
        serde_json::from_slice::<Format>(&encoded).unwrap(),
        Format::JsonLines
    );
    assert_eq!(
        serde_json::from_slice::<Format>(br#""toml""#).unwrap(),
        Format::Toml
    );
}

#[test]
fn format_dispatch_uses_the_same_natural_codec() {
    let value = Scalar::from(42_i64);
    let direct = text::into_bytes(&value, Format::Json).unwrap();
    assert_eq!(text::from_utf8("42", Format::Json).unwrap(), value);
    assert_eq!(
        text::from_utf8_all_with_limits(
            "1\n2\n",
            Format::JsonLines,
            yggdryl::Limits::new(1, 4, 1, 2),
        )
        .unwrap(),
        vec![Scalar::from(1_u64), Scalar::from(2_u64)]
    );
    assert_eq!(json::from_bytes(&direct).unwrap(), value);
    let table = Scalar::from_struct([("value", value)]).unwrap();
    let encoded = toml::into_bytes(&table).unwrap();
    assert_eq!(toml::from_bytes(&encoded).unwrap(), table);
}

mod runtime_format {
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
}

mod mime {

    use yggdryl::{Format, MimeType};

    #[test]
    fn format_and_mime_tables_are_bidirectional_without_alias_drift() {
        for (format, mime) in [
            (Format::Json, MimeType::JSON),
            (Format::JsonLines, MimeType::JSON_LINES),
            (Format::Yaml, MimeType::YAML),
            (Format::Toml, MimeType::TOML),
        ] {
            assert_eq!(format.mime_type(), mime);
            assert_eq!(mime.format(), Some(format));
            assert_eq!(Format::from_str(mime.as_str()).unwrap(), format);
        }
        for (alias, format) in [
            ("json", Format::Json),
            ("jsonl", Format::JsonLines),
            ("ndjson", Format::JsonLines),
            ("json_lines", Format::JsonLines),
            ("json-lines", Format::JsonLines),
            ("yaml", Format::Yaml),
            ("yml", Format::Yaml),
            ("application/x-yaml", Format::Yaml),
            ("toml", Format::Toml),
        ] {
            assert_eq!(Format::from_str(alias).unwrap(), format, "{alias:?}");
        }
    }
}
