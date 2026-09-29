//! `rust/src/media/magic.rs`: what a payload says it is, read from its bytes.
//!
//! Filename inference answers what a location claims to be; these signatures
//! answer what the bytes are, codings peeled in application order. Every door
//! here is public, so the whole file reaches the crate through `yggdryl::`.

use yggdryl::media::{IORecordOptions, MAGIC_PROBE_LEN};
use yggdryl::{DataType, IOBase, IOMedia, MediaType, MimeType, Scalar, StructType, Url};
use yggdryl::{gzip, zstd};

/// The bytes of a one-sheet `.xlsx` package the crate writes.
fn workbook_bytes() -> Vec<u8> {
    let field =
        DataType::from(StructType::from_fields([DataType::Int64.required_field("id")]).unwrap())
            .required_field("row");
    let mut handle = yggdryl::holder::Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options().unwrap().with_field(field);
    handle
        .overwrite_records([Scalar::from_sequence([Scalar::from(1_i64)])], &options)
        .unwrap();
    handle.read_all_bytes().unwrap()
}

#[test]
fn container_signatures_are_identified() {
    assert_eq!(
        MimeType::from_magic_bytes(b"PAR1data"),
        Some(MimeType::PARQUET)
    );
    assert_eq!(
        MimeType::from_magic_bytes(b"ARROW1\0\0"),
        Some(MimeType::ARROW_FILE)
    );
    assert_eq!(
        MimeType::from_magic_bytes(b"Obj\x01rest"),
        Some(MimeType::AVRO)
    );
    assert_eq!(MimeType::from_magic_bytes(b"ORCrest"), Some(MimeType::ORC));
    assert_eq!(
        MimeType::from_magic_bytes(b"PFA1rest"),
        Some(MimeType::PUFFIN)
    );
    assert_eq!(
        MimeType::from_magic_bytes(b"SQLite format 3\0rest"),
        Some(MimeType::SQLITE3)
    );
    assert_eq!(MimeType::from_magic_bytes(b"%PDF-1.7"), Some(MimeType::PDF));
}

#[test]
fn a_workbook_is_named_by_its_suffix_and_not_by_its_bytes() {
    let bytes = workbook_bytes();
    assert_eq!(bytes[..4], *b"PK\x03\x04");

    // measured: the signature table carries no ZIP entry, so a package's
    // `PK` header names nothing - neither ZIP nor the workbook inside it.
    assert_eq!(MimeType::from_magic_bytes(&bytes), None);
    assert_eq!(MimeType::from_bytes(&bytes), None);
    assert_eq!(MediaType::from_magic_bytes(&bytes), None);

    // The name is what says workbook, and a named handle keeps it.
    assert_eq!(
        Url::from_str("file:///t.xlsx").unwrap().media_type().base(),
        &MimeType::XLSX
    );
    assert_eq!(
        Url::from_str("file:///t.xls").unwrap().media_type().base(),
        &MimeType::XLS
    );

    // The legacy BIFF workbook is the one spreadsheet its bytes name: the
    // OLE compound-file header.
    assert_eq!(
        MimeType::from_magic_bytes(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00]),
        Some(MimeType::XLS)
    );
}

#[test]
fn offset_signatures_respect_their_offset() {
    // `WEBP` lives at byte 8, after the RIFF header.
    let mut webp = Vec::from(*b"RIFF\0\0\0\0WEBPVP8 ");
    assert_eq!(MimeType::from_magic_bytes(&webp), Some(MimeType::WEBP));

    // The same token at offset 0 must not match.
    webp = Vec::from(*b"WEBPRIFF\0\0\0\0");
    assert_ne!(MimeType::from_magic_bytes(&webp), Some(MimeType::WEBP));
}

#[test]
fn coding_signatures_are_identified() {
    let gzipped = gzip::dump(b"hello").unwrap();
    assert_eq!(MimeType::from_magic_bytes(&gzipped), Some(MimeType::GZIP));

    let zstd_payload = zstd::dump(b"hello").unwrap();
    assert_eq!(
        MimeType::from_magic_bytes(&zstd_payload),
        Some(MimeType::ZSTD)
    );

    let zlib_payload = yggdryl::zlib::dump(b"hello").unwrap();
    assert_eq!(
        MimeType::from_magic_bytes(&zlib_payload),
        Some(MimeType::ZLIB)
    );
}

#[test]
fn textual_formats_are_sniffed_structurally() {
    assert_eq!(
        MimeType::from_text_bytes(b"  {\"a\":1}"),
        Some(MimeType::JSON)
    );
    assert_eq!(MimeType::from_text_bytes(b"[1,2,3]"), Some(MimeType::JSON));
    assert_eq!(
        MimeType::from_text_bytes(b"<?xml version=\"1.0\"?>"),
        Some(MimeType::XML)
    );
    assert_eq!(
        MimeType::from_text_bytes(b"<svg xmlns="),
        Some(MimeType::SVG)
    );
    assert_eq!(
        MimeType::from_text_bytes(b"---\nkey: value"),
        Some(MimeType::YAML)
    );

    // A bare key/value line is ambiguous between YAML and TOML.
    assert_eq!(MimeType::from_text_bytes(b"key = 1"), None);
    assert_eq!(MimeType::from_text_bytes(b"plain prose"), None);
}

#[test]
fn a_byte_order_mark_does_not_hide_the_content() {
    let mut input = Vec::from("\u{feff}".as_bytes());
    input.extend_from_slice(b"{\"a\":1}");
    assert_eq!(MimeType::from_text_bytes(&input), Some(MimeType::JSON));
}

#[test]
fn nested_codings_are_peeled_in_application_order() {
    let json = br#"{"symbol":"AAPL"}"#;
    let gzipped = gzip::dump(json).unwrap();

    let media = MediaType::from_magic_bytes(&gzipped).expect("gzip of json");
    assert_eq!(media.base(), &MimeType::JSON);
    assert_eq!(media.encodings(), &[MimeType::GZIP]);

    // Two layers: gzip over zstd over JSON.
    let doubled = gzip::dump(&zstd::dump(json).unwrap()).unwrap();
    let media = MediaType::from_magic_bytes(&doubled).expect("gzip of zstd of json");
    assert_eq!(media.base(), &MimeType::JSON);
    assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);
}

#[test]
fn a_coding_over_an_unknown_payload_reports_the_coding() {
    let opaque = gzip::dump(b"plain prose with no signature").unwrap();
    let media = MediaType::from_magic_bytes(&opaque).expect("gzip of prose");
    assert_eq!(media.base(), &MimeType::GZIP);
}

#[test]
fn nothing_is_inferred_from_an_empty_or_unknown_payload() {
    assert_eq!(MimeType::from_magic_bytes(b""), None);
    assert_eq!(MimeType::from_bytes(b""), None);
    assert_eq!(MimeType::from_bytes(&[0xAB, 0xCD, 0xEF]), None);
    assert_eq!(MediaType::from_magic_bytes(b""), None);
}

#[test]
fn only_the_probe_window_is_examined() {
    // A signature past the probe window must not be found.
    let mut buried = vec![0_u8; MAGIC_PROBE_LEN * 2];
    buried.extend_from_slice(b"PAR1");
    assert_eq!(MimeType::from_magic_bytes(&buried), None);
}
