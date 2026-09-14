//! The chunked reader an integration test cannot reach.
//!
//! `Reader::new` takes a `Decoder` directly, which is how a transcription is
//! driven at an arbitrary chunk boundary; the public door hands one a whole
//! handle. Everything a caller can observe lives in `tests/charset/codecs.rs`.

use std::io::Read;

use crate::Charset;

#[test]
fn a_transcribing_reader_agrees_with_a_whole_transcription_at_every_split() {
    // The transport the text reader lays over a declared charset: whatever
    // chunks the source answers in, the text is what `transcribe` answers
    // for the whole buffer. A lone surrogate, an odd trailing byte, an
    // unassigned byte and a stray byte among UTF-8 all included, since those
    // are the bytes the two could disagree on.
    let mut utf16 = Charset::Utf16Le
        .encode("Gr\u{fc}\u{df}e ")
        .unwrap()
        .into_owned();
    utf16.extend_from_slice(b"\x00\xd8");
    utf16.extend_from_slice(&Charset::Utf16Le.encode(" \u{1F600} \u{3a9}").unwrap());
    utf16.push(0x41);
    let mut cp1252 = Charset::Cp1252
        .encode("symbol,d\u{e9}sk")
        .unwrap()
        .into_owned();
    cp1252.extend_from_slice(b"\x81 \x80\n");
    let utf8 = b"caf\xe9 caf\xc3\xa9 \x93x\x94 \xe2\x82 z".to_vec();
    // A lead byte broken by another lead, with the sequence the breaker
    // begins completed by what follows: a join that reads the held lead
    // alone leaves the breaker in the carry, and it has to be joined with
    // what follows too rather than overwritten - `C3 C3 A9` is `Ãé`, not
    // `Ã©`, and the lone high surrogate before a pair is `U+FFFD U+10000`.
    assert_eq!(Charset::Utf8.transcribe(b"\xc3\xc3\xa9"), "\u{c3}\u{e9}");
    assert_eq!(
        Charset::Utf8.transcribe(b"\xe2\xe2\x82\xac"),
        "\u{e2}\u{20ac}"
    );
    assert_eq!(
        Charset::Utf16Le.transcribe(b"\x00\xd8\x00\xd8\x00\xdc"),
        "\u{fffd}\u{10000}"
    );
    for (charset, wire) in [
        (Charset::Utf16Le, utf16),
        (Charset::Cp1252, cp1252),
        (Charset::Utf8, utf8),
        (Charset::Utf8, b"\xc3\xc3\xa9".to_vec()),
        (Charset::Utf8, b"\xe2\xe2\x82\xac".to_vec()),
        (Charset::Utf16Le, b"\x00\xd8\x00\xd8\x00\xdc".to_vec()),
    ] {
        let expected = charset.transcribe(&wire).into_owned();
        for split in 0..=wire.len() {
            let source = Split {
                parts: [&wire[..split], &wire[split..]],
                next: 0,
            };
            let mut decoded = String::new();
            super::Reader::new(charset.transcriber(), source)
                .read_to_string(&mut decoded)
                .unwrap();
            assert_eq!(decoded, expected, "{charset} split at {split}");
        }
    }
}

#[test]
fn a_transcribing_reader_ends_a_cut_short_sequence_as_a_replacement() {
    // The one place a chunked reading and a whole one part: bytes still
    // waiting for a continuation when the source ends were never read, so
    // the stream answers `U+FFFD` for them and ends cleanly, where a strict
    // reader fails on the read that reaches the end.
    let mut decoded = String::new();
    super::Reader::new(
        Charset::Utf8.transcriber(),
        std::io::Cursor::new(b"caf\xc3".to_vec()),
    )
    .read_to_string(&mut decoded)
    .unwrap();
    assert_eq!(decoded, "caf\u{FFFD}");

    let mut decoded = String::new();
    super::Reader::new(
        Charset::Utf16Le.transcriber(),
        std::io::Cursor::new(b"A\x00\x00\xd8B".to_vec()),
    )
    .read_to_string(&mut decoded)
    .unwrap();
    assert_eq!(
        decoded, "A\u{FFFD}\u{FFFD}",
        "one for the unpaired surrogate, one for the half unit"
    );
}
/// A source that answers its bytes in two reads, split where the test says.
struct Split<'bytes> {
    parts: [&'bytes [u8]; 2],
    next: usize,
}

impl Read for Split<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        // An empty part is skipped rather than answered: zero is the end of
        // the source, and a split at either edge has an empty part.
        while let Some(part) = self.parts.get(self.next) {
            self.next += 1;
            if !part.is_empty() {
                buffer[..part.len()].copy_from_slice(part);
                return Ok(part.len());
            }
        }
        Ok(0)
    }
}
