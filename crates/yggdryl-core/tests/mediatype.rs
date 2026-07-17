//! Functional tests for [`MediaType`](yggdryl_core::mediatype::MediaType) — the ordered list
//! of mime types built from a mime-list string or a file's extensions.

use yggdryl_core::io::{IoError, Serializable};
use yggdryl_core::mediatype::MediaType;
use yggdryl_core::mimetype::MimeType;

#[test]
fn parse_str_reads_a_comma_separated_mime_list() {
    let m = MediaType::parse_str("application/json, text/html ;q=0.9 , ").unwrap();
    assert_eq!(m.len(), 2); // the trailing empty item is skipped
    assert_eq!(m.essences(), vec!["application/json", "text/html"]);
    assert_eq!(m.primary().unwrap().essence(), "application/json");
    assert!(m.contains("TEXT/HTML")); // case-insensitive
    assert!(!m.contains("image/png"));

    // Display is the inverse of parse (comma-joined essences).
    assert_eq!(m.to_string(), "application/json, text/html");

    // One malformed item fails the whole parse with a guided error.
    assert!(matches!(
        MediaType::parse_str("application/json, bogus").unwrap_err(),
        IoError::UnknownName {
            kind: "MimeType",
            ..
        }
    ));
}

#[test]
fn from_extensions_layers_the_type_stack() {
    // archive.tar.gz -> the content type then its wrapping encoding.
    let tgz = MediaType::from_extensions(["tar", "gz"]);
    assert_eq!(
        tgz.essences(),
        vec!["application/x-tar", "application/gzip"]
    );
    assert_eq!(tgz.primary().unwrap().essence(), "application/x-tar");

    // A single known extension is a one-element media type.
    assert_eq!(
        MediaType::from_extensions(["json"]).essences(),
        vec!["application/json"]
    );

    // Unknown extensions are skipped, so an all-unknown name yields an empty media type.
    let empty = MediaType::from_extensions(["xyz", "qqq"]);
    assert!(empty.is_empty() && empty.primary().is_none());
}

#[test]
fn builders_and_value_contract() {
    let m = MediaType::of(MimeType::parse_str("text/plain").unwrap())
        .with(MimeType::from_extension("gz").unwrap());
    assert_eq!(m.essences(), vec!["text/plain", "application/gzip"]);

    // Byte codec round-trips through the comma-joined essences.
    let bytes = m.serialize_bytes();
    assert_eq!(bytes, b"text/plain, application/gzip");
    assert_eq!(
        MediaType::deserialize_bytes(&bytes).unwrap().essences(),
        m.essences()
    );

    // Equatable + hashable.
    use std::collections::HashSet;
    let a = MediaType::parse_str("application/json").unwrap();
    let b = MediaType::parse_str("application/json").unwrap();
    assert_eq!(a, b);
    let set: HashSet<MediaType> = [a, b].into_iter().collect();
    assert_eq!(set.len(), 1);
}
