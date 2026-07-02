//! Round-trip tests for the `Base` trait (behind the `json` feature).
#![cfg(feature = "json")]

use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, BaseError};

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
    let json = record.serialize_json().unwrap();
    assert_eq!(json, r#"{"name":"café","tags":["a","b"]}"#);
    assert_eq!(Record::deserialize_json(&json).unwrap(), record);
}

#[test]
fn bytes_are_compact_utf8_json() {
    let record = sample();
    assert_eq!(
        record.serialize_bytes().unwrap(),
        record.serialize_json().unwrap().into_bytes()
    );
    assert_eq!(
        Record::deserialize_bytes(&record.serialize_bytes().unwrap()).unwrap(),
        record
    );
}

#[test]
fn invalid_utf8_bytes_surface_as_base_error() {
    // 0xFF is not valid UTF-8, so decoding the byte form fails as a charset error.
    let error = Record::deserialize_bytes(&[0xFF, 0xFF]).unwrap_err();
    assert!(matches!(error, BaseError::Charset(_)));
}
