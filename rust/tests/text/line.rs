//! `rust/src/text/line.rs`: one decode per line, and what it counted.
//!
//! A line reads its bytes once, on the page they were read into, and answers
//! how many of them were not UTF-8. The decode step itself is private, so it
//! is reached through `yggdryl::internals`; everything else here is the
//! `TextLine` a caller holds.

#[cfg(feature = "internals")]
mod internal {
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
        let page =
            TextBytes::from_bytes("8=FIX.4.4|58=caf\u{e9}|10=0|".as_bytes()).expect("a page");
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
}

mod charsets {
    use arrow_array::{Array as _, RecordBatch, StringArray};
    use std::sync::Arc;
    use yggdryl::charset::Transcoded;
    use yggdryl::holder::Buffer;
    use yggdryl::text::{TextLine, TextOptions, read_text_lines};
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
        Buffer::from_bytes(wire)
            .with_media_type(MediaType::from_str(media_type).expect("a media type"))
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
        // The body is the whole line, the row header included, read as declared.
        assert_eq!(
            strings(&batches, "body"),
            [
                Some("\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430} first".to_owned()),
                Some("London second".to_owned())
            ]
        );
        for line in lines(&source, &city_header()) {
            assert_eq!(
                line.decoded_byte_size(),
                0,
                "read as declared, nothing repaired"
            );
        }

        // Undeclared: the same bytes are the wire, read by rule one - never
        // refused, each stray byte as the Windows-1252 character it is - and the
        // line matches its header over the text it is, so a class that matches
        // no stray byte matches the characters they were read as. The line
        // counts the six bytes it read that way.
        let source = declared("text/plain", wire);
        let read_lines = lines(&source, &city_header());
        assert_eq!(read_lines.len(), 2);
        assert_eq!(
            read_lines[0].body(),
            "\u{cc}\u{ee}\u{f1}\u{ea}\u{e2}\u{e0} first"
        );
        assert_eq!(
            read_lines[0].capture(0),
            Some("\u{cc}\u{ee}\u{f1}\u{ea}\u{e2}\u{e0}")
        );
        assert_eq!(read_lines[0].decoded_byte_size(), 6);
        assert_eq!(read_lines[1].body(), "London second");
        assert_eq!(read_lines[1].capture(0), Some("London"));
        assert_eq!(read_lines[1].decoded_byte_size(), 0);
    }

    #[test]
    fn two_latin_1_letters_are_told_from_one_utf_8_scalar_by_the_declaration() {
        // What a declaration tells and an undeclared read cannot: `C3 A9` is one
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
        assert_eq!(
            strings(&batches, "body"),
            [Some("Z\u{fc}rich premi\u{e8}r".to_owned())]
        );
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
                [
                    Some("Z\u{fc}rich first".to_owned()),
                    Some("London second".to_owned())
                ]
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
        // boundary: a limit of 2 over `Zürich`
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
}

mod text {
    use arrow_array::{Array as _, StringArray};
    use yggdryl::holder::Buffer;

    use yggdryl::text::{TextLine, TextOptions, read_text_lines};

    fn named(name: &str, bytes: &[u8]) -> Buffer {
        Buffer::from_bytes(bytes.to_vec()).with_media_type(
            yggdryl::Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

    fn options(rowheader: &str) -> TextOptions {
        TextOptions::new().try_with_rowheader(rowheader).unwrap()
    }

    fn framed(rowheader: &str) -> TextOptions {
        options(rowheader).with_framing(true)
    }

    /// The nineteen event columns every line batch opens with, in front of the
    /// line's own: the line is an event of the graph, and a message parsed out
    /// of it contains the same nineteen under the same names and datatypes.
    const EVENT_COLUMNS: [&str; 19] = [
        "currunix",
        "creaunix",
        "execunix",
        "recdunix",
        "refrecdunix",
        "exprtime",
        "prevunix",
        "snapunix",
        "curruuid",
        "crossuuid",
        "crosscode",
        "currhashcode",
        "crosshashcode",
        "prevuuid",
        "seqnum",
        "parentuuids",
        "srcuuids",
        "identifiers",
        "state",
    ];

    /// The names of a line batch: the event columns, then the line's own.
    fn with_event(rest: &[&'static str]) -> Vec<&'static str> {
        EVENT_COLUMNS.iter().chain(rest).copied().collect()
    }

    fn collect(
        source: &impl yggdryl::IOBase,
        options: TextOptions,
    ) -> Vec<arrow_array::RecordBatch> {
        source
            .read_arrow_reader(&options.into())
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    }

    fn bodies(batches: &[arrow_array::RecordBatch]) -> Vec<Vec<u8>> {
        batches
            .iter()
            .flat_map(|batch| {
                let index = batch.schema().index_of("body").unwrap();
                batch
                    .column(index)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .iter()
                    .map(|value| value.unwrap().as_bytes().to_vec())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn a_line_that_was_not_utf_8_reaches_its_row_decoded_and_says_so() {
        // One Latin-1 byte in the body and one in a capture: each reads as the
        // character Windows-1252 gives it, the row's body is text, and the line
        // counts the two bytes it repaired. The counts the reader took before
        // the line existed - the record's bytes over its limit - are counts of
        // the bytes as read.
        let source = named("latin1.log", b"[caf\xE9] first \xE9 line\n[plain] second\n");
        // A byte class, because a Unicode class matches characters and a byte
        // that is not one is not matched by it.
        let mut options = framed(r"^\[(?<kind>(?-u:[^\]]+))\] ");
        options.set_max_record_byte_size(Some(7));
        let lines: Vec<TextLine> = read_text_lines(&source, &options)
            .unwrap()
            .map(|line| line.unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        // The header is retained whole and the limit bounds what follows it:
        // `[café] ` then seven of the twelve wire bytes past it.
        assert_eq!(lines[0].body(), "[caf\u{e9}] first \u{e9}");
        assert_eq!(lines[0].capture(0), Some("caf\u{e9}"));
        assert_eq!(lines[0].decoded_byte_size(), 2);
        assert_eq!(
            lines[0].dropped_byte_size(),
            Some(5),
            "the limit and the count are wire bytes past the header: 12 read, 7 kept"
        );
        assert_eq!(lines[1].body(), "[plain] second");
        assert_eq!(lines[1].decoded_byte_size(), 0);

        let batches = collect(&source, framed(r"^\[(?<kind>(?-u:[^\]]+))\] "));
        assert_eq!(
            bodies(&batches),
            [
                "[caf\u{e9}] first \u{e9} line".as_bytes().to_vec(),
                b"[plain] second".to_vec()
            ]
        );
        let batch = &batches[0];
        assert_eq!(
            batch.schema().field_with_name("body").unwrap().data_type(),
            &arrow_schema::DataType::Utf8
        );
        let kinds = batch
            .column_by_name("kind")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(kinds.value(0), "caf\u{e9}");
    }

    #[test]
    fn a_record_cut_inside_a_character_reads_the_bytes_that_are_left() {
        // The three-byte euro sign, cut after two of its bytes by the limit: the
        // orphans read as the two Windows-1252 characters they are, not as a
        // replacement character, and both are counted as decoded.
        let source = named("cut.log", "[A] x\u{20AC}\n".as_bytes());
        let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
        options.set_max_record_byte_size(Some(3));
        let lines: Vec<TextLine> = read_text_lines(&source, &options)
            .unwrap()
            .map(|line| line.unwrap())
            .collect();
        assert_eq!(lines[0].body(), "[A] x\u{e2}\u{201a}");
        assert_eq!(lines[0].decoded_byte_size(), 2);
        assert_eq!(lines[0].dropped_byte_size(), Some(1));
    }

    // --- The line as an event: every reading resolved from the body on ask ---

    mod event {
        use std::sync::Arc;

        use yggdryl::graph::{Element, Event, EventColumn, EventIterator};
        use yggdryl::text::{
            DEFAULT_TEXT_BATCH_BYTE_SIZE, DEFAULT_TEXT_BATCH_ROW_SIZE, TextBytes, TextEntries,
            TextLine, TextOptions, into_arrow_batch, read_text_lines,
        };
        use yggdryl::{FieldPath, Scalar, Url, Uuid};

        use super::named;

        /// The header naming the facts an event reads off captures. The anonymous
        /// count and code are ordinary payload framing, never alternate owners of
        /// `seqnum` or `crosscode`.
        const HEADER: &str = concat!(
            r"^(?<mtime>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ",
            r"\[(?<state>[A-Za-z]+)\] \d+ (?<prevuuid>[0-9a-f-]{36}) \S+ ",
        );
        /// The same facts, each capture admitting any spelling, so a line that
        /// spells one wrongly still matches and the fact's own reading refuses.
        const LOOSE: &str = r"^(?<mtime>\S+) \[(?<state>[^\]]+)\] \S+ (?<prevuuid>\S+) ";
        /// A chain's lines: an instant and a state, with an anonymous token in the
        /// wire grammar. The source URL is the code every line shares.
        const CHAIN: &str = r"^(?<mtime>\S+) \[(?<state>[A-Za-z]+)\] \S+ ";
        const EVENT_TIMES: &str = r"^(?<execunix>\S+) (?<recdunix>\S+) (?<refrecdunix>\S+) ";
        const INSTANT: i64 = 1_767_348_930_000_000_000;
        const PREVIOUS: &str = "0198a3b2-1c4d-7e5f-8a9b-0c1d2e3f4a5b";

        fn options() -> Arc<TextOptions> {
            Arc::new(
                TextOptions::new()
                    .try_with_rowheader(HEADER)
                    .expect("a header"),
            )
        }

        fn line(body: &str, options: &Arc<TextOptions>) -> TextLine {
            TextLine::from_bytes(
                0,
                TextBytes::from_bytes(body).expect("a page"),
                Arc::clone(options),
            )
            .expect("a line")
        }

        #[test]
        fn a_new_value_batches_on_rows_or_bytes_whichever_binds_first() {
            let options = TextOptions::new();
            assert_eq!(options.batch_row_size, Some(DEFAULT_TEXT_BATCH_ROW_SIZE));
            assert_eq!(options.batch_byte_size, Some(DEFAULT_TEXT_BATCH_BYTE_SIZE));
            assert_eq!(DEFAULT_TEXT_BATCH_ROW_SIZE, 35 * 1024);
            assert_eq!(DEFAULT_TEXT_BATCH_BYTE_SIZE, 64 * 1024 * 1024);
        }

        #[test]
        fn every_reading_resolves_from_the_body_under_the_header() {
            let options = options();
            let body = format!("2026-01-02T10:15:30Z [Filled] 7 {PREVIOUS} O-100 k=v|x=y");
            let line = line(&body, &options);
            // The body is the whole line; the payload is what follows the header.
            assert_eq!(line.body(), body);
            assert_eq!(line.payload_bytes().as_str(), Some("k=v|x=y"));
            assert_eq!(line.entries().map(yggdryl::text::TextEntries::len), Some(2));
            // The captures, in the order the header declares them, and the
            // identifiers the named ones make.
            assert_eq!(line.capture(0), Some("2026-01-02T10:15:30Z"));
            assert_eq!(line.capture(2), Some(PREVIOUS));
            assert_eq!(line.get_identifiers()["state"], "Filled");
            assert_eq!(line.get_identifiers()["prevuuid"], PREVIOUS);
            assert_eq!(line.get_identifiers().len(), 3);
            // Captured facts read at their own types; the place and source facts
            // come from the line itself. This manually made line is row zero and
            // has no source.
            assert_eq!(line.mtime().unwrap(), Some(INSTANT));
            assert_eq!(line.get_currunix(), INSTANT);
            assert!(line.get_state().is_done());
            assert_eq!(line.get_seqnum(), 0);
            let Scalar::Uuid(previous) = yggdryl::DataType::Uuid
                .scalar(Scalar::from(PREVIOUS))
                .expect("a uuid")
            else {
                panic!("a uuid")
            };
            assert_eq!(line.get_prevuuid(), Some(previous));
            assert_eq!(line.get_crosscode(), "");
            assert_eq!(line.get_crosshashcode(), 0);
            assert_eq!(line.get_crossuuid(), line.cross_uuid());
            assert_eq!(line.get_crossuuid(), line.get_curruuid());
            assert_eq!((line.get_creaunix(), line.get_exprtime()), (None, None));
            assert_eq!(
                (
                    line.get_execunix(),
                    line.get_recdunix(),
                    line.get_refrecdunix()
                ),
                (None, None, None)
            );
            assert_eq!((line.get_prevunix(), line.get_snapunix()), (None, None));
            // The identity: instant, physical sequence and body digest, with no
            // cross-hash seed on this unlocated line.
            assert_eq!(
                line.get_currhashcode(),
                yggdryl::xxhash::xxh3(body.as_bytes())
            );
            assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
            // A line is read from a handle: no source, and no parent until a
            // walk states one.
            assert!(line.get_srcuuids().is_empty() && line.get_parentuuids().is_empty());
            // The same bytes, instant, physical sequence and absent cross seed
            // derive the same identity.
            assert_eq!(
                line.get_curruuid(),
                self::line(&body, &options).get_curruuid()
            );
        }

        #[test]
        fn execution_and_recording_captures_are_typed_event_instants() {
            const EXECUTED: i64 = INSTANT + 1_000_000_000;
            const RECORDED: i64 = INSTANT + 2_000_000_000;
            const REFERENCE_RECORDED: i64 = INSTANT + 3_000_000_000;
            let options = Arc::new(
                TextOptions::new()
                    .try_with_rowheader(EVENT_TIMES)
                    .expect("a header"),
            );
            let mut line = line(
                "2026-01-02T10:15:31Z 2026-01-02T10:15:32Z 2026-01-02T10:15:33Z body",
                &options,
            );

            assert_eq!(line.execunix().unwrap(), Some(EXECUTED));
            assert_eq!(line.recdunix().unwrap(), Some(RECORDED));
            assert_eq!(line.refrecdunix().unwrap(), Some(REFERENCE_RECORDED));
            assert_eq!(line.get_execunix(), Some(EXECUTED));
            assert_eq!(line.get_recdunix(), Some(RECORDED));
            assert_eq!(line.get_refrecdunix(), Some(REFERENCE_RECORDED));
            assert_eq!(
                line.event_fact(EventColumn::ExecUnix)
                    .unwrap()
                    .and_then(|value| value.temporal_count()),
                Some(EXECUTED)
            );
            assert_eq!(
                line.event_fact(EventColumn::RecdUnix)
                    .unwrap()
                    .and_then(|value| value.temporal_count()),
                Some(RECORDED)
            );
            assert_eq!(
                line.event_fact(EventColumn::RefRecdUnix)
                    .unwrap()
                    .and_then(|value| value.temporal_count()),
                Some(REFERENCE_RECORDED)
            );

            // A changed body drops both resolved readings.
            line.set_body(
                TextBytes::from_bytes(
                    "2026-01-02T10:15:33Z 2026-01-02T10:15:34Z 2026-01-02T10:15:35Z changed",
                )
                .expect("a page"),
            )
            .expect("a body");
            assert_eq!(line.get_execunix(), Some(INSTANT + 3_000_000_000));
            assert_eq!(line.get_recdunix(), Some(INSTANT + 4_000_000_000));
            assert_eq!(line.get_refrecdunix(), Some(INSTANT + 5_000_000_000));

            // A value stated through the Event contract stands over later bodies.
            line.set_execunix(Some(7));
            line.set_recdunix(Some(8));
            line.set_refrecdunix(Some(9));
            line.set_body(
                TextBytes::from_bytes(
                    "2026-01-02T10:15:35Z 2026-01-02T10:15:36Z 2026-01-02T10:15:37Z stated",
                )
                .expect("a page"),
            )
            .expect("a body");
            assert_eq!(
                (
                    line.get_execunix(),
                    line.get_recdunix(),
                    line.get_refrecdunix()
                ),
                (Some(7), Some(8), Some(9))
            );

            // The capture feeds the event columns themselves, not duplicate text
            // columns behind them, and a row read back states both facts again.
            let source = self::line(
                "2026-01-02T10:15:31Z 2026-01-02T10:15:32Z 2026-01-02T10:15:33Z body",
                &options,
            );
            let batch = into_arrow_batch([source], &options).expect("an event row");
            let schema = batch.schema();
            let names: Vec<&str> = schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect();
            assert_eq!(names, super::with_event(&["sourceurl", "mtime", "body"]));
            for name in ["execunix", "recdunix", "refrecdunix"] {
                assert_eq!(
                    schema.field_with_name(name).unwrap().data_type(),
                    &arrow_schema::DataType::Timestamp(
                        arrow_schema::TimeUnit::Nanosecond,
                        Some("UTC".into())
                    )
                );
            }
            let back = yggdryl::text::from_arrow_batch(&batch, &options).expect("a line");
            assert_eq!(back[0].get_execunix(), Some(EXECUTED));
            assert_eq!(back[0].get_recdunix(), Some(RECORDED));
            assert_eq!(back[0].get_refrecdunix(), Some(REFERENCE_RECORDED));
        }

        #[test]
        fn execution_and_recording_captures_refuse_bad_instants_by_name() {
            let options = Arc::new(
                TextOptions::new()
                    .try_with_rowheader(EVENT_TIMES)
                    .expect("a header"),
            );
            let line = line(
                "not-an-instant still-not-an-instant nor-an-instant body",
                &options,
            );
            for (name, refused) in [
                ("execunix", line.execunix().err()),
                ("recdunix", line.recdunix().err()),
                ("refrecdunix", line.refrecdunix().err()),
            ] {
                let refused = refused
                    .unwrap_or_else(|| panic!("{name} refuses"))
                    .to_string();
                assert!(refused.contains(&format!("$[0].{name}")), "{refused}");
            }
            assert_eq!(
                (
                    line.get_execunix(),
                    line.get_recdunix(),
                    line.get_refrecdunix()
                ),
                (None, None, None)
            );
            let error = line
                .event_fact(EventColumn::ExecUnix)
                .expect_err("the event column refuses")
                .to_string();
            assert!(error.contains("$[0].execunix"), "{error}");
        }

        #[test]
        fn a_line_the_header_does_not_date_stands_at_the_handles_time_else_the_epoch() {
            let options = Arc::new(TextOptions::new());
            let mut line = line("plain", &options);
            assert_eq!(line.mtime().unwrap(), None);
            assert_eq!(line.get_currunix(), 0);
            assert!(line.captures().is_empty());
            assert!(line.get_identifiers().is_empty());
            assert_eq!(line.get_crosscode(), "");
            assert_eq!(line.get_crosshashcode(), 0);
            assert_eq!(line.get_crossuuid(), line.get_curruuid());
            assert_eq!(line.get_state().as_str(), "00UNKNOWN");
            // The place in the chain is the row number under `start_rownum`,
            // else the physical line number.
            assert_eq!(line.get_seqnum(), 0);
            line.set_index(3);
            assert_eq!(line.get_seqnum(), 3);
            let mut numbered = TextOptions::new();
            numbered.start_rownum = Some(10);
            let numbered = self::line("plain", &Arc::new(numbered));
            assert_eq!(numbered.get_seqnum(), 10);
            // The handle's own time dates it, and the identity follows.
            let before = line.get_curruuid();
            line.set_handle_mtime(Some(INSTANT));
            assert_eq!(line.mtime().unwrap(), Some(INSTANT));
            assert_eq!(line.get_currunix(), INSTANT);
            assert_ne!(line.get_curruuid(), before);
            assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
        }

        #[test]
        fn changing_the_source_refreshes_derived_cross_facts_but_not_a_stated_code() {
            let options = Arc::new(TextOptions::new());
            let mut line = line("plain", &options);
            let anonymous_uuid = line.get_curruuid();
            let first = Arc::new(Url::from_str("file:///first.log").expect("a source URL"));
            line.set_sourceurl(Some(Arc::clone(&first)));
            let first_code = first.to_string();
            assert_eq!(line.get_crosscode(), first_code.as_str());
            assert_eq!(
                line.get_crosshashcode(),
                yggdryl::xxhash::xxh3(first_code.as_bytes())
            );
            let first_uuid = line.get_curruuid();
            assert_ne!(first_uuid, anonymous_uuid);
            let first_crossuuid = line.get_crossuuid();

            let second = Arc::new(Url::from_str("file:///second.log").expect("a source URL"));
            line.set_sourceurl(Some(Arc::clone(&second)));
            let second_code = second.to_string();
            assert_eq!(line.get_crosscode(), second_code.as_str());
            assert_eq!(
                line.get_crosshashcode(),
                yggdryl::xxhash::xxh3(second_code.as_bytes())
            );
            let second_uuid = line.get_curruuid();
            assert_ne!(second_uuid, first_uuid);
            assert_ne!(line.get_crossuuid(), first_crossuuid);

            line.set_sourceurl(None);
            assert_eq!(line.get_crosscode(), "");
            assert_eq!(line.get_crosshashcode(), 0);
            assert_eq!(line.get_curruuid(), anonymous_uuid);
            assert_eq!(line.get_crossuuid(), line.get_curruuid());

            line.set_crosscode("stated-chain".to_owned());
            let stated_uuid = line.get_curruuid();
            assert_ne!(stated_uuid, anonymous_uuid);
            line.set_sourceurl(Some(first));
            assert_eq!(line.get_crosscode(), "stated-chain");
            assert_eq!(line.get_curruuid(), stated_uuid);
        }

        #[test]
        fn changing_identity_inputs_rederives_a_restored_lines_generic_identities() {
            fn state_generic_identities(line: &mut TextLine) {
                line.set_curruuid(Uuid::from_v8(1));
                line.set_crossuuid(Uuid::from_v8(2));
            }

            fn assert_derived_identities(line: &TextLine) {
                assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
                assert_eq!(line.get_crossuuid(), line.cross_uuid());
            }

            let options = Arc::new(TextOptions::new());
            let mut line = line("plain", &options);
            line.set_crosshashcode(0xCD);
            state_generic_identities(&mut line);

            line.set_crosscode("stated-chain".to_owned());
            let crosshashcode = yggdryl::xxhash::xxh3(b"stated-chain");
            assert_eq!(line.get_crosshashcode(), crosshashcode);
            assert_derived_identities(&line);
            assert_ne!(line.get_curruuid(), Uuid::from_v8(1));
            assert_eq!(
                line.get_crossuuid(),
                Uuid::from_v8(u128::from(crosshashcode))
            );

            state_generic_identities(&mut line);
            line.set_crosshashcode(0xEF);
            assert_derived_identities(&line);

            state_generic_identities(&mut line);
            line.set_currhashcode(0xAB);
            assert_derived_identities(&line);

            state_generic_identities(&mut line);
            line.set_currunix(1_000_000);
            assert_derived_identities(&line);

            state_generic_identities(&mut line);
            line.set_seqnum(17);
            assert_derived_identities(&line);

            state_generic_identities(&mut line);
            line.set_crosscode(String::new());
            assert_eq!(line.get_crosshashcode(), 0);
            assert_derived_identities(&line);
            assert_eq!(line.get_crossuuid(), line.get_curruuid());
        }

        #[test]
        fn a_capture_that_does_not_read_as_its_fact_refuses_by_name() {
            let options = Arc::new(
                TextOptions::new()
                    .try_with_rowheader(LOOSE)
                    .expect("a header"),
            );
            let line = line("nope [what] x y body", &options);
            for (name, refused) in [
                ("mtime", line.mtime().err()),
                ("state", line.state().err()),
                ("prevuuid", line.prevuuid().err()),
            ] {
                let refused = refused
                    .unwrap_or_else(|| panic!("{name} refuses"))
                    .to_string();
                assert!(
                    refused.contains(&format!("$[0].{name}")),
                    "{name}: {refused}"
                );
                assert!(refused.contains("physical line 1"), "{name}: {refused}");
            }
            // The trait door cannot refuse: it answers each fact's default.
            assert_eq!(line.get_currunix(), 0);
            assert_eq!(line.get_state().as_str(), "00UNKNOWN");
            assert_eq!(line.get_seqnum(), 0);
            assert_eq!(line.get_prevuuid(), None);
            // And the column built from the reading refuses the same way.
            let error = into_arrow_batch([line], &options)
                .expect_err("the mtime column refuses")
                .to_string();
            assert!(error.contains("$[0].mtime"), "{error}");
        }

        #[test]
        fn a_stated_fact_wins_and_a_new_body_drops_every_reading() {
            let options = options();
            let body = format!("2026-01-02T10:15:30Z [New] 1 {PREVIOUS} O-100 k=v");
            let mut line = line(&body, &options);
            let resolved = line.get_curruuid();
            line.set_state(yggdryl::State::from_spelling("Filled").expect("a state"));
            line.set_seqnum(9);
            line.set_srcuuids(vec![Uuid::from_v8(70)]);
            assert!(line.get_state().is_done());
            assert_eq!(line.get_seqnum(), 9);
            let sequenced = line.get_curruuid();
            assert_ne!(sequenced, resolved, "the sequence is part of the identity");
            assert_eq!(sequenced, line.time_uuid().expect("an identity"));
            // A new body: the readings resolve afresh from it, and the stated
            // facts stand.
            line.set_body(TextBytes::from_bytes("plain").expect("a page"))
                .expect("a body");
            assert_ne!(line.get_curruuid(), sequenced);
            assert_eq!(line.get_currhashcode(), yggdryl::xxhash::xxh3(b"plain"));
            assert_eq!(line.mtime().unwrap(), None, "the header no longer matches");
            assert!(line.get_state().is_done(), "stated, so it stands");
            assert_eq!(line.get_seqnum(), 9);
            assert_eq!(line.get_srcuuids(), [Uuid::from_v8(70)]);
            // The stated identity is dropped by finalizing, which derives it.
            line.set_curruuid(Uuid::from_v8(1));
            assert_eq!(line.get_curruuid(), Uuid::from_v8(1));
            line.finalize();
            assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
        }

        /// A line is read across threads as any event is: the slots its readings
        /// resolve into are shared and sent with it.
        #[test]
        fn a_line_is_sent_and_shared_across_threads() {
            fn sent_and_shared<T: Send + Sync>() {}
            sent_and_shared::<TextLine>();
        }

        #[test]
        fn a_stated_tree_stands_over_a_new_body_and_a_matched_header_does_not() {
            let options = TextOptions::new()
                .try_with_rowheader(r"^\[(?<level>[A-Z]+)\] ")
                .expect("a header")
                .with_max_record_byte_size(64);
            let page = |text: &str| TextBytes::from_bytes(text).expect("a page");
            // A line the cut matched: the header's end and captures were read
            // over the body the reader cut, so a new body reads them afresh, and
            // the tree with them.
            let mut cut = read_text_lines(&named("cut.log", b"[INFO] a=1|b=2\n"), &options)
                .expect("a reader")
                .next()
                .expect("a line")
                .expect("a line");
            assert_eq!(cut.capture(0), Some("INFO"));
            assert_eq!(cut.payload_bytes().as_str(), Some("a=1|b=2"));
            cut.set_body(page("[DEBUG] c=3")).expect("a body");
            assert_eq!(cut.capture(0), Some("DEBUG"), "read off the new body");
            assert_eq!(cut.payload_bytes().as_str(), Some("c=3"));
            assert_eq!(cut.entries().map(TextEntries::len), Some(1));
            // Captures a caller stated are the line's word, and stand over every
            // body: the payload is then the whole of the new one.
            let options = Arc::new(options);
            let mut line = TextLine::from_bytes(0, page("[INFO] a=1|b=2"), Arc::clone(&options))
                .expect("a line");
            line.set_captures(vec![Some(page("WARN"))])
                .expect("captures");
            assert_eq!(line.capture(0), Some("WARN"), "stated, the captures win");
            assert_eq!(line.entries().map(TextEntries::len), Some(2));
            line.set_body(page("[DEBUG] c=3")).expect("a body");
            assert_eq!(line.capture(0), Some("WARN"), "stated, the captures stand");
            assert_eq!(line.body(), "[DEBUG] c=3");
            assert_eq!(line.payload_bytes().as_str(), Some("[DEBUG] c=3"));
            assert_eq!(line.entries().map(TextEntries::len), Some(1));
            // A stated tree stands over every body, and so does a stated absence.
            line.set_entries(TextEntries::from_bytes(&page("z=9")));
            line.set_body(page("[DEBUG] c=3|d=4")).expect("a body");
            assert_eq!(
                line.get_entry_by_path(&FieldPath::from_str("z").unwrap())
                    .map(|held| held.value().into_owned()),
                Some("9".to_owned()),
                "stated, the tree stands"
            );
            line.set_entries(None);
            assert!(line.entries().is_none(), "cleared, no tree stands");
            // A tree handed out for mutation is the line's word from then on.
            let mut mutated =
                TextLine::from_bytes(1, page("[INFO] a=1"), Arc::clone(&options)).expect("a line");
            mutated
                .set_entry_by_path(&FieldPath::from_str("b").unwrap(), page("2"))
                .expect("an entry");
            mutated.set_body(page("[INFO] c=3")).expect("a body");
            assert_eq!(
                mutated.entries().map(TextEntries::len),
                Some(2),
                "mutated, the tree stands"
            );
        }

        #[test]
        fn the_identity_reads_the_cross_hash_but_not_a_stated_cross_element_or_source() {
            let options = options();
            let body = format!("2026-01-02T10:15:30Z [New] 1 {PREVIOUS} O-100 k=v");
            let stated = line(&body, &options);
            let mut crossed = line(&body, &options);
            crossed.set_crosshashcode(0xCD);
            let crossed_identity = crossed.get_curruuid();
            assert_ne!(crossed_identity, stated.get_curruuid());
            crossed.set_crossuuid(Uuid::from_v8(77));
            crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
            assert_eq!(crossed.get_currhashcode(), stated.get_currhashcode());
            assert_eq!(crossed.get_curruuid(), crossed_identity);
            assert_eq!(
                crossed.get_crossuuid(),
                Uuid::from_v8(77),
                "stated, so answered"
            );
            crossed.finalize();
            assert_eq!(crossed.get_curruuid(), stated.get_curruuid());
            assert_eq!(crossed.get_crosshashcode(), stated.get_crosshashcode());
            assert_eq!(crossed.get_crossuuid(), stated.get_crossuuid());
        }

        #[test]
        fn equality_reads_the_stated_facts_and_never_a_resolved_slot() {
            let options = options();
            let body = format!("2026-01-02T10:15:30Z [New] 1 {PREVIOUS} O-100 k=v");
            let left = line(&body, &options);
            let mut right = line(&body, &options);
            assert_eq!(left, right);
            let _ = right.captures();
            let _ = right.get_curruuid();
            let _ = right.entries();
            assert_eq!(left, right, "a resolved slot is not a fact");
            right.set_seqnum(4);
            assert_ne!(left, right, "a stated one is");
        }

        #[test]
        fn lines_walk_as_events_and_carry_their_lineage() {
            let options = Arc::new(
                TextOptions::new()
                    .try_with_rowheader(CHAIN)
                    .expect("a header"),
            );
            let source = Arc::new(Url::from_str("file:///events.log").expect("a source URL"));
            let read = |instant: &str, state: &str| {
                line(&format!("{instant} [{state}] O-100 k=v"), &options)
                    .with_sourceurl(Arc::clone(&source))
            };
            let arrived = vec![
                read("2026-01-02T10:15:30Z", "New"),
                read("2026-01-02T10:15:31Z", "PartiallyFilled"),
                read("2026-01-02T10:15:32Z", "Filled"),
            ];
            let walked: Vec<TextLine> = EventIterator::new(arrived, true).collect();
            let [first, second, third] = walked.as_slice() else {
                panic!("three lines")
            };
            assert_eq!((first.get_seqnum(), first.get_prevuuid()), (0, None));
            assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
            assert_eq!(second.get_prevunix(), Some(first.get_currunix()));
            assert_eq!(second.get_seqnum(), 1);
            assert_eq!(second.get_parentuuids(), [first.get_curruuid()]);
            assert_eq!(
                third.get_parentuuids(),
                [first.get_curruuid(), second.get_curruuid()]
            );
            assert!(third.get_state().is_done());
            assert!(walked.iter().all(|line| line.get_srcuuids().is_empty()));
            assert!(
                walked
                    .iter()
                    .all(|line| line.get_crosscode() == source.to_string())
            );
            assert!(
                walked
                    .iter()
                    .all(|line| line.get_crossuuid() == first.get_crossuuid())
            );
        }

        #[test]
        fn a_read_line_reads_as_the_batch_reads_it() {
            let mut options = TextOptions::new()
                .try_with_rowheader(HEADER)
                .expect("a header");
            options.start_rownum = Some(7);
            let text = format!("2026-01-02T10:15:30Z [Filled] 7 {PREVIOUS} O-100 k=v\n");
            let source = named("events.log", text.as_bytes());
            let lines: Vec<TextLine> = read_text_lines(&source, &options)
                .expect("a reader")
                .map(|line| line.expect("a line"))
                .collect();
            assert_eq!(lines[0].get_currunix(), INSTANT);
            assert_eq!(lines[0].get_seqnum(), 7);
            let sourceurl = lines[0].sourceurl().expect("a source").to_string();
            assert_eq!(lines[0].get_crosscode(), sourceurl.as_str());
            let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
            let schema = batch.schema();
            let names: Vec<&str> = schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect();
            // The batch opens with the nineteen event columns the line is stated
            // in. Captured facts feed their own event columns, while `seqnum` and
            // `crosscode` come from the line's row number and source URL.
            assert_eq!(
                names,
                super::with_event(&["sourceurl", "rownum", "mtime", "body"])
            );
            let cell = |name: &str| batch.column_by_name(name).expect(name).clone();
            assert_eq!(
                cell("seqnum")
                    .as_any()
                    .downcast_ref::<arrow_array::UInt64Array>()
                    .expect("a count")
                    .value(0),
                7
            );
            assert_eq!(
                cell("crosscode")
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .expect("a code")
                    .value(0),
                sourceurl.as_str()
            );
            assert_eq!(
                cell("state")
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .expect("a state")
                    .value(0),
                lines[0].get_state().as_str()
            );
            // And a line read back out of the batch states every one of them
            // again, the identity a message named as its source included.
            let back = yggdryl::text::from_arrow_batch(&batch, &options).expect("lines read back");
            assert_eq!(back.len(), 1);
            assert_eq!(back[0].get_currunix(), INSTANT);
            assert_eq!(back[0].get_seqnum(), 7);
            assert!(back[0].get_state().is_done());
            assert_eq!(back[0].get_prevuuid(), lines[0].get_prevuuid());
            assert_eq!(back[0].get_crosscode(), sourceurl.as_str());
            assert_eq!(back[0].get_curruuid(), lines[0].get_curruuid());
            assert_eq!(back[0].get_crossuuid(), lines[0].get_crossuuid());
            assert_eq!(back[0].get_currhashcode(), lines[0].get_currhashcode());
            assert_eq!(back[0].get_identifiers(), lines[0].get_identifiers());
        }

        #[test]
        fn explicit_event_sequence_and_cross_code_win_over_arrow_base_facts() {
            let mut options = TextOptions::new();
            options.start_rownum = Some(10);
            let mut lines: Vec<TextLine> =
                read_text_lines(&named("events.log", b"body\n"), &options)
                    .expect("a reader")
                    .map(|line| line.expect("a line"))
                    .collect();
            let sourceurl = lines[0].sourceurl().expect("a source").to_string();
            assert_eq!(lines[0].get_seqnum(), 10);
            assert_eq!(lines[0].get_crosscode(), sourceurl.as_str());

            lines[0].set_seqnum(77);
            lines[0].set_crosscode("explicit-chain".to_owned());
            let batch = into_arrow_batch(lines, &options).expect("a batch");
            assert_eq!(
                batch
                    .column_by_name("rownum")
                    .expect("rownum")
                    .as_any()
                    .downcast_ref::<arrow_array::Int64Array>()
                    .expect("a signed row number")
                    .value(0),
                10
            );
            assert_eq!(
                batch
                    .column_by_name("sourceurl")
                    .expect("sourceurl")
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .expect("a URL")
                    .value(0),
                sourceurl
            );
            assert_eq!(
                batch
                    .column_by_name("seqnum")
                    .expect("seqnum")
                    .as_any()
                    .downcast_ref::<arrow_array::UInt64Array>()
                    .expect("a sequence")
                    .value(0),
                77
            );
            assert_eq!(
                batch
                    .column_by_name("crosscode")
                    .expect("crosscode")
                    .as_any()
                    .downcast_ref::<arrow_array::StringArray>()
                    .expect("a code")
                    .value(0),
                "explicit-chain"
            );

            let back = yggdryl::text::from_arrow_batch(&batch, &options).expect("lines read back");
            assert_eq!(back[0].index(), 0);
            assert_eq!(
                back[0].sourceurl().map(ToString::to_string),
                Some(sourceurl)
            );
            assert_eq!(back[0].get_seqnum(), 77);
            assert_eq!(back[0].get_crosscode(), "explicit-chain");
        }

        #[test]
        fn a_negative_row_number_refuses_the_unsigned_event_sequence() {
            let mut options = TextOptions::new();
            options.start_rownum = Some(-1);
            let source = named("events.log", b"body\n");
            let lines: Vec<TextLine> = read_text_lines(&source, &options)
                .expect("a reader")
                .map(|line| line.expect("a line"))
                .collect();
            let refusal = lines[0]
                .seqnum()
                .expect_err("a negative row number is not a sequence")
                .to_string();
            assert!(refusal.contains("rownum"), "{refusal}");
            assert!(refusal.contains("-1"), "{refusal}");
            // The infallible trait door falls back to the physical index.
            assert_eq!(lines[0].get_seqnum(), 0);
            let error = into_arrow_batch(lines, &options)
                .expect_err("the event column refuses the negative sequence")
                .to_string();
            assert!(error.contains("rownum"), "{error}");
            assert!(error.contains("-1"), "{error}");
            assert!(error.contains("physical line 1"), "{error}");
        }
    }
}
