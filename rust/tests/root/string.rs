//! `rust/src/string.rs`: the inline threshold and the code readers.
//!
//! `INLINE_CAPACITY` is the byte count below which a `Str` stores its text in
//! the value rather than behind an `Arc`. It is published as
//! `yggdryl::INLINE_CAPACITY`, so `scalars` crosses the boundary through
//! `yggdryl::` like any other value here and runs in a default build; the
//! rest of what a caller can observe about `Str` lives in
//! `tests/types/strings.rs`.
//!
//! `code_text`, `code_cell_text` and `code_for_extension` are crate-private:
//! they are the doors every registered code goes through, and a caller sees
//! only the datatype they answer for, so `codes` reaches them through
//! `yggdryl::internals`. The rest of that suite lives in
//! `tests/types/datatype/coded.rs`.

mod scalars {
    use yggdryl::StringType;
    use yggdryl::{Charset, DataType, Scalar};
    use yggdryl::{INLINE_CAPACITY, Str};

    #[test]
    fn short_text_is_inline_and_long_text_is_shared() {
        let short = Str::new("a".repeat(INLINE_CAPACITY));
        assert!(short.is_inline());
        assert_eq!(short.len(), INLINE_CAPACITY);
        let long = Str::new("a".repeat(INLINE_CAPACITY + 1));
        assert!(!long.is_inline());
        assert_eq!(long.len(), INLINE_CAPACITY + 1);
        assert!(Str::new_static("held").is_inline());
        assert_eq!(Str::default(), "");
        assert_eq!(std::mem::size_of::<Str>(), 32);
    }

    #[test]
    fn equality_order_and_hash_read_the_characters_only() {
        use std::collections::HashSet;

        let plain = Str::new("Grüße");
        let latin = Str::new("Grüße")
            .try_with_parameters(StringType::LargeCp1252String)
            .unwrap();
        assert_eq!(plain, latin);
        assert_eq!(plain.cmp(&latin), std::cmp::Ordering::Equal);
        assert_eq!(HashSet::from([plain.clone(), latin.clone()]).len(), 1);
        assert_ne!(plain.parameters(), latin.parameters());
        assert!(Str::new("b") > Str::new("a"));
        assert_eq!(format!("{latin:?}"), "\"Grüße\" as large_cp1252");
        assert_eq!(format!("{plain:?}"), "\"Grüße\"");
    }

    #[test]
    fn restating_shares_the_storage_and_checks_but_never_carries_a_maximum() {
        let text = "x".repeat(INLINE_CAPACITY + 22);
        let shared = Str::new(&text);
        let restated = shared
            .clone()
            .try_with_parameters(StringType::SizedUtf8String(64))
            .unwrap();
        assert!(std::ptr::eq(shared.as_str(), restated.as_str()));
        assert_eq!(restated.parameters(), StringType::default());
        assert_eq!(restated.dtype().unwrap(), DataType::utf8());

        let refused = shared
            .try_with_parameters(StringType::SizedUtf8String(8))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 8 bytes"), "{refused}");

        // The bound counts stored bytes, so five scalars are five bytes in
        // windows-1252 and seven in UTF-8.
        assert!(
            Str::new("Grüße")
                .try_with_parameters(StringType::SizedCp1252String(5))
                .is_ok()
        );
        assert!(
            Str::new("Grüße")
                .try_with_parameters(StringType::SizedUtf8String(5))
                .is_err()
        );
    }

    #[test]
    fn us_ascii_is_a_repertoire_and_every_other_charset_is_counted() {
        let ascii = StringType::AsciiString;
        assert!(Str::new("plain").try_with_parameters(ascii).is_ok());
        let refused = Str::new("café")
            .try_with_parameters(ascii)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("non-ASCII byte"), "{refused}");
        assert!(Str::new("a\0b").try_with_parameters(ascii).is_err());
        // `U+0081` has no windows-1252 byte, and the value door only counts.
        let recovered = Str::new("ok\u{0081}")
            .try_with_parameters(StringType::Cp1252String)
            .unwrap();
        assert!(recovered.encode().is_err());
        assert_eq!(recovered.encoded_len(), 3);
    }

    #[test]
    fn a_fixed_leaf_trims_its_padding_and_pads_on_the_way_out() {
        let fixed = StringType::FixedAsciiString(4);
        let value = Str::new("USD\0").try_with_parameters(fixed).unwrap();
        assert_eq!(value, "USD");
        assert_eq!(value.fixed(), Some(4));
        assert_eq!(value.parameters(), fixed);
        assert_eq!(value.encode().unwrap().as_ref(), b"USD\0");
        assert_eq!(value.encoded_len(), 4);
        assert!(Str::new("EURO!").try_with_parameters(fixed).is_err());
        assert!(
            Str::new("x")
                .try_with_parameters(StringType::FixedUtf8String(0))
                .is_err()
        );
    }

    #[test]
    fn bytes_are_read_strictly_or_transcribed_by_their_charset() {
        let latin = StringType::Cp1252String;
        let value = Str::from_bytes(b"Gr\xFC\xDFe", latin).unwrap();
        assert_eq!(value, "Grüße");
        assert_eq!(value.charset(), Charset::Cp1252);
        assert_eq!(value.encode().unwrap().as_ref(), b"Gr\xFC\xDFe");
        // `0x81` is unassigned in windows-1252 and still reads.
        assert_eq!(Str::from_bytes(b"ok\x81", latin).unwrap(), "ok\u{0081}");
        // UTF-8 and US-ASCII are validated, not transcribed.
        assert!(Str::from_bytes(b"caf\xe9", StringType::default()).is_err());
        assert!(Str::from_bytes(b"caf\xc3\xa9", StringType::AsciiString).is_err());
        assert_eq!(
            Str::from_bytes(b"caf\xc3\xa9", StringType::default()).unwrap(),
            "café"
        );
        // A padded slot comes back trimmed and carries its width.
        let padded = Str::from_bytes(b"ab\0\0\0\0", StringType::FixedUtf8String(6)).unwrap();
        assert_eq!(padded, "ab");
        assert_eq!(padded.fixed(), Some(6));
    }

    #[test]
    fn serde_writes_the_text_alone_unless_the_value_declares_more() {
        let plain = Str::new("plain");
        assert_eq!(serde_json::to_string(&plain).unwrap(), "\"plain\"");
        assert_eq!(serde_json::from_str::<Str>("\"plain\"").unwrap(), plain);
        let latin = Str::new("Grüße")
            .try_with_parameters(StringType::FixedCp1252String(8))
            .unwrap();
        let document = serde_json::to_string(&latin).unwrap();
        assert_eq!(
            document,
            r#"{"layout":"fixed_cp1252","fixed":8,"text":"Grüße"}"#
        );
        let back = serde_json::from_str::<Str>(&document).unwrap();
        assert_eq!(back.parameters(), latin.parameters());
        assert_eq!(back, latin);
        // A charset beside a charset-free layout restates the leaf.
        let restated = serde_json::from_str::<Str>(
            r#"{"layout":"large_string","charset":"windows-1252","text":"x"}"#,
        )
        .unwrap();
        assert_eq!(restated.parameters(), StringType::LargeCp1252String);
        // A maximum is never part of a value, so it never reaches the
        // wire: the value answers the plain leaf its storage is.
        let bounded = Str::new("x")
            .try_with_parameters(StringType::SizedCp1252String(8))
            .unwrap();
        assert_eq!(bounded.parameters(), StringType::Cp1252String);
        assert_eq!(
            serde_json::to_string(&bounded).unwrap(),
            r#"{"layout":"cp1252","text":"x"}"#
        );
        // A numbered layout with no number is refused, never read as the
        // placeholder width its name carries.
        let missing = serde_json::from_str::<Str>(r#"{"layout":"fixed_utf8","text":"a"}"#)
            .unwrap_err()
            .to_string();
        assert!(
            missing.contains("expected fixed_utf8(number), got none"),
            "{missing}"
        );
        assert!(serde_json::from_str::<Str>(r#"{"layout":"sized_utf8","text":"xx"}"#).is_err());
        // A width on a leaf that takes none is refused as `with_bound` refuses it.
        assert!(
            serde_json::from_str::<Str>(r#"{"layout":"large_utf8","fixed":4,"text":"x"}"#).is_err()
        );
    }

    #[test]
    fn a_string_scalar_names_its_own_datatype() {
        assert_eq!(Scalar::from("x").as_str(), Some("x"));
        assert_eq!(Scalar::from("x").dtype().unwrap(), DataType::utf8());
        let latin = Str::new("x")
            .try_with_parameters(StringType::Cp1252StringView)
            .unwrap();
        assert_eq!(
            Scalar::String(latin).dtype().unwrap(),
            DataType::from_str("string_view(windows-1252)").unwrap()
        );
    }
}

#[cfg(feature = "internals")]
mod codes {
    use yggdryl::DataType;
    use yggdryl::internals::code::{code_cell_text, code_for_extension, code_text};

    #[test]
    fn every_code_names_itself_and_its_width() {
        for (name, dtype, width) in DataType::CODES {
            assert_eq!(dtype.code_name(), Some(*name));
            assert_eq!(dtype.code_width(), Some(*width));
            assert_eq!(dtype.id().code_width(), Some(*width));
            // The width is a maximum, so no code claims a fixed layout.
            assert_eq!(dtype.fixed_byte_width(), None);
            assert_eq!(dtype.to_string(), *name);
            assert_eq!(DataType::from_str(name).unwrap(), *dtype);
            assert!(dtype.is_code());
            assert!(!dtype.is_string());
        }
        assert!(!DataType::fixed_ascii(3).unwrap().is_code());
        assert_eq!(DataType::fixed_ascii(3).unwrap().code_width(), None);
    }

    #[test]
    fn only_a_registered_name_is_a_code() {
        assert_eq!(
            code_for_extension("yggdryl.currency"),
            Some(DataType::Currency)
        );
        assert_eq!(code_for_extension("yggdryl.cfi"), Some(DataType::CfiCode));
        assert_eq!(
            code_for_extension("yggdryl.cusip"),
            Some(DataType::CusipCode)
        );
        assert_eq!(
            code_for_extension("yggdryl.sedol"),
            Some(DataType::SedolCode)
        );
        assert_eq!(code_for_extension("yggdryl.ascii"), None);
        assert_eq!(code_for_extension("arrow.uuid"), None);
    }

    #[test]
    fn a_code_packs_at_the_width_its_standard_fixes() {
        // The packing pads; the column does not. Both codes and fixed ASCII
        // widths answer, and nothing else does.
        assert_eq!(
            DataType::Currency.ascii_packed(b"USD").unwrap(),
            0x0055_5344
        );
        assert_eq!(DataType::Currency.ascii_value(0x0055_5344).unwrap(), "USD");
        assert_eq!(DataType::Country.ascii_packed(b"FR").unwrap(), 0x4652);
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .ascii_packed(b"USD")
                .unwrap(),
            0x5553_4400
        );
        assert!(DataType::Currency.ascii_packed(b"EURO").is_err());
        assert!(DataType::utf8().ascii_packed(b"USD").is_err());
    }

    #[test]
    fn a_code_holds_ascii_text_up_to_its_width() {
        assert_eq!(code_text::<3>(b"USD").unwrap(), "USD");
        assert_eq!(code_text::<3>(b"US\0").unwrap(), "US");
        assert_eq!(code_text::<6>(b"ESVUFR").unwrap(), "ESVUFR");
        let refused = code_text::<3>(b"EURO").unwrap_err().to_string();
        assert!(refused.contains("at most 3 bytes"), "{refused}");
    }

    #[test]
    fn a_cell_is_validated_at_the_code_width() {
        assert_eq!(code_cell_text(&DataType::Currency, b"USD").unwrap(), "USD");
        assert_eq!(code_cell_text(&DataType::Country, b"FR").unwrap(), "FR");
        assert_eq!(
            code_cell_text(&DataType::CfiCode, b"ESVUFR").unwrap(),
            "ESVUFR"
        );
        let refused = code_cell_text(&DataType::Country, b"USD")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 2 bytes"), "{refused}");
        let wrong = code_cell_text(&DataType::fixed_ascii(3).unwrap(), b"USD")
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("registered codes"), "{wrong}");
    }
}
