use std::path::Path;

use yggdryl::{Format, Value, json, text, toml};

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
fn codec_namespace_remains_a_thin_compatible_facade() {
    let value = Value::from(42_i64);
    let direct = text::to_vec(&value, Format::Json).unwrap();
    let compatible = text::to_vec(&value, Format::Json).unwrap();
    assert_eq!(compatible, direct);
    assert_eq!(text::from_str("42", Format::Json).unwrap(), value);
    assert_eq!(
        text::from_str_all_with_limits(
            "1\n2\n",
            Format::JsonLines,
            yggdryl::Limits::new(1, 4, 1, 2),
        )
        .unwrap(),
        vec![Value::from(1_u64), Value::from(2_u64)]
    );
    assert_eq!(json::from_slice(&compatible).unwrap(), value);
    let toml = toml::to_vec(&value).unwrap();
    assert_eq!(toml::from_slice(&toml).unwrap(), value);
}
