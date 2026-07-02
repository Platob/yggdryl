//! Tests for the `Charset` string/bytes codec.

use yggdryl_core::{Charset, CharsetError};

#[test]
fn utf_variants_round_trip() {
    let text = "héllo 🌳";
    for charset in [Charset::Utf8, Charset::Utf16Le, Charset::Utf16Be] {
        let bytes = charset.encode(text).unwrap();
        assert_eq!(charset.decode(&bytes).unwrap(), text, "{charset:?}");
    }
}

#[test]
fn latin1_round_trips_within_range() {
    let text = "àéîõü";
    let bytes = Charset::Latin1.encode(text).unwrap();
    assert_eq!(bytes.len(), text.chars().count());
    assert_eq!(Charset::Latin1.decode(&bytes).unwrap(), text);
}

#[test]
fn ascii_rejects_out_of_range_char() {
    let error = Charset::Ascii.encode("é").unwrap_err();
    assert!(matches!(
        error,
        CharsetError::Unrepresentable { ch: 'é', .. }
    ));
}

#[test]
fn utf16_rejects_odd_length() {
    let error = Charset::Utf16Le.decode(&[0x00]).unwrap_err();
    assert!(matches!(error, CharsetError::InvalidBytes { .. }));
}
