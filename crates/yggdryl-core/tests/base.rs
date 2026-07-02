//! Round-trip tests for the `Base` trait (behind the `json` feature).
#![cfg(feature = "json")]

use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, BaseError, Latin1, Utf8};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Record {
    name: String,
    tags: Vec<String>,
}

impl Base for Record {}

fn sample() -> Record {
    Record {
        name: "café".to_string(),
        tags: vec!["a".to_string(), "b".to_string()],
    }
}

#[test]
fn json_string_round_trips() {
    let record = sample();
    let json = record.to_json().unwrap();
    assert_eq!(json, r#"{"name":"café","tags":["a","b"]}"#);
    assert_eq!(Record::from_json(&json).unwrap(), record);
}

#[test]
fn bytes_are_compact_utf8_json() {
    let record = sample();
    assert_eq!(
        record.serialize_bytes().unwrap(),
        record.to_json().unwrap().into_bytes()
    );
    assert_eq!(
        Record::deserialize_bytes(&record.serialize_bytes().unwrap()).unwrap(),
        record
    );
}

#[test]
fn indent_pretty_prints_but_round_trips() {
    let record = sample();
    let compact = record.to_bson(None, Utf8).unwrap();
    let pretty = record.to_bson(Some(2), Utf8).unwrap();
    assert!(pretty.len() > compact.len());
    assert!(pretty.starts_with(b"{\n  \"name\""));
    assert_eq!(Record::from_bson(&pretty, Utf8).unwrap(), record);
}

#[test]
fn bson_honours_charset() {
    let record = sample();
    let utf8 = record.to_bson(None, Utf8).unwrap();
    let latin1 = record.to_bson(None, Latin1).unwrap();
    // 'é' is two bytes in UTF-8 but one in Latin-1; everything else is ASCII.
    assert_eq!(latin1.len() + 1, utf8.len());
    assert_eq!(Record::from_bson(&latin1, Latin1).unwrap(), record);
}

#[test]
fn charset_error_surfaces_as_base_error() {
    // 'Ω' (U+03A9) is not representable in Latin-1.
    let record = Record {
        name: "Ω".to_string(),
        tags: vec![],
    };
    let error = record.to_bson(None, Latin1).unwrap_err();
    assert!(matches!(error, BaseError::Charset(_)));
}
