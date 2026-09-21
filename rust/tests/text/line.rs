//! `rust/src/text/line.rs`: one decode per line, and what it counted.
//!
//! A line reads its bytes once, on the page they were read into, and answers
//! how many of them were not UTF-8. The decode step itself is private, so it
//! is reached through `yggdryl::internals`; everything else here is the
//! `TextLine` a caller holds.

use std::sync::Arc;

use yggdryl::internals::text_line::decoded;
use yggdryl::text::{TextBytes, TextLine, TextOptions};

fn line(bytes: &[u8]) -> TextLine {
    TextLine::from_bytes(
        0,
        TextBytes::from_bytes(bytes).expect("a page"),
        Arc::new(TextOptions::new()),
    )
    .expect("a line")
}

#[test]
fn text_as_read_is_the_range_it_was_read_into() {
    let page = TextBytes::from_bytes("8=FIX.4.4|58=caf\u{e9}|10=0|".as_bytes()).expect("a page");
    let (body, count) = decoded(page.clone()).expect("text");
    assert_eq!(count, 0);
    assert!(Arc::ptr_eq(body.page().unwrap(), page.page().unwrap()));
    assert_eq!((body.start(), body.end()), (page.start(), page.end()));
}

#[test]
fn one_latin_1_byte_among_utf_8_decodes_alone() {
    // `caf\xE9` beside a UTF-8 `\u{e9}`: the valid run is kept, and the
    // lone byte reads as the one character it is.
    let read = line(b"58=caf\xE9 caf\xC3\xA9|10=0|");
    assert_eq!(read.body(), "58=caf\u{e9} caf\u{e9}|10=0|");
    assert_eq!(read.decoded_byte_size(), 1);
}

#[test]
fn a_wholly_windows_1252_line_reads_byte_for_byte() {
    let read = line(b"\x80 \x93quoted\x94 \x96 na\xEFve");
    assert_eq!(
        read.body(),
        "\u{20AC} \u{201C}quoted\u{201D} \u{2013} na\u{ef}ve"
    );
    assert_eq!(read.decoded_byte_size(), 5);
}

#[test]
fn the_five_undefined_bytes_read_as_the_controls_of_their_number() {
    for byte in [0x81_u8, 0x8D, 0x8F, 0x90, 0x9D] {
        assert!(yggdryl::Charset::Cp1252.scalar_of(byte).is_none());
        let read = line(&[b'a', byte, b'b']);
        assert_eq!(read.body(), format!("a{}b", byte as char));
        assert_eq!(read.decoded_byte_size(), 1);
    }
}

#[test]
fn a_character_cut_in_two_reads_as_the_bytes_that_are_left() {
    // The first two bytes of a three-byte `\u{20AC}`, as a byte limit
    // would leave them: not `U+FFFD`, the two characters those bytes are.
    let read = line(b"58=\xE2\x82");
    assert_eq!(read.body(), "58=\u{e2}\u{201A}");
    assert_eq!(read.decoded_byte_size(), 2);
}

#[test]
fn stated_captures_take_the_same_decode_and_make_the_body_the_payload() {
    let mut read = line(b"body \xE9");
    read.set_captures(vec![
        Some(TextBytes::from_bytes(b"caf\xE9").expect("a page")),
        None,
        Some(TextBytes::from_bytes(b"plain").expect("a page")),
    ])
    .expect("captures");
    assert_eq!(read.capture(0), Some("caf\u{e9}"));
    assert_eq!(read.capture(1), None);
    assert_eq!(read.capture(2), Some("plain"));
    assert_eq!(read.capture(3), None);
    assert_eq!(read.decoded_byte_size(), 1, "the body's own count");
    assert_eq!(
        read.payload_bytes().as_bytes(),
        read.body_bytes().as_bytes()
    );
    read.set_body(TextBytes::from_bytes(b"clean").expect("a page"))
        .expect("a body");
    assert_eq!(read.body(), "clean");
    assert_eq!(read.decoded_byte_size(), 0);
    assert_eq!(read.capture(0), Some("caf\u{e9}"), "a stated fact stands");
}
