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
