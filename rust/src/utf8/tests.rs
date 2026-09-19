//! The transcriber an integration test cannot reach.
//!
//! `transcribe_into` is the crate-private reading every legacy-charset door
//! goes through, and what it counts is the whole contract. Everything a
//! caller can observe lives in `tests/charset/codecs.rs`.

use super::transcribe_into;

#[test]
fn a_transcription_counts_the_bytes_it_read_as_windows_1252() {
    fn read(input: &[u8]) -> (String, usize) {
        let mut text = String::new();
        let count = transcribe_into(input, &mut text);
        (text, count)
    }
    assert_eq!(read(b"caf\xe9"), (String::from("caf\u{e9}"), 1));
    // A character a byte limit cut in two: the two bytes that are left, not
    // a pending sequence and not `U+FFFD`.
    assert_eq!(read(b"58=\xE2\x82"), (String::from("58=\u{e2}\u{201A}"), 2));
    assert_eq!(read(b"symbol,price"), (String::from("symbol,price"), 0));
    // Valid UTF-8 above US-ASCII is kept, and counts nothing.
    assert_eq!(
        read("Gr\u{fc}\u{df}e \u{20AC}".as_bytes()),
        (String::from("Gr\u{fc}\u{df}e \u{20AC}"), 0)
    );
    assert_eq!(read(b""), (String::new(), 0));
    // It appends: what the target held stays in front of the reading.
    let mut text = String::from("58=");
    assert_eq!(transcribe_into(b"caf\xe9", &mut text), 1);
    assert_eq!(text, "58=caf\u{e9}");
}
