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
fn format_dispatch_uses_the_same_natural_codec() {
    let value = Value::from(42_i64);
    let direct = text::into_bytes(&value, Format::Json).unwrap();
    assert_eq!(text::from_utf8("42", Format::Json).unwrap(), value);
    assert_eq!(
        text::from_utf8_all_with_limits(
            "1\n2\n",
            Format::JsonLines,
            yggdryl::Limits::new(1, 4, 1, 2),
        )
        .unwrap(),
        vec![Value::from(1_u64), Value::from(2_u64)]
    );
    assert_eq!(json::from_bytes(&direct).unwrap(), value);
    let table = Value::from_record([("value", value)]).unwrap();
    let encoded = toml::into_bytes(&table).unwrap();
    assert_eq!(toml::from_bytes(&encoded).unwrap(), table);
}
