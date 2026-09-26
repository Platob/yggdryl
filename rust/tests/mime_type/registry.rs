//! `rust/src/mime_type/registry.rs`.

mod mime {

    use std::path::Path;
    use yggdryl::{Error, MediaType, MimeType};

    #[test]
    fn extension_inference_covers_data_documents_and_media() {
        for (extension, expected) in [
            (".JSON", MimeType::JSON),
            ("ndjson", MimeType::JSON_LINES),
            ("yml", MimeType::YAML),
            ("csv", MimeType::CSV),
            ("parquet", MimeType::PARQUET),
            ("puffin", MimeType::PUFFIN),
            ("http", MimeType::HTTP),
            (".HTTP", MimeType::HTTP),
            ("arrow", MimeType::ARROW_FILE),
            ("md", MimeType::MARKDOWN),
            ("css", MimeType::CSS),
            ("mjs", MimeType::JAVASCRIPT),
            ("png", MimeType::PNG),
            ("jpeg", MimeType::JPEG),
            ("webp", MimeType::WEBP),
            ("svg", MimeType::SVG),
            ("mp3", MimeType::MP3),
            ("wav", MimeType::WAV),
            ("ogg", MimeType::OGG),
            ("flac", MimeType::FLAC),
            ("mp4", MimeType::MP4),
            ("webm", MimeType::WEBM),
            ("woff2", MimeType::WOFF2),
            ("ttf", MimeType::TTF),
            ("xlsx", MimeType::XLSX),
            ("ods", MimeType::ODS),
            ("docx", MimeType::DOCX),
            ("pdf", MimeType::PDF),
            ("zip", MimeType::ZIP),
            ("7z", MimeType::SEVEN_ZIP),
            ("rar", MimeType::RAR),
            ("tar", MimeType::TAR),
            ("gz", MimeType::GZIP),
            ("zst", MimeType::ZSTD),
        ] {
            assert_eq!(MimeType::from_extension(extension).unwrap(), expected);
            assert_eq!(MimeType::from_str(extension).unwrap(), expected);
        }

        assert_eq!(
            MimeType::from_path(Path::new("folder/report.PARQUET")).unwrap(),
            MimeType::PARQUET
        );
        for source in ["", ".", "csv.gz", "unknown", "a/b"] {
            assert!(MimeType::from_extension(source).is_err(), "{source:?}");
        }
    }

    #[test]
    fn compound_filename_inference_preserves_encoding_application_order() {
        let encoded = MediaType::from_file_name("orders.CSV.GZ.ZST");
        assert_eq!(encoded.base(), &MimeType::CSV);
        assert_eq!(encoded.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);
        assert_eq!(encoded.encoding(), Some(&MimeType::ZSTD));
        assert_eq!(encoded.extension(), Some("zst"));
        assert_eq!(
            encoded.extensions().collect::<Vec<_>>(),
            ["csv", "gz", "zst"]
        );
        assert!(encoded.is_tabular());
        assert!(encoded.is_textual());
        assert!(encoded.is_binary());

        let unknown = MediaType::from_file_name("orders.csv.backup.gz");
        assert_eq!(unknown.base(), &MimeType::OCTET_STREAM);
        assert_eq!(unknown.encodings(), &[MimeType::GZIP]);

        let encoding_only = MediaType::from_file_name("data.gz");
        assert_eq!(encoding_only.base(), &MimeType::OCTET_STREAM);
        assert_eq!(encoding_only.encodings(), &[MimeType::GZIP]);

        let archive = MediaType::from_file_name("archive.zip");
        assert_eq!(archive.base(), &MimeType::ZIP);
        assert!(archive.encodings().is_empty());
        assert!(archive.is_archive());

        let twice = MediaType::from_extensions(["csv", "gz", "gz"]);
        assert_eq!(twice.encodings(), &[MimeType::GZIP, MimeType::GZIP]);
    }

    #[test]
    fn filename_edge_cases_and_compound_aliases_are_deterministic() {
        for file_name in [
            "",
            "README",
            ".env",
            "name.",
            ".config.local",
            "data.unknown",
        ] {
            assert_eq!(MediaType::from_file_name(file_name), MediaType::default());
        }
        assert_eq!(MediaType::from_str("README").unwrap(), MediaType::default());
        assert_eq!(
            MediaType::from_str("folder/orders.csv.gz").unwrap(),
            MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap()
        );
        assert_eq!(
            MediaType::from_str("application/vnd.example.report+json")
                .unwrap()
                .base()
                .as_str(),
            "application/vnd.example.report+json"
        );
        assert!(MediaType::from_str("application//json").is_err());
        let error = MediaType::from_str("  application//json  ").unwrap_err();
        assert!(matches!(error, Error::Parse { position: 14, .. }));
        let error = MediaType::from_str("  text/csv;encodings=application//gzip  ").unwrap_err();
        assert!(matches!(error, Error::Parse { position: 33, .. }));
        assert_eq!(
            MediaType::from_file_name(".orders.csv.gz"),
            MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap()
        );
        for (file_name, base, encoding) in [
            ("archive.tgz", MimeType::TAR, MimeType::GZIP),
            ("archive.tbz2", MimeType::TAR, MimeType::BZIP2),
            ("archive.txz", MimeType::TAR, MimeType::XZ),
            ("archive.tzst", MimeType::TAR, MimeType::ZSTD),
            ("drawing.svgz", MimeType::SVG, MimeType::GZIP),
        ] {
            let media = MediaType::from_file_name(file_name);
            assert_eq!(media.base(), &base);
            assert_eq!(media.encodings(), &[encoding]);
        }
        for (extension, base, encoding) in [
            (".tgz", MimeType::TAR, MimeType::GZIP),
            (" .TBZ2\t", MimeType::TAR, MimeType::BZIP2),
            (".txz", MimeType::TAR, MimeType::XZ),
            (".tzst", MimeType::TAR, MimeType::ZSTD),
            (".svgz", MimeType::SVG, MimeType::GZIP),
        ] {
            let media = MediaType::from_extension(extension);
            assert_eq!(media.base(), &base);
            assert_eq!(media.encodings(), &[encoding]);
        }
        assert_eq!(
            MediaType::from_path(Path::new("folder/orders.csv.gz")).unwrap(),
            MediaType::from_file_name("orders.csv.gz")
        );
    }
}
