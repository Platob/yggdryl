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
    // The permissive door recovers rather than replaces: ISO 8859-1 assigns
    // every byte, and for these five it is what the WHATWG Encoding
    // Standard's own index answers.
    assert_eq!(Charset::Cp1252.transcribe(damaged), "ok\u{0081}");

    // Bytes offered as UTF-8 that are not UTF-8 still read.
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
