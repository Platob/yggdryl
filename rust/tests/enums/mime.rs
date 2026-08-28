use std::collections::{BTreeSet, HashSet};
use std::path::Path;

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
    let encoded_directory = MediaType::from_parts(MimeType::DIRECTORY, [MimeType::GZIP]).unwrap();
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
fn extension_inference_covers_data_documents_and_media() {
    for (extension, expected) in [
        (".JSON", MimeType::JSON),
        ("ndjson", MimeType::JSON_LINES),
        ("yml", MimeType::YAML),
        ("csv", MimeType::CSV),
        ("parquet", MimeType::PARQUET),
        ("puffin", MimeType::PUFFIN),
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
fn category_helpers_cover_known_and_structured_suffix_values() {
    assert!(MimeType::CSV.is_tabular());
    assert!(MimeType::XLSX.is_tabular());
    assert!(MimeType::PARQUET.is_binary());
    assert!(MimeType::PUFFIN.is_binary());
    assert!(MimeType::PUFFIN.is_structured());
    assert!(!MimeType::PUFFIN.is_tabular());
    assert!(MimeType::JSON.is_textual());
    assert!(MimeType::JSON.is_structured());
    assert!(!MimeType::JSON.is_binary());
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
    assert!(vendor_xml.is_structured());
}

#[test]
fn content_coding_conversion_is_strict_and_distinguishes_file_only_encodings() {
    for (source, expected, canonical) in [
        ("gzip", MimeType::GZIP, "gzip"),
        ("X-GZIP", MimeType::GZIP, "gzip"),
        ("br", MimeType::BROTLI, "br"),
        ("deflate", MimeType::ZLIB, "deflate"),
        ("compress", MimeType::COMPRESS, "compress"),
        ("x-compress", MimeType::COMPRESS, "compress"),
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
        serde_json::from_str::<MediaType>(r#"{"base":"text/csv","encodings":["application/zip"]}"#)
            .is_err()
    );

    assert_eq!(
        serde_json::from_str::<MimeType>(r#""APPLICATION/JSON""#).unwrap(),
        MimeType::JSON
    );
}

#[test]
fn content_headers_build_media_and_reject_invalid_lists() {
    let media =
        MediaType::from_content_headers(Some("text/csv; charset=\"utf-8\""), Some("gzip, zstd"))
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
fn format_and_mime_tables_are_bidirectional_without_alias_drift() {
    for (format, mime) in [
        (Format::Json, MimeType::JSON),
        (Format::JsonLines, MimeType::JSON_LINES),
        (Format::Yaml, MimeType::YAML),
        (Format::Toml, MimeType::TOML),
    ] {
        assert_eq!(format.mime_type(), mime);
        assert_eq!(mime.format(), Some(format));
        assert_eq!(Format::from_str(mime.as_str()).unwrap(), format);
    }
    for (alias, format) in [
        ("json", Format::Json),
        ("jsonl", Format::JsonLines),
        ("ndjson", Format::JsonLines),
        ("json_lines", Format::JsonLines),
        ("json-lines", Format::JsonLines),
        ("yaml", Format::Yaml),
        ("yml", Format::Yaml),
        ("application/x-yaml", Format::Yaml),
        ("toml", Format::Toml),
    ] {
        assert_eq!(Format::from_str(alias).unwrap(), format, "{alias:?}");
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
