//! `rust/src/text/reader.rs`: one shared window, cut into physical lines.
//!
//! The window is an allocation strategy rather than a contract - a caller
//! reads lines and never the page they are ranges of - so the splitter and
//! its window size are reached through `yggdryl::internals`.

use std::io::Read;

use yggdryl::internals::text_reader::{Lines, WINDOW_SIZE};
use yggdryl::text::LineSep;

struct Chunked {
    bytes: std::io::Cursor<Vec<u8>>,
    size: usize,
}

impl Read for Chunked {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        let size = target.len().min(self.size);
        self.bytes.read(&mut target[..size])
    }
}

fn read(input: &[u8], linesep: Option<&LineSep>) -> Vec<Vec<u8>> {
    let mut lines = Lines::new(std::io::Cursor::new(input));
    let mut values = Vec::new();
    while let Some(line) = lines.next_line(linesep) {
        values.push(line.unwrap());
    }
    values
}

#[test]
fn flexible_and_pinned_terminators_stream_lines() {
    assert_eq!(
        read(b"a\r\nb\nc\rd", None),
        [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]
    );
    assert_eq!(
        read(b"a\nb\r\nc", Some(&LineSep::CRLF)),
        [b"a\nb".to_vec(), b"c".to_vec()]
    );
    assert_eq!(read(b"\xef\xbb\xbfa\n", None), [b"\xef\xbb\xbfa".to_vec()]);
}

#[test]
fn terminators_can_cross_short_source_reads() {
    let mut flexible = Lines::new(Chunked {
        bytes: std::io::Cursor::new(b"a\r\nb\rc\nlast".to_vec()),
        size: 1,
    });
    let mut values = Vec::new();
    while let Some(line) = flexible.next_line(None) {
        values.push(line.unwrap());
    }
    assert_eq!(
        values,
        [
            b"a".to_vec(),
            b"b".to_vec(),
            b"c".to_vec(),
            b"last".to_vec()
        ]
    );

    let mut pinned = Lines::new(Chunked {
        bytes: std::io::Cursor::new(b"a\r\nb\r\nc".to_vec()),
        size: 1,
    });
    let mut values = Vec::new();
    while let Some(line) = pinned.next_line(Some(&LineSep::CRLF)) {
        values.push(line.unwrap());
    }
    assert_eq!(values, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
}

#[test]
fn a_line_larger_than_the_window_is_returned_as_bounded_parts() {
    let mut input = vec![b'x'; WINDOW_SIZE * 3 + 7];
    input.extend_from_slice(b"\nnext");
    let mut lines = Lines::new(std::io::Cursor::new(input));
    let mut sizes = Vec::new();
    loop {
        let part = lines.next_part(None).unwrap().unwrap();
        sizes.push(part.len());
        if part.end {
            break;
        }
    }
    assert!(sizes.len() >= 3);
    assert!(sizes.iter().all(|size| *size <= WINDOW_SIZE));
    assert_eq!(lines.next_line(None).unwrap().unwrap(), b"next");
}
