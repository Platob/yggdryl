//! Tests for the `Charset` implementations.

use yggdryl_core::{Charset, CharsetError, Latin1, Utf8};

#[test]
fn utf8_round_trips() {
    let text = "héllo 🌳";
    let bytes = Utf8.encode_bytes(text).unwrap();
    assert_eq!(bytes, text.as_bytes());
    assert_eq!(Utf8.decode_bytes(&bytes).unwrap(), text);
}

#[test]
fn utf8_rejects_invalid_bytes() {
    let error = Utf8.decode_bytes(&[0xFF, 0xFF]).unwrap_err();
    assert!(matches!(error, CharsetError::InvalidBytes { .. }));
}

#[test]
fn latin1_round_trips_within_range() {
    let text = "àéîõü";
    let bytes = Latin1.encode_bytes(text).unwrap();
    assert_eq!(bytes.len(), text.chars().count());
    assert_eq!(Latin1.decode_bytes(&bytes).unwrap(), text);
}

#[test]
fn latin1_rejects_out_of_range_char() {
    // 'Ω' is U+03A9, above the Latin-1 range.
    let error = Latin1.encode_bytes("Ω").unwrap_err();
    assert!(matches!(
        error,
        CharsetError::Unrepresentable { ch: 'Ω', .. }
    ));
}
