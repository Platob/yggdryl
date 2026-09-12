//! Edge behavior of the charset vocabulary and its four operations.

use std::borrow::Cow;
use std::io::{Read, Write};

use super::{Charset, Transcoded, utf8_transcribe_into};
use crate::holder::Buffer;
use crate::{Error, IOBase, MediaType};

/// Every byte a single-byte charset assigns, as that charset spells them.
fn every_assigned_byte(charset: Charset) -> Vec<u8> {
    (0..=u8::MAX)
        .filter(|byte| charset.scalar_of(*byte).is_some())
        .collect()
}

#[test]
fn every_canonical_name_parses_back_to_its_charset() {
    for charset in Charset::ALL {
        assert_eq!(Charset::from_str(charset.as_str()).unwrap(), charset);
        assert_eq!(charset.to_string(), charset.as_str());
        assert_eq!(
            Charset::from_str(&charset.as_str().to_uppercase()).unwrap(),
            charset
        );
    }
}

#[test]
fn canonical_names_are_distinct() {
    let mut names: Vec<&str> = Charset::ALL
        .iter()
        .map(|charset| charset.as_str())
        .collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(names.len(), total);
}

#[test]
fn customary_aliases_resolve_to_the_same_value() {
    for (alias, expected) in [
        ("utf8", Charset::Utf8),
        ("  UTF-8  ", Charset::Utf8),
        ("cp1252", Charset::Cp1252),
        ("Windows-1252", Charset::Cp1252),
        ("latin1", Charset::Latin1),
        ("ISO_8859-1", Charset::Latin1),
        ("l9", Charset::Latin9),
        ("ansi_x3.4-1968", Charset::Ascii),
        ("cp437", Charset::Cp437),
        ("macroman", Charset::MacRoman),
        ("utf16le", Charset::Utf16Le),
    ] {
        assert_eq!(Charset::from_str(alias).unwrap(), expected, "{alias}");
    }
}

#[test]
fn latin1_is_not_a_spelling_of_windows_1252() {
    // The two disagree exactly over the C1 range, which is the whole reason
    // they are separate values rather than one with an alias.
    assert_eq!(Charset::Latin1.decode(b"\x80").unwrap(), "\u{0080}");
    assert_eq!(Charset::Cp1252.decode(b"\x80").unwrap(), "\u{20AC}");
}

#[test]
fn an_ambiguous_or_unknown_name_is_refused() {
    for name in ["utf-16", "utf16", "shift_jis", "", "cp9999"] {
        let error = Charset::from_str(name).unwrap_err();
        assert!(
            matches!(error, Error::Parse { target, .. } if target == "charset"),
            "{name}: {error:?}"
        );
    }
}

#[test]
fn a_charset_serializes_as_its_canonical_name() {
    for charset in Charset::ALL {
        let json = serde_json::to_string(&charset).unwrap();
        assert_eq!(json, format!("{:?}", charset.as_str()));
        assert_eq!(
            serde_json::from_str::<Charset>(&json).unwrap(),
            charset,
            "{charset}"
        );
    }
    assert!(serde_json::from_str::<Charset>("\"utf-16\"").is_err());
    // A parsed document cannot lend its strings, and reads all the same.
    assert_eq!(
        serde_json::from_value::<Charset>(serde_json::json!("us-ascii")).unwrap(),
        Charset::Ascii
    );
}

#[test]
fn the_default_charset_is_utf8() {
    assert_eq!(Charset::default(), Charset::Utf8);
    assert!(Charset::Utf8.is_utf8());
    assert!(Charset::Utf8.is_unicode());
    assert!(!Charset::Utf8.is_single_byte());
}

#[test]
fn ascii_input_is_borrowed_rather_than_transcoded() {
    let plain = b"symbol,price\nAAPL,187.23\n";
    for charset in Charset::ALL {
        if !charset.is_ascii_compatible() {
            continue;
        }
        assert!(
            matches!(charset.decode(plain).unwrap(), Cow::Borrowed(_)),
            "{charset} copied an all-ASCII payload"
        );
        assert!(
            matches!(charset.encode("symbol,price").unwrap(), Cow::Borrowed(_)),
            "{charset} copied all-ASCII text"
        );
    }
}

#[test]
fn a_long_ascii_run_crossing_word_lanes_is_still_borrowed() {
    // The scan reads a machine word at a time and then finishes byte by byte,
    // so every length modulo the lane width has to answer the same.
    for length in 0..64 {
        let payload = vec![b'a'; length];
        assert!(matches!(
            Charset::Cp1252.decode(&payload).unwrap(),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            Charset::Cp1252.decode(&payload).unwrap().len(),
            length,
            "{length}"
        );
    }
}

#[test]
fn a_high_byte_anywhere_in_a_long_run_is_found() {
    for position in 0..40 {
        let mut payload = vec![b'a'; 40];
        payload[position] = 0xE9;
        let decoded = Charset::Latin1.decode(&payload).unwrap();
        assert!(matches!(decoded, Cow::Owned(_)));
        assert_eq!(decoded.chars().nth(position), Some('é'), "{position}");
        assert_eq!(decoded.chars().count(), 40);
    }
}

#[test]
fn every_single_byte_charset_round_trips_its_whole_repertoire() {
    for charset in Charset::ALL {
        if !charset.is_single_byte() || charset == Charset::Ascii {
            continue;
        }
        let bytes = every_assigned_byte(charset);
        let decoded = charset.decode(&bytes).unwrap();
        assert_eq!(decoded.chars().count(), bytes.len(), "{charset}");
        assert_eq!(
            charset.encode(&decoded).unwrap().as_ref(),
            bytes,
            "{charset}"
        );
        for (scalar, byte) in decoded.chars().zip(&bytes) {
            assert_eq!(charset.byte_of(scalar), Some(*byte), "{charset} {scalar}");
            assert_eq!(
                charset.scalar_of(*byte),
                Some(scalar),
                "{charset} {byte:#04x}"
            );
        }
    }
}

#[test]
fn an_unassigned_byte_is_refused_with_its_position() {
    // Windows-1250, -1251 and -1252 are the charsets here that leave bytes
    // unassigned; every other one answers for all 256.
    let error = Charset::Cp1252.decode(b"ok\x81").unwrap_err();
    let Error::Codec {
        format,
        position,
        reason,
    } = error
    else {
        panic!("expected a codec refusal");
    };
    assert_eq!(format, "windows-1252");
    assert_eq!(position, 2);
    assert!(reason.contains("0x81"), "{reason}");
}

#[test]
fn a_lossy_decode_replaces_what_it_cannot_read() {
    assert_eq!(Charset::Cp1252.decode_lossy(b"ok\x81!"), "ok\u{FFFD}!");
    assert_eq!(Charset::Ascii.decode_lossy(b"ok\xff"), "ok\u{FFFD}");
    assert_eq!(Charset::Utf8.decode_lossy(b"ok\xff"), "ok\u{FFFD}");
    assert_eq!(Charset::Utf16Le.decode_lossy(b"A\x00\x00\xd8"), "A\u{FFFD}");
}

#[test]
fn a_scalar_the_charset_has_no_byte_for_is_refused() {
    let error = Charset::Latin1.encode("prix 12€").unwrap_err();
    let Error::Codec {
        format,
        position,
        reason,
    } = error
    else {
        panic!("expected a codec refusal");
    };
    assert_eq!(format, "iso-8859-1");
    assert_eq!(position, 7);
    assert!(reason.contains("U+20AC"), "{reason}");
    // The same scalar is one byte in the charset that has it.
    assert_eq!(
        Charset::Cp1252.encode("prix 12€").unwrap().as_ref(),
        b"prix 12\x80"
    );
}

#[test]
fn ascii_refuses_every_byte_above_its_range() {
    let error = Charset::Ascii.decode(b"caf\xe9").unwrap_err();
    assert!(matches!(error, Error::Codec { position: 3, .. }));
    assert!(Charset::Ascii.encode("café").is_err());
    assert_eq!(Charset::Ascii.decode(b"cafe").unwrap(), "cafe");
}

#[test]
fn utf8_is_validated_and_located() {
    assert_eq!(Charset::Utf8.decode("café".as_bytes()).unwrap(), "café");
    let error = Charset::Utf8.decode(b"caf\xc3").unwrap_err();
    assert!(
        matches!(&error, Error::Codec { format, position, reason }
            if *format == "utf-8" && *position == 3 && reason.contains("end of the input")),
        "{error:?}"
    );
    let error = Charset::Utf8.decode(b"caf\xff!").unwrap_err();
    assert!(
        matches!(error, Error::Codec { position: 3, .. }),
        "{error:?}"
    );
}

#[test]
fn utf16_round_trips_both_orders_including_astral_scalars() {
    let text = "Grüße 😀 Ω";
    for charset in [Charset::Utf16Le, Charset::Utf16Be] {
        let encoded = charset.encode(text).unwrap();
        assert_eq!(encoded.len() % 2, 0);
        assert_eq!(charset.decode(&encoded).unwrap(), text, "{charset}");
        assert_eq!(charset.unit_size(), 2);
        assert!(!charset.is_ascii_compatible());
    }
    // The two orders are byte-swapped spellings of the same units.
    let little = Charset::Utf16Le.encode(text).unwrap().into_owned();
    let big = Charset::Utf16Be.encode(text).unwrap().into_owned();
    assert_eq!(little.len(), big.len());
    for pair in 0..little.len() / 2 {
        assert_eq!(little[pair * 2], big[pair * 2 + 1]);
    }
}

#[test]
fn utf16_refuses_a_half_unit_and_a_lone_surrogate() {
    let error = Charset::Utf16Le.decode(b"A\x00B").unwrap_err();
    assert!(
        matches!(&error, Error::Codec { format, position, .. }
            if *format == "utf-16le" && *position == 2),
        "{error:?}"
    );
    let error = Charset::Utf16Le.decode(b"\x00\xd8\x00\x00").unwrap_err();
    assert!(
        matches!(&error, Error::Codec { reason, .. } if reason.contains("surrogate")),
        "{error:?}"
    );
}

#[test]
fn a_byte_order_mark_names_its_charset_and_its_own_length() {
    assert_eq!(
        Charset::from_bom(b"\xef\xbb\xbfid"),
        Some((Charset::Utf8, 3))
    );
    assert_eq!(
        Charset::from_bom(b"\xff\xfeA\x00"),
        Some((Charset::Utf16Le, 2))
    );
    assert_eq!(
        Charset::from_bom(b"\xfe\xff\x00A"),
        Some((Charset::Utf16Be, 2))
    );
    assert_eq!(Charset::from_bom(b"id,name"), None);
    assert_eq!(Charset::from_bom(b""), None);

    assert_eq!(Charset::Utf8.bom(), Some(b"\xef\xbb\xbf".as_slice()));
    assert_eq!(Charset::Cp1252.bom(), None);
    // Nothing is stripped on a caller's behalf: the mark is a scalar until a
    // caller decides it is framing.
    assert_eq!(
        Charset::Utf8.decode(b"\xef\xbb\xbfid").unwrap(),
        "\u{FEFF}id"
    );
}

#[test]
fn a_chunked_decode_agrees_with_a_whole_one_at_every_split() {
    let text = "Grüße 😀 Ω tail";
    for charset in [Charset::Utf8, Charset::Utf16Le, Charset::Utf16Be] {
        let encoded = charset.encode(text).unwrap().into_owned();
        for split in 0..=encoded.len() {
            let mut decoder = charset.decoder();
            let mut decoded = String::new();
            decoder.push(&encoded[..split], &mut decoded).unwrap();
            decoder.push(&encoded[split..], &mut decoded).unwrap();
            decoder.finish().unwrap();
            assert_eq!(decoded, text, "{charset} split at {split}");
        }
    }
}

#[test]
fn a_chunked_decode_survives_one_byte_at_a_time() {
    let text = "Grüße 😀 Ω";
    for charset in [Charset::Utf8, Charset::Utf16Be, Charset::Cp1252] {
        let encoded = charset.encode("Gru\u{00df}e").unwrap().into_owned();
        let expected = charset.decode(&encoded).unwrap().into_owned();
        let mut decoder = charset.decoder();
        let mut decoded = String::new();
        for byte in &encoded {
            decoder.push(&[*byte], &mut decoded).unwrap();
        }
        decoder.finish().unwrap();
        assert_eq!(decoded, expected, "{charset}");
    }
    let mut decoder = Charset::Utf8.decoder();
    let mut decoded = String::new();
    decoder.push(text.as_bytes(), &mut decoded).unwrap();
    assert_eq!(decoded, text);
    assert_eq!(decoder.consumed(), text.len() as u64);
}

#[test]
fn a_decode_that_stops_mid_sequence_is_a_refusal_not_silence() {
    let mut decoder = Charset::Utf8.decoder();
    let mut decoded = String::new();
    decoder.push(b"caf\xc3", &mut decoded).unwrap();
    assert_eq!(decoded, "caf");
    assert!(decoder.is_pending());
    assert!(decoder.finish().is_err());
}

#[test]
fn a_chunked_decode_reports_a_broken_sequence_rather_than_holding_it() {
    let mut decoder = Charset::Utf16Le.decoder();
    let mut decoded = String::new();
    // A run of high surrogates never pairs, so it must refuse rather than
    // accumulate without bound.
    let error = (0..8)
        .map(|_| decoder.push(b"\x00\xd8", &mut decoded))
        .find_map(std::result::Result::err);
    assert!(
        error.is_some(),
        "an unpaired run was held rather than refused"
    );
}

#[test]
fn a_reader_yields_utf8_for_an_encoded_source() {
    let wire = Charset::Cp1252
        .encode("symbol,désk\nAAPL,€1\n")
        .unwrap()
        .into_owned();
    let mut decoded = String::new();
    Charset::Cp1252
        .reader(std::io::Cursor::new(wire))
        .read_to_string(&mut decoded)
        .unwrap();
    assert_eq!(decoded, "symbol,désk\nAAPL,€1\n");
}

#[test]
fn a_reader_over_a_truncated_source_fails_at_its_end() {
    let mut decoded = String::new();
    let failure = Charset::Utf8
        .reader(std::io::Cursor::new(b"caf\xc3".to_vec()))
        .read_to_string(&mut decoded);
    // UTF-8 reads through unchanged, so the refusal is the standard library's.
    assert!(failure.is_err());

    let mut decoded = Vec::new();
    let failure = Charset::Utf16Le
        .reader(std::io::Cursor::new(b"A\x00B".to_vec()))
        .read_to_end(&mut decoded);
    assert!(failure.is_err());
}

#[test]
fn a_writer_encodes_text_written_in_any_chunks() {
    let text = "symbol,désk\nAAPL,€1\n";
    for chunk in [1, 2, 3, 7, text.len()] {
        let mut target = Vec::new();
        {
            let mut writer = Charset::Cp1252.writer(&mut target);
            for piece in text.as_bytes().chunks(chunk) {
                writer.write_all(piece).unwrap();
            }
            writer.finish().unwrap();
        }
        assert_eq!(
            target,
            Charset::Cp1252.encode(text).unwrap().as_ref(),
            "{chunk}"
        );
    }
}

#[test]
fn a_writer_left_mid_scalar_refuses_to_finish() {
    let mut target = Vec::new();
    let mut writer = Charset::Cp1252.writer(&mut target);
    writer.write_all(b"caf\xc3").unwrap();
    assert!(writer.finish().is_err());
}

#[test]
fn a_transcoded_handle_reads_and_writes_utf8_over_encoded_bytes() {
    let mut handle = Transcoded::new(Buffer::new(), Charset::Cp1252);
    handle.write_all_bytes("symbol,désk\n".as_bytes()).unwrap();
    handle.flush().unwrap();

    assert_eq!(handle.charset(), Charset::Cp1252);
    assert_eq!(handle.read_all_bytes().unwrap(), "symbol,désk\n".as_bytes());
    assert_eq!(
        handle.handle().read_all_bytes().unwrap(),
        b"symbol,d\xe9sk\n"
    );
    assert_eq!(handle.size(), "symbol,désk\n".len() as u64);
}

#[test]
fn a_transcoded_handle_answers_ranges_out_of_the_decoded_value() {
    let mut source = Buffer::new();
    source
        .write_all_bytes(&Charset::Latin1.encode("étoile du matin").unwrap())
        .unwrap();
    let handle = Transcoded::new(source, Charset::Latin1);

    let decoded = "étoile du matin";
    assert_eq!(handle.read_all_bytes().unwrap(), decoded.as_bytes());
    assert_eq!(
        handle.read_range_bytes(0, 3).unwrap(),
        decoded.as_bytes()[..3]
    );
    let mut window = [0_u8; 4];
    let read = handle.pread(2, &mut window).unwrap();
    assert_eq!(&window[..read], &decoded.as_bytes()[2..2 + read]);
}

#[test]
fn a_transcoded_handle_reports_the_decoded_media_type() {
    let declared = MediaType::from_str("text/csv;charset=windows-1252").unwrap();
    let source = Buffer::new().with_media_type(declared);
    let handle = Transcoded::infer(source);

    assert_eq!(handle.charset(), Charset::Cp1252);
    assert_eq!(handle.media_type().charset(), None);
    assert_eq!(handle.media_type().to_string(), "text/csv");
}

#[test]
fn a_utf8_transcoded_handle_changes_nothing() {
    let mut handle = Transcoded::new(Buffer::new(), Charset::Utf8);
    handle.write_all_bytes("plain".as_bytes()).unwrap();
    handle.flush().unwrap();
    assert_eq!(handle.read_all_bytes().unwrap(), b"plain");
    assert_eq!(handle.handle().read_all_bytes().unwrap(), b"plain");
}

#[test]
fn a_media_type_carries_the_charset_through_text_and_serde() {
    let declared = MediaType::from_str("text/csv;charset=windows-1252").unwrap();
    assert_eq!(declared.charset(), Some(Charset::Cp1252));
    assert!(declared.is_charset_declared());
    assert_eq!(declared.to_string(), "text/csv;charset=windows-1252");
    assert_eq!(
        MediaType::from_str(&declared.to_string()).unwrap(),
        declared
    );

    let json = serde_json::to_string(&declared).unwrap();
    assert_eq!(serde_json::from_str::<MediaType>(&json).unwrap(), declared);

    let both = MediaType::from_str("text/csv;charset=latin1;encodings=application/gzip").unwrap();
    assert_eq!(both.charset(), Some(Charset::Latin1));
    assert_eq!(both.encodings().len(), 1);
    assert_eq!(
        both.to_string(),
        "text/csv;charset=iso-8859-1;encodings=application/gzip"
    );
    // The two attributes are order-free on the way in.
    assert_eq!(
        MediaType::from_str("text/csv;encodings=application/gzip;charset=latin1").unwrap(),
        both
    );
    assert_eq!(both.without_charset().charset(), None);
}

#[test]
fn a_media_type_refuses_an_unknown_or_repeated_attribute() {
    assert!(MediaType::from_str("text/csv;boundary=xyz").is_err());
    assert!(MediaType::from_str("text/csv;charset=nope").is_err());
    assert!(MediaType::from_str("text/csv;charset=utf-8;charset=latin1").is_err());
}

#[test]
fn content_headers_keep_the_charset_the_header_declared() {
    let media_type =
        MediaType::from_content_headers(Some("text/csv; charset=Windows-1252"), None).unwrap();
    assert_eq!(media_type.charset(), Some(Charset::Cp1252));
    assert_eq!(Charset::from_media_type(&media_type), Charset::Cp1252);

    let encoded =
        MediaType::from_content_headers(Some("text/csv; charset=\"utf-8\""), Some("gzip")).unwrap();
    assert_eq!(encoded.charset(), Some(Charset::Utf8));
    assert_eq!(encoded.encodings().len(), 1);

    let bare = MediaType::from_content_headers(Some("text/csv"), None).unwrap();
    assert_eq!(bare.charset(), None);
    // Declaring nothing reads as UTF-8 without recording a declaration.
    assert_eq!(Charset::from_media_type(&bare), Charset::Utf8);

    assert!(MediaType::from_content_headers(Some("text/csv; charset=nope"), None).is_err());
}

#[test]
fn a_charset_is_send_and_sync_like_every_other_shared_enum() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Charset>();
}

#[test]
fn a_transcription_counts_the_bytes_it_read_as_windows_1252() {
    fn read(input: &[u8]) -> (String, usize) {
        let mut text = String::new();
        let count = utf8_transcribe_into(input, &mut text);
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
    assert_eq!(utf8_transcribe_into(b"caf\xe9", &mut text), 1);
    assert_eq!(text, "58=caf\u{e9}");
}

#[test]
fn valid_utf8_is_borrowed_under_utf8_and_under_us_ascii() {
    // A US-ASCII declaration is a UTF-8 declaration with a narrower promise,
    // so the borrow `transcribe` takes for UTF-8 it takes for US-ASCII too,
    // where `decode` still refuses the byte above `0x7F`.
    for charset in [Charset::Utf8, Charset::Ascii] {
        assert!(
            matches!(
                charset.transcribe("caf\u{e9}".as_bytes()),
                Cow::Borrowed("caf\u{e9}")
            ),
            "{charset}"
        );
        assert!(
            matches!(charset.transcribe(b"caf\xe9"), Cow::Owned(text) if text == "caf\u{e9}"),
            "{charset}"
        );
    }
    assert!(Charset::Ascii.decode("caf\u{e9}".as_bytes()).is_err());
}
