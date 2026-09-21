//! `rust/src/text/position.rs`: the byte offsets a parser's refusal names.
//!
//! A caller reads the offset in the message; the line-and-column arithmetic
//! and the bounded window of recent line starts behind it are the crate's
//! own, so both are reached through `yggdryl::internals`.

use yggdryl::internals::text_position::{LineOffsets, line_column_to_byte_offset};

#[test]
fn positions_are_bounded_byte_offsets() {
    let input = b"one\ntwo\nthree";
    assert_eq!(line_column_to_byte_offset(input, 2, 2), 5);
    assert_eq!(line_column_to_byte_offset(input, 2, 1), 4);
    assert_eq!(line_column_to_byte_offset(input, 9, 1), input.len());
    assert_eq!(line_column_to_byte_offset(input, 9, 9), input.len());

    let mut offsets = LineOffsets::new(2);
    offsets.observe(b"one\ntwo\n");
    offsets.observe(b"three");
    assert_eq!(offsets.position(2, 2), 5);
    assert_eq!(offsets.position(1, 1), input.len());
}
