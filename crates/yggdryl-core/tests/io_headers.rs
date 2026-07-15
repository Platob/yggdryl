//! Edge-case tests for [`Headers`] — the case-insensitive, ordered, multi-value byte-string
//! header map: str/bytes accessors, insert-vs-append semantics, HTTP text render/parse, and
//! the binary round-trip through the `IOCursor` abstraction.

use std::collections::HashSet;

use yggdryl_core::io::{Bytes, Headers, IOCursor};

#[test]
fn case_insensitive_get_and_contains() {
    let mut headers = Headers::new();
    headers.insert(Headers::CONTENT_TYPE, "application/json");
    assert_eq!(headers.get("content-type"), Some("application/json"));
    assert_eq!(headers.get("CONTENT-TYPE"), Some("application/json"));
    assert_eq!(headers.get("Content-Type"), Some("application/json"));
    assert!(headers.contains("content-TYPE"));
    assert_eq!(headers.get("missing"), None);
    assert!(!headers.contains("missing"));
    assert_eq!(headers.len(), 1);
}

#[test]
fn insert_replaces_append_keeps() {
    let mut headers = Headers::new();
    headers.append("Set-Cookie", "a=1");
    headers.append("set-cookie", "b=2"); // multi-value, case-insensitive name
    assert_eq!(headers.get_all("Set-Cookie"), vec!["a=1", "b=2"]);
    assert_eq!(headers.get("set-cookie"), Some("a=1")); // first value

    // insert replaces every existing occurrence with a single value.
    headers.insert("SET-COOKIE", "c=3");
    assert_eq!(headers.get_all("set-cookie"), vec!["c=3"]);
    assert_eq!(headers.len(), 1);
}

#[test]
fn remove_returns_count_and_clears() {
    let mut headers = Headers::new();
    headers.append("X-Trace", "1");
    headers.append("x-trace", "2");
    headers.insert("Host", "example.com");
    assert_eq!(headers.remove("X-Trace"), 2); // both removed
    assert_eq!(headers.remove("absent"), 0);
    assert_eq!(headers.len(), 1);
    headers.clear();
    assert!(headers.is_empty());
}

#[test]
fn insertion_order_is_preserved() {
    let mut headers = Headers::new();
    headers.append("B", "1");
    headers.append("A", "2");
    headers.append("C", "3");
    let names: Vec<&[u8]> = headers.iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        vec![b"B".as_slice(), b"A".as_slice(), b"C".as_slice()]
    );
}

#[test]
fn byte_accessors_handle_non_utf8_values() {
    let mut headers = Headers::new();
    headers.append_bytes(b"X-Blob", &[0xff, 0xfe, 0x00]);
    // The str accessor rejects the non-UTF-8 value...
    assert_eq!(headers.get("X-Blob"), None);
    // ...but the bytes accessor returns it verbatim, and it is still "present".
    assert_eq!(
        headers.get_bytes(b"x-blob"),
        Some([0xff, 0xfe, 0x00].as_slice())
    );
    assert!(headers.contains("X-Blob"));
}

#[test]
fn typed_content_helpers() {
    let mut headers = Headers::new();
    headers.insert(Headers::CONTENT_TYPE, "text/html; charset=utf-8");
    headers.insert(Headers::CONTENT_LENGTH, "  2048  "); // OWS tolerated by the parse
    assert_eq!(headers.content_type(), Some("text/html; charset=utf-8"));
    assert_eq!(headers.content_length(), Some(2048));

    let mut bad = Headers::new();
    bad.insert(Headers::CONTENT_LENGTH, "not-a-number");
    assert_eq!(bad.content_length(), None);
}

#[test]
fn http_text_render_and_parse() {
    let mut headers = Headers::new();
    headers.insert("Host", "example.com");
    headers.append("Accept", "text/html");
    headers.append("Accept", "application/json");
    assert_eq!(
        headers.to_http_bytes(),
        b"Host: example.com\r\nAccept: text/html\r\nAccept: application/json\r\n"
    );

    // Parse tolerates \n or \r\n, trims OWS, and stops at the blank line.
    let parsed =
        Headers::parse_http(b"Host: example.com\r\nAccept:  */* \r\n\r\nignored: body\r\n");
    assert_eq!(parsed.get("host"), Some("example.com"));
    assert_eq!(parsed.get("accept"), Some("*/*")); // value trimmed
    assert!(!parsed.contains("ignored")); // past the blank line
    assert_eq!(parsed.len(), 2);

    // A line with no colon is skipped (lenient).
    let lenient = Headers::parse_http(b"Good: 1\nnonsense line\nAlso: 2\n");
    assert_eq!(lenient.len(), 2);
    assert_eq!(lenient.get("also"), Some("2"));
}

#[test]
fn http_render_parse_round_trips() {
    let mut headers = Headers::new();
    headers.append("Host", "h");
    headers.append("Set-Cookie", "a=1");
    headers.append("Set-Cookie", "b=2");
    let reparsed = Headers::parse_http(&headers.to_http_bytes());
    assert_eq!(reparsed, headers);
}

#[test]
fn binary_codec_round_trips_arbitrary_bytes() {
    let mut headers = Headers::new();
    headers.append("Host", "example.com");
    headers.append_bytes(b"X-Weird", b"has: colon and \r\n newline"); // HTTP text couldn't
    headers.append_bytes(b"X-Blob", &[0, 1, 2, 255]);

    let mut sink = Bytes::new();
    headers.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(Headers::read_from(&mut sink).unwrap(), headers);

    // An empty map round-trips too.
    let mut empty_sink = Bytes::new();
    Headers::new().write_to(&mut empty_sink).unwrap();
    empty_sink.rewind();
    assert_eq!(Headers::read_from(&mut empty_sink).unwrap(), Headers::new());
}

#[test]
fn with_builder_is_non_mutating_and_replaces() {
    let base = Headers::new().with("unit", "seconds");
    let extended = base.clone().with("source", "sensor-3");
    assert_eq!(extended.get("unit"), Some("seconds"));
    assert_eq!(extended.get("source"), Some("sensor-3"));
    assert_eq!(base.len(), 1); // base untouched

    // `with` uses insert semantics — it replaces any existing value for the name.
    let replaced = base.with("unit", "millis");
    assert_eq!(replaced.get("unit"), Some("millis"));
    assert_eq!(replaced.len(), 1);
}

#[test]
fn is_hashable_and_usable_as_a_key() {
    // Equal-valued header maps hash equal, so `Headers` works as a set/map key — which is what
    // lets a `Field` carrying `Headers` stay hashable.
    let a = Headers::new().with("a", "1").with("b", "2");
    let b = Headers::new().with("a", "1").with("b", "2");

    let mut set = HashSet::new();
    set.insert(a.clone());
    set.insert(b); // equal value -> no new entry
    set.insert(Headers::new().with("a", "9"));
    assert_eq!(set.len(), 2);
    assert!(set.contains(&a));
}

#[test]
fn binary_read_truncated_is_a_guided_error() {
    // A count of 1 but no entry bytes -> unexpected end of data.
    let mut sink = Bytes::from_slice(&[1, 0, 0, 0]);
    assert!(Headers::read_from(&mut sink).is_err());
}
