//! `rust/src/mime_type.rs`.

mod mime {

    use yggdryl::{Error, Format, MediaType, MimeType};

    #[test]
    fn known_and_custom_mime_names_are_canonical_and_round_trip() {
        for (source, expected, canonical) in [
            ("APPLICATION/JSON", MimeType::JSON, "application/json"),
            ("text/CSV", MimeType::CSV, "text/csv"),
            (
                "Application/Vnd.Apache.Parquet",
                MimeType::PARQUET,
                "application/vnd.apache.parquet",
            ),
            (
                "Application/Vnd.Apache.Puffin",
                MimeType::PUFFIN,
                "application/vnd.apache.puffin",
            ),
            ("TEXT/ULLINK", MimeType::ULLINK, "text/ullink"),
            ("TEXT/FIX", MimeType::FIX, "text/fix"),
            ("TEXT/FIXUL", MimeType::FIXUL, "text/fixul"),
            ("TEXT/FIXML", MimeType::FIXML, "text/fixml"),
            ("Message/HTTP", MimeType::HTTP, "message/http"),
            ("IMAGE/JPEG", MimeType::JPEG, "image/jpeg"),
            (
                "Acme/X.Custom+JSON",
                MimeType::from_str("acme/x.custom+json").unwrap(),
                "acme/x.custom+json",
            ),
        ] {
            let mime = MimeType::from_str(source).unwrap();
            assert_eq!(mime, expected);
            assert_eq!(mime.as_str(), canonical);
            assert_eq!(mime.to_string().parse::<MimeType>().unwrap(), mime);
            assert_eq!(AsRef::<str>::as_ref(&mime), canonical);
        }

        assert!(MimeType::JSON.is_known());
        assert!(
            !MimeType::from_str("application/vnd.acme+json")
                .unwrap()
                .is_known()
        );
        assert_eq!(MimeType::default(), MimeType::OCTET_STREAM);
    }

    #[test]
    fn io_identity_is_derived_from_the_unencoded_mime_value() {
        let custom = MimeType::from_str("application/vnd.example.rows").unwrap();
        for mime in [MimeType::FILE, MimeType::CSV, custom] {
            assert!(mime.is_io(), "{mime} should describe an I/O value");
            assert!(MediaType::new(mime).is_io());
        }

        assert!(!MimeType::DIRECTORY.is_io());
        assert!(MimeType::DIRECTORY.is_directory());
        let encoded_directory =
            MediaType::from_parts(MimeType::DIRECTORY, [MimeType::GZIP]).unwrap();
        assert!(!encoded_directory.is_io());
    }

    #[test]
    fn mime_parser_rejects_invalid_restricted_names_with_byte_positions() {
        let long = format!("application/{}", "a".repeat(128));
        for (source, position) in [
            ("", 0),
            ("/json", 0),
            ("application/", 12),
            ("application//json", 12),
            ("application/json/more", 16),
            ("application/@json", 12),
            ("app lication/json", 3),
            ("application/jo%n", 14),
            (long.as_str(), 139),
        ] {
            let error = MimeType::from_str(source).unwrap_err();
            assert!(
                matches!(error, Error::Parse { position: actual, .. } if actual == position),
                "unexpected error for {source:?}: {error}"
            );
        }

        let error = MimeType::from_str("  unknown  ").unwrap_err();
        assert!(matches!(error, Error::Parse { position: 2, .. }));
    }

    #[test]
    fn category_helpers_cover_known_and_structured_suffix_values() {
        for mime in [
            MimeType::ULLINK,
            MimeType::FIX,
            MimeType::FIXUL,
            MimeType::FIXML,
        ] {
            assert!(mime.is_known());
            assert!(mime.is_textual());
            assert!(!mime.is_binary());
            // Each classifies a line rather than naming a file format, so none
            // answers a preferred extension.
            assert_eq!(mime.extension(), None);
        }
        // A bridge configuration is JSON and reads as JSON - it is named
        // `application/json` now, so it is the JSON row below that
        // pins it and no line type of its own; a FIX frame carrying XML in a tag
        // is not a document and does not read as one.
        assert_eq!(MimeType::FIXML.format(), None);
        assert!(!MimeType::FIXML.is_structured());
        assert!(MimeType::CSV.is_tabular());
        assert!(MimeType::XLSX.is_tabular());
        assert!(MimeType::PARQUET.is_binary());
        assert!(MimeType::PUFFIN.is_binary());
        assert!(MimeType::PUFFIN.is_structured());
        assert!(!MimeType::PUFFIN.is_tabular());
        // An HTTP message is its own family: the head is text and the body is
        // whatever the head says, so it is neither textual nor binary, and it
        // is no document.
        assert!(MimeType::HTTP.is_message());
        assert!(!MimeType::HTTP.is_text());
        assert!(!MimeType::HTTP.is_textual());
        assert!(!MimeType::HTTP.is_binary());
        assert!(!MimeType::HTTP.is_structured());
        assert!(!MimeType::HTTP.is_tabular());
        assert_eq!(MimeType::HTTP.top_level(), "message");
        assert_eq!(MimeType::HTTP.subtype(), "http");
        assert_eq!(MimeType::HTTP.extension(), Some("http"));
        assert_eq!(MimeType::HTTP.format(), None);
        assert!(MimeType::HTTP.is_known());
        assert!(MimeType::JSON.is_textual());
        assert!(MimeType::JSON.is_structured());
        assert!(!MimeType::JSON.is_binary());
        assert_eq!(MimeType::JSON.format(), Some(Format::Json));
        assert_eq!(MimeType::XML.format(), Some(Format::Xml));
        assert_eq!(MimeType::SVG.format(), Some(Format::Xml), "SVG is XML");
        assert!(MimeType::PNG.is_image());
        assert!(MimeType::MP3.is_audio());
        assert!(MimeType::MP4.is_video());
        assert!(MimeType::WOFF.is_font());
        assert!(MimeType::ZIP.is_archive());
        assert!(!MimeType::ZIP.is_encoding());
        assert!(MimeType::GZIP.is_encoding());

        let vendor_json = MimeType::from_str("application/vnd.acme.trade+json").unwrap();
        let vendor_xml = MimeType::from_str("application/vnd.acme.trade+xml").unwrap();
        assert_eq!(vendor_json.structured_suffix(), Some("json"));
        assert_eq!(vendor_json.extension(), Some("json"));
        assert_eq!(vendor_json.format(), Some(Format::Json));
        assert!(vendor_json.is_textual());
        assert!(vendor_json.is_structured());
        assert!(vendor_xml.is_textual());
        assert_eq!(vendor_xml.format(), Some(Format::Xml));
        assert!(vendor_xml.is_structured());
    }
}
