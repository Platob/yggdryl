//! `rust/src/object/azure/sign.rs`: the Shared Key signature no caller can
//! name.
//!
//! Azure signs a fixed list of headers in a fixed, non-alphabetical order, then
//! every `x-ms-` header sorted, then the resource the request addresses. The
//! order is load-bearing, so the document itself is pinned rather than only the
//! header that comes out of it.

use std::time::{Duration, UNIX_EPOCH};

use yggdryl::internals::object_azure_sign::{SharedKey, http_date};

fn key() -> SharedKey {
    // Azurite's published development key, which is not a secret.
    SharedKey::new(
        "devstoreaccount1",
        "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==",
    )
    .expect("a base64 key")
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn a_date_renders_the_way_a_header_spells_one() {
    assert_eq!(http_date(UNIX_EPOCH), "Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(
        http_date(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        "Tue, 14 Nov 2023 22:13:20 GMT"
    );
}

#[test]
fn the_signed_document_keeps_the_order_the_service_verifies() {
    let document = key().string_to_sign(
        "GET",
        "/lake/part.parquet",
        &pairs(&[("comp", "list"), ("restype", "container")]),
        &pairs(&[
            ("x-ms-version", "2025-05-05"),
            ("x-ms-date", "Thu, 01 Jan 1970 00:00:00 GMT"),
            ("content-length", "0"),
        ]),
    );
    assert_eq!(
        document,
        "GET\n\n\n\n\n\n\n\n\n\n\n\n\
         x-ms-date:Thu, 01 Jan 1970 00:00:00 GMT\nx-ms-version:2025-05-05\n\
         /devstoreaccount1/lake/part.parquet\ncomp:list\nrestype:container"
    );
}

#[test]
fn a_zero_length_body_signs_as_nothing_and_a_real_one_signs_as_its_length() {
    let signed = |length: &str| {
        key().string_to_sign(
            "PUT",
            "/lake/part.bin",
            &[],
            &pairs(&[("content-length", length), ("x-ms-date", "d")]),
        )
    };
    assert!(signed("0").starts_with("PUT\n\n\n\n"), "{}", signed("0"));
    assert!(
        signed("512").starts_with("PUT\n\n\n512\n"),
        "{}",
        signed("512")
    );
}

#[test]
fn every_x_ms_header_is_signed_sorted_and_whitespace_collapsed() {
    let document = key().string_to_sign(
        "PUT",
        "/lake/part.bin",
        &[],
        &pairs(&[
            ("x-ms-meta-desk", "  power   trading "),
            ("x-ms-blob-type", "BlockBlob"),
            ("x-ms-date", "d"),
            ("authorization", "should not be signed"),
        ]),
    );
    let block = document
        .split_once("x-ms-blob-type")
        .expect("the first x-ms header")
        .1;
    assert!(
        block.starts_with(":BlockBlob\nx-ms-date:d\nx-ms-meta-desk:power trading\n"),
        "{document}"
    );
    assert!(!document.contains("should not be signed"));
}

#[test]
fn the_signature_is_a_header_and_the_key_is_never_in_it() {
    let headers = key().sign("GET", "/lake", &[], &[], UNIX_EPOCH);
    let authorization = headers
        .iter()
        .find(|(name, _)| name == "authorization")
        .expect("an authorization header");
    assert!(authorization.1.starts_with("SharedKey devstoreaccount1:"));
    assert!(!authorization.1.contains("Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1"));
}

#[test]
fn a_repeated_query_name_contributes_its_values_sorted() {
    let resource = key().canonical_resource(
        "/lake",
        &pairs(&[("include", "snapshots"), ("include", "metadata")]),
    );
    assert_eq!(
        resource,
        "/devstoreaccount1/lake\ninclude:metadata,snapshots"
    );
}
