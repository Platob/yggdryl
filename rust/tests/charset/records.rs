//! Record reads over payloads that are not UTF-8.
//!
//! The charset a line is read in is what its handle declares (decision 12):
//! the record reader lays the declaration over its transport, below the line
//! splitter, and the writer follows the same declaration back out.

use arrow_array::{Array as _, RecordBatch, StringArray};
use std::sync::Arc;
use yggdryl::charset::Transcoded;
use yggdryl::holder::Buffer;
use yggdryl::media::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::{Charset, IOBase, IOMedia as _, MediaType};

/// The string column `name` holds, across every batch.
fn strings(batches: &[arrow_array::RecordBatch], name: &str) -> Vec<Option<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of(name).expect("a declared column");
            batch
                .column(index)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("a string column")
                .iter()
                .map(|value| value.map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn read(source: &impl yggdryl::IOBase, options: TextOptions) -> Vec<arrow_array::RecordBatch> {
    source
        .read_arrow_reader(&options.into())
        .expect("a reader")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("batches")
}

/// The lines a text read answers, through the line door.
fn lines(source: &impl yggdryl::IOBase, options: &TextOptions) -> Vec<TextLine> {
    read_text_lines(source, options)
        .expect("a reader")
        .map(|line| line.expect("a line"))
        .collect()
}

/// A buffer holding `wire`, declaring `media_type`.
fn declared(media_type: &str, wire: Vec<u8>) -> Buffer {
    Buffer::from_bytes(wire).with_media_type(MediaType::from_str(media_type).expect("a media type"))
}

/// A row header capturing the first word as a Unicode class matches it.
fn city_header() -> TextOptions {
    TextOptions::new()
        .try_with_rowheader(r"^(?<city>\S+) ")
        .expect("a header")
}

/// The record write's input: one `body` per row, under the `url` a text row
/// starts with.
fn rows(bodies: &[&str]) -> RecordBatch {
    let schema = Arc::new(arrow_schema::Schema::new(vec![
        arrow_schema::Field::new("url", arrow_schema::DataType::Utf8, false),
        arrow_schema::Field::new("body", arrow_schema::DataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["input"; bodies.len()])),
            Arc::new(StringArray::from(bodies.to_vec())),
        ],
    )
    .expect("a batch")
}

#[test]
fn a_declared_charset_is_read_below_the_splitter_and_an_undeclared_one_is_the_wire() {
    let text = "\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430} first\nLondon second\n";
    let wire = Charset::Cp1251.encode(text).expect("Cyrillic").into_owned();

    // Declared: the row header sees the declared text, so a Unicode class
    // matches the Cyrillic word, and the capture and the body are the text.
    let source = declared("text/plain;charset=windows-1251", wire.clone());
    let batches = read(&source, city_header());
    assert_eq!(
        strings(&batches, "city"),
        [
            Some("\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430}".to_owned()),
            Some("London".to_owned())
        ]
    );
    assert_eq!(
        strings(&batches, "body"),
        [Some("first".to_owned()), Some("second".to_owned())]
    );
    for line in lines(&source, &city_header()) {
        assert_eq!(
            line.decoded_byte_size(),
            0,
            "read as declared, nothing repaired"
        );
    }

    // Undeclared: the same bytes are the wire, read by rule one - never
    // refused, each stray byte as the Windows-1252 character it is - and a
    // Unicode class matches no byte that is not a character, so the header
    // does not match and the whole line is the body. The line counts the six
    // bytes it read that way.
    let source = declared("text/plain", wire);
    let read_lines = lines(&source, &city_header());
    assert_eq!(read_lines.len(), 2);
    assert_eq!(
        read_lines[0].body(),
        "\u{cc}\u{ee}\u{f1}\u{ea}\u{e2}\u{e0} first"
    );
    assert_eq!(read_lines[0].capture(0), None);
    assert_eq!(read_lines[0].decoded_byte_size(), 6);
    assert_eq!(read_lines[1].body(), "second");
    assert_eq!(read_lines[1].capture(0), Some("London"));
    assert_eq!(read_lines[1].decoded_byte_size(), 0);
}

#[test]
fn two_latin_1_letters_are_told_from_one_utf_8_scalar_by_the_declaration() {
    // What decision 10 could not tell and decision 12 can: `C3 A9` is one
    // UTF-8 `é` where nothing is declared, and two windows-1252 letters where
    // windows-1252 is.
    let wire = b"caf\xc3\xa9\n".to_vec();
    let source = declared("text/plain;charset=windows-1252", wire.clone());
    assert_eq!(
        strings(&read(&source, TextOptions::new()), "body"),
        [Some("caf\u{c3}\u{a9}".to_owned())]
    );
    let source = declared("text/plain", wire);
    assert_eq!(
        strings(&read(&source, TextOptions::new()), "body"),
        [Some("caf\u{e9}".to_owned())]
    );
}

#[test]
fn a_declared_line_reaches_its_row_as_text_with_nothing_repaired() {
    let text = "Z\u{fc}rich premi\u{e8}r\n";
    let source = declared(
        "text/plain;charset=windows-1252",
        Charset::Cp1252.encode(text).expect("Latin").into_owned(),
    );
    let batches = read(&source, city_header());
    assert_eq!(
        batches[0]
            .schema()
            .field_with_name("body")
            .expect("a body")
            .data_type(),
        &arrow_schema::DataType::Utf8
    );
    assert_eq!(strings(&batches, "body"), [Some("premi\u{e8}r".to_owned())]);
    assert_eq!(strings(&batches, "city"), [Some("Z\u{fc}rich".to_owned())]);
    let without_header = lines(&source, &TextOptions::new());
    assert_eq!(without_header[0].body(), "Z\u{fc}rich premi\u{e8}r");
    for line in without_header
        .iter()
        .chain(lines(&source, &city_header()).iter())
    {
        assert_eq!(line.decoded_byte_size(), 0);
    }
}

#[test]
fn an_unassigned_byte_under_a_declared_windows_page_is_its_c1_control() {
    // The transport transcribes and never refuses: `0x81` is unassigned in
    // windows-1252, and reads as `U+0081` as the layer reads it everywhere.
    let source = declared(
        "text/plain;charset=windows-1252",
        b"ok\x81\nnext\n".to_vec(),
    );
    assert_eq!(
        strings(&read(&source, TextOptions::new()), "body"),
        [Some("ok\u{81}".to_owned()), Some("next".to_owned())]
    );
}

#[test]
fn a_utf_16_capture_splits_into_rows_with_its_mark_gone() {
    let text = "Z\u{fc}rich first\nLondon second\n";
    let encoded = Charset::Utf16Le.encode(text).expect("UTF-16").into_owned();
    let mut marked = b"\xff\xfe".to_vec();
    marked.extend_from_slice(&encoded);

    // With the mark: it names the declared form, so it is framing and comes
    // off; without it the rows read the same.
    for wire in [marked, encoded.clone()] {
        let source = declared("text/plain;charset=utf-16le", wire);
        let batches = read(&source, city_header());
        assert_eq!(
            strings(&batches, "city"),
            [Some("Z\u{fc}rich".to_owned()), Some("London".to_owned())]
        );
        assert_eq!(
            strings(&batches, "body"),
            [Some("first".to_owned()), Some("second".to_owned())]
        );
        for line in lines(&source, &TextOptions::new()) {
            assert!(!line.body().contains('\u{feff}'), "{:?}", line.body());
        }
    }

    // A mark for another form is data: the declaration wins, and the bytes
    // are read as the code unit they are under it.
    let mut other = b"\xfe\xff".to_vec();
    other.extend_from_slice(&encoded);
    let source = declared("text/plain;charset=utf-16le", other);
    let read_lines = lines(&source, &TextOptions::new());
    assert_eq!(read_lines[0].body(), "\u{fffe}Z\u{fc}rich first");

    // An odd trailing byte is half a code unit the source cut short: the
    // stream answers `U+FFFD` for it and refuses nothing.
    let mut odd = Charset::Utf16Le
        .encode("Z\u{fc}rich first\nLondon second")
        .expect("UTF-16")
        .into_owned();
    odd.push(0x41);
    let source = declared("text/plain;charset=utf-16le", odd);
    let read_lines = lines(&source, &TextOptions::new());
    assert_eq!(read_lines.len(), 2);
    assert_eq!(read_lines[1].body(), "London second\u{fffd}");
}

#[test]
fn a_mis_declared_handle_reads_the_mojibake_it_declares() {
    // The hazard, pinned as a fact: a stale `charset=iso-8859-1` over UTF-8
    // bytes reads by declaration. The override is the handle's media type,
    // or `Transcoded::new(handle, Charset::Utf8)`.
    let source = declared(
        "text/plain;charset=iso-8859-1",
        "caf\u{e9}\n".as_bytes().to_vec(),
    );
    let read_lines = lines(&source, &TextOptions::new());
    assert_eq!(read_lines[0].body(), "caf\u{c3}\u{a9}");
    assert_eq!(read_lines[0].decoded_byte_size(), 0);
}

#[test]
fn a_declared_us_ascii_handle_is_never_wrapped_and_reads_by_rule_one() {
    // A US-ASCII declaration is a UTF-8 declaration with a narrower promise,
    // and a broken promise has one rule: the transport is the object built
    // for UTF-8, and the line reads the stray byte and counts it.
    let source = declared("text/plain;charset=us-ascii", b"caf\xe9 x\n".to_vec());
    let read_lines = lines(&source, &TextOptions::new());
    assert_eq!(read_lines[0].body(), "caf\u{e9} x");
    assert_eq!(read_lines[0].decoded_byte_size(), 1);
}

#[test]
fn a_byte_limit_under_a_declaration_counts_the_decoded_bytes() {
    // "Bytes as read" means below the transport, and the charset is
    // transport: the bytes the reader split are the decoded ones, the units
    // a coding already gives. `Zürich premièr` is 14 windows-1252 bytes on
    // the wire and 16 bytes decoded - `ü` and `è` are two each - so a limit
    // of 7 keeps `Zürich`, the first 7 decoded bytes, and drops the 9 that
    // are left of the 16, where a wire count would keep `Zürich ` and drop 7.
    let text = "Z\u{fc}rich premi\u{e8}r\n";
    let wire = Charset::Cp1252.encode(text).expect("Latin").into_owned();
    assert_eq!(wire.len(), 15, "14 wire bytes and the terminator");
    let source = declared("text/plain;charset=windows-1252", wire);
    let mut options = TextOptions::new();
    options.set_max_record_byte_size(Some(7));
    let read_lines = lines(&source, &options);
    assert_eq!(read_lines.len(), 1);
    assert_eq!(read_lines[0].body(), "Z\u{fc}rich");
    assert_eq!(read_lines[0].body().len(), 7);
    assert_eq!(read_lines[0].dropped_byte_size(), Some(9));
    assert_eq!(read_lines[0].decoded_byte_size(), 0);
}

#[test]
fn the_writer_follows_the_declaration_and_an_append_ends_the_last_line_as_declared() {
    // One wire byte per scalar out, and the same rows back in.
    let mut target = declared("text/plain;charset=windows-1252", Vec::new());
    let options = TextOptions::new().into();
    target
        .overwrite_arrow_batch(rows(&["Z\u{fc}rich", "premi\u{e8}r"]), &options)
        .expect("a write");
    assert_eq!(target.as_slice(), b"Z\xfcrich\npremi\xe8r\n");
    let written = read(&target, TextOptions::new());
    assert_eq!(
        strings(&written, "body"),
        [
            Some("Z\u{fc}rich".to_owned()),
            Some("premi\u{e8}r".to_owned())
        ]
    );
    target
        .append_arrow_batch(rows(&["\u{c4}rger"]), &options)
        .expect("an append");
    assert_eq!(target.as_slice(), b"Z\xfcrich\npremi\xe8r\n\xc4rger\n");
    assert_eq!(
        strings(&read(&target, TextOptions::new()), "body"),
        [
            Some("Z\u{fc}rich".to_owned()),
            Some("premi\u{e8}r".to_owned()),
            Some("\u{c4}rger".to_owned())
        ]
    );

    // Under UTF-16 the terminator itself is two bytes: an append compares
    // the tail with the terminator as encoded, and writes it so.
    let mut target = declared(
        "text/plain;charset=utf-16le",
        Charset::Utf16Le
            .encode("Z\u{fc}rich")
            .expect("UTF-16")
            .into_owned(),
    );
    target
        .append_arrow_batch(rows(&["x"]), &options)
        .expect("an append");
    assert_eq!(
        target.as_slice(),
        Charset::Utf16Le
            .encode("Z\u{fc}rich\nx\n")
            .expect("UTF-16")
            .as_ref()
    );
    assert_eq!(
        strings(&read(&target, TextOptions::new()), "body"),
        [Some("Z\u{fc}rich".to_owned()), Some("x".to_owned())]
    );

    // The same through a coding, where the tail is tracked as the coding is
    // read back and the rows are rendered through the charset writer inside
    // the coding writer.
    let media_type = yggdryl::Url::from_str("file:///rows.log.gz")
        .expect("a location")
        .media_type()
        .with_charset(Charset::Utf16Le);
    let mut target = Buffer::from_bytes(
        yggdryl::Codec::Gzip
            .dump(&Charset::Utf16Le.encode("Z\u{fc}rich").expect("UTF-16"))
            .expect("gzip"),
    )
    .with_media_type(media_type);
    target
        .append_arrow_batch(rows(&["x"]), &options)
        .expect("an append");
    assert_eq!(
        strings(&read(&target, TextOptions::new()), "body"),
        [Some("Z\u{fc}rich".to_owned()), Some("x".to_owned())]
    );
    let mut coded = target.read_all_bytes().expect("the bytes");
    coded = yggdryl::Codec::Gzip.load(&coded).expect("the coded bytes");
    assert_eq!(
        coded,
        Charset::Utf16Le
            .encode("Z\u{fc}rich\nx\n")
            .expect("UTF-16")
            .as_ref()
    );
}

#[test]
fn a_transcoded_handle_needs_no_charset_on_the_options() {
    let lines = "Zürich first\nLondon second\n";
    let source = Buffer::from_bytes(Charset::Cp1252.encode(lines).unwrap().into_owned())
        .with_media_type(MediaType::from_str("text/plain;charset=windows-1252").unwrap());

    // The other door: decode once at the handle, and the record layer is
    // reading UTF-8 like any other payload.
    let handle = Transcoded::infer(source);
    let batches = read(
        &handle,
        TextOptions::new()
            .try_with_rowheader(r"^(?<city>(?-u:\S)+) ")
            .expect("a header"),
    );
    assert_eq!(
        strings(&batches, "city"),
        [Some("Zürich".to_owned()), Some("London".to_owned())]
    );
}

#[test]
fn a_surrogate_pair_straddling_a_chunk_edge_is_joined_under_a_declaration() {
    // The transport decodes one 64 KiB chunk at a time and carries the bytes
    // an edge splits a sequence across. 32 767 `a` units are two bytes short
    // of one chunk, so the pair that follows crosses the edge: its high unit
    // ends the first chunk and its low unit begins the second. The mark is
    // taken off before the chunking starts and moves the edge nowhere. The
    // second shape is the one a lost carry mishandled: a lone high surrogate
    // held at the edge, then the pair - it reads `U+FFFD` for the lone unit
    // and the pair whole, and never drops a unit.
    let text = "a".repeat(32_767);
    let units = Charset::Utf16Le.encode(&text).expect("UTF-16").into_owned();
    assert_eq!(units.len(), 65_534, "two bytes short of one chunk");
    let shapes: [(&[u8], String); 2] = [
        (b"\x00\xd8\x00\xdc\x0a\x00", format!("{text}\u{10000}")),
        (
            b"\x00\xd8\x00\xd8\x00\xdc\x0a\x00",
            format!("{text}\u{fffd}\u{10000}"),
        ),
    ];
    for (tail, expected) in shapes {
        let mut wire = units.clone();
        wire.extend_from_slice(tail);
        let mut marked = b"\xff\xfe".to_vec();
        marked.extend_from_slice(&wire);
        for (marked, wire) in [(false, wire), (true, marked)] {
            let source = declared("text/plain;charset=utf-16le", wire);
            let read_lines = lines(&source, &TextOptions::new());
            assert_eq!(read_lines.len(), 1, "marked: {marked}");
            assert_eq!(read_lines[0].body(), expected, "marked: {marked}");
            assert_eq!(read_lines[0].decoded_byte_size(), 0);
        }
    }
}

#[test]
fn a_byte_limit_that_cuts_inside_a_decoded_scalar_leaves_stray_bytes_the_line_counts() {
    // The limit counts decoded bytes and is not backed off to a character
    // boundary (decision 10's edge, unmoved): a limit of 2 over `Zürich`
    // keeps `Z` and the first byte of `ü`, and that stray byte is read and
    // counted exactly as on an undeclared read. `0` is what a declared line
    // counts when the limit did not cut inside a scalar.
    let source = declared("text/plain;charset=windows-1252", b"Z\xfcrich\n".to_vec());
    let mut options = TextOptions::new();
    options.set_max_record_byte_size(Some(2));
    let read_lines = lines(&source, &options);
    assert_eq!(read_lines.len(), 1);
    assert_eq!(read_lines[0].body(), "Z\u{c3}");
    assert_eq!(read_lines[0].dropped_byte_size(), Some(5));
    assert_eq!(read_lines[0].decoded_byte_size(), 1);
}

#[test]
fn a_utf_8_mark_under_a_utf_8_declaration_is_data_as_it_is_undeclared() {
    // UTF-8 is never wrapped, so no mark is looked for under it: the line
    // layer reads the bytes as they are, and `EF BB BF` is the first three
    // bytes of the first line under `charset=utf-8` exactly as it is
    // undeclared. Only the declared non-UTF-8 form's own mark comes off.
    for media_type in ["text/plain;charset=utf-8", "text/plain"] {
        let source = declared(media_type, b"\xef\xbb\xbfabc\n".to_vec());
        let read_lines = lines(&source, &TextOptions::new());
        assert_eq!(read_lines.len(), 1, "{media_type}");
        assert_eq!(read_lines[0].body(), "\u{feff}abc", "{media_type}");
        assert_eq!(read_lines[0].decoded_byte_size(), 0, "{media_type}");
    }
}

#[test]
fn a_declared_charset_is_read_below_a_coding() {
    // Coding first, charset second: the gzip comes off, then the declared
    // windows-1252 is decoded, and every line is text as read.
    let text = "Z\u{fc}rich\npremi\u{e8}r\n\u{c4}rger\n";
    let plain = Charset::Cp1252.encode(text).expect("Latin").into_owned();
    let source = declared(
        "text/plain;charset=windows-1252;encodings=application/gzip",
        yggdryl::Codec::Gzip.dump(&plain).expect("gzip"),
    );
    let read_lines = lines(&source, &TextOptions::new());
    assert_eq!(
        read_lines
            .iter()
            .map(|line| line.body().to_owned())
            .collect::<Vec<_>>(),
        ["Z\u{fc}rich", "premi\u{e8}r", "\u{c4}rger"]
    );
    for line in &read_lines {
        assert_eq!(line.decoded_byte_size(), 0);
    }
    assert_eq!(
        strings(&read(&source, TextOptions::new()), "body"),
        [
            Some("Z\u{fc}rich".to_owned()),
            Some("premi\u{e8}r".to_owned()),
            Some("\u{c4}rger".to_owned())
        ]
    );
}

#[test]
fn row_size_under_a_declaration_is_the_row_count() {
    // `row_size` builds the same transport the readers do, so it counts the
    // declared text's lines: a UTF-16 wire counted in bytes would be twice
    // the terminators it holds, or none of them.
    let text = "Z\u{fc}rich first\nLondon second\n";
    let plain = declared(
        "text/plain;charset=windows-1252",
        Charset::Cp1252.encode(text).expect("Latin").into_owned(),
    );
    assert_eq!(plain.row_size().expect("rows"), 2);

    let mut marked = b"\xff\xfe".to_vec();
    marked.extend_from_slice(&Charset::Utf16Le.encode(text).expect("UTF-16"));
    let marked = declared("text/plain;charset=utf-16le", marked);
    assert_eq!(marked.row_size().expect("rows"), 2);

    // The other order's mark is data, and a row all the same.
    let mut other = b"\xfe\xff".to_vec();
    other.extend_from_slice(&Charset::Utf16Le.encode("a\nb\n").expect("UTF-16"));
    let other = declared("text/plain;charset=utf-16le", other);
    assert_eq!(other.row_size().expect("rows"), 2);
    assert_eq!(lines(&other, &TextOptions::new())[0].body(), "\u{fffe}a");

    // An odd trailing byte is half a unit read as `U+FFFD`, still one row.
    let mut odd = Charset::Utf16Le
        .encode("a\nb")
        .expect("UTF-16")
        .into_owned();
    odd.push(0x41);
    let odd = declared("text/plain;charset=utf-16le", odd);
    assert_eq!(odd.row_size().expect("rows"), 2);
    let read_lines = lines(&odd, &TextOptions::new());
    assert_eq!(read_lines[1].body(), "b\u{fffd}");
}

#[test]
fn adjacent_deduplication_under_a_declaration_compares_the_declared_text() {
    let source = declared(
        "text/plain;charset=windows-1252",
        Charset::Cp1252
            .encode("Z\u{fc}rich\nZ\u{fc}rich\nLondon\n")
            .expect("Latin")
            .into_owned(),
    );
    let mut options = TextOptions::new();
    options.dedup_adjacent = true;
    assert_eq!(
        strings(&read(&source, options), "body"),
        [Some("Z\u{fc}rich".to_owned()), Some("London".to_owned())]
    );
}

#[test]
fn a_declared_utf_8_handle_reads_as_the_undeclared_one_and_writes_utf_8() {
    // `charset=utf-8` wraps nothing: the stray byte reads by rule one and
    // is counted, exactly as with nothing declared.
    let wire = b"caf\xe9\n".to_vec();
    let read_declared = lines(
        &declared("text/plain;charset=utf-8", wire.clone()),
        &TextOptions::new(),
    );
    let read_undeclared = lines(&declared("text/plain", wire), &TextOptions::new());
    assert_eq!(read_declared[0].body(), "caf\u{e9}");
    assert_eq!(read_declared[0].decoded_byte_size(), 1);
    assert_eq!(read_undeclared[0].body(), read_declared[0].body());
    assert_eq!(read_undeclared[0].decoded_byte_size(), 1);

    // And the writer follows the declaration: UTF-8 out, as declared.
    let mut target = declared("text/plain;charset=utf-8", Vec::new());
    target
        .overwrite_arrow_batch(rows(&["Z\u{fc}rich"]), &TextOptions::new().into())
        .expect("a write");
    assert_eq!(target.as_slice(), "Z\u{fc}rich\n".as_bytes());
}

#[test]
fn an_append_onto_an_unterminated_declared_tail_terminates_it_as_declared() {
    let options = TextOptions::new().into();
    let mut target = declared("text/plain;charset=windows-1252", b"Z\xfcrich".to_vec());
    target
        .append_arrow_batch(rows(&["caf\u{e9}"]), &options)
        .expect("an append");
    assert_eq!(target.as_slice(), b"Z\xfcrich\ncaf\xe9\n");
    assert_eq!(
        strings(&read(&target, TextOptions::new()), "body"),
        [Some("Z\u{fc}rich".to_owned()), Some("caf\u{e9}".to_owned())]
    );
}

#[test]
fn a_scalar_the_declared_charset_has_no_byte_for_is_refused_on_write() {
    // Reading transcribes and never refuses; writing has no byte to invent,
    // so `Ω` under windows-1252 is refused, naming the charset.
    let mut target = declared("text/plain;charset=windows-1252", Vec::new());
    let refusal = target
        .overwrite_arrow_batch(rows(&["\u{3a9}"]), &TextOptions::new().into())
        .expect_err("no byte for omega");
    let message = refusal.to_string();
    assert!(message.contains("windows-1252"), "{message}");
    assert!(message.contains("U+03A9"), "{message}");
}
