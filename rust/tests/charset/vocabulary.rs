//! The charset vocabulary from outside the crate: names, refusals, repertoire.

use yggdryl::{Charset, Error, MediaType};

/// Every byte the charset assigns, in ascending order.
fn repertoire(charset: Charset) -> Vec<u8> {
    (0..=u8::MAX)
        .filter(|byte| charset.scalar_of(*byte).is_some())
        .collect()
}

#[test]
fn the_crate_root_re_exports_the_vocabulary() {
    let charset: yggdryl::Charset = Charset::Cp1252;
    assert_eq!(charset, yggdryl::charset::Charset::Cp1252);
    assert_eq!(Charset::ALL.len(), 13);
    assert_eq!(Charset::default(), Charset::Utf8);
    assert_eq!(Charset::PARAMETER, "charset");
}

#[test]
fn every_charset_has_one_required_canonical_spelling() {
    for (charset, canonical) in [
        (Charset::Utf8, "utf-8"),
        (Charset::Utf16Le, "utf-16le"),
        (Charset::Utf16Be, "utf-16be"),
        (Charset::Ascii, "us-ascii"),
        (Charset::Latin1, "iso-8859-1"),
        (Charset::Latin2, "iso-8859-2"),
        (Charset::Latin9, "iso-8859-15"),
        (Charset::Cp1250, "windows-1250"),
        (Charset::Cp1251, "windows-1251"),
        (Charset::Cp1252, "windows-1252"),
        (Charset::Cp437, "ibm437"),
        (Charset::Cp850, "ibm850"),
        (Charset::MacRoman, "macintosh"),
    ] {
        assert_eq!(charset.as_str(), canonical);
        assert_eq!(charset.to_string(), canonical);
        assert_eq!(Charset::from_str(canonical).unwrap(), charset);
        assert_eq!(
            Charset::from_str(&canonical.to_uppercase()).unwrap(),
            charset
        );
        assert_eq!(
            Charset::from_str(&format!("  {canonical}\t")).unwrap(),
            charset
        );
        assert_eq!(
            serde_json::to_string(&charset).unwrap(),
            format!("\"{canonical}\"")
        );
        assert_eq!(
            serde_json::from_str::<Charset>(&format!("\"{canonical}\"")).unwrap(),
            charset
        );
    }
    assert_eq!(Charset::ALL.len(), 13);
}

#[test]
fn an_unknown_charset_name_names_the_vocabulary_it_is_not_in() {
    let error = Charset::from_str("shift_jis").unwrap_err();
    assert!(matches!(
        error,
        Error::Parse {
            target: "charset",
            ..
        }
    ));
    let rendered = error.to_string();
    assert!(rendered.contains("utf-8"), "{rendered}");
    assert!(rendered.contains("windows-1252"), "{rendered}");
    assert!(rendered.contains("shift_jis"), "{rendered}");
}

#[test]
fn a_charset_reports_the_shape_of_its_units() {
    for charset in Charset::ALL {
        assert_eq!(
            charset.is_single_byte(),
            charset.unit_size() == 1 && !charset.is_utf8()
        );
        assert_eq!(charset.is_unicode(), charset.bom().is_some());
        assert_eq!(
            charset.is_ascii_compatible(),
            !matches!(charset, Charset::Utf16Le | Charset::Utf16Be)
        );
    }
    assert_eq!(Charset::Utf16Be.unit_size(), 2);
    assert_eq!(Charset::Cp850.unit_size(), 1);
}

#[test]
fn every_charset_round_trips_the_text_it_can_hold() {
    for charset in Charset::ALL {
        if !charset.is_single_byte() {
            continue;
        }
        let bytes = repertoire(charset);
        let decoded = charset.decode(&bytes).expect("its own repertoire");
        assert_eq!(
            charset.encode(&decoded).unwrap().as_ref(),
            bytes,
            "{charset}"
        );
    }

    // The Unicode forms hold every scalar rather than a repertoire.
    let text = "id,name\n1,Grüße 😀 Ω\n";
    for charset in [Charset::Utf8, Charset::Utf16Le, Charset::Utf16Be] {
        let encoded = charset.encode(text).unwrap();
        assert_eq!(charset.decode(&encoded).unwrap(), text, "{charset}");
    }
}

#[test]
fn one_payload_reads_differently_under_each_charset_that_claims_it() {
    let wire = b"caf\xe9 \x80";
    assert_eq!(Charset::Latin1.decode(wire).unwrap(), "café \u{0080}");
    assert_eq!(Charset::Cp1252.decode(wire).unwrap(), "café €");
    assert_eq!(Charset::Latin9.decode(wire).unwrap(), "café \u{0080}");
    assert_eq!(Charset::Cp437.decode(wire).unwrap(), "cafΘ Ç");
    assert!(Charset::Ascii.decode(wire).is_err());
    assert!(Charset::Utf8.decode(wire).is_err());
}

#[test]
fn a_refusal_names_the_charset_the_position_and_the_byte() {
    let error = Charset::Cp1250.decode(b"id,\x83").unwrap_err();
    let Error::Codec {
        format,
        position,
        reason,
    } = error
    else {
        panic!("expected a codec refusal");
    };
    assert_eq!(format, "windows-1250");
    assert_eq!(position, 3);
    assert!(reason.contains("0x83"), "{reason}");
}

#[test]
fn a_media_type_declaration_is_what_selects_a_charset() {
    let declared = MediaType::from_str("text/csv;charset=cp1252").unwrap();
    assert_eq!(Charset::from_media_type(&declared), Charset::Cp1252);
    assert_eq!(declared.charset(), Some(Charset::Cp1252));

    let silent = MediaType::from_str("text/csv").unwrap();
    assert_eq!(silent.charset(), None);
    assert_eq!(Charset::from_media_type(&silent), Charset::Utf8);

    let url = yggdryl::Url::from_str("file:///trades.csv").unwrap();
    assert_eq!(Charset::from_url(&url), Charset::Utf8);
}

#[test]
fn three_doors_read_a_damaged_payload_three_ways() {
    // `0x81` is one of the five bytes windows-1252 leaves unassigned.
    let damaged = b"ok\x81";
    assert!(Charset::Cp1252.decode(damaged).is_err());
    assert_eq!(Charset::Cp1252.decode_lossy(damaged), "ok\u{FFFD}");
    // The permissive door recovers rather than replaces: a byte the table
    // leaves unassigned reads as the C1 control of its number, which for
    // these five is what the WHATWG Encoding Standard's own index answers.
    assert_eq!(Charset::Cp1252.transcribe(damaged), "ok\u{0081}");

    // Bytes offered as UTF-8 that are not UTF-8 still read: every valid run
    // kept, every other byte as Windows-1252, through the same table.
    assert!(Charset::Utf8.decode(b"caf\xe9").is_err());
    assert_eq!(Charset::Utf8.decode_lossy(b"caf\xe9"), "caf\u{FFFD}");
    assert_eq!(Charset::Utf8.transcribe(b"caf\xe9"), "café");

    // A lone surrogate is not a scalar in any encoding, so there is nothing
    // to transcribe it to and the permissive door replaces it too.
    assert_eq!(Charset::Utf16Le.transcribe(b"\x00\xd8"), "\u{FFFD}");

    // A charset that assigns every byte has nothing to recover.
    for charset in [Charset::Latin1, Charset::Cp437, Charset::MacRoman] {
        assert_eq!(charset.transcribe(damaged), charset.decode_lossy(damaged));
    }
}

/// One reading through both transcribing doors, which must agree.
fn transcribed(charset: Charset, input: &[u8]) -> String {
    let text = charset.transcribe(input);
    assert_eq!(
        charset.transcribe_smol(input).as_str(),
        &*text,
        "{charset} {input:?}"
    );
    text.into_owned()
}

/// The WHATWG Encoding Standard's `windows-1252` index over `0x80..=0x9F`,
/// the row where it and ISO 8859-1 disagree. The five C1 controls in it are
/// the bytes the classic table leaves undefined.
const WINDOWS_1252_C1_ROW: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

#[test]
fn bytes_offered_as_utf8_read_by_one_rule_per_invalid_run() {
    // Decision 10's lines, once the text line's own, read at the layer now
    // and unmoved in value: a stray byte among UTF-8, a wholly Windows-1252
    // line, each of the five holes, and a character a byte limit cut in two.
    assert_eq!(
        transcribed(Charset::Utf8, b"58=caf\xE9 caf\xC3\xA9|10=0|"),
        "58=caf\u{e9} caf\u{e9}|10=0|"
    );
    assert_eq!(
        transcribed(Charset::Utf8, b"\x80 \x93quoted\x94 \x96 na\xEFve"),
        "\u{20AC} \u{201C}quoted\u{201D} \u{2013} na\u{ef}ve"
    );
    for byte in [0x81_u8, 0x8D, 0x8F, 0x90, 0x9D] {
        assert_eq!(
            transcribed(Charset::Utf8, &[b'a', byte, b'b']),
            format!("a{}b", char::from(byte))
        );
    }
    assert_eq!(
        transcribed(Charset::Utf8, b"58=\xE2\x82"),
        "58=\u{e2}\u{201A}"
    );

    // The mixed line a whole-buffer reading got wrong: the valid `é` stays
    // `é`, and only the stray byte reads as Windows-1252.
    assert_eq!(transcribed(Charset::Utf8, b"caf\xC3\xA9 \xE9"), "café é");
    assert_eq!(
        transcribed(Charset::Utf8, b"\x93x\x94"),
        "\u{201C}x\u{201D}"
    );
    // Past the inline width the compact door builds a `String`; same rule.
    assert_eq!(
        transcribed(
            Charset::Utf8,
            b"caf\xC3\xA9 \xE9 with a tail past the inline width"
        ),
        "café é with a tail past the inline width"
    );
}

#[test]
fn the_c1_row_reads_as_the_whatwg_index_under_utf8_and_under_windows_1252() {
    for (byte, expected) in (0x80_u8..=0x9F).zip(WINDOWS_1252_C1_ROW) {
        let expected = expected.to_string();
        assert_eq!(
            transcribed(Charset::Utf8, &[byte]),
            expected,
            "{byte:#04x} offered as UTF-8"
        );
        assert_eq!(
            transcribed(Charset::Cp1252, &[byte]),
            expected,
            "{byte:#04x} under windows-1252"
        );
    }
}

#[test]
fn us_ascii_reads_a_broken_promise_exactly_as_utf8_does() {
    // A US-ASCII declaration is a UTF-8 declaration with a narrower promise,
    // so a valid UTF-8 `é` stays `é` rather than reading as two Latin-1
    // letters, and a stray byte reads as Windows-1252; `decode` still refuses
    // it, and `decode_lossy` still marks it.
    assert_eq!(transcribed(Charset::Ascii, b"caf\xC3\xA9"), "café");
    assert_eq!(transcribed(Charset::Ascii, b"\x80"), "\u{20AC}");
    assert!(Charset::Ascii.decode(b"\x80").is_err());
    assert_eq!(Charset::Ascii.decode_lossy(b"\x80"), "\u{FFFD}");
    for input in [b"caf\xE9".as_slice(), b"58=\xE2\x82", b"\x93x\x94"] {
        assert_eq!(
            transcribed(Charset::Ascii, input),
            transcribed(Charset::Utf8, input),
            "{input:?}"
        );
    }
}

#[test]
fn a_stored_length_is_counted_rather_than_built() {
    // Four scalars: four bytes in windows-1252, five in UTF-8.
    assert_eq!(Charset::Cp1252.encoded_len("café"), 4);
    assert_eq!(Charset::Utf8.encoded_len("café"), 5);
    // UTF-16 counts its units, surrogate pairs included.
    assert_eq!(Charset::Utf16Le.encoded_len("a😀"), 6);

    // Whatever it counts, it is the length the encoder writes.
    for charset in Charset::ALL {
        for text in ["", "AAPL", "café", "Grüße"] {
            if let Ok(encoded) = charset.encode(text) {
                assert_eq!(
                    charset.encoded_len(text),
                    encoded.len(),
                    "{charset} {text:?}"
                );
            }
        }
    }

    // It counts rather than judges: a scalar the charset has no byte for
    // still occupies the byte it would occupy, because that is what a bound
    // asks about, and refusing here would make `transcribe` unusable - the
    // scalars it recovers are exactly the ones the charset does not assign.
    assert_eq!(Charset::Cp1252.encoded_len("ok東京"), 4);
    assert!(Charset::Cp1252.encode("ok東京").is_err());
    assert_eq!(
        Charset::Cp1252.encoded_len(&Charset::Cp1252.transcribe(b"ok\x81")),
        3
    );
}
