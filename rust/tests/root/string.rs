//! `rust/src/string.rs`: the inline threshold and the code readers.
//!
//! `INLINE_CAPACITY` is the byte count below which a `Str` stores its text in
//! the value rather than behind an `Arc`. It is published as
//! `yggdryl::INLINE_CAPACITY`, so `scalars` crosses the boundary through
//! `yggdryl::` like any other value here and runs in a default build; the
//! rest of what a caller can observe about `Str` lives beside it here.
//!
//! `code_text`, `code_cell_text` and `code_for_extension` are crate-private:
//! they are the doors every registered code goes through, and a caller sees
//! only the datatype they answer for, so `codes` reaches them through
//! `yggdryl::internals`. The rest of that suite lives in
//! `rust/tests/root/code.rs`.

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
        assert_eq!(std::mem::size_of::<Str>(), 24);
        assert_eq!(std::mem::size_of::<Scalar>(), 48);
    }

    #[test]
    fn equality_order_and_hash_read_the_characters_only() {
        use std::collections::HashSet;

        let plain = Scalar::from("Grüße");
        let latin = StringType::LargeCp1252String.scalar("Grüße").unwrap();
        assert_eq!(plain, latin);
        assert_eq!(plain.cmp(&latin), std::cmp::Ordering::Equal);
        assert_eq!(HashSet::from([plain.clone(), latin.clone()]).len(), 1);
        assert_ne!(plain.string_parameters(), latin.string_parameters());
        assert!(Str::new("b") > Str::new("a"));
        // The leaf is the variant, so a failing assertion prints it.
        assert_eq!(format!("{latin:?}"), "LargeCp1252String(\"Grüße\")");
        assert_eq!(format!("{plain:?}"), "Utf8String(\"Grüße\")");
        assert_eq!(format!("{:?}", Str::new("Grüße")), "\"Grüße\"");
    }

    #[test]
    fn restating_shares_the_storage_and_carries_the_maximum() {
        let text = "x".repeat(INLINE_CAPACITY + 22);
        let shared = Str::new(&text);
        let restated = StringType::SizedUtf8String(64)
            .scalar(shared.clone())
            .unwrap();
        assert!(std::ptr::eq(
            shared.as_str(),
            restated.as_str().expect("a string")
        ));
        assert_eq!(
            restated.string_parameters(),
            Some(StringType::SizedUtf8String(64))
        );
        assert_eq!(restated.dtype().unwrap(), DataType::sized_utf8(64).unwrap());

        let refused = StringType::SizedUtf8String(8)
            .scalar(shared)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 8 bytes"), "{refused}");

        // The bound counts stored bytes, so five scalars are five bytes in
        // windows-1252 and seven in UTF-8.
        assert!(StringType::SizedCp1252String(5).scalar("Grüße").is_ok());
        assert!(StringType::SizedUtf8String(5).scalar("Grüße").is_err());
    }

    #[test]
    fn us_ascii_is_a_repertoire_and_every_other_charset_is_counted() {
        let ascii = StringType::AsciiString;
        assert!(ascii.scalar("plain").is_ok());
        let refused = ascii.scalar("café").unwrap_err().to_string();
        assert!(refused.contains("non-ASCII byte"), "{refused}");
        assert!(ascii.scalar("a\0b").is_err());
        // `U+0081` has no windows-1252 byte, and the value door only counts.
        let latin = StringType::Cp1252String;
        let recovered = latin.scalar("ok\u{0081}").unwrap();
        assert!(latin.encode("ok\u{0081}").is_err());
        assert_eq!(latin.encoded_len(recovered.as_str().unwrap()), 3);
    }

    #[test]
    fn a_fixed_leaf_trims_its_padding_and_pads_on_the_way_out() {
        let fixed = StringType::FixedAsciiString(4);
        let value = fixed.scalar("USD\0").unwrap();
        assert_eq!(value.as_str(), Some("USD"));
        assert_eq!(value.string_parameters(), Some(fixed));
        assert_eq!(value, Scalar::FixedAsciiString(Str::new("USD"), 4));
        assert_eq!(fixed.encode("USD").unwrap().as_ref(), b"USD\0");
        assert_eq!(fixed.encoded_len("USD"), 4);
        assert!(fixed.scalar("EURO!").is_err());
        // A slot never truncates what it cannot hold.
        assert!(fixed.encode("EURO!").is_err());
        assert!(StringType::FixedUtf8String(0).scalar("x").is_err());
    }

    #[test]
    fn bytes_are_read_strictly_or_transcribed_by_their_charset() {
        let latin = StringType::Cp1252String;
        let value = latin.scalar_from_bytes(b"Gr\xFC\xDFe").unwrap();
        assert_eq!(value.as_str(), Some("Grüße"));
        assert_eq!(
            value.string_parameters().map(StringType::charset),
            Some(Charset::Cp1252)
        );
        assert_eq!(latin.encode("Grüße").unwrap().as_ref(), b"Gr\xFC\xDFe");
        // `0x81` is unassigned in windows-1252 and still reads.
        assert_eq!(
            latin.scalar_from_bytes(b"ok\x81").unwrap().as_str(),
            Some("ok\u{0081}")
        );
        // UTF-8 and US-ASCII are validated, not transcribed.
        assert!(StringType::default().scalar_from_bytes(b"caf\xe9").is_err());
        assert!(
            StringType::AsciiString
                .scalar_from_bytes(b"caf\xc3\xa9")
                .is_err()
        );
        assert_eq!(
            StringType::default()
                .scalar_from_bytes(b"caf\xc3\xa9")
                .unwrap()
                .as_str(),
            Some("café")
        );
        // A padded slot comes back trimmed and carries its width.
        let padded = StringType::FixedUtf8String(6)
            .scalar_from_bytes(b"ab\0\0\0\0")
            .unwrap();
        assert_eq!(padded, Scalar::FixedUtf8String(Str::new("ab"), 6));
    }

    #[test]
    fn serde_writes_the_text_alone_unless_the_value_declares_more() {
        // `Str` alone is its characters; the leaf is the `Scalar` variant's,
        // and the `Scalar` wire writes it.
        let plain = Str::new("plain");
        assert_eq!(serde_json::to_string(&plain).unwrap(), "\"plain\"");
        assert_eq!(serde_json::from_str::<Str>("\"plain\"").unwrap(), plain);
        assert_eq!(
            serde_json::to_string(&Scalar::from(plain.clone())).unwrap(),
            r#"{"type":"string","value":"plain"}"#
        );
        let wire = |value: &str| {
            serde_json::from_str::<Scalar>(&format!(r#"{{"type":"string","value":{value}}}"#))
        };
        let latin = StringType::FixedCp1252String(8).scalar("Grüße").unwrap();
        let document = serde_json::to_string(&latin).unwrap();
        assert_eq!(
            document,
            r#"{"type":"string","value":{"layout":"fixed_cp1252","fixed":8,"text":"Grüße"}}"#
        );
        let back = serde_json::from_str::<Scalar>(&document).unwrap();
        assert_eq!(back.string_parameters(), latin.string_parameters());
        assert_eq!(back, latin);
        // A charset beside a charset-free layout restates the leaf.
        let restated =
            wire(r#"{"layout":"large_string","charset":"windows-1252","text":"x"}"#).unwrap();
        assert_eq!(
            restated.string_parameters(),
            Some(StringType::LargeCp1252String)
        );
        // A value keeps its column's maximum, so the wire writes it under
        // `fixed`, the one number key a value document has.
        let bounded = StringType::SizedCp1252String(8).scalar("x").unwrap();
        assert_eq!(
            serde_json::to_string(&bounded).unwrap(),
            r#"{"type":"string","value":{"layout":"sized_cp1252","fixed":8,"text":"x"}}"#
        );
        assert_eq!(
            serde_json::from_str::<Scalar>(&serde_json::to_string(&bounded).unwrap())
                .unwrap()
                .string_parameters(),
            Some(StringType::SizedCp1252String(8))
        );
        // A numbered layout with no number is refused, never read as the
        // placeholder width its name carries.
        let missing = wire(r#"{"layout":"fixed_utf8","text":"a"}"#)
            .unwrap_err()
            .to_string();
        assert!(
            missing.contains("expected fixed_utf8(number), got none"),
            "{missing}"
        );
        assert!(wire(r#"{"layout":"sized_utf8","text":"xx"}"#).is_err());
        // A width on a leaf that takes none is refused as `with_bound` refuses it.
        assert!(wire(r#"{"layout":"large_utf8","fixed":4,"text":"x"}"#).is_err());
    }

    #[test]
    fn a_string_scalar_names_its_own_datatype() {
        assert_eq!(Scalar::from("x").as_str(), Some("x"));
        assert_eq!(Scalar::from("x").dtype().unwrap(), DataType::utf8());
        let latin = StringType::Cp1252StringView.scalar("x").unwrap();
        assert_eq!(
            latin.dtype().unwrap(),
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
        assert_eq!(code_for_extension("yggdryl.ccy"), Some(DataType::Ccy));
        assert_eq!(code_for_extension("yggdryl.currency"), None);
        assert_eq!(code_for_extension("yggdryl.cfi"), Some(DataType::Cfi));
        assert_eq!(code_for_extension("yggdryl.cusip"), Some(DataType::Cusip));
        assert_eq!(code_for_extension("yggdryl.sedol"), Some(DataType::Sedol));
        assert_eq!(code_for_extension("yggdryl.bbg"), Some(DataType::Bbg));
        assert_eq!(code_for_extension("yggdryl.ric"), Some(DataType::Ric));
        assert_eq!(code_for_extension("yggdryl.ascii"), None);
        assert_eq!(code_for_extension("arrow.uuid"), None);
    }

    #[test]
    fn a_code_packs_at_the_width_its_standard_fixes() {
        // The packing pads; the column does not. Both codes and fixed ASCII
        // widths answer, and nothing else does.
        assert_eq!(DataType::Ccy.ascii_packed(b"USD").unwrap(), 0x0055_5344);
        assert_eq!(DataType::Ccy.ascii_value(0x0055_5344).unwrap(), "USD");
        assert_eq!(DataType::Country.ascii_packed(b"FR").unwrap(), 0x4652);
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .ascii_packed(b"USD")
                .unwrap(),
            0x5553_4400
        );
        assert!(DataType::Ccy.ascii_packed(b"EURO").is_err());
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
        assert_eq!(code_cell_text(&DataType::Ccy, b"USD").unwrap(), "USD");
        assert_eq!(code_cell_text(&DataType::Country, b"FR").unwrap(), "FR");
        assert_eq!(code_cell_text(&DataType::Cfi, b"ESVUFR").unwrap(), "ESVUFR");
        assert_eq!(
            code_cell_text(&DataType::Ric, b"AAPL.OQ\0\0").unwrap(),
            "AAPL.OQ"
        );
        let refused = code_cell_text(&DataType::Ric, &[b'X'; 33])
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 32 bytes"), "{refused}");
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

mod leaves {
    use arrow_schema::DataType as ArrowDataType;
    use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
    use yggdryl::{Charset, DataType, DataTypeId, Field, Scalar, Serie};
    use yggdryl::{INLINE_CAPACITY, STRING_EXTENSION_NAME, Str, StringType};

    /// Every leaf in `StringType::ALL` order, with its identifier, its canonical
    /// name and its charset; the numbered leaves state sixteen.
    const LEAVES: [(StringType, DataTypeId, &str, Charset); 18] = [
        (
            StringType::Utf8String,
            DataTypeId::Utf8String,
            "utf8",
            Charset::Utf8,
        ),
        (
            StringType::LargeUtf8String,
            DataTypeId::LargeUtf8String,
            "large_utf8",
            Charset::Utf8,
        ),
        (
            StringType::Utf8StringView,
            DataTypeId::Utf8StringView,
            "utf8_view",
            Charset::Utf8,
        ),
        (
            StringType::LargeUtf8StringView,
            DataTypeId::LargeUtf8StringView,
            "large_utf8_view",
            Charset::Utf8,
        ),
        (
            StringType::FixedUtf8String(16),
            DataTypeId::FixedUtf8String,
            "fixed_utf8",
            Charset::Utf8,
        ),
        (
            StringType::SizedUtf8String(16),
            DataTypeId::SizedUtf8String,
            "sized_utf8",
            Charset::Utf8,
        ),
        (
            StringType::AsciiString,
            DataTypeId::AsciiString,
            "ascii",
            Charset::Ascii,
        ),
        (
            StringType::LargeAsciiString,
            DataTypeId::LargeAsciiString,
            "large_ascii",
            Charset::Ascii,
        ),
        (
            StringType::AsciiStringView,
            DataTypeId::AsciiStringView,
            "ascii_view",
            Charset::Ascii,
        ),
        (
            StringType::LargeAsciiStringView,
            DataTypeId::LargeAsciiStringView,
            "large_ascii_view",
            Charset::Ascii,
        ),
        (
            StringType::FixedAsciiString(16),
            DataTypeId::FixedAsciiString,
            "fixed_ascii",
            Charset::Ascii,
        ),
        (
            StringType::SizedAsciiString(16),
            DataTypeId::SizedAsciiString,
            "sized_ascii",
            Charset::Ascii,
        ),
        (
            StringType::Cp1252String,
            DataTypeId::Cp1252String,
            "cp1252",
            Charset::Cp1252,
        ),
        (
            StringType::LargeCp1252String,
            DataTypeId::LargeCp1252String,
            "large_cp1252",
            Charset::Cp1252,
        ),
        (
            StringType::Cp1252StringView,
            DataTypeId::Cp1252StringView,
            "cp1252_view",
            Charset::Cp1252,
        ),
        (
            StringType::LargeCp1252StringView,
            DataTypeId::LargeCp1252StringView,
            "large_cp1252_view",
            Charset::Cp1252,
        ),
        (
            StringType::FixedCp1252String(16),
            DataTypeId::FixedCp1252String,
            "fixed_cp1252",
            Charset::Cp1252,
        ),
        (
            StringType::SizedCp1252String(16),
            DataTypeId::SizedCp1252String,
            "sized_cp1252",
            Charset::Cp1252,
        ),
    ];

    #[test]
    fn every_leaf_is_one_datatype_under_every_spelling() {
        // The table is `ALL` with a number filled in: every leaf in the one
        // declaration order, six shapes per charset.
        assert_eq!(
            LEAVES.map(|(leaf, ..)| leaf.id()),
            StringType::ALL.map(StringType::id)
        );
        for (leaf, id, spelling, charset) in LEAVES {
            assert_eq!(leaf.as_str(), spelling);
            assert_eq!(leaf.id(), id);
            assert_eq!(leaf.charset(), charset);
            assert_eq!(StringType::from_id(id, 16), Some(leaf));
            assert_eq!(StringType::from_str(spelling).unwrap().id(), id);
            // The fold is the grammar's: case, underscores and hyphens all drop.
            assert_eq!(
                StringType::from_str(&spelling.to_uppercase()).unwrap().id(),
                id
            );
            assert_eq!(
                StringType::from_str(&spelling.replace('_', "-"))
                    .unwrap()
                    .id(),
                id
            );
            assert_eq!(
                StringType::from_str(&spelling.replace('_', ""))
                    .unwrap()
                    .id(),
                id
            );
            // A leaf with no number is its identifier, so only the fixed and
            // sized ones are parameterized; the width is the leaf's rather
            // than the identifier's.
            assert_eq!(id.is_parameterized(), leaf.bound().is_some(), "{id}");
            assert!(id.is_string(), "{id}");
            assert_eq!(id.fixed_byte_width(), None, "{id}");

            let dtype = DataType::string(leaf).unwrap();
            assert!(dtype.to_string().starts_with(spelling), "{dtype}");
            assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
            assert_eq!(dtype.id(), id);
            assert_eq!(dtype.name(), spelling);
            assert_eq!(dtype.charset(), Some(charset));
            assert_eq!(dtype.string_parameters(), Some(leaf));
            assert_eq!(dtype.bytes_parameters(), None);
            assert!(dtype.is_string());
        }

        // The sugar constructors are the same datatypes, and `utf8` is the
        // family's default.
        assert_eq!(DataType::utf8(), DataType::from(StringType::default()));
        assert_eq!(DataType::utf8(), DataType::Utf8String);
        for (sugar, leaf) in [
            (DataType::utf8(), StringType::Utf8String),
            (DataType::large_utf8(), StringType::LargeUtf8String),
            (DataType::utf8_view(), StringType::Utf8StringView),
            (DataType::large_utf8_view(), StringType::LargeUtf8StringView),
            (
                DataType::fixed_utf8(8).unwrap(),
                StringType::FixedUtf8String(8),
            ),
            (
                DataType::sized_utf8(32).unwrap(),
                StringType::SizedUtf8String(32),
            ),
            (DataType::ascii(), StringType::AsciiString),
            (DataType::large_ascii(), StringType::LargeAsciiString),
            (DataType::ascii_view(), StringType::AsciiStringView),
            (
                DataType::large_ascii_view(),
                StringType::LargeAsciiStringView,
            ),
            (
                DataType::fixed_ascii(4).unwrap(),
                StringType::FixedAsciiString(4),
            ),
            (
                DataType::sized_ascii(4).unwrap(),
                StringType::SizedAsciiString(4),
            ),
            (DataType::cp1252(), StringType::Cp1252String),
            (DataType::large_cp1252(), StringType::LargeCp1252String),
            (DataType::cp1252_view(), StringType::Cp1252StringView),
            (
                DataType::large_cp1252_view(),
                StringType::LargeCp1252StringView,
            ),
            (
                DataType::fixed_cp1252(8).unwrap(),
                StringType::FixedCp1252String(8),
            ),
            (
                DataType::sized_cp1252(32).unwrap(),
                StringType::SizedCp1252String(32),
            ),
        ] {
            assert_eq!(sugar, DataType::string(leaf).unwrap());
        }
        assert_eq!(
            DataType::fixed_utf8(8).unwrap().to_string(),
            "fixed_utf8(8)"
        );
        assert_eq!(
            DataType::sized_cp1252(32).unwrap().to_string(),
            "sized_cp1252(32)"
        );
    }

    #[test]
    fn every_spelling_renders_as_its_canonical_leaf() {
        // The charset-free spellings name the UTF-8 leaf of their shape and are
        // the only ones that take a `(charset)`; a charset-named spelling is the
        // leaf itself; a number after a plain spelling is a maximum, which is
        // its own leaf.
        for (spelling, canonical) in [
            ("string", "utf8"),
            ("str", "utf8"),
            ("text", "utf8"),
            ("varchar", "utf8"),
            ("nvarchar", "utf8"),
            ("character varying", "utf8"),
            ("utf8_string", "utf8"),
            ("string()", "utf8"),
            ("utf8(32)", "sized_utf8(32)"),
            ("varchar(32)", "sized_utf8(32)"),
            ("string(32)", "sized_utf8(32)"),
            ("sized_string(32)", "sized_utf8(32)"),
            ("sized_utf8(32)", "sized_utf8(32)"),
            ("string(utf-8,32)", "sized_utf8(32)"),
            ("char(8)", "fixed_utf8(8)"),
            ("fixed_string(8)", "fixed_utf8(8)"),
            ("FixedString(8)", "fixed_utf8(8)"),
            ("fixed_utf8(8)", "fixed_utf8(8)"),
            ("string_view", "utf8_view"),
            ("stringview", "utf8_view"),
            ("utf8view", "utf8_view"),
            ("LargeString", "large_utf8"),
            ("large-utf8", "large_utf8"),
            ("large_string_view", "large_utf8_view"),
            ("large_utf8_view", "large_utf8_view"),
            ("ascii", "ascii"),
            ("us-ascii", "ascii"),
            ("ascii_string", "ascii"),
            ("string(us-ascii)", "ascii"),
            ("ascii(4)", "sized_ascii(4)"),
            ("string(us-ascii,4)", "sized_ascii(4)"),
            ("sized_ascii(4)", "sized_ascii(4)"),
            ("fixed_ascii(4)", "fixed_ascii(4)"),
            ("fixed_string(us-ascii,4)", "fixed_ascii(4)"),
            ("large_ascii", "large_ascii"),
            ("large_string(us-ascii)", "large_ascii"),
            ("ascii_view", "ascii_view"),
            ("string_view(us-ascii)", "ascii_view"),
            ("large_ascii_view", "large_ascii_view"),
            ("large_string_view(us-ascii)", "large_ascii_view"),
            ("cp1252", "cp1252"),
            ("windows-1252", "cp1252"),
            ("string(windows-1252)", "cp1252"),
            ("string(cp1252)", "cp1252"),
            ("string(windows-1252,32)", "sized_cp1252(32)"),
            ("sized_cp1252(32)", "sized_cp1252(32)"),
            ("sized_windows_1252(32)", "sized_cp1252(32)"),
            ("fixed_string(windows-1252,8)", "fixed_cp1252(8)"),
            ("fixed_cp1252(8)", "fixed_cp1252(8)"),
            ("large_string(windows-1252)", "large_cp1252"),
            ("large_cp1252", "large_cp1252"),
            ("string_view(windows-1252)", "cp1252_view"),
            ("cp1252_view", "cp1252_view"),
            ("large_string_view(windows-1252)", "large_cp1252_view"),
            ("large_windows_1252_view", "large_cp1252_view"),
        ] {
            assert_eq!(
                DataType::from_str(spelling).unwrap().to_string(),
                canonical,
                "{spelling}"
            );
        }

        // A maximum only lands on a plain leaf: the large and the viewed shapes
        // refuse it naming the sized leaf a bounded column would be.
        for (spelling, sized) in [
            ("large_utf8(64)", "sized_utf8(maximum)"),
            ("utf8_view(8)", "sized_utf8(maximum)"),
            ("large_string_view(8)", "sized_utf8(maximum)"),
            ("large_ascii(16)", "sized_ascii(maximum)"),
            ("ascii_view(4)", "sized_ascii(maximum)"),
            ("large_string(windows-1252,32)", "sized_cp1252(maximum)"),
            ("cp1252_view(8)", "sized_cp1252(maximum)"),
        ] {
            let refusal = DataType::from_str(spelling).unwrap_err().to_string();
            assert!(refusal.contains(sized), "{spelling}: {refusal}");
        }
        // A charset-named spelling already answered the charset question.
        for spelling in [
            "utf8(windows-1252)",
            "fixed_utf8(windows-1252,8)",
            "ascii(utf-8)",
            "large_ascii(us-ascii)",
            "cp1252(utf-8)",
            "sized_cp1252(windows-1252,8)",
        ] {
            let refusal = DataType::from_str(spelling).unwrap_err().to_string();
            assert!(
                refusal.contains("string is the spelling that takes one"),
                "{spelling}: {refusal}"
            );
        }
        // Only three charsets have leaves; the rest stay the decoding vocabulary.
        for spelling in [
            "string(iso-8859-1)",
            "string(latin1)",
            "large_string(iso-8859-15)",
            "fixed_string(utf-16-le,8)",
        ] {
            let refusal = DataType::from_str(spelling).unwrap_err().to_string();
            assert!(
                refusal.contains("utf-8, us-ascii or windows-1252"),
                "{spelling}: {refusal}"
            );
        }
        // And a word that is no charset is refused as one.
        assert!(DataType::from_str("string(nowhere)").is_err());
    }

    #[test]
    fn a_width_and_a_maximum_are_two_leaves_and_never_one_column() {
        // `sized_cp1252(32)` is a maximum of thirty-two bytes and
        // `fixed_cp1252(8)` an exact width; the two readings never both answer.
        let bounded = DataType::from_str("string(windows-1252,32)").unwrap();
        let parameters = bounded.string_parameters().unwrap();
        assert_eq!(parameters, StringType::SizedCp1252String(32));
        assert_eq!(parameters.max(), Some(32));
        assert_eq!(parameters.fixed(), None);
        assert_eq!(parameters.bound(), Some(32));
        assert!(parameters.is_bounded());
        assert!(!parameters.is_fixed());
        assert_eq!(bounded.fixed_byte_width(), None);
        assert_eq!(bounded.to_string(), "sized_cp1252(32)");
        assert_ne!(bounded, DataType::cp1252());

        let fixed = DataType::from_str("fixed_string(windows-1252,8)").unwrap();
        let parameters = fixed.string_parameters().unwrap();
        assert_eq!(parameters, StringType::FixedCp1252String(8));
        assert_eq!(parameters.fixed(), Some(8));
        assert_eq!(parameters.max(), None);
        assert!(parameters.is_fixed());
        assert_eq!(fixed.fixed_byte_width(), Some(8));
        assert_eq!(fixed.to_string(), "fixed_cp1252(8)");
        assert_ne!(bounded, fixed);

        // A sized column stores as the plain leaf of its charset: the maximum
        // is a rule laid over that storage, which the value carries beside it.
        assert_eq!(
            StringType::SizedUtf8String(16).storage(),
            StringType::Utf8String
        );
        assert_eq!(
            StringType::SizedAsciiString(4).storage(),
            StringType::AsciiString
        );
        assert_eq!(
            StringType::SizedCp1252String(32).storage(),
            StringType::Cp1252String
        );
        assert_eq!(
            StringType::FixedAsciiString(4).storage(),
            StringType::FixedAsciiString(4)
        );
        assert_eq!(
            StringType::LargeCp1252StringView.storage(),
            StringType::LargeCp1252StringView
        );

        // A column of no bytes is not a column, whichever leaf states the number.
        assert!(DataType::from_str("utf8(0)").is_err());
        assert!(DataType::from_str("fixed_ascii(0)").is_err());
        assert!(DataType::from_str("string(windows-1252,0)").is_err());
        assert!(DataType::fixed_utf8(0).is_err());
        assert!(DataType::sized_cp1252(0).is_err());
        assert!(StringType::FixedUtf8String(0).validate().is_err());
        assert!(StringType::SizedAsciiString(0).validate().is_err());
        let refusal = StringType::FixedCp1252String(0)
            .validate()
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("expected a width of at least one byte, got 0"),
            "{refusal}"
        );
        assert!(DataType::FixedCp1252String(0).validate().is_err());

        // A bare numbered spelling is a question rather than a declaration.
        for spelling in [
            "fixed_string",
            "fixedutf8",
            "fixed_ascii",
            "sized_string",
            "sized_ascii",
            "sized_cp1252()",
        ] {
            assert!(DataType::from_str(spelling).is_err(), "{spelling}");
        }

        // Which offsets, whether it is viewed and which charset are the leaf's,
        // and answered without a match.
        assert!(StringType::LargeAsciiStringView.is_view());
        assert!(StringType::LargeAsciiStringView.is_large());
        assert!(StringType::Cp1252StringView.is_view());
        assert!(!StringType::Cp1252StringView.is_large());
        assert!(StringType::LargeUtf8String.is_large());
        assert!(!StringType::LargeUtf8String.is_view());
        assert!(!StringType::SizedUtf8String(16).is_view());
        assert!(!StringType::SizedUtf8String(16).is_large());
        assert_eq!(StringType::SizedUtf8String(16).charset(), Charset::Utf8);
        assert_eq!(StringType::LargeAsciiString.charset(), Charset::Ascii);
        assert_eq!(StringType::FixedCp1252String(8).charset(), Charset::Cp1252);
    }

    #[test]
    fn a_number_and_a_charset_restate_a_leaf_under_one_rule_each() {
        // A stated number: the width on a fixed leaf, the maximum on a plain or
        // a sized one, refused on the large and the viewed shapes.
        assert_eq!(
            StringType::Utf8String.with_bound(32).unwrap(),
            StringType::SizedUtf8String(32)
        );
        assert_eq!(
            StringType::AsciiString.with_bound(4).unwrap(),
            StringType::SizedAsciiString(4)
        );
        assert_eq!(
            StringType::SizedCp1252String(8).with_bound(32).unwrap(),
            StringType::SizedCp1252String(32)
        );
        assert_eq!(
            StringType::FixedAsciiString(1).with_bound(4).unwrap(),
            StringType::FixedAsciiString(4)
        );
        for leaf in [
            StringType::LargeUtf8String,
            StringType::Utf8StringView,
            StringType::LargeUtf8StringView,
            StringType::LargeAsciiString,
            StringType::AsciiStringView,
            StringType::LargeCp1252StringView,
        ] {
            let refusal = leaf.with_bound(8).unwrap_err().to_string();
            assert!(refusal.contains("expected no maximum"), "{leaf}: {refusal}");
            assert_eq!(leaf.with_declared_bound(None).unwrap(), leaf);
        }
        assert!(StringType::Utf8String.with_bound(0).is_err());

        // A number the caller may not have stated: the six numbered leaves are
        // their number and stand without none.
        assert_eq!(
            StringType::Utf8String.with_declared_bound(None).unwrap(),
            StringType::Utf8String
        );
        assert_eq!(
            StringType::SizedAsciiString(1)
                .with_declared_bound(Some(4))
                .unwrap(),
            StringType::SizedAsciiString(4)
        );
        let refusal = StringType::FixedUtf8String(1)
            .with_declared_bound(None)
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("expected fixed_utf8(number), got none"),
            "{refusal}"
        );
        assert!(
            StringType::SizedCp1252String(1)
                .with_declared_bound(None)
                .is_err()
        );

        // A charset: the same shape in another family, the number kept.
        assert_eq!(
            StringType::Utf8String
                .with_charset(Charset::Cp1252)
                .unwrap(),
            StringType::Cp1252String
        );
        assert_eq!(
            StringType::FixedUtf8String(4)
                .with_charset(Charset::Ascii)
                .unwrap(),
            StringType::FixedAsciiString(4)
        );
        assert_eq!(
            StringType::SizedAsciiString(3)
                .with_charset(Charset::Utf8)
                .unwrap(),
            StringType::SizedUtf8String(3)
        );
        assert_eq!(
            StringType::LargeUtf8StringView
                .with_charset(Charset::Cp1252)
                .unwrap(),
            StringType::LargeCp1252StringView
        );
        for (leaf, _, _, charset) in LEAVES {
            assert_eq!(leaf.with_charset(charset).unwrap(), leaf);
            // Three families, one shape each: a round trip through the other
            // two comes back to the leaf.
            let ascii = leaf.with_charset(Charset::Ascii).unwrap();
            let latin = ascii.with_charset(Charset::Cp1252).unwrap();
            assert_eq!(latin.with_charset(charset).unwrap(), leaf, "{leaf}");
            assert_eq!(latin.bound(), leaf.bound(), "{leaf}");
            let refusal = leaf.with_charset(Charset::Latin1).unwrap_err().to_string();
            assert!(
                refusal.contains("utf-8, us-ascii or windows-1252"),
                "{leaf}: {refusal}"
            );
            assert!(leaf.with_charset(Charset::Utf16Le).is_err(), "{leaf}");
        }

        // A binding's `(layout, charset, bound)` arguments run the same rules in
        // the same order, so neither binding decides anything.
        assert_eq!(
            StringType::from_declaration("string", Some("windows-1252"), Some(32)).unwrap(),
            StringType::SizedCp1252String(32)
        );
        assert_eq!(
            StringType::from_declaration("fixed_string", Some("us-ascii"), Some(4)).unwrap(),
            StringType::FixedAsciiString(4)
        );
        assert_eq!(
            StringType::from_declaration("large-utf8", None, None).unwrap(),
            StringType::LargeUtf8String
        );
        assert_eq!(
            StringType::from_declaration("ASCII", None, Some(4)).unwrap(),
            StringType::SizedAsciiString(4)
        );
        let refusal = StringType::from_declaration("utf8", Some("windows-1252"), None)
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("string is the spelling that takes one"),
            "{refusal}"
        );
        assert!(StringType::from_declaration("fixed_utf8", None, None).is_err());
        assert!(StringType::from_declaration("large_utf8", None, Some(64)).is_err());
        assert!(StringType::from_declaration("string", Some("latin1"), None).is_err());
        let refusal = StringType::from_declaration("blob", None, None)
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("expected a string layout"), "{refusal}");

        // The spellings, already folded: the charset-free ones are a subset.
        assert_eq!(
            StringType::general_spelling("string"),
            Some(StringType::Utf8String)
        );
        assert_eq!(
            StringType::general_spelling("fixedstring"),
            Some(StringType::FixedUtf8String(1))
        );
        assert_eq!(StringType::general_spelling("utf8"), None);
        assert_eq!(
            StringType::from_spelling("utf8"),
            Some(StringType::Utf8String)
        );
        assert_eq!(
            StringType::from_spelling("sizedwindows1252"),
            Some(StringType::SizedCp1252String(1))
        );
        assert_eq!(StringType::from_spelling("blob"), None);
        let refusal = StringType::from_str("blob").unwrap_err().to_string();
        assert!(refusal.contains("expected a string layout"), "{refusal}");
    }

    #[test]
    fn sql_reads_its_own_two_string_shapes() {
        // `varchar(n)` bounds; `char(n)` is blank-padded to exactly n.
        assert_eq!(
            DataType::from_str("varchar(32)").unwrap().to_string(),
            "sized_utf8(32)"
        );
        assert_eq!(
            DataType::from_str("character varying(32)")
                .unwrap()
                .to_string(),
            "sized_utf8(32)"
        );
        assert_eq!(
            DataType::from_str("char(8)").unwrap().to_string(),
            "fixed_utf8(8)"
        );
        // A width is what makes a string fixed, so a bare `char` is not.
        assert_eq!(DataType::from_str("char").unwrap(), DataType::utf8());
        // And the SQL spellings are charset-free, so they take one.
        assert_eq!(
            DataType::from_str("char(us-ascii,8)").unwrap().to_string(),
            "fixed_ascii(8)"
        );
        assert_eq!(
            DataType::from_str("varchar(windows-1252,32)")
                .unwrap()
                .to_string(),
            "sized_cp1252(32)"
        );
    }

    #[test]
    fn every_string_datatype_answers_one_question_about_its_charset() {
        // One accessor for every string, whatever its spelling.
        assert_eq!(DataType::utf8().charset(), Some(Charset::Utf8));
        assert_eq!(DataType::utf8_view().charset(), Some(Charset::Utf8));
        assert_eq!(DataType::ascii().charset(), Some(Charset::Ascii));
        assert_eq!(
            DataType::fixed_ascii(3).unwrap().charset(),
            Some(Charset::Ascii)
        );
        assert_eq!(
            DataType::fixed_ascii(3)
                .unwrap()
                .string_parameters()
                .unwrap()
                .fixed(),
            Some(3)
        );
        assert_eq!(
            DataType::from_str("large_string(windows-1252)")
                .unwrap()
                .charset(),
            Some(Charset::Cp1252)
        );
        // A charset with no leaf is not a string datatype anyone can name.
        assert!(DataType::from_str("large_string(iso-8859-15)").is_err());

        // A code is an identity over a registry, not a string with a charset:
        // it stores as text without declaring one.
        assert_eq!(DataType::Ccy.charset(), None);
        assert!(!DataType::Ccy.is_string());
        assert!(DataType::utf8().is_string());
    }

    #[test]
    fn every_leaf_rides_its_arrow_storage_and_all_but_three_ride_the_document() {
        // Arrow's string layouts declare UTF-8, so text that is not UTF-8 rides
        // the binary layout beside them and the charset rides the document;
        // US-ASCII is UTF-8 and rides the text layouts under the same document.
        // Plain `utf8`, `large_utf8` and `utf8_view` are Arrow's own and cross
        // bare; every other leaf is written whole, charset beside the name, since
        // the charset is the one fact Arrow cannot state.
        let cases: [(&str, ArrowDataType, Option<&str>); 18] = [
            ("utf8", ArrowDataType::Utf8, None),
            ("large_utf8", ArrowDataType::LargeUtf8, None),
            ("utf8_view", ArrowDataType::Utf8View, None),
            (
                "large_utf8_view",
                ArrowDataType::Utf8View,
                Some(r#"{"layout":"large_utf8_view","charset":"utf-8"}"#),
            ),
            (
                "fixed_utf8(8)",
                ArrowDataType::FixedSizeBinary(8),
                Some(r#"{"layout":"fixed_utf8","charset":"utf-8","fixed":8}"#),
            ),
            (
                "utf8(32)",
                ArrowDataType::Utf8,
                Some(r#"{"layout":"sized_utf8","charset":"utf-8","max":32}"#),
            ),
            (
                "ascii",
                ArrowDataType::Utf8,
                Some(r#"{"layout":"ascii","charset":"us-ascii"}"#),
            ),
            (
                "large_ascii",
                ArrowDataType::LargeUtf8,
                Some(r#"{"layout":"large_ascii","charset":"us-ascii"}"#),
            ),
            (
                "ascii_view",
                ArrowDataType::Utf8View,
                Some(r#"{"layout":"ascii_view","charset":"us-ascii"}"#),
            ),
            (
                "large_ascii_view",
                ArrowDataType::Utf8View,
                Some(r#"{"layout":"large_ascii_view","charset":"us-ascii"}"#),
            ),
            (
                "fixed_ascii(4)",
                ArrowDataType::FixedSizeBinary(4),
                Some(r#"{"layout":"fixed_ascii","charset":"us-ascii","fixed":4}"#),
            ),
            (
                "sized_ascii(16)",
                ArrowDataType::Utf8,
                Some(r#"{"layout":"sized_ascii","charset":"us-ascii","max":16}"#),
            ),
            (
                "string(windows-1252)",
                ArrowDataType::Binary,
                Some(r#"{"layout":"cp1252","charset":"windows-1252"}"#),
            ),
            (
                "largestring(windows-1252)",
                ArrowDataType::LargeBinary,
                Some(r#"{"layout":"large_cp1252","charset":"windows-1252"}"#),
            ),
            (
                "stringview(windows-1252)",
                ArrowDataType::BinaryView,
                Some(r#"{"layout":"cp1252_view","charset":"windows-1252"}"#),
            ),
            (
                "large_cp1252_view",
                ArrowDataType::BinaryView,
                Some(r#"{"layout":"large_cp1252_view","charset":"windows-1252"}"#),
            ),
            (
                "fixed_string(windows-1252,8)",
                ArrowDataType::FixedSizeBinary(8),
                Some(r#"{"layout":"fixed_cp1252","charset":"windows-1252","fixed":8}"#),
            ),
            (
                "string(windows-1252,32)",
                ArrowDataType::Binary,
                Some(r#"{"layout":"sized_cp1252","charset":"windows-1252","max":32}"#),
            ),
        ];
        for (spelling, storage, document) in cases {
            let dtype = DataType::from_str(spelling).unwrap();
            assert_eq!(
                dtype.clone().into_arrow_datatype().unwrap(),
                storage,
                "{spelling}"
            );
            // A bare Arrow text storage is the plain UTF-8 leaf it declares.
            if document.is_none() {
                assert_eq!(
                    DataType::from_arrow_datatype(&storage).unwrap(),
                    dtype,
                    "{spelling}"
                );
            }

            let field = dtype.clone().nullable_field("value");
            let arrow = field.clone().into_arrow_field().unwrap();
            assert_eq!(arrow.data_type(), &storage, "{spelling}");
            assert_eq!(
                arrow
                    .metadata()
                    .get(EXTENSION_TYPE_NAME_KEY)
                    .map(String::as_str),
                document.map(|_| STRING_EXTENSION_NAME),
                "{spelling}"
            );
            assert_eq!(
                arrow
                    .metadata()
                    .get(EXTENSION_TYPE_METADATA_KEY)
                    .map(String::as_str),
                document,
                "{spelling}"
            );
            assert_eq!(
                Field::from_arrow_field(&arrow).unwrap(),
                field,
                "{spelling}"
            );
        }

        // The document round-trips through its own door for every leaf, and an
        // older document that named a shape beside a charset reads as the leaf
        // it meant: a maximum makes the column sized whichever unbounded shape
        // was named.
        for (leaf, ..) in LEAVES {
            assert_eq!(
                StringType::from_extension_json(&leaf.extension_json()).unwrap(),
                leaf,
                "{leaf}"
            );
        }
        for (document, leaf) in [
            (
                r#"{"layout":"string","charset":"us-ascii"}"#,
                StringType::AsciiString,
            ),
            (
                r#"{"layout":"string","charset":"us-ascii","max":4}"#,
                StringType::SizedAsciiString(4),
            ),
            (
                r#"{"layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
                StringType::FixedAsciiString(4),
            ),
            (
                r#"{"layout":"large_string","charset":"utf-8","max":64}"#,
                StringType::SizedUtf8String(64),
            ),
            (
                r#"{"layout":"large_string","charset":"windows-1252"}"#,
                StringType::LargeCp1252String,
            ),
            (
                r#"{"layout":"string_view","charset":"windows-1252","max":32}"#,
                StringType::SizedCp1252String(32),
            ),
            (
                r#"{"layout":"large_string_view","charset":"utf-8"}"#,
                StringType::LargeUtf8StringView,
            ),
            (
                r#"{"layout":"utf8","max":16}"#,
                StringType::SizedUtf8String(16),
            ),
            (
                r#"{"layout":"sized_cp1252","max":32}"#,
                StringType::SizedCp1252String(32),
            ),
            (
                r#"{"layout":"fixed_ascii","fixed":4}"#,
                StringType::FixedAsciiString(4),
            ),
        ] {
            assert_eq!(
                StringType::from_extension_json(document).unwrap(),
                leaf,
                "{document}"
            );
        }
        for retired in [
            r#"{"layout":"fixed_utf8"}"#,
            r#"{"layout":"fixed_utf8","max":8}"#,
            r#"{"layout":"utf8","fixed":4}"#,
            r#"{"layout":"sized_utf8"}"#,
            r#"{"layout":"fixed_ascii","fixed":4,"max":4}"#,
            r#"{"layout":"string","charset":"iso-8859-1"}"#,
            r#"{"layout":"utf8","max":0}"#,
            r#"{"layout":"blob"}"#,
            r#""utf8""#,
        ] {
            assert!(
                StringType::from_extension_json(retired).is_err(),
                "{retired}"
            );
        }

        // A document over a storage it does not describe imports as the storage.
        let latin = DataType::cp1252().string_parameters().unwrap();
        let foreign = arrow_schema::Field::new("value", ArrowDataType::Utf8, true).with_metadata(
            [
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    STRING_EXTENSION_NAME.to_owned(),
                ),
                (
                    EXTENSION_TYPE_METADATA_KEY.to_owned(),
                    latin.extension_json(),
                ),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(
            Field::from_arrow_field(&foreign).unwrap().dtype(),
            &DataType::utf8()
        );
    }

    #[test]
    fn the_two_view_layouts_share_one_arrow_layout_and_stay_distinct_here() {
        // Arrow has one view layout; this crate declares two, and the difference
        // travels in the metadata rather than in the buffers.
        let view = DataType::utf8_view();
        let large = DataType::from_str("large_utf8_view").unwrap();
        assert_ne!(view, large);
        assert_eq!(
            view.clone().into_arrow_datatype().unwrap(),
            large.clone().into_arrow_datatype().unwrap()
        );
        let field = large.clone().nullable_field("value");
        assert_eq!(
            Field::from_arrow_field(&field.clone().into_arrow_field().unwrap()).unwrap(),
            field
        );
    }

    #[test]
    fn a_string_column_stores_the_bytes_its_charset_writes() {
        let dtype = DataType::from_str("string(windows-1252)").unwrap();
        let value = dtype.scalar("Grüße").unwrap();
        let text = value.as_str().unwrap();
        assert_eq!(text, "Grüße");
        assert_eq!(value.id(), DataTypeId::Cp1252String);
        assert_eq!(value.dtype().unwrap(), dtype);

        // The characters are UTF-8 in memory and windows-1252 on the way out.
        let encoded = Charset::Cp1252.encode("Grüße").unwrap();
        assert_eq!(encoded.len(), 5);
        assert_eq!(encoded.as_ref(), &[0x47, 0x72, 0xFC, 0xDF, 0x65]);
    }

    #[test]
    fn bytes_arriving_at_a_string_column_are_read_through_its_charset() {
        let dtype = DataType::from_str("string(windows-1252)").unwrap();
        let value = dtype
            .scalar(Scalar::from(vec![0x47_u8, 0x72, 0xFC, 0xDF, 0x65]))
            .unwrap();
        assert_eq!(value.as_str(), Some("Grüße"));

        // And a byte the charset leaves unassigned is transcribed rather than
        // refusing the whole value: `0x81` is a C1 control in ISO 8859-1, which
        // is what the WHATWG index maps it to too.
        let recovered = dtype
            .scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))
            .unwrap();
        assert_eq!(recovered.as_str(), Some("ok\u{0081}"));
    }

    #[test]
    fn utf8_reads_bytes_strictly_in_every_layout() {
        // UTF-8 is a validated repertoire: bytes that are not UTF-8 are refused
        // naming the charset, under every UTF-8 leaf, while a legacy charset
        // transcribes the same bytes.
        let damaged = vec![0x6F_u8, 0x6B, 0x81];
        for spelling in [
            "utf8",
            "utf8(8)",
            "fixed_utf8(8)",
            "large_utf8",
            "utf8_view",
            "large_utf8_view",
        ] {
            let dtype = DataType::from_str(spelling).unwrap();
            let refused = dtype
                .scalar(Scalar::from(damaged.clone()))
                .unwrap_err()
                .to_string();
            assert!(refused.contains("utf-8"), "{spelling}: {refused}");
            assert_eq!(
                dtype
                    .scalar(Scalar::from("caf\u{e9}".as_bytes().to_vec()))
                    .unwrap()
                    .as_str(),
                Some("caf\u{e9}"),
                "{spelling}"
            );
        }
        assert!(StringType::default().scalar_from_bytes(&damaged).is_err());
        assert_eq!(
            StringType::Cp1252String
                .scalar_from_bytes(&damaged)
                .unwrap()
                .as_str(),
            Some("ok\u{0081}")
        );
    }

    #[test]
    fn a_maximum_is_counted_in_the_bytes_the_charset_stores() {
        let dtype = DataType::from_str("string(windows-1252,4)").unwrap();
        // Five scalars, five stored bytes: over the bound.
        assert!(dtype.scalar("Grüße").is_err());
        // Four scalars in one byte each: within it.
        assert_eq!(dtype.scalar("Grüß").unwrap().as_str(), Some("Grüß"));

        // UTF-8 counts the bytes it stores, which is not the scalar count.
        let utf8 = DataType::from_str("utf8(4)").unwrap();
        assert!(utf8.scalar("Grüß").is_err());
        assert_eq!(utf8.scalar("Grü").unwrap().as_str(), Some("Grü"));
    }

    #[test]
    fn a_fixed_string_pads_its_storage_and_reads_back_trimmed() {
        let dtype = DataType::from_str("fixed_string(windows-1252,8)").unwrap();
        let value = dtype.scalar("café").unwrap();
        assert_eq!(value.as_str(), Some("café"));
        assert_eq!(value.dtype().unwrap(), dtype);
        // Four stored bytes in windows-1252, so it fits eight; five in UTF-8
        // would too, and neither is the scalar count.
        assert!(
            DataType::from_str("fixedstring(windows-1252,3)")
                .unwrap()
                .scalar("café")
                .is_err()
        );
    }

    #[test]
    fn a_string_restates_into_another_layout_without_copying_its_characters() {
        // Long enough to be heap storage: below the inline buffer a rewrite
        // copies the bytes into the new value and no pointer identity could be
        // observed, so a short value cannot witness this claim at all.
        const LONG: &str = "a value well past the twenty-three byte inline buffer";
        assert!(LONG.len() > INLINE_CAPACITY);

        let source = DataType::utf8().scalar(LONG).unwrap();
        let origin = source.as_str().unwrap().as_ptr();
        let target = DataType::from_str("large_string(windows-1252)").unwrap();
        let restated = target.scalar(source).unwrap();
        assert_eq!(restated.as_str(), Some(LONG));
        assert_eq!(restated.id(), DataTypeId::LargeCp1252String);
        assert_eq!(restated.dtype().unwrap(), target);
        // The leaf is the offset width and the charset, not the bytes: the
        // rewrite adopts the storage handle rather than copying the characters.
        assert!(
            std::ptr::eq(restated.as_str().unwrap().as_ptr(), origin),
            "restating a leaf should share its storage"
        );

        let short = DataType::utf8().scalar("AAPL").unwrap();
        assert_eq!(target.scalar(short).unwrap().as_str(), Some("AAPL"));
    }

    #[test]
    fn str_is_the_compact_string_and_compares_by_its_characters() {
        // Short text lives inside the value, a static spelling costs nothing,
        // and longer text is one shared handle.
        let short = Str::new("AAPL");
        assert!(short.is_inline());
        assert!(Str::new("a".repeat(INLINE_CAPACITY)).is_inline());
        assert!(!Str::new("a".repeat(INLINE_CAPACITY + 1)).is_inline());
        assert_eq!(std::mem::size_of::<Str>(), 24);
        assert_eq!(Str::new_static("AAPL"), short);
        assert_eq!(Str::default(), "");
        assert_eq!(
            Scalar::from(short.clone()).string_parameters(),
            Some(StringType::Utf8String)
        );

        // Equality, order and hash read the characters alone, so a value is one
        // value whichever leaf holds it.
        let latin = StringType::LargeCp1252String.scalar(short.clone()).unwrap();
        assert_eq!(latin, Scalar::from(short.clone()));
        assert_eq!(latin.as_str(), Some("AAPL"));
        let leaf = latin.string_parameters().expect("a string value");
        assert_eq!(leaf.charset(), Charset::Cp1252);
        assert_eq!(leaf, StringType::LargeCp1252String);
        assert!(leaf.is_large());
        assert_eq!(latin.dtype().unwrap(), DataType::large_cp1252());
        assert_ne!(
            format!("{latin:?}"),
            format!("{:?}", Scalar::from(short.clone()))
        );
        let text = latin.as_string().expect("a string value").clone();
        assert_eq!(text, short);
        assert_eq!(text, "AAPL");
        assert_eq!("AAPL", text);
        assert_eq!(text, String::from("AAPL"));
        assert_eq!(text.to_string(), "AAPL");
        let mut members = std::collections::BTreeMap::new();
        members.insert(text, 1);
        assert_eq!(members.get("AAPL"), Some(&1));

        // The conversions every string API leans on.
        assert_eq!(Str::from(String::from("x")), "x");
        assert_eq!(Str::from('x'), "x");
        assert_eq!(String::from(Str::new("x")), "x");
        assert_eq!("a b".parse::<Str>().unwrap(), "a b");
        assert_eq!(["a", "b"].into_iter().collect::<Str>(), "ab");
        assert_eq!(&*Str::new("deref"), "deref");
        assert_eq!(Scalar::from("x"), Scalar::Utf8String(Str::new("x")));

        // A maximum is carried with the value, and checked.
        let bounded = StringType::SizedUtf8String(4).scalar("USD").unwrap();
        assert_eq!(
            bounded.string_parameters(),
            Some(StringType::SizedUtf8String(4))
        );
        assert!(StringType::SizedUtf8String(4).scalar("EURO!").is_err());
        // So is a fixed width: the leaf pads to it on the way out.
        let leaf = StringType::FixedUtf8String(4);
        let fixed = leaf.scalar("USD\0").unwrap();
        assert_eq!(fixed.as_str(), Some("USD"));
        assert_eq!(
            fixed.string_parameters().and_then(StringType::fixed),
            Some(4)
        );
        assert_eq!(leaf.encode("USD").unwrap().as_ref(), b"USD\0");
        assert_eq!(leaf.encoded_len("USD"), 4);
        assert_eq!(fixed.dtype().unwrap(), DataType::fixed_utf8(4).unwrap());
        assert!(StringType::FixedUtf8String(0).scalar("x").is_err());
    }

    #[test]
    fn a_string_datatype_survives_both_serde_doors() {
        // One `string` tag for every string, with only what it declares beside
        // it: the leaf when not `utf8`, and the number under the key its leaf
        // gives it. The leaf's name says its charset, so no `charset` key is
        // written.
        for (spelling, json) in [
            ("utf8", r#"{"type":"string"}"#),
            (
                "utf8(32)",
                r#"{"type":"string","layout":"sized_utf8","max":32}"#,
            ),
            (
                "fixed_utf8(8)",
                r#"{"type":"string","layout":"fixed_utf8","fixed":8}"#,
            ),
            ("large_utf8", r#"{"type":"string","layout":"large_utf8"}"#),
            ("utf8_view", r#"{"type":"string","layout":"utf8_view"}"#),
            (
                "large_utf8_view",
                r#"{"type":"string","layout":"large_utf8_view"}"#,
            ),
            ("ascii", r#"{"type":"string","layout":"ascii"}"#),
            (
                "sized_ascii(3)",
                r#"{"type":"string","layout":"sized_ascii","max":3}"#,
            ),
            (
                "fixed_ascii(4)",
                r#"{"type":"string","layout":"fixed_ascii","fixed":4}"#,
            ),
            (
                "string(windows-1252)",
                r#"{"type":"string","layout":"cp1252"}"#,
            ),
            (
                "fixed_string(windows-1252,8)",
                r#"{"type":"string","layout":"fixed_cp1252","fixed":8}"#,
            ),
            (
                "string(windows-1252,32)",
                r#"{"type":"string","layout":"sized_cp1252","max":32}"#,
            ),
            (
                "string_view(windows-1252)",
                r#"{"type":"string","layout":"cp1252_view"}"#,
            ),
            (
                "large_cp1252_view",
                r#"{"type":"string","layout":"large_cp1252_view"}"#,
            ),
        ] {
            let dtype = DataType::from_str(spelling).unwrap();
            assert_eq!(dtype.clone().into_json().unwrap(), json, "{spelling}");
            assert_eq!(DataType::from_json(json).unwrap(), dtype, "{spelling}");
            assert_eq!(
                serde_json::from_str::<DataType>(&serde_json::to_string(&dtype).unwrap()).unwrap(),
                dtype,
                "{spelling}"
            );

            let value = dtype.clone().into_value();
            assert_eq!(DataType::from_value(value).unwrap(), dtype, "{spelling}");
        }

        // An older document spelled a shape beside a charset, and a maximum
        // beside a plain or a large shape; each still reads as the leaf it
        // meant, through both doors.
        for (document, spelling) in [
            (r#"{"type":"string","charset":"us-ascii"}"#, "ascii"),
            (
                r#"{"type":"string","charset":"us-ascii","max":3}"#,
                "sized_ascii(3)",
            ),
            (
                r#"{"type":"string","layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
                "fixed_ascii(4)",
            ),
            (
                r#"{"type":"string","layout":"large_string","charset":"windows-1252"}"#,
                "large_cp1252",
            ),
            (
                r#"{"type":"string","layout":"string_view","charset":"windows-1252","max":32}"#,
                "sized_cp1252(32)",
            ),
            (r#"{"type":"string","max":32}"#, "sized_utf8(32)"),
            (
                r#"{"type":"string","layout":"large_string","max":64}"#,
                "sized_utf8(64)",
            ),
        ] {
            let dtype = DataType::from_str(spelling).unwrap();
            assert_eq!(DataType::from_json(document).unwrap(), dtype, "{document}");
            let entries =
                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(document)
                    .unwrap()
                    .into_iter()
                    .map(|(key, value)| {
                        let value = match value {
                            serde_json::Value::String(text) => Scalar::from(text.as_str()),
                            serde_json::Value::Number(number) => {
                                Scalar::from(number.as_i64().unwrap())
                            }
                            other => panic!("{document}: {other}"),
                        };
                        (Scalar::from(key.as_str()), value)
                    });
            let value = Scalar::from_mapping(entries).unwrap();
            assert_eq!(DataType::from_value(value).unwrap(), dtype, "{document}");
        }

        // The retired tags are nobody's, a number the leaf does not read is
        // refused, and so is a charset with no leaf.
        for retired in [
            r#"{"type":"utf8"}"#,
            r#"{"type":"large_utf8"}"#,
            r#"{"type":"utf8_view"}"#,
            r#"{"type":"ascii"}"#,
            r#"{"type":"fixed_ascii","width":4}"#,
            r#"{"type":"sized_utf8","max":4}"#,
            r#"{"type":"string","layout":"fixed_string"}"#,
            r#"{"type":"string","layout":"fixed_utf8"}"#,
            r#"{"type":"string","layout":"fixed_utf8","max":4}"#,
            r#"{"type":"string","layout":"sized_utf8"}"#,
            r#"{"type":"string","fixed":4}"#,
            r#"{"type":"string","max":0}"#,
            r#"{"type":"string","charset":"iso-8859-1"}"#,
            r#"{"type":"string","layout":"blob"}"#,
        ] {
            assert!(DataType::from_json(retired).is_err(), "{retired}");
        }
    }

    #[test]
    fn a_string_value_survives_the_scalar_wire_format() {
        for spelling in [
            "string(windows-1252)",
            "fixed_string(windows-1252,8)",
            "large_utf8_view",
            "utf8(32)",
            "ascii",
            "sized_ascii(4)",
            "large_cp1252_view",
        ] {
            let dtype = DataType::from_str(spelling).unwrap();
            // US-ASCII is a repertoire, so its columns get text they can hold.
            let text = match dtype.charset() {
                Some(Charset::Ascii) => "cafe",
                _ => "caf\u{e9}",
            };
            let value = dtype.scalar(text).unwrap();
            let json = serde_json::to_string(&value).unwrap();
            let read: Scalar = serde_json::from_str(&json).unwrap();
            assert_eq!(read, value, "{spelling}");
            // A value declares the leaf it is stored in, its number included.
            assert_eq!(read.id(), value.id(), "{spelling}");
            assert_eq!(
                read.string_parameters(),
                value.string_parameters(),
                "{spelling}"
            );
            assert_eq!(dtype.scalar(read).unwrap(), value, "{spelling}");
        }

        // The ordinary string still writes its characters and nothing else; a
        // value that declares more writes the leaf and its number, never a
        // charset - the leaf's name says it.
        assert_eq!(
            serde_json::to_string(&Scalar::from("AAPL")).unwrap(),
            r#"{"type":"string","value":"AAPL"}"#
        );
        assert_eq!(
            serde_json::to_string(
                &DataType::from_str("utf8(32)")
                    .unwrap()
                    .scalar("AAPL")
                    .unwrap()
            )
            .unwrap(),
            r#"{"type":"string","value":{"layout":"sized_utf8","fixed":32,"text":"AAPL"}}"#
        );
        assert_eq!(
            serde_json::to_string(&DataType::fixed_ascii(4).unwrap().scalar("USD").unwrap())
                .unwrap(),
            r#"{"type":"string","value":{"layout":"fixed_ascii","fixed":4,"text":"USD"}}"#
        );
        assert_eq!(
            serde_json::to_string(&DataType::ascii().scalar("USD").unwrap()).unwrap(),
            r#"{"type":"string","value":{"layout":"ascii","text":"USD"}}"#
        );
        assert_eq!(
            serde_json::to_string(&DataType::sized_cp1252(8).unwrap().scalar("x").unwrap())
                .unwrap(),
            r#"{"type":"string","value":{"layout":"sized_cp1252","fixed":8,"text":"x"}}"#
        );

        // An older value document spelled a shape beside a charset, and reads
        // as the leaf it meant.
        for (document, spelling) in [
            (
                r#"{"type":"string","value":{"layout":"fixed_string","charset":"us-ascii","fixed":4,"text":"USD"}}"#,
                "fixed_ascii(4)",
            ),
            (
                r#"{"type":"string","value":{"layout":"string","charset":"us-ascii","text":"USD"}}"#,
                "ascii",
            ),
            (
                r#"{"type":"string","value":{"layout":"large_string","charset":"windows-1252","text":"USD"}}"#,
                "large_cp1252",
            ),
        ] {
            let read: Scalar = serde_json::from_str(document).unwrap();
            assert_eq!(
                read,
                DataType::from_str(spelling).unwrap().scalar("USD").unwrap(),
                "{document}"
            );
            assert_eq!(read.dtype().unwrap().to_string(), spelling, "{document}");
        }
        // A numbered leaf with no width, a width on a leaf that takes none, and
        // text that does not fit are refused.
        for retired in [
            r#"{"type":"string","value":{"layout":"fixed_utf8","text":"x"}}"#,
            r#"{"type":"string","value":{"layout":"large_utf8","fixed":4,"text":"x"}}"#,
            r#"{"type":"string","value":{"layout":"fixed_ascii","fixed":4,"text":"EURO!"}}"#,
            r#"{"type":"string","value":{"layout":"ascii","text":"café"}}"#,
            r#"{"type":"string","value":{"layout":"string","charset":"iso-8859-1","text":"x"}}"#,
        ] {
            assert!(
                serde_json::from_str::<Scalar>(retired).is_err(),
                "{retired}"
            );
        }
    }

    #[test]
    fn a_hand_built_string_with_no_width_is_refused_before_a_boundary() {
        // The variant is public, so a caller can build what the constructor would
        // have refused; `validate` is where that stops.
        let unwidened = DataType::FixedCp1252String(0);
        assert!(unwidened.validate().is_err());
        assert!(unwidened.clone().into_arrow_datatype().is_err());
        assert!(
            unwidened
                .clone()
                .nullable_field("value")
                .into_arrow_field()
                .is_err()
        );
    }

    #[test]
    fn a_cast_into_another_charset_re_encodes_and_refuses_what_it_cannot_spell() {
        use arrow_array::{Array, ArrayRef, BinaryArray, StringArray};
        use std::sync::Arc;
        use yggdryl::ArrowCastOptions;

        // Arrow's kernel would hand a UTF-8 buffer to a windows-1252 column and
        // call it a framing change, and the column would read back as mojibake.
        // The string cast re-encodes every cell instead, and a scalar the
        // charset has no byte for is refused naming the charset, the row and
        // the column.
        let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café"), None]));
        let target = DataType::from_str("string(windows-1252)")
            .unwrap()
            .nullable_field("value");
        let cast = Serie::from_arrow_array(
            Some(&target),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let cast = cast.as_any().downcast_ref::<BinaryArray>().unwrap();
        assert_eq!(cast.value(0), &[0x63, 0x61, 0x66, 0xE9]);
        assert!(cast.is_null(1));

        let source: ArrayRef = Arc::new(StringArray::from(vec![Some("café"), Some("東京")]));
        let refusal = Serie::from_arrow_array(
            Some(&target),
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refusal.contains("windows-1252"), "{refusal}");
        assert!(refusal.contains("row 1"), "{refusal}");
        assert!(refusal.contains("\"value\""), "{refusal}");
        // Under `safe`, the cell that cannot be spelled becomes null.
        let lenient = Serie::from_arrow_array(Some(&target), source, ArrowCastOptions::new())
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let lenient = lenient.as_any().downcast_ref::<BinaryArray>().unwrap();
        assert_eq!(lenient.value(0), &[0x63, 0x61, 0x66, 0xE9]);
        assert!(lenient.is_null(1));

        // A field already in that exact string reads back as itself.
        let arrow = target.clone().into_arrow_field().unwrap();
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), target);
    }

    #[test]
    fn a_charset_a_column_cannot_write_is_refused_where_the_bytes_are_written() {
        // The value door counts rather than judges, because the permissive read
        // recovers damage precisely by answering scalars the charset does not
        // assign. The write is where a scalar with no byte is refused, by name.
        let dtype = DataType::from_str("string(windows-1252)").unwrap();
        let value = dtype.scalar("東京").unwrap();
        assert_eq!(value.as_str(), Some("東京"));

        let field = dtype.nullable_field("value");
        let paired = yggdryl::FieldScalar::new(&field, value).unwrap();
        let refusal = Serie::from_scalars(field.clone(), [paired.into_value()])
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("windows-1252"), "{refusal}");

        // A recovered byte survives the round trip the strict door would refuse.
        let damaged = DataType::from_str("string(windows-1252)")
            .unwrap()
            .scalar(Scalar::from(vec![0x6F_u8, 0x6B, 0x81]))
            .unwrap();
        assert_eq!(damaged.as_str(), Some("ok\u{0081}"));
    }
}

mod unit {

    use yggdryl::Field;
    use yggdryl::{DataType, StringEnum};

    #[test]
    fn a_field_declares_the_enum_its_string_values_name() {
        let side = StringEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")]).unwrap();
        let field = Field::new("side", DataType::fixed_ascii(4).unwrap(), false)
            .try_with_string_enum(&side)
            .unwrap();

        // One reserved document, readable through the `FIELD:` protocol view
        // and through the typed accessor that owns it.
        assert_eq!(
            field.get_metadata("FIELD:enum"),
            Some(r#"{"members":{"BUY":"B","SELL":"S"},"name":"Side"}"#)
        );
        assert_eq!(
            field.as_field_properties().get("enum"),
            field.get_metadata("FIELD:enum")
        );
        assert_eq!(field.string_enum().unwrap(), Some(side.clone()));
        assert_eq!(
            field.as_metadata().as_field_properties().get("enum"),
            field.get_metadata("FIELD:enum")
        );

        // The members carry the packed codes of this field's own width.
        assert_eq!(
            side.into_members(field.dtype()).unwrap(),
            [("BUY".into(), 0x4200_0000), ("SELL".into(), 0x5300_0000)]
        );

        // Metadata canonicalizes the document, so one enum is one stored text
        // whichever spelling reached the field.
        let restated = Field::new("side", DataType::fixed_ascii(4).unwrap(), false)
            .try_with_metadata(
                "FIELD:enum",
                r#"{"name":"Side","members":{"SELL":"S","BUY":"B"}}"#,
            )
            .unwrap();
        assert_eq!(
            restated.get_metadata("FIELD:enum"),
            field.get_metadata("FIELD:enum")
        );
        assert_eq!(restated.stable_hash(), field.stable_hash());

        // A declaration the width could not store is refused whole.
        let wide = StringEnum::from_members("Venue", [("LONG", "EUREX")]).unwrap();
        let mut narrow = Field::new("venue", DataType::fixed_ascii(4).unwrap(), false);
        let refused = narrow.set_string_enum(&wide).unwrap_err().to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        assert_eq!(narrow.string_enum().unwrap(), None);
        // Only a fixed US-ASCII string of at most sixteen bytes, or a code,
        // packs a member; every other string is refused by name.
        for dtype in [
            DataType::utf8(),
            DataType::ascii(),
            DataType::fixed_utf8(8).unwrap(),
            DataType::fixed_ascii(17).unwrap(),
        ] {
            let refused = Field::new("venue", dtype.clone(), false)
                .set_string_enum(&wide)
                .unwrap_err()
                .to_string();
            assert!(refused.contains(&dtype.to_string()), "{refused}");
        }
        Field::new("venue", DataType::Cfi, false)
            .set_string_enum(&wide)
            .unwrap();

        // A stored document that is not one is refused where it is written.
        let refused = Field::new("side", DataType::fixed_ascii(4).unwrap(), false)
            .try_with_metadata("FIELD:enum", "[]")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("FIELD:enum"), "{refused}");

        let mut removed = field.clone();
        assert_eq!(removed.remove_string_enum().unwrap(), Some(side));
        assert_eq!(removed.remove_string_enum().unwrap(), None);
        assert_eq!(
            removed,
            Field::new("side", DataType::fixed_ascii(4).unwrap(), false)
        );
    }
}

mod enumerated {
    use yggdryl::{DataType, StringEnum};

    #[test]
    fn the_member_name_rule_is_applied_once_per_value() {
        // An ASCII letter is kept uppercased, a digit is kept, every other
        // byte becomes `_`, a leading digit takes a `_` in front, and a name
        // that both opens and closes with `_` drops its trailing ones.
        for (value, member) in [
            ("USD", "USD"),
            ("usd", "USD"),
            ("n/a", "N_A"),
            ("-a-", "_A"),
            ("3M", "_3M"),
            ("", "_"),
        ] {
            assert_eq!(StringEnum::member_name(value).as_str(), member, "{value:?}");
        }
    }

    #[test]
    fn an_enum_names_its_values_and_renders_one_document() {
        let mut side =
            StringEnum::from_members("Side", [("SELL", "S"), ("BUY", "B"), ("BID", "B")]).unwrap();
        assert_eq!(side.name(), "Side");
        assert_eq!(side.len(), 3);
        assert_eq!(side.get("BID"), Some("B"));

        // Two members may name one value; the first by name reads it back, so
        // an alias never changes what a stored code decodes as.
        assert_eq!(side.get_member("B"), Some("BID"));
        assert_eq!(side.get_member("X"), None);
        assert_eq!(
            side.into_members(&DataType::fixed_ascii(4).unwrap())
                .unwrap(),
            [
                ("BID".into(), 0x4200_0000),
                ("BUY".into(), 0x4200_0000),
                ("SELL".into(), 0x5300_0000),
            ]
        );

        // One enum is one document however it was built, and the document is
        // what comes back.
        let document = side.into_json();
        assert_eq!(
            document,
            r#"{"members":{"BID":"B","BUY":"B","SELL":"S"},"name":"Side"}"#
        );
        assert_eq!(StringEnum::from_json(&document).unwrap(), side);
        assert_eq!(
            StringEnum::from_json(
                r#" {"name":"Side","members":{"SELL":"S","BUY":"B","BID":"B"}} "#
            )
            .unwrap(),
            side
        );
        assert_eq!(
            StringEnum::from_json(r#"{"name":"Empty"}"#).unwrap(),
            StringEnum::new("Empty").unwrap()
        );
        assert!(StringEnum::new("Empty").unwrap().is_empty());

        assert_eq!(side.remove("BID"), Some("B".into()));
        assert_eq!(side.remove("BID"), None);
        assert_eq!(side.get_member("B"), Some("BUY"));
        assert_eq!(
            side.iter().collect::<Vec<_>>(),
            [("BUY", "B"), ("SELL", "S")]
        );

        // A member the width cannot store is refused when the codes are asked
        // for, which is where the width is known.
        let refused = side
            .into_members(&DataType::utf8())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("a fixed US-ASCII string"), "{refused}");
    }

    #[test]
    fn an_enum_refuses_what_a_document_could_not_carry_back() {
        for (name, member) in [("", "BUY"), ("Side", ""), ("Si\u{7}de", "BUY")] {
            assert!(StringEnum::from_members(name, [(member, "B")]).is_err());
        }
        for document in [
            "[]",
            "not json",
            r#"{"members":{"BUY":"B"}}"#,
            r#"{"name":7}"#,
            r#"{"name":"Side","members":[]}"#,
            r#"{"name":"Side","members":{"BUY":7}}"#,
            r#"{"name":"Side","members":{"":"B"}}"#,
        ] {
            assert!(StringEnum::from_json(document).is_err(), "{document}");
        }
    }

    #[test]
    fn an_ascii_value_packs_into_the_integer_its_storage_reads_as() {
        for (dtype, value, packed) in [
            (DataType::fixed_ascii(4).unwrap(), "USD", 0x5553_4400_i128),
            (DataType::fixed_ascii(4).unwrap(), "", 0),
            (
                DataType::fixed_ascii(8).unwrap(),
                "EUREX",
                0x4555_5245_5800_0000,
            ),
            (
                DataType::fixed_ascii(16).unwrap(),
                "US0378331005",
                0x5553_3033_3738_3333_3130_3035_0000_0000,
            ),
        ] {
            assert_eq!(dtype.ascii_packed(value.as_bytes()).unwrap(), packed);
            // The storage padding is the same value, and reads back trimmed.
            let padded = format!("{value}\0");
            assert_eq!(dtype.ascii_packed(padded.as_bytes()).unwrap(), packed);
            assert_eq!(dtype.ascii_value(packed).unwrap(), value);
        }

        // An ASCII byte never sets the sign bit, so the order of the packed
        // integers is the order of the text.
        assert!(
            DataType::fixed_ascii(4)
                .unwrap()
                .ascii_packed(b"EUR")
                .unwrap()
                < DataType::fixed_ascii(4)
                    .unwrap()
                    .ascii_packed(b"USD")
                    .unwrap()
        );
        assert!(
            DataType::fixed_ascii(8)
                .unwrap()
                .ascii_packed(b"\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f")
                .unwrap()
                > 0
        );

        // What the width refuses, the packing refuses, in both directions.
        let refused = DataType::fixed_ascii(4)
            .unwrap()
            .ascii_packed(b"EURO!")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        let refused = DataType::fixed_ascii(4)
            .unwrap()
            .ascii_value(-1)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("wider than the width"), "{refused}");
        let refused = DataType::fixed_ascii(4)
            .unwrap()
            .ascii_value(0x1_5553_4400)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("wider than the width"), "{refused}");
        let refused = DataType::fixed_ascii(4)
            .unwrap()
            .ascii_value(0x0055_4400)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("a NUL byte at 0"), "{refused}");
        let refused = DataType::utf8()
            .ascii_packed(b"USD")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("a fixed US-ASCII string"), "{refused}");
        assert!(DataType::utf8().ascii_value(0).is_err());
        // The variable shape has no width, so it has no packed integer; nor
        // does a width wider than the widest integer this crate carries.
        assert!(DataType::ascii().ascii_packed(b"USD").is_err());
        assert!(
            DataType::fixed_ascii(17)
                .unwrap()
                .ascii_packed(b"USD")
                .is_err()
        );
    }
}

mod widths {
    use std::cmp::Ordering;

    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;

    use yggdryl::{ArrowCastOptions, Charset, DataTypeId, DataTypeKind, StructType};
    use yggdryl::{DataType, Serie, StringType};
    use yggdryl::{Error, Field, Scalar, Scheme};

    fn hash_of(value: &DataType) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn stored(array: &dyn Array) -> &FixedSizeBinaryArray {
        array
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .expect("fixed-width string storage")
    }

    /// Lay one value out as the one-row column `field` types.
    fn lay_out(field: &Field, value: &Scalar) -> yggdryl::Result<ArrayRef> {
        Serie::from_scalars(field.clone(), [value.clone()])?.require_arrow_array()
    }

    /// Read row 0 of an Arrow array back as the value `field` types.
    fn read_back(field: &Field, array: ArrayRef) -> yggdryl::arrow::Result<Scalar> {
        Ok(Serie::from_arrow_array(Some(field), array, ArrowCastOptions::default())?.scalar(0)?)
    }

    /// A US-ASCII string bounded to `max` bytes: the sized leaf.
    fn bounded_ascii(max: u32) -> DataType {
        DataType::sized_ascii(max).unwrap()
    }

    #[test]
    fn every_spelling_parses_and_displays_as_its_datatype() {
        for (spelling, dtype) in [
            ("ascii", DataType::ascii()),
            ("ASCII", DataType::ascii()),
            ("string(us-ascii)", DataType::ascii()),
            ("utf8", DataType::utf8()),
            ("string", DataType::utf8()),
            ("large_utf8", DataType::large_utf8()),
            ("utf8_view", DataType::utf8_view()),
            // A number after the plain spelling is a maximum, which is the
            // sized leaf.
            ("ascii(3)", bounded_ascii(3)),
            ("string(us-ascii,3)", bounded_ascii(3)),
            ("sized_ascii(3)", bounded_ascii(3)),
            // A fixed width is exactly what it says, at any length.
            ("fixed_ascii(1)", DataType::fixed_ascii(1).unwrap()),
            ("fixed_ascii(3)", DataType::fixed_ascii(3).unwrap()),
            (
                "fixed_string(us-ascii,6)",
                DataType::fixed_ascii(6).unwrap(),
            ),
            ("fixed_ascii(12)", DataType::fixed_ascii(12).unwrap()),
            ("fixed_ascii(64)", DataType::fixed_ascii(64).unwrap()),
            ("fixed_utf8(8)", DataType::fixed_utf8(8).unwrap()),
            ("char(8)", DataType::fixed_utf8(8).unwrap()),
            ("country", DataType::Country),
            ("Country", DataType::Country),
            ("ccy", DataType::Ccy),
            ("Ccy", DataType::Ccy),
            ("mic", DataType::Mic),
            ("MIC", DataType::Mic),
            ("Exchange", DataType::Mic),
            ("cfi", DataType::Cfi),
            ("CFI", DataType::Cfi),
            ("MonthYear", DataType::fixed_ascii(8).unwrap()),
        ] {
            let parsed: DataType = spelling
                .parse()
                .unwrap_or_else(|error| panic!("{spelling} must parse: {error}"));
            assert_eq!(parsed, dtype, "{spelling}");
            // One canonical spelling: every alias displays as the datatype,
            // and that display re-parses to the same value.
            assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
        }
        let row: DataType =
            "struct<ccy: ccy, isin: fixed_ascii(12), code: cfi, iso: country, name: utf8(32)>"
                .parse()
                .unwrap();
        assert_eq!(
            row.get_field_by_path("ccy").map(Field::dtype),
            Some(&DataType::Ccy)
        );
        assert_eq!(
            row.get_field_by_path("isin").map(Field::dtype),
            Some(&DataType::fixed_ascii(12).unwrap())
        );
        assert_eq!(
            row.get_field_by_path("code").map(Field::dtype),
            Some(&DataType::Cfi)
        );
        assert_eq!(
            row.get_field_by_path("iso").map(Field::dtype),
            Some(&DataType::Country)
        );
        assert_eq!(
            row.get_field_by_path("name")
                .and_then(|field| field.dtype().string_parameters())
                .and_then(StringType::max),
            Some(32)
        );
    }

    #[test]
    fn a_width_of_no_bytes_is_refused_by_name() {
        let error = "fixed_ascii(0)"
            .parse::<DataType>()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("expected a width of at least one byte, got 0"),
            "{error}"
        );
        let error = "ascii(0)".parse::<DataType>().unwrap_err().to_string();
        assert!(
            error.contains("expected a width of at least one byte, got 0"),
            "{error}"
        );
        // Bare `ascii` is the variable shape, not a missing width, and empty
        // parentheses are that spelling with punctuation.
        assert_eq!("ascii".parse::<DataType>().unwrap(), DataType::ascii());
        assert_eq!("ascii()".parse::<DataType>().unwrap(), DataType::ascii());
        // A width is what makes a string fixed, so there is no width-free spelling.
        assert!(matches!(
            "fixed_ascii".parse::<DataType>(),
            Err(Error::Parse { .. })
        ));
        assert!(matches!(
            "fixed_ascii()".parse::<DataType>(),
            Err(Error::Parse { .. })
        ));
        assert!(DataType::FixedAsciiString(0).validate().is_err());

        // A width above the packed limit is a legal column and simply has no
        // packed integer.
        assert_eq!(
            DataType::fixed_ascii(17).unwrap(),
            "fixed_ascii(17)".parse::<DataType>().unwrap()
        );
        assert!(
            DataType::fixed_ascii(17)
                .unwrap()
                .ascii_packed(b"USD")
                .is_err()
        );
        assert!(DataType::fixed_ascii(0).is_err());
        assert!(DataType::fixed_utf8(0).is_err());
    }

    #[test]
    fn a_registered_code_is_its_own_datatype_over_its_standard_width() {
        // A code parses to itself and displays as itself: there is no width
        // hiding behind the name, and no second spelling of either.
        for (name, dtype, width) in DataType::CODES {
            assert_eq!(name.parse::<DataType>().unwrap(), *dtype);
            assert_eq!(dtype.to_string(), *name);
            assert_eq!(dtype.name(), *name);
            // The width bounds the value; it is not a layout, so a code names
            // no fixed width.
            assert_eq!(dtype.code_width(), Some(*width));
            assert_eq!(dtype.fixed_byte_width(), None);
            assert_eq!(dtype.kind(), DataTypeKind::Code);
            assert!(dtype.is_code());
            assert!(!dtype.is_string());
            assert_eq!(dtype.string_parameters(), None);
        }
        assert_eq!("ccy".parse::<DataType>().unwrap(), DataType::Ccy);
        for retired in ["currency", "Currency", "CURRENCY"] {
            assert!(retired.parse::<DataType>().is_err(), "{retired}");
        }
        assert_ne!(DataType::Ccy, DataType::fixed_ascii(3).unwrap());
        // ISO 10962 is six characters, and `cfi` holds at most those six.
        assert_eq!(DataType::Cfi.code_width(), Some(6));
        // A width of six bytes is spellable, and it is still not a CFI code.
        assert_ne!(DataType::Cfi, DataType::fixed_ascii(6).unwrap());
        assert!(!DataType::fixed_ascii(6).unwrap().is_code());
        // ISO 6166 is twelve characters closed by a check digit, and `isin`
        // stores exactly those twelve.
        assert_eq!("isin".parse::<DataType>().unwrap(), DataType::Isin);
        assert_eq!(DataType::Isin.code_width(), Some(12));
        assert_ne!(DataType::Isin, DataType::fixed_ascii(12).unwrap());
        // A CUSIP is nine and a SEDOL seven, each closed by its own check digit,
        // and neither is the ASCII width that would hold the same text.
        assert_eq!("cusip".parse::<DataType>().unwrap(), DataType::Cusip);
        assert_eq!(DataType::Cusip.code_width(), Some(9));
        assert_ne!(DataType::Cusip, DataType::fixed_ascii(9).unwrap());
        assert_eq!("sedol".parse::<DataType>().unwrap(), DataType::Sedol);
        assert_eq!(DataType::Sedol.code_width(), Some(7));
        assert_ne!(DataType::Sedol, DataType::fixed_ascii(7).unwrap());
        // A Bloomberg identifier and a RIC are bounded rather than shaped: at
        // most thirty-two bytes each, and neither is the ASCII width that
        // would hold the same text.
        assert_eq!("bbg".parse::<DataType>().unwrap(), DataType::Bbg);
        assert_eq!(DataType::Bbg.code_width(), Some(32));
        assert_ne!(DataType::Bbg, DataType::from_str("ascii(32)").unwrap());
        assert_eq!("ric".parse::<DataType>().unwrap(), DataType::Ric);
        assert_eq!(DataType::Ric.code_width(), Some(32));
        assert_ne!(DataType::Ric, DataType::from_str("ascii(32)").unwrap());

        // A code name is a grammar keyword like every other, so the parser
        // reads it case-insensitively and trimmed.
        assert_eq!(" CCY ".parse::<DataType>().unwrap(), DataType::Ccy);
        assert!(" CURRENCY ".parse::<DataType>().is_err());
        assert_eq!(" BBG ".parse::<DataType>().unwrap(), DataType::Bbg);
        assert!(" BLOOMBERG ".parse::<DataType>().is_err());
        assert_eq!(" RIC ".parse::<DataType>().unwrap(), DataType::Ric);
        // FIGI is its own checked twelve-character code, not a Bloomberg alias.
        assert_eq!(" FIGI ".parse::<DataType>().unwrap(), DataType::Figi);
        // The grammar still reports words that name nothing as unknown.
        let error = "figx".parse::<DataType>().unwrap_err().to_string();
        assert!(error.contains("unknown datatype \"figx\""), "{error}");
    }

    #[test]
    fn a_code_packs_and_merges_by_the_ascii_rules() {
        // The packed integer is the value's own storage bytes, exactly as it
        // is for a width: the code is a datatype, not a second encoding.
        assert_eq!(DataType::Ccy.ascii_packed(b"USD").unwrap(), 0x0055_5344);
        assert_eq!(DataType::Ccy.ascii_value(0x0055_5344).unwrap(), "USD");
        assert_eq!(
            DataType::Ccy.ascii_packed(b"USD").unwrap(),
            DataType::fixed_ascii(3)
                .unwrap()
                .ascii_packed(b"USD")
                .unwrap()
        );
        assert_eq!(DataType::Country.ascii_packed(b"FR").unwrap(), 0x4652);
        assert_eq!(
            DataType::Cfi.ascii_packed(b"ESVUFR").unwrap(),
            0x4553_5655_4652
        );
        let refused = DataType::Country
            .ascii_packed(b"USD")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 2 bytes"), "{refused}");
        // Only a fixed US-ASCII width packs: a maximum, another charset and a
        // width past sixteen bytes are refused by name.
        assert!(bounded_ascii(3).ascii_packed(b"USD").is_err());
        assert!(
            DataType::fixed_utf8(3)
                .unwrap()
                .ascii_packed(b"USD")
                .is_err()
        );

        // Two schemas that agree on a code keep it; a code reconciled with
        // anything else answers the plain text both fit in.
        assert_eq!(
            DataType::Ccy.merge_with(&DataType::Ccy, true).unwrap(),
            DataType::Ccy
        );
        assert_eq!(
            DataType::Ccy
                .merge_with(&DataType::fixed_ascii(3).unwrap(), true)
                .unwrap(),
            DataType::from_str("ascii(3)").unwrap()
        );
        assert_eq!(
            DataType::Ccy.merge_with(&DataType::Country, true).unwrap(),
            DataType::from_str("ascii(3)").unwrap()
        );
        assert_eq!(
            DataType::Ccy.merge_with(&DataType::utf8(), true).unwrap(),
            DataType::utf8()
        );
    }

    #[test]
    fn serde_and_the_structural_value_round_trip() {
        // One `string` tag for every string: the leaf is written where it is
        // not `utf8` and the number beside it; the leaf's name says its charset,
        // so a fixed width carries the leaf and the width beside the tag.
        for width in [1, 3, 4, 6, 12, 16, 64] {
            let dtype = DataType::fixed_ascii(width).unwrap();
            let json = dtype.clone().into_json().unwrap();
            assert_eq!(
                json,
                format!(r#"{{"type":"string","layout":"fixed_ascii","fixed":{width}}}"#)
            );
            assert_eq!(DataType::from_json(&json).unwrap(), dtype);

            let value = dtype.clone().into_value();
            assert_eq!(
                value.get_key_str("type").and_then(Scalar::as_str),
                Some("string")
            );
            assert_eq!(
                value.get_key_str("fixed").and_then(Scalar::as_i128),
                Some(i128::from(width))
            );
            assert_eq!(DataType::from_value(value).unwrap(), dtype);
        }

        // The variable shape states its leaf and nothing else; plain UTF-8 is
        // the bare tag.
        for (dtype, json) in [
            (DataType::ascii(), r#"{"type":"string","layout":"ascii"}"#),
            (DataType::utf8(), r#"{"type":"string"}"#),
            (
                DataType::large_utf8(),
                r#"{"type":"string","layout":"large_utf8"}"#,
            ),
            (
                bounded_ascii(3),
                r#"{"type":"string","layout":"sized_ascii","max":3}"#,
            ),
            (
                "fixed_string(windows-1252,8)".parse().unwrap(),
                r#"{"type":"string","layout":"fixed_cp1252","fixed":8}"#,
            ),
        ] {
            assert_eq!(dtype.clone().into_json().unwrap(), json);
            assert_eq!(DataType::from_json(json).unwrap(), dtype);
            assert_eq!(
                DataType::from_value(dtype.clone().into_value()).unwrap(),
                dtype
            );
        }
        // An older document spelled the shape beside a charset, and still
        // reads as the leaf it meant.
        for (document, dtype) in [
            (
                r#"{"type":"string","charset":"us-ascii"}"#,
                DataType::ascii(),
            ),
            (
                r#"{"type":"string","charset":"us-ascii","max":3}"#,
                bounded_ascii(3),
            ),
            (
                r#"{"type":"string","layout":"fixed_string","charset":"us-ascii","fixed":4}"#,
                DataType::fixed_ascii(4).unwrap(),
            ),
        ] {
            assert_eq!(DataType::from_json(document).unwrap(), dtype, "{document}");
        }

        // The retired tags name nothing.
        for tag in ["utf8", "large_utf8", "utf8_view", "ascii", "fixed_ascii"] {
            assert!(
                DataType::from_json(&format!(r#"{{"type":"{tag}","width":4}}"#)).is_err(),
                "{tag}"
            );
        }
        // A fixed leaf with no width, and a bound on the wrong side, are refused.
        assert!(DataType::from_json(r#"{"type":"string","layout":"fixed_ascii"}"#).is_err());
        assert!(DataType::from_json(r#"{"type":"string","layout":"fixed_string"}"#).is_err());
        assert!(DataType::from_json(r#"{"type":"string","fixed":4}"#).is_err());
    }

    #[test]
    fn identity_kind_and_widths_answer_for_every_width() {
        // One identifier covers every fixed width, because the width is the
        // leaf's number and not a leaf of its own.
        for width in [1, 2, 3, 4, 6, 8, 12, 16, 64] {
            let dtype = DataType::fixed_ascii(width).unwrap();
            assert_eq!(dtype.id(), DataTypeId::FixedAsciiString);
            assert_eq!(dtype.kind(), DataTypeKind::Text);
            assert_eq!(dtype.name(), "fixed_ascii");
            assert_eq!(dtype.to_string(), format!("fixed_ascii({width})"));
            assert_eq!(dtype.fixed_byte_width(), Some(width as usize));
            assert_eq!(dtype.charset(), Some(Charset::Ascii));
            assert!(dtype.is_string());
            assert!(!dtype.is_nested());
            dtype.validate().unwrap();
        }
        // The variable shape is the same family with no width at all.
        assert_eq!(DataType::ascii().id(), DataTypeId::AsciiString);
        assert_eq!(bounded_ascii(3).id(), DataTypeId::SizedAsciiString);
        assert_eq!(DataType::ascii().kind(), DataTypeKind::Text);
        assert_eq!(DataType::ascii().fixed_byte_width(), None);
        assert_eq!(DataType::ascii().charset(), Some(Charset::Ascii));
        assert!(DataType::ascii().is_string());
        assert_eq!(DataType::utf8().fixed_byte_width(), None);
        assert_eq!(DataType::utf8().charset(), Some(Charset::Utf8));
        assert_eq!(bounded_ascii(3).fixed_byte_width(), None);
        assert_eq!(
            DataType::fixed_binary(4).unwrap().fixed_byte_width(),
            Some(4)
        );
        assert_eq!(DataType::binary().charset(), None);
    }

    #[test]
    fn ordering_and_hashing_are_consistent_for_every_width() {
        // Every string is one variant, ordered by leaf - the six UTF-8 shapes,
        // then the six US-ASCII ones, then windows-1252, the number ordering
        // within a leaf; the codes follow the strings and the nested datatypes
        // follow the codes.
        assert!(DataType::utf8() < DataType::large_utf8());
        assert!(DataType::large_utf8() < DataType::utf8_view());
        assert!(DataType::utf8_view() < DataType::fixed_utf8(8).unwrap());
        assert!(DataType::fixed_utf8(8).unwrap() < DataType::ascii());
        assert!(DataType::ascii() < DataType::fixed_ascii(2).unwrap());
        assert!(DataType::fixed_ascii(2).unwrap() < DataType::fixed_ascii(3).unwrap());
        assert!(DataType::fixed_ascii(3).unwrap() < DataType::fixed_ascii(4).unwrap());
        assert!(DataType::fixed_ascii(4).unwrap() < DataType::fixed_ascii(8).unwrap());
        assert!(DataType::fixed_ascii(8).unwrap() < DataType::fixed_ascii(12).unwrap());
        assert!(DataType::fixed_ascii(12).unwrap() < DataType::fixed_ascii(16).unwrap());
        assert!(DataType::fixed_ascii(16).unwrap() < bounded_ascii(3));
        assert!(bounded_ascii(3) < DataType::cp1252());
        assert!(DataType::cp1252() < DataType::Country);
        assert!(DataType::Country < DataType::serie(DataType::utf8().nullable_field("item")));
        assert_eq!(
            DataType::fixed_ascii(8)
                .unwrap()
                .cmp(&DataType::fixed_ascii(8).unwrap()),
            Ordering::Equal
        );
        assert_eq!(
            hash_of(&DataType::fixed_ascii(8).unwrap()),
            hash_of(&DataType::fixed_ascii(8).unwrap())
        );
        assert_ne!(
            DataType::fixed_ascii(4).unwrap().stable_hash(),
            DataType::fixed_ascii(8).unwrap().stable_hash()
        );
        // A code and the width that would hold it are two identities over the
        // same characters, and the hash is what tells them apart.
        for (code, width) in [
            (DataType::Country, DataType::fixed_ascii(2).unwrap()),
            (DataType::Ccy, DataType::fixed_ascii(3).unwrap()),
            (DataType::Mic, DataType::fixed_ascii(4).unwrap()),
            (DataType::Cfi, DataType::fixed_ascii(6).unwrap()),
        ] {
            assert_ne!(code.stable_hash(), width.stable_hash(), "{code}");
            assert_eq!(code.stable_hash(), code.clone().stable_hash(), "{code}");
        }
    }

    #[test]
    fn the_default_is_the_empty_string_stored_as_all_nul() {
        for dtype in [
            DataType::fixed_ascii(2).unwrap(),
            DataType::fixed_ascii(3).unwrap(),
            DataType::fixed_ascii(4).unwrap(),
            DataType::fixed_ascii(8).unwrap(),
            DataType::fixed_ascii(12).unwrap(),
            DataType::fixed_ascii(16).unwrap(),
            DataType::fixed_utf8(8).unwrap(),
        ] {
            let exact_empty = dtype.scalar(Scalar::from("")).unwrap();
            assert_eq!(dtype.default_value().unwrap(), exact_empty);
            assert!(dtype.is_default_value(&Scalar::from("")).unwrap());
            assert!(!dtype.is_default_value(&Scalar::from("USD")).unwrap());
            // The default is the empty text under the column's own parameters,
            // and its stored bytes are the padded slot.
            let leaf = exact_empty.string_parameters().expect("a string default");
            assert_eq!(leaf, dtype.string_parameters().unwrap());
            let width = dtype.fixed_byte_width().unwrap();
            assert_eq!(
                leaf.encode(exact_empty.as_str().unwrap()).unwrap().len(),
                width
            );

            let field = dtype.required_field("ccy");
            assert_eq!(field.default_value().unwrap(), exact_empty);
            let array = Serie::from_default(field, 1)
                .unwrap()
                .require_arrow_array()
                .unwrap();
            assert_eq!(
                array.data_type(),
                &ArrowDataType::FixedSizeBinary(i32::try_from(width).unwrap())
            );
            let stored = stored(array.as_ref());
            assert_eq!(stored.len(), 1);
            assert!(stored.value(0).iter().all(|byte| *byte == 0));
        }
        // A variable string defaults to the empty text under its charset.
        let empty = DataType::ascii().default_value().unwrap();
        assert_eq!(empty, Scalar::from(""));
        let leaf = empty.string_parameters().expect("a string default");
        assert_eq!(leaf.charset(), Charset::Ascii);
    }

    #[test]
    fn values_validate_and_canonicalize_under_the_one_ascii_rule() {
        let dtype = DataType::fixed_ascii(4).unwrap();
        let root = StructType::from_fields([dtype.clone().required_field("ccy")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let row = |value: Scalar| Scalar::from_sequence([value]);
        let canonical = |value: Scalar| root.canonicalize_value(row(value)).unwrap();
        let fixed = |value: &str| dtype.string_parameters().unwrap().scalar(value).unwrap();

        // Text inputs canonicalize to the fixed-width US-ASCII value.
        assert_eq!(canonical(Scalar::from("USD")), row(fixed("USD")));
        assert_eq!(canonical(Scalar::from("ABCD")), row(fixed("ABCD")));
        assert_eq!(canonical(Scalar::from("")), row(fixed("")));
        let exact = fixed("USD");
        assert_eq!(canonical(exact.clone()), row(exact));
        // Trailing NULs are trimmed, and bytes are rewritten to that value.
        assert_eq!(canonical(Scalar::from("USD\0")), row(fixed("USD")));
        assert_eq!(
            canonical(Scalar::from(b"USD\0".to_vec())),
            row(fixed("USD"))
        );
        assert_eq!(canonical(Scalar::from(b"EUR".to_vec())), row(fixed("EUR")));
        // The canonical value carries the column's parameters, not the text's.
        let restated = canonical(Scalar::from("USD"));
        let Some(cell) = restated.get(0) else {
            panic!("a string cell");
        };
        let held = cell.string_parameters().expect("a string cell");
        assert_eq!(held.fixed(), Some(4));
        assert_eq!(held.charset(), Charset::Ascii);

        // Every refusal names the offending fact and the column.
        for (value, fact) in [
            (Scalar::from("EURO!"), "at most 4 bytes"),
            (Scalar::from(b"ABCDE".to_vec()), "at most 4 bytes"),
            (Scalar::from("\u{20AC}"), "non-ASCII byte 0xE2 at 0"),
            (Scalar::from("U\0D"), "NUL byte at 1"),
        ] {
            let refused = root
                .validate_value(&row(value.clone()))
                .unwrap_err()
                .to_string();
            assert!(refused.contains(fact), "{refused}");
            assert!(refused.contains("ccy"), "{refused}");
            assert!(root.canonicalize_value(row(value)).is_err());
        }
        // A value that spells no text at all is refused by kind.
        let refused = root
            .validate_value(&row(Scalar::from_sequence([Scalar::from(7)])))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("expected fixed_ascii"), "{refused}");

        // A fixed value carries its own padded width, so one written at another
        // width is restated at the column's rather than kept as it arrived: the
        // column declares the storage, and the value must name the same one.
        let wider = StringType::FixedAsciiString(8).scalar("USD").unwrap();
        root.validate_value(&row(wider.clone())).unwrap();
        assert_eq!(canonical(wider), row(fixed("USD")));

        // The variable US-ASCII layout keeps a NUL-free value as it is and
        // refuses the repertoire the same way.
        let ascii = DataType::ascii();
        assert_eq!(ascii.scalar("USD").unwrap(), Scalar::from("USD"));
        assert!(ascii.scalar("\u{20AC}").is_err());
        assert!(ascii.scalar("U\0D").is_err());
        assert!(bounded_ascii(3).scalar("EURO").is_err());
        let bounded = bounded_ascii(3).scalar("USD").unwrap();
        assert_eq!(bounded, Scalar::from("USD"));
        assert_eq!(
            bounded.string_parameters(),
            Some(StringType::SizedAsciiString(3))
        );
    }

    #[test]
    fn arrow_storage_is_padded_and_reads_back_trimmed() {
        let field = DataType::fixed_ascii(8).unwrap().nullable_field("code");
        let array = lay_out(&field, &Scalar::from("ABC")).unwrap();
        assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(8));
        assert_eq!(stored(array.as_ref()).value(0), b"ABC\0\0\0\0\0");
        assert_eq!(
            read_back(&field, array).unwrap(),
            field.dtype().scalar(Scalar::from("ABC")).unwrap()
        );

        // Padded bytes, a null, and the empty string, through the array boundary.
        let values = [
            Scalar::from(b"XY\0\0\0\0\0\0".to_vec()),
            Scalar::Null,
            Scalar::from(""),
        ];
        let array = Serie::from_scalars(field.clone(), values)
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let fixed = stored(array.as_ref());
        assert_eq!(fixed.len(), 3);
        assert_eq!(fixed.value(0), b"XY\0\0\0\0\0\0");
        assert!(fixed.is_null(1));
        assert_eq!(fixed.value(2), &[0; 8]);
        // One row at a time through the public boundary: a one-row slice read
        // under the field, and slicing costs no copy.
        let read = |index: usize| read_back(&field, array.slice(index, 1)).unwrap();
        assert_eq!(read(0), field.dtype().scalar(Scalar::from("XY")).unwrap());
        assert_eq!(read(2), field.dtype().scalar(Scalar::from("")).unwrap());

        // What does not fit is refused at this boundary too.
        assert!(lay_out(&field, &Scalar::from("ABCDEFGHI")).is_err());

        // The variable US-ASCII layout rides Arrow's own text storage.
        let field = DataType::ascii().nullable_field("code");
        let array = lay_out(&field, &Scalar::from("ABC")).unwrap();
        assert_eq!(array.data_type(), &ArrowDataType::Utf8);
    }

    #[test]
    fn compatibility_reads_every_width_as_str() {
        let schema = StructType::from_fields([
            DataType::fixed_ascii(4).unwrap().nullable_field("ccy"),
            DataType::fixed_ascii(16).unwrap().required_field("code"),
            bounded_ascii(3).required_field("bounded"),
            "string(windows-1252)"
                .parse::<DataType>()
                .unwrap()
                .required_field("latin"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        for scheme in [
            Scheme::SPARK,
            Scheme::POLARS,
            Scheme::PANDAS,
            Scheme::ICEBERG,
        ] {
            let compat = schema.clone().into_scheme_compat(&scheme).unwrap();
            assert_eq!(compat["ccy"].dtype(), &DataType::utf8());
            assert_eq!(compat["code"].dtype(), &DataType::utf8());
            assert_eq!(compat["bounded"].dtype(), &DataType::utf8());
            assert_eq!(compat["latin"].dtype(), &DataType::utf8());
            assert!(!compat["code"].is_nullable());
        }
        assert_eq!(
            schema.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
            schema
        );
        assert_eq!(
            DataType::fixed_ascii(8)
                .unwrap()
                .into_scheme_compat(&Scheme::ICEBERG)
                .unwrap(),
            DataType::utf8()
        );
        assert_eq!(
            DataType::utf8().into_scheme_compat(&Scheme::SPARK).unwrap(),
            DataType::utf8()
        );
    }
}

mod listings {
    use yggdryl::DataType;
    use yggdryl::StringEnum;

    fn lists() -> [(&'static str, &'static [&'static str]); 3] {
        [
            ("ccy", StringEnum::CURRENCIES),
            ("country", StringEnum::COUNTRIES),
            ("mic", StringEnum::MICS),
        ]
    }

    #[test]
    fn every_constant_is_sorted_unique_and_fits_its_width() {
        for (name, values) in lists() {
            assert!(!values.is_empty(), "{name} prebuilds nothing");
            assert!(
                values.windows(2).all(|pair| pair[0] < pair[1]),
                "{name} is not sorted and deduplicated"
            );
            let width = DataType::from_logical_name(name)
                .unwrap()
                .code_width()
                .unwrap();
            for value in values {
                assert!(
                    value.is_ascii() && !value.is_empty() && value.len() <= width,
                    "{name} holds {value:?}, which does not fit {width} bytes"
                );
                assert!(
                    value
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
                    "{name} holds {value:?}, which is not an uppercase code"
                );
            }
        }
        // The code sets are the standards' own shapes.
        assert!(StringEnum::CURRENCIES.iter().all(|code| code.len() == 3));
        assert!(StringEnum::COUNTRIES.iter().all(|code| code.len() == 2));
        assert!(StringEnum::MICS.iter().all(|code| code.len() == 4));
    }

    /// An ISO code is already an identifier, so no two members collide and
    /// the member is the code itself.
    #[test]
    fn every_constant_names_its_own_enum_members() {
        for (name, values) in lists() {
            let declared = StringEnum::from_logical_name(name).unwrap();
            let dtype = DataType::from_logical_name(name).unwrap();
            assert_eq!(declared.len(), values.len(), "{name}");
            assert_eq!(declared.name(), name, "{name}");
            for value in values {
                assert_eq!(declared.get(value), Some(*value), "{value}");
            }
            // A member is the integer its value packs into under the width
            // the name resolves to, never a position in this listing.
            let members = declared.into_members(&dtype).unwrap();
            for (member, code) in &members {
                assert_eq!(*code, dtype.ascii_packed(member.as_bytes()).unwrap());
            }
        }
    }

    #[test]
    fn the_two_names_of_one_list_prebuild_one_vocabulary() {
        assert_eq!(
            StringEnum::from_logical_name("Exchange").unwrap().len(),
            StringEnum::from_logical_name("mic").unwrap().len()
        );
        assert_eq!(StringEnum::prebuilt_values(" MIC "), StringEnum::MICS);
        assert!(StringEnum::prebuilt_values("isin").is_empty());
        assert!(StringEnum::prebuilt_values("cusip").is_empty());
        assert!(StringEnum::prebuilt_values("sedol").is_empty());
    }

    #[test]
    fn a_registered_name_with_no_constant_prebuilds_no_members() {
        for name in ["language", "monthyear", "tenor", "figi", "bbg", "ric"] {
            assert!(
                StringEnum::from_logical_name(name).unwrap().is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_name_that_is_not_registered_is_refused_by_the_vocabulary() {
        let refused = StringEnum::from_logical_name("unregistered_code")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("ccy"), "{refused}");
    }
}

/// Every string and byte leaf is a variant of its own on `DataType`, `Field`
/// and `Scalar`, one to one with its identifier, and every door a value
/// crosses - the value door, the wire, the value stream, the order - keeps
/// the leaf, its number included.
mod leaf_variants {
    use std::cmp::Ordering;

    use yggdryl::{Bytes, BytesType, DataType, Field, Scalar, Str, StringType};

    fn strings() -> Vec<StringType> {
        [4_u32, 9]
            .into_iter()
            .flat_map(|number| {
                StringType::ALL.map(|leaf| {
                    leaf.with_declared_bound(leaf.bound().map(|_| number))
                        .unwrap()
                })
            })
            .collect()
    }

    fn bytes() -> Vec<BytesType> {
        [3_u32, 9]
            .into_iter()
            .flat_map(|number| {
                BytesType::ALL.map(|leaf| {
                    leaf.with_declared_bound(leaf.bound().map(|_| number))
                        .unwrap()
                })
            })
            .collect()
    }

    #[test]
    fn every_string_leaf_is_one_variant_on_every_root() {
        for leaf in strings() {
            let dtype = DataType::from(leaf);
            assert_eq!(DataType::string(leaf).unwrap(), dtype);
            assert_eq!(dtype.id(), leaf.id(), "{leaf}");
            assert_eq!(dtype.string_parameters(), Some(leaf));
            assert_eq!(StringType::try_from(&dtype).unwrap(), leaf);
            assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
            let field = Field::new("x", dtype.clone(), true);
            assert_eq!(field.dtype(), &dtype);
            assert_eq!(field.id(), leaf.id());

            let value = leaf.scalar("ab").unwrap();
            assert_eq!(value.id(), leaf.id(), "{leaf}");
            assert_eq!(value.string_parameters(), Some(leaf));
            assert_eq!(value.dtype().unwrap(), dtype, "the value keeps its number");
            assert_eq!(value.as_str(), Some("ab"));
            assert_eq!(value.as_string().map(Str::as_str), Some("ab"));
            assert_eq!(value.bytes_parameters(), None);
            assert_eq!(
                value,
                Scalar::from("ab"),
                "a value is one value in any leaf"
            );
            assert_eq!(dtype.scalar("ab").unwrap().string_parameters(), Some(leaf));
            assert_eq!(field.scalar("ab").unwrap().string_parameters(), Some(leaf));
        }
    }

    #[test]
    fn every_byte_leaf_is_one_variant_on_every_root() {
        for leaf in bytes() {
            let dtype = DataType::from(leaf);
            assert_eq!(DataType::bytes(leaf).unwrap(), dtype);
            assert_eq!(dtype.id(), leaf.id(), "{leaf}");
            assert_eq!(dtype.bytes_parameters(), Some(leaf));
            assert_eq!(BytesType::try_from(&dtype).unwrap(), leaf);
            assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);

            let payload = vec![1_u8; leaf.fixed().map_or(2, |width| width as usize)];
            let value = leaf.scalar(payload.clone()).unwrap();
            assert_eq!(value.id(), leaf.id(), "{leaf}");
            assert_eq!(value.bytes_parameters(), Some(leaf));
            assert_eq!(value.dtype().unwrap(), dtype, "the value keeps its number");
            assert_eq!(value.as_bytes(), Some(payload.as_slice()));
            assert_eq!(
                value.as_binary().map(Bytes::as_bytes),
                Some(payload.as_slice())
            );
            assert_eq!(value.string_parameters(), None);
            assert_eq!(value, Scalar::from(payload.clone()));
        }
    }

    #[test]
    fn the_wire_and_the_value_stream_keep_the_leaf_and_its_number() {
        let values = strings()
            .into_iter()
            .map(|leaf| leaf.scalar("ab").unwrap())
            .chain(bytes().into_iter().map(|leaf| {
                leaf.scalar(vec![7_u8; leaf.fixed().map_or(2, |width| width as usize)])
                    .unwrap()
            }));
        for value in values {
            let json = serde_json::to_string(&value).unwrap();
            let read = serde_json::from_str::<Scalar>(&json).unwrap();
            assert_eq!(read, value, "{json}");
            assert_eq!(read.id(), value.id(), "{json}");
            assert_eq!(read.dtype().unwrap(), value.dtype().unwrap(), "{json}");

            let stream = value.into_value_bytes();
            let read = Scalar::decode_value_bytes(&stream).unwrap();
            assert_eq!(read, value);
            assert_eq!(read.dtype().unwrap(), value.dtype().unwrap(), "{value:?}");
        }
        // The plain leaves write their text or payload alone, as they always
        // did; a numbered one writes its number under `fixed`.
        assert_eq!(
            serde_json::to_string(&Scalar::from("ab")).unwrap(),
            r#"{"type":"string","value":"ab"}"#
        );
        assert_eq!(
            serde_json::to_string(&StringType::SizedUtf8String(32).scalar("ab").unwrap()).unwrap(),
            r#"{"type":"string","value":{"layout":"sized_utf8","fixed":32,"text":"ab"}}"#
        );
        assert_eq!(
            serde_json::to_string(&BytesType::SizedBinary(16).scalar(vec![1_u8]).unwrap()).unwrap(),
            r#"{"type":"bytes","value":{"layout":"sized_binary","fixed":16,"bytes":[1]}}"#
        );
    }

    #[test]
    fn a_value_from_outside_crosses_the_leaf_door() {
        // A sized value whose text is past its maximum, a fixed one past its
        // width and a US-ASCII one holding a scalar above 0x7F are refused
        // whether they arrive on the wire or in the value stream.
        for json in [
            r#"{"type":"string","value":{"layout":"sized_utf8","fixed":2,"text":"abc"}}"#,
            r#"{"type":"string","value":{"layout":"fixed_ascii","fixed":1,"text":"ab"}}"#,
            r#"{"type":"string","value":{"layout":"ascii","text":"é"}}"#,
            r#"{"type":"string","value":{"layout":"sized_utf8","text":"a"}}"#,
            r#"{"type":"bytes","value":{"layout":"sized_binary","fixed":1,"bytes":[1,2]}}"#,
            r#"{"type":"bytes","value":{"layout":"fixed_binary","bytes":[1]}}"#,
        ] {
            assert!(serde_json::from_str::<Scalar>(json).is_err(), "{json}");
        }
        let over = StringType::SizedUtf8String(8)
            .scalar("abc")
            .unwrap()
            .into_value_bytes();
        let mut tampered = over.clone();
        // The byte after the identifier is the number, as one LEB128 byte.
        tampered[2] = 2;
        assert!(Scalar::decode_value_bytes(&over).is_ok());
        assert!(Scalar::decode_value_bytes(&tampered).is_err());
    }

    #[test]
    fn a_stray_number_on_the_wire_reads_as_it_always_did() {
        let read = |json: &str| serde_json::from_str::<Scalar>(json);
        // A number beside a byte leaf that states none is not read.
        for layout in ["large_binary", "binary_view", "large_binary_view"] {
            let value = read(&format!(
                r#"{{"type":"bytes","value":{{"layout":"{layout}","fixed":4,"bytes":[1]}}}}"#
            ))
            .unwrap();
            assert_eq!(value.bytes_parameters().unwrap().as_str(), layout);
        }
        let plain =
            read(r#"{"type":"bytes","value":{"layout":"binary","fixed":4,"bytes":[1,2,3,4,5]}}"#)
                .unwrap();
        assert_eq!(plain.bytes_parameters(), Some(BytesType::Binary));
        // Beside a plain string it is a maximum the text is checked against,
        // and the value is the leaf the document names; the large and viewed
        // leaves refuse one.
        let text =
            read(r#"{"type":"string","value":{"layout":"utf8","fixed":8,"text":"abc"}}"#).unwrap();
        assert_eq!(text.string_parameters(), Some(StringType::Utf8String));
        assert!(
            read(r#"{"type":"string","value":{"layout":"utf8","fixed":2,"text":"abc"}}"#).is_err()
        );
        assert!(
            read(r#"{"type":"string","value":{"layout":"large_utf8","fixed":8,"text":"abc"}}"#)
                .is_err()
        );
        // A leaf that is its number still needs one.
        assert!(read(r#"{"type":"bytes","value":{"layout":"sized_binary","bytes":[1]}}"#).is_err());
    }

    #[test]
    fn values_of_sized_and_plain_leaves_infer_one_column() {
        let item = |values: [Scalar; 2]| {
            let DataType::Serie(item) = Scalar::from_sequence(values).dtype().unwrap() else {
                panic!("a run infers a serie");
            };
            item.dtype().clone()
        };
        let sized = |max: u32, text: &str| StringType::SizedUtf8String(max).scalar(text).unwrap();
        // A plain value beside a sized one widens to the plain leaf, in
        // either order; two maxima keep the larger.
        assert_eq!(
            item([sized(32, "abc"), Scalar::from("d")]),
            DataType::utf8()
        );
        assert_eq!(
            item([Scalar::from("d"), sized(32, "abc")]),
            DataType::utf8()
        );
        assert_eq!(
            item([sized(32, "abc"), sized(8, "d")]),
            DataType::sized_utf8(32).unwrap()
        );
        let bytes = BytesType::SizedBinary(16).scalar(vec![1_u8]).unwrap();
        assert_eq!(item([bytes, Scalar::from(vec![2_u8])]), DataType::binary());
    }

    #[test]
    fn a_canonical_value_holds_its_columns_exact_leaf() {
        let sized = DataType::sized_utf8(8).unwrap();
        // A plain value is restated under the column's leaf, number included.
        let landed = sized.scalar(Scalar::from("abc")).unwrap();
        assert_eq!(
            landed.string_parameters(),
            Some(StringType::SizedUtf8String(8))
        );
        // A value of the same leaf under another number is restated too, and
        // judged by the column's number, never its own.
        let wider = StringType::SizedUtf8String(32)
            .scalar("abcdefghij")
            .unwrap();
        assert!(sized.scalar(wider).is_err());
        let narrower = StringType::SizedUtf8String(4).scalar("abc").unwrap();
        assert_eq!(
            sized.scalar(narrower).unwrap().string_parameters(),
            Some(StringType::SizedUtf8String(8))
        );
        // A hand-built variant is judged the same way.
        let forged = Scalar::FixedAsciiString(Str::new("é"), 4);
        assert!(DataType::fixed_ascii(4).unwrap().scalar(forged).is_err());
        let padded = Scalar::FixedUtf8String(Str::new("ab\0\0"), 4);
        assert_eq!(
            DataType::fixed_utf8(4).unwrap().scalar(padded).unwrap(),
            Scalar::FixedUtf8String(Str::new("ab"), 4)
        );
        assert_eq!(
            DataType::fixed_utf8(4)
                .unwrap()
                .scalar(Scalar::FixedUtf8String(Str::new("ab\0\0"), 4))
                .unwrap()
                .as_str(),
            Some("ab"),
            "the padding is the slot's, not the value's"
        );
    }

    #[test]
    fn leaves_order_as_their_view_and_equal_only_themselves() {
        let dtypes: Vec<DataType> = strings()
            .into_iter()
            .map(DataType::from)
            .chain(bytes().into_iter().map(DataType::from))
            .collect();
        for left in &dtypes {
            for right in &dtypes {
                assert_eq!(
                    left.cmp(right) == Ordering::Equal,
                    left == right,
                    "{left} {right}"
                );
                if let (Some(l), Some(r)) = (left.string_parameters(), right.string_parameters()) {
                    assert_eq!(left.cmp(right), l.cmp(&r), "{left} {right}");
                }
                if let (Some(l), Some(r)) = (left.bytes_parameters(), right.bytes_parameters()) {
                    assert_eq!(left.cmp(right), l.cmp(&r), "{left} {right}");
                }
                let named = |dtype: &DataType| Field::new("x", dtype.clone(), true);
                assert_eq!(named(left) == named(right), left == right, "{left} {right}");
            }
        }
    }
}
