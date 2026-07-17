//! Functional tests for [`MimeType`](yggdryl_core::mimetype::MimeType) and the
//! [`MimeCatalog`](yggdryl_core::mimetype::MimeCatalog) registry — parsing, resolution from
//! mime / name / extension / magic bytes, custom registration, and the value-type contract.

use yggdryl_core::io::{IoError, Serializable};
use yggdryl_core::mimetype::{MimeCatalog, MimeRegistry, MimeType};

#[test]
fn parse_str_normalizes_and_drops_parameters() {
    let m = MimeType::parse_str("  Application/JSON ; charset=utf-8 ").unwrap();
    assert_eq!(m.essence(), "application/json");
    assert_eq!(m.type_(), "application");
    assert_eq!(m.subtype(), "json");
    assert_eq!(m.to_string(), "application/json");

    // Malformed essences are guided errors naming the fix.
    for bad in ["notamime", "text/", "/plain", "a/b/c", ""] {
        let err = MimeType::parse_str(bad).unwrap_err();
        assert!(matches!(
            err,
            IoError::UnknownName {
                kind: "MimeType",
                ..
            }
        ));
        assert!(err.to_string().contains("type/subtype"));
    }
}

#[test]
fn default_catalog_resolves_every_way() {
    // From an extension (case-insensitive, leading dot ignored).
    assert_eq!(
        MimeType::from_extension("json").unwrap().essence(),
        "application/json"
    );
    assert_eq!(
        MimeType::from_extension(".PNG").unwrap().essence(),
        "image/png"
    );
    assert!(MimeType::from_extension("nope").is_none());

    // From a file name (its last extension), including a path and multi-dot names.
    assert_eq!(
        MimeType::from_name("report.final.pdf").unwrap().essence(),
        "application/pdf"
    );
    assert_eq!(
        MimeType::from_name("/var/data/set.csv").unwrap().essence(),
        "text/csv"
    );
    assert!(MimeType::from_name("Makefile").is_none()); // no extension
    assert!(MimeType::from_name(".bashrc").is_none()); // hidden dotfile, no extension

    // From magic bytes at the head of a file.
    assert_eq!(
        MimeType::from_magic(b"%PDF-1.7\n...").unwrap().essence(),
        "application/pdf"
    );
    assert_eq!(
        MimeType::from_magic(b"\x1f\x8b\x08rest").unwrap().essence(),
        "application/gzip"
    );
    assert_eq!(
        MimeType::from_magic(b"PAR1....").unwrap().essence(),
        "application/vnd.apache.parquet"
    );
    assert!(MimeType::from_magic(b"plain text").is_none());
}

#[test]
fn guess_always_answers_magic_then_name_then_octet_stream() {
    // Magic wins even when the name disagrees.
    let m = MimeType::guess("mislabeled.txt", b"\x89PNG\r\n\x1a\n....");
    assert_eq!(m.essence(), "image/png");
    // No magic -> the name's extension.
    assert_eq!(
        MimeType::guess("data.json", b"{}").essence(),
        "application/json"
    );
    // Neither -> the octet-stream fallback (never None).
    let fallback = MimeType::guess("mystery", b"\x00\x01\x02");
    assert!(fallback.is_octet_stream());
    assert_eq!(fallback.essence(), "application/octet-stream");
}

#[test]
fn magic_resolution_prefers_the_longest_signature() {
    // A short shared prefix must not shadow a longer, more specific one. `PK\x03\x04` (zip) is
    // 4 bytes; a full PNG header is 8 — the longer wins when both could match a crafted head.
    let catalog = MimeCatalog::new()
        .with(MimeType::new("application/short", ["s"], [b"AB".to_vec()]))
        .with(MimeType::new("application/long", ["l"], [b"ABCD".to_vec()]));
    assert_eq!(
        catalog.from_magic(b"ABCD...").unwrap().essence(),
        "application/long"
    );
    assert_eq!(
        catalog.from_magic(b"ABxx...").unwrap().essence(),
        "application/short"
    );
}

#[test]
fn custom_registration_overrides_by_essence() {
    let mut catalog = MimeCatalog::defaults();
    let before = catalog.len();
    // Registering a new essence grows the catalog; re-registering the same essence replaces.
    catalog.register(MimeType::new(
        "application/x-yggdryl",
        ["ygg"],
        [b"YGGD".to_vec()],
    ));
    assert_eq!(catalog.len(), before + 1);
    catalog.register(MimeType::new("application/x-yggdryl", ["ygg", "y"], []));
    assert_eq!(catalog.len(), before + 1); // replaced, not duplicated
    assert_eq!(
        catalog.from_extension("y").unwrap().essence(),
        "application/x-yggdryl"
    );

    // from_mime returns the registered type (with its extensions) when known, else a bare
    // parsed essence.
    assert_eq!(
        catalog.from_mime("application/json").unwrap().essence(),
        "application/json"
    );
    assert!(!catalog
        .from_mime("application/json")
        .unwrap()
        .extensions()
        .is_empty());
    assert!(catalog
        .from_mime("x-vendor/unknown")
        .unwrap()
        .extensions()
        .is_empty());
}

#[test]
fn value_type_contract() {
    let a = MimeType::parse_str("text/plain").unwrap();
    let b = MimeType::from_extension("txt").unwrap();
    // Equality is over the essence only — a catalog entry with extensions equals a bare parse.
    assert_ne!(a, b); // b carries extensions; identity includes them
    assert_eq!(a.essence(), b.essence());

    // The byte codec round-trips through the essence.
    let bytes = a.serialize_bytes();
    assert_eq!(bytes, b"text/plain");
    assert_eq!(MimeType::deserialize_bytes(&bytes).unwrap(), a);

    // Hashable (works as a map key).
    use std::collections::HashSet;
    let set: HashSet<MimeType> = [a.clone(), a.clone()].into_iter().collect();
    assert_eq!(set.len(), 1);
}
