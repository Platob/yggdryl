//! Round-trip tests for the `Base` trait (behind the `json` feature).
#![cfg(feature = "json")]

use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, BaseError};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

impl Base for Point {
    fn serialize_bytes(&self) -> Result<Vec<u8>, BaseError> {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        Ok(out)
    }

    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, BaseError> {
        let a: [u8; 8] = bytes.try_into().map_err(|_| BaseError::InvalidBytes {
            reason: format!("expected 8 bytes, got {}", bytes.len()),
        })?;
        Ok(Point {
            x: i32::from_le_bytes([a[0], a[1], a[2], a[3]]),
            y: i32::from_le_bytes([a[4], a[5], a[6], a[7]]),
        })
    }
}

#[test]
fn json_round_trips() {
    let p = Point { x: -1, y: 258 };
    let json = p.serialize_json().unwrap();
    assert_eq!(json, r#"{"x":-1,"y":258}"#);
    assert_eq!(Point::deserialize_json(&json).unwrap(), p);
}

#[test]
fn bytes_use_the_implementor_layout_not_json() {
    let p = Point { x: 1, y: 2 };
    let bytes = p.serialize_bytes().unwrap();
    assert_eq!(bytes, vec![1, 0, 0, 0, 2, 0, 0, 0]); // little-endian, not JSON text
    assert_eq!(Point::deserialize_bytes(&bytes).unwrap(), p);
}

#[test]
fn wrong_length_bytes_error() {
    let error = Point::deserialize_bytes(&[0, 1, 2]).unwrap_err();
    assert!(matches!(error, BaseError::InvalidBytes { .. }));
}
