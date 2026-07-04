//! Integration tests for [`StringBuffer`]: the UTF-8 byte surface (`RawIOBase`),
//! the typed `char` view (`IOBase<char>`), and the UTF-8 validation boundary.

use yggdryl_core::{IOBase, IOError, RawIOBase, StringBuffer, Whence};

#[test]
fn holds_utf8_and_counts_bytes_and_chars() {
    let text = StringBuffer::from("hé");
    assert_eq!(text.as_str().unwrap(), "hé");
    assert_eq!(text.as_bytes(), &[b'h', 0xC3, 0xA9]); // 'h' = 1 byte, 'é' = 2
    assert_eq!(text.byte_size(), 3);
    assert_eq!(text.char_len(), 2);
    assert_eq!(IOBase::<char>::size(&text), 2);
    assert!(!text.is_empty());
    assert!(StringBuffer::new().is_empty());

    // Owned round trips.
    assert_eq!(
        StringBuffer::from("abc".to_string()).into_string().unwrap(),
        "abc"
    );
}

#[test]
fn raw_byte_and_bit_surface_delegates_to_bytes() {
    let mut text = StringBuffer::from("hi");
    assert_eq!(text.pread_byte_one(0, Whence::Start).unwrap(), b'h');
    assert!(!text.pread_bit_one(0, Whence::Start).unwrap()); // MSB of 'h' (0x68) is 0

    // Append raw bytes at the end, then read back.
    text.pwrite_byte_array(0, Whence::End, b"!").unwrap();
    assert_eq!(text.as_str().unwrap(), "hi!");
    assert_eq!(text.byte_size(), 3);
}

#[test]
fn typed_char_view_writes_utf8_and_sizes_in_chars() {
    let mut text = StringBuffer::new();
    // Writing chars appends their UTF-8 encodings at the given byte offset.
    text.pwrite_one(0, Whence::Start, &'A').unwrap();
    text.pwrite_array(text.byte_size(), Whence::Start, &['é', '中'])
        .unwrap();
    assert_eq!(text.as_str().unwrap(), "Aé中");
    assert_eq!(IOBase::<char>::size(&text), 3); // three chars...
    assert_eq!(text.byte_size(), 1 + 2 + 3); // ...six bytes

    // value_to_bytes encodes one char.
    assert_eq!(
        IOBase::<char>::value_to_bytes(&text, &'中'),
        vec![0xE4, 0xB8, 0xAD]
    );
}

#[test]
fn char_resize_truncates_and_pads_on_char_boundaries() {
    let mut text = StringBuffer::from("héllo"); // 5 chars, 6 bytes
    assert_eq!(text.char_len(), 5);

    // Truncate to two chars: "hé" (3 bytes), not a byte cut through 'é'.
    IOBase::<char>::resize(&mut text, 2).unwrap();
    assert_eq!(text.as_str().unwrap(), "hé");
    assert_eq!(text.byte_size(), 3);

    // Pad to four chars with NUL chars (one byte each).
    IOBase::<char>::resize(&mut text, 4).unwrap();
    assert_eq!(IOBase::<char>::size(&text), 4);
    assert_eq!(text.as_str().unwrap(), "hé\0\0");
}

#[test]
fn invalid_utf8_is_an_actionable_error() {
    let mut text = StringBuffer::new();
    // A lone continuation byte is not valid UTF-8.
    text.pwrite_byte_array(0, Whence::Start, &[b'a', 0xFF])
        .unwrap();
    assert!(matches!(
        text.as_str(),
        Err(IOError::InvalidUtf8 { offset: 1 })
    ));
    // char_len falls back to the byte length for non-UTF-8 content.
    assert_eq!(text.char_len(), 2);
    // A char resize needs valid UTF-8.
    assert!(matches!(
        IOBase::<char>::resize(&mut text, 1),
        Err(IOError::InvalidUtf8 { .. })
    ));
}
