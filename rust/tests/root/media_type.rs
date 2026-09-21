//! `rust/src/media_type.rs`: the `type/subtype; charset=...` value a record
//! layer routes on, and the attributes it accepts.

mod codecs {

    use yggdryl::charset::Charset;

    use yggdryl::MediaType;

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

        let both =
            MediaType::from_str("text/csv;charset=latin1;encodings=application/gzip").unwrap();
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
            MediaType::from_content_headers(Some("text/csv; charset=\"utf-8\""), Some("gzip"))
                .unwrap();
        assert_eq!(encoded.charset(), Some(Charset::Utf8));
        assert_eq!(encoded.encodings().len(), 1);

        let bare = MediaType::from_content_headers(Some("text/csv"), None).unwrap();
        assert_eq!(bare.charset(), None);
        // Declaring nothing reads as UTF-8 without recording a declaration.
        assert_eq!(Charset::from_media_type(&bare), Charset::Utf8);

        assert!(MediaType::from_content_headers(Some("text/csv; charset=nope"), None).is_err());
    }
}

mod vocabulary {
    use std::collections::{BTreeSet, HashSet};

    use yggdryl::{Format, MediaType, MimeType};

    #[test]
    fn content_type_parameters_are_validated_without_becoming_mime_state() {
        for source in [
            "application/json",
            " application/json ; charset=utf-8 ",
            "application/json;charset=\"utf-8\"",
            "application/json; profile=\"https://example.test/a;b\\\"c\"; version=1",
            "application/vnd.acme+json; a=1; b=2; c=3; d=4; e=5; f=6; g=7; h=8; i=9; j=10",
        ] {
            let mime = MimeType::from_content_type(source).unwrap();
            assert_eq!(mime.format(), Some(Format::Json));
        }

        for source in [
            "json; charset=utf-8",
            "application/json;",
            "application/json; charset",
            "application/json; charset=",
            "application/json; charset=\"unterminated",
            "application/json; charset=utf-8 junk",
            "application/json; charset=utf-8; CHARSET=ascii",
            "application/json; a=1; b=2; c=3; d=4; e=5; f=6; g=7; h=8; i=9; A=10",
        ] {
            assert!(MimeType::from_content_type(source).is_err(), "{source:?}");
        }
    }

    #[test]
    fn content_coding_conversion_is_strict_and_distinguishes_file_only_encodings() {
        for (source, expected, canonical) in [
            ("gzip", MimeType::GZIP, "gzip"),
            ("br", MimeType::BROTLI, "br"),
            ("deflate", MimeType::ZLIB, "deflate"),
            ("compress", MimeType::COMPRESS, "compress"),
            ("zstd", MimeType::ZSTD, "zstd"),
        ] {
            let mime = MimeType::from_content_coding(source).unwrap();
            assert_eq!(mime, expected);
            assert_eq!(mime.content_coding(), Some(canonical));
            assert!(mime.is_encoding());
        }

        for source in ["identity", "bzip2", "xz", "lz4", "snappy", "unknown", ""] {
            assert!(MimeType::from_content_coding(source).is_err(), "{source:?}");
        }
        for file_only in [
            MimeType::BZIP2,
            MimeType::XZ,
            MimeType::LZ4,
            MimeType::SNAPPY,
        ] {
            assert!(file_only.is_encoding());
            assert_eq!(file_only.content_coding(), None);
        }
    }

    #[test]
    fn media_construction_and_mutation_validate_before_changing_state() {
        assert!(MediaType::from_parts(MimeType::CSV, [MimeType::ZIP]).is_err());

        let mut media = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap();
        let before = media.clone();
        assert!(
            media
                .set_encodings([MimeType::ZSTD, MimeType::ZIP])
                .is_err()
        );
        assert_eq!(media, before);

        media.push_encoding(MimeType::ZSTD).unwrap();
        assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);
        assert!(media.push_encoding(MimeType::TAR).is_err());
        assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);
        assert!(media.clear_encodings());
        assert!(!media.clear_encodings());
        media.set_base(MimeType::JSON);
        assert_eq!(media.base(), &MimeType::JSON);
    }

    #[test]
    fn media_display_parse_and_structural_serde_are_lossless() {
        let media = MediaType::from_parts(
            MimeType::CSV,
            [MimeType::GZIP, MimeType::ZSTD, MimeType::GZIP],
        )
        .unwrap();
        let canonical = "text/csv;encodings=application/gzip,application/zstd,application/gzip";
        assert_eq!(media.to_string(), canonical);
        assert_eq!(MediaType::from_str(canonical).unwrap(), media);

        let json = serde_json::to_string(&media).unwrap();
        assert_eq!(
            json,
            r#"{"base":"text/csv","encodings":["application/gzip","application/zstd","application/gzip"]}"#
        );
        assert_eq!(serde_json::from_str::<MediaType>(&json).unwrap(), media);
        assert!(
            serde_json::from_str::<MediaType>(
                r#"{"base":"text/csv","encodings":["application/zip"]}"#
            )
            .is_err()
        );

        assert_eq!(
            serde_json::from_str::<MimeType>(r#""APPLICATION/JSON""#).unwrap(),
            MimeType::JSON
        );
    }

    #[test]
    fn content_headers_build_media_and_reject_invalid_lists() {
        let media = MediaType::from_content_headers(
            Some("text/csv; charset=\"utf-8\""),
            Some("gzip, zstd"),
        )
        .unwrap();
        assert_eq!(media.base(), &MimeType::CSV);
        assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);

        let defaulted = MediaType::from_content_headers(None, None).unwrap();
        assert_eq!(defaulted, MediaType::default());
        for coding in [
            Some(""),
            Some("identity"),
            Some("gzip,"),
            Some("gzip,bzip2"),
        ] {
            assert!(MediaType::from_content_headers(None, coding).is_err());
        }
    }

    #[test]
    fn mime_and_media_values_have_total_collection_semantics() {
        let ordered = BTreeSet::from([
            MimeType::JSON,
            MimeType::CSV,
            MimeType::from_str("application/vnd.acme+json").unwrap(),
        ]);
        assert_eq!(ordered.len(), 3);
        let hashed = HashSet::from([
            MimeType::from_str("APPLICATION/JSON").unwrap(),
            MimeType::JSON,
        ]);
        assert_eq!(hashed.len(), 1);

        let media = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap();
        assert_eq!((&media).into_iter().collect::<Vec<_>>(), [&MimeType::GZIP]);
        assert_eq!(media.encoding_len(), 1);
        assert_eq!(media.get_encoding(0), Some(&MimeType::GZIP));
        assert_eq!(media.get_encoding(1), None);
        assert_eq!(media.stable_hash(), media.clone().stable_hash());
    }
}
