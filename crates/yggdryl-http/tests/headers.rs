//! Behavioural + edge-case tests for [`Headers`] and [`HeadersBased`].

use yggdryl_http::{Headers, HeadersBased, HeadersError};

#[test]
fn byte_and_string_access() {
    let mut headers = Headers::new();
    assert!(headers.is_empty());
    assert_eq!(headers.insert(b"unit".to_vec(), b"ms".to_vec()), None); // add
    assert_eq!(headers.set_str("lang", "en"), None);
    assert_eq!(headers.get(b"unit"), Some(b"ms".as_slice()));
    assert_eq!(headers.get_str("lang"), Some(b"en".as_slice()));
    assert!(headers.contains(b"unit"));
    assert_eq!(headers.len(), 2);

    // update returns the previous value.
    assert_eq!(
        headers.insert(b"unit".to_vec(), b"us".to_vec()),
        Some(b"ms".to_vec())
    );
    assert_eq!(headers.remove_str("lang"), Some(b"en".to_vec()));
    headers.clear();
    assert!(headers.is_empty());
}

#[test]
fn zero_copy_in_place_mutation() {
    let mut headers = Headers::new();
    headers.set_content_type("text/plain");
    // Extend the value's bytes in place — no re-insert, no map clone.
    headers
        .get_mut(Headers::CONTENT_TYPE)
        .unwrap()
        .extend_from_slice(b"; charset=utf-8");
    assert_eq!(
        headers.content_type(),
        Some(b"text/plain; charset=utf-8".as_slice())
    );
    assert!(headers.get_mut(b"absent").is_none());
}

#[test]
fn common_key_accessors() {
    let mut headers = Headers::new();
    headers.set_name("id");
    headers.set_comment("a note");
    headers.set_content_type("application/x.int64");
    headers.set_content_encoding("gzip");
    assert_eq!(headers.name(), Some(b"id".as_slice()));
    assert_eq!(headers.comment(), Some(b"a note".as_slice()));
    assert_eq!(
        headers.content_type(),
        Some(b"application/x.int64".as_slice())
    );
    assert_eq!(headers.content_encoding(), Some(b"gzip".as_slice()));
    // The constants are the raw keys.
    assert_eq!(headers.get(Headers::NAME), Some(b"id".as_slice()));
}

#[test]
fn byte_round_trip_and_ordering_is_deterministic() {
    // Insertion order differs, but the serialised bytes (key-ordered) match, so equal.
    let a = Headers::from_pairs([
        (b"b".to_vec(), b"2".to_vec()),
        (b"a".to_vec(), b"1".to_vec()),
    ]);
    let b = Headers::from_pairs([
        (b"a".to_vec(), b"1".to_vec()),
        (b"b".to_vec(), b"2".to_vec()),
    ]);
    assert_eq!(a, b);
    assert_eq!(a.serialize_bytes(), b.serialize_bytes());
    assert_eq!(Headers::deserialize_bytes(&a.serialize_bytes()).unwrap(), a);

    // Empty round-trips.
    let empty = Headers::new();
    assert_eq!(empty.serialize_bytes(), vec![0, 0, 0, 0]); // count 0
    assert_eq!(
        Headers::deserialize_bytes(&empty.serialize_bytes()).unwrap(),
        empty
    );

    // Non-UTF-8 keys/values survive.
    let binary = Headers::from_pairs([(vec![0xFF, 0x00], vec![0x00, 0xFE])]);
    assert_eq!(
        Headers::deserialize_bytes(&binary.serialize_bytes()).unwrap(),
        binary
    );
}

#[test]
fn truncated_deserialize_is_guided() {
    // A count promising one entry but no bytes for the key length.
    assert_eq!(
        Headers::deserialize_bytes(&[1, 0, 0, 0]).unwrap_err(),
        HeadersError::Truncated
    );
    // A key length longer than the remaining bytes.
    assert_eq!(
        Headers::deserialize_bytes(&[1, 0, 0, 0, 9, 0, 0, 0, b'x']).unwrap_err(),
        HeadersError::Truncated
    );
    // An empty payload has no count prefix.
    assert_eq!(
        Headers::deserialize_bytes(&[]).unwrap_err(),
        HeadersError::Truncated
    );
}

/// A minimal header-carrying type to exercise the `HeadersBased` default surface.
#[derive(Default)]
struct Column {
    headers: Option<Headers>,
}

impl HeadersBased for Column {
    fn headers(&self) -> Option<&Headers> {
        self.headers.as_ref()
    }
    fn headers_mut(&mut self) -> &mut Option<Headers> {
        &mut self.headers
    }
}

#[test]
fn headers_based_full_surface() {
    let mut column = Column::default();
    assert_eq!(column.get_header(b"unit"), None);

    // add / update via the trait; the slot is created lazily.
    assert_eq!(column.set_header_str("unit", "ms"), None);
    assert_eq!(column.set_content_type("text/plain"), None);
    assert_eq!(column.content_type(), Some(b"text/plain".as_slice()));

    // zero-copy per-key mutation through the trait.
    column
        .get_header_mut(b"unit")
        .unwrap()
        .extend_from_slice(b"ec");
    assert_eq!(column.get_header_str("unit"), Some(b"msec".as_slice()));

    // delete; the slot clears to None once the last entry is gone.
    assert_eq!(column.remove_header_str("unit"), Some(b"msec".to_vec()));
    assert_eq!(
        column.remove_header(Headers::CONTENT_TYPE),
        Some(b"text/plain".to_vec())
    );
    assert!(column.headers().is_none());
}
