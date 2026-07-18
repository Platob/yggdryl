//! Functional tests for [`Headers`](yggdryl_core::headers::Headers) — the project's one metadata
//! map: insertion order, ASCII-case-insensitive matching, multi-value entries, the byte codec
//! (with truncation errors), the HTTP text form, and value semantics.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl_core::headers::Headers;
use yggdryl_core::io::{IoError, Serializable};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn get_is_case_insensitive_and_first_wins() {
    let mut headers = Headers::new();
    headers.append("Content-Type", "text/plain");
    headers.append("content-type", "application/json");
    assert_eq!(headers.get("CONTENT-TYPE"), Some("text/plain")); // first occurrence
    assert_eq!(
        headers.get_all("Content-Type"),
        vec!["text/plain", "application/json"]
    );
    assert!(headers.contains("cOnTeNt-TyPe"));
    assert_eq!(headers.get("missing"), None);
}

#[test]
fn insert_replaces_append_keeps() {
    let mut headers = Headers::new();
    headers.append("Set-Cookie", "a=1");
    headers.append("Set-Cookie", "b=2");
    assert_eq!(headers.len(), 2);
    headers.insert("Set-Cookie", "only"); // replace semantics
    assert_eq!(headers.get_all("set-cookie"), vec!["only"]);
    assert_eq!(headers.len(), 1);
}

#[test]
fn remove_reports_count_and_clear_empties() {
    let mut headers = Headers::new().with("a", "1").with("b", "2");
    headers.append("a", "3");
    assert_eq!(headers.remove("a"), 2);
    assert_eq!(headers.remove("a"), 0);
    assert!(!headers.is_empty());
    headers.clear();
    assert!(headers.is_empty());
}

#[test]
fn bytes_surface_roundtrips_non_utf8() {
    let mut headers = Headers::new();
    headers.append_bytes(b"X-Raw", &[0xff, 0xfe]);
    assert_eq!(headers.get_bytes(b"x-raw"), Some(&[0xff, 0xfe][..]));
    assert_eq!(headers.get("X-Raw"), None); // not UTF-8 -> the &str view skips it
    assert_eq!(headers.get_all_bytes(b"x-raw").len(), 1);
}

#[test]
fn insertion_order_preserved_in_iter() {
    let headers = Headers::new().with("z", "1").with("a", "2").with("m", "3");
    let names: Vec<&[u8]> = headers.iter().map(|(name, _)| name).collect();
    assert_eq!(names, vec![&b"z"[..], &b"a"[..], &b"m"[..]]);
}

#[test]
fn typed_conveniences() {
    let mut headers = Headers::new();
    headers.insert(Headers::CONTENT_TYPE, "application/json");
    headers.insert(Headers::CONTENT_LENGTH, " 1024 ");
    assert_eq!(headers.content_type(), Some("application/json"));
    assert_eq!(headers.content_length(), Some(1024));
    headers.insert(Headers::CONTENT_LENGTH, "not-a-number");
    assert_eq!(headers.content_length(), None);
}

#[test]
fn merge_with_overlays_other() {
    let base = Headers::new().with("Keep", "1").with("Replace", "old");
    let mut patch = Headers::new();
    patch.append("Replace", "new1");
    patch.append("Replace", "new2");
    let merged = base.merge_with(&patch);
    assert_eq!(merged.get("Keep"), Some("1"));
    assert_eq!(merged.get_all("Replace"), vec!["new1", "new2"]);
}

#[test]
fn http_text_form_roundtrips() {
    let headers = Headers::new()
        .with("Host", "example.com")
        .with("Accept", "*/*");
    let wire = headers.to_http_bytes();
    assert_eq!(Headers::parse_http(&wire), headers);
    // Lenient parse: blank line stops, colon-less lines skipped.
    let partial = Headers::parse_http(b"A: 1\r\n\r\nB: ignored-after-blank\r\n");
    assert_eq!(partial.get("A"), Some("1"));
    assert!(!partial.contains("B"));
}

#[test]
fn byte_codec_roundtrips_arbitrary_bytes_and_order() {
    let mut headers = Headers::new();
    headers.append_bytes(b"bin", &[0, 1, 2, 0xff]);
    headers.append("Set-Cookie", "a=1");
    headers.append("Set-Cookie", "b=2");
    let decoded = Headers::deserialize_bytes(&headers.serialize_bytes()).unwrap();
    assert_eq!(decoded, headers);

    // Truncated frames error with the guided EOF, never panic.
    let bytes = headers.serialize_bytes();
    for cut in [0, 3, 5, bytes.len() - 1] {
        assert!(matches!(
            Headers::deserialize_bytes(&bytes[..cut]),
            Err(IoError::UnexpectedEof { .. })
        ));
    }
}

#[test]
fn serializable_trait_and_value_semantics() {
    // The trait impl is the same codec (generic round-trip), and equal maps hash equal.
    fn roundtrip<T: Serializable>(value: &T) -> Result<T, T::Error> {
        T::deserialize_bytes(&value.serialize_bytes())
    }
    let headers = Headers::new().with("a", "1").with("b", "2");
    assert_eq!(roundtrip(&headers).unwrap(), headers);
    assert_eq!(hash_of(&headers), hash_of(&headers.copy()));
    // Name case matters for equality of the stored form (matching is what is case-insensitive).
    let other = Headers::new().with("A", "1").with("b", "2");
    assert_ne!(headers, other);
}

// -------------------------------------------------------------------------------------
// Media type + mtime (the centralized Content-Type / Content-Encoding accessors)
// -------------------------------------------------------------------------------------

#[test]
fn media_type_centralizes_content_type_and_encoding() {
    use yggdryl_core::mediatype::MediaType;
    use yggdryl_core::mimetype::MimeType;

    let mut headers = Headers::new();
    // No Content-Type -> no media type declared.
    assert!(headers.mime_type().is_none());
    assert!(headers.media_type().is_none());

    // set_mime_type / mime_type round-trip through Content-Type (parameters dropped on read).
    headers.set_content_type("application/json; charset=utf-8");
    assert_eq!(headers.mime_type().unwrap().essence(), "application/json");
    assert_eq!(
        headers.content_type(),
        Some("application/json; charset=utf-8")
    );

    // media_type folds Content-Encoding into the layered stack.
    headers.set_content_type("application/x-tar");
    headers.set_content_encoding("gzip");
    assert_eq!(headers.content_encoding(), Some("gzip"));
    assert_eq!(
        headers.media_type().unwrap().essences(),
        vec!["application/x-tar", "application/gzip"]
    );

    // set_media_type writes the comma-joined essences back to Content-Type.
    let media = MediaType::of(MimeType::parse_str("text/html").unwrap())
        .with(MimeType::from_extension("gz").unwrap());
    headers.set_media_type(&media);
    assert_eq!(headers.content_type(), Some("text/html, application/gzip"));
    assert_eq!(headers.mime_type().unwrap().essence(), "text/html"); // primary
}

#[test]
fn mtime_epoch_micros_round_trips_including_negatives() {
    let mut headers = Headers::new();
    assert!(headers.mtime().is_none());

    for micros in [0i64, 1, 1_600_000_000_000_000, i64::MAX, -1, i64::MIN] {
        headers.set_mtime(micros);
        assert_eq!(headers.mtime(), Some(micros), "mtime {micros}");
    }
    // The stored form is a compact decimal under the MTIME header.
    headers.set_mtime(-42);
    assert_eq!(headers.get(Headers::MTIME), Some("-42"));
    // set_mtime replaces (single value), never appends.
    headers.set_mtime(7);
    assert_eq!(headers.get_all(Headers::MTIME), vec!["7"]);
}

#[test]
fn content_length_renders_replaces_and_trims() {
    let mut headers = Headers::new();
    assert!(headers.content_length().is_none());

    headers.set_content_length(0);
    assert_eq!(headers.get(Headers::CONTENT_LENGTH), Some("0"));
    assert_eq!(headers.content_length(), Some(0));

    // Replaces (single value), never appends, and renders the decimal directly.
    headers.set_content_length(4096);
    headers.set_content_length(1_048_576);
    assert_eq!(headers.get_all(Headers::CONTENT_LENGTH), vec!["1048576"]);
    assert_eq!(headers.content_length(), Some(1_048_576));

    // The reader trims surrounding whitespace and rejects a non-numeric value.
    headers.insert(Headers::CONTENT_LENGTH, "  512  ");
    assert_eq!(headers.content_length(), Some(512));
    headers.insert(Headers::CONTENT_LENGTH, "not-a-number");
    assert_eq!(headers.content_length(), None);

    // The full u64 range round-trips.
    headers.set_content_length(u64::MAX);
    assert_eq!(headers.content_length(), Some(u64::MAX));
}

#[test]
fn touch_mtime_stamps_a_recent_positive_epoch() {
    let mut headers = Headers::new();
    headers.touch_mtime();
    let stamped = headers.mtime().expect("touch_mtime sets the header");
    // A microsecond epoch after 2000-01-01 (946684800_000000) and non-negative.
    assert!(stamped > 946_684_800_000_000, "mtime {stamped} looks unset");
    // It replaces rather than appends.
    headers.touch_mtime();
    assert_eq!(headers.get_all(Headers::MTIME).len(), 1);
}

#[test]
fn lookup_is_correct_across_a_large_ordered_set() {
    // The ordered-vector layout must still find any key in a large set, at any position.
    let mut headers = Headers::new();
    for i in 0..64u32 {
        headers.insert(&format!("X-Header-{i}"), &format!("v{i}"));
    }
    assert_eq!(headers.len(), 64);
    assert_eq!(headers.get("x-header-0"), Some("v0")); // first
    assert_eq!(headers.get("X-HEADER-63"), Some("v63")); // last, case-folded
    assert_eq!(headers.get("x-header-64"), None); // absent
                                                  // Insertion order is preserved end to end.
    let first = headers.iter().next().unwrap();
    assert_eq!(first.0, b"X-Header-0");
}

#[test]
fn empty_value_is_distinct_from_absent() {
    let mut headers = Headers::new();
    headers.insert("X-Empty", "");
    assert_eq!(headers.get("x-empty"), Some("")); // present, empty
    assert!(headers.contains("X-Empty"));
    assert_eq!(headers.get("x-missing"), None); // absent
}
