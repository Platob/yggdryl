//! Percent-encoding: components are stored **encoded**, and query-param values decode on
//! demand. Covers the encode-on-store contract, set→get round-trips through decoding, keys
//! with special characters, the literal `%`, Unicode, `parse` staying verbatim, and
//! malformed escapes.

use yggdryl_core::uri::Uri;

#[test]
fn query_param_value_is_encoded_on_store_and_decoded_on_read() {
    let mut uri = Uri::parse_str("http://h/p").unwrap();
    uri.set_query_param("q", "a b&c=d#e");
    // Stored percent-encoded so the value cannot break the surrounding query.
    assert_eq!(uri.query(), Some("q=a%20b%26c%3Dd%23e"));
    // Default read returns the stored (encoded) form; the decoded read gives it back.
    assert_eq!(uri.query_param("q"), Some("a%20b%26c%3Dd%23e"));
    assert_eq!(uri.query_param_decoded("q").as_deref(), Some("a b&c=d#e"));
}

#[test]
fn set_get_round_trips_through_decode() {
    for value in ["plain", "a b", "100%", "x&y=z", "café", "π=3.14", ""] {
        let uri = Uri::parse_str("http://h/p")
            .unwrap()
            .with_query_param("k", value);
        assert_eq!(
            uri.query_param_decoded("k").as_deref(),
            Some(value),
            "round-trip for {value:?}"
        );
    }
}

#[test]
fn key_with_special_chars_is_encoded_and_looked_up_decoded() {
    let uri = Uri::parse_str("http://h/p")
        .unwrap()
        .with_query_param("a b", "1");
    assert_eq!(uri.query(), Some("a%20b=1"));
    assert_eq!(uri.query_param("a b"), Some("1")); // the lookup key is encoded internally
    assert!(uri.has_query_param("a b"));
    assert!(!uri.has_query_param("a%20b")); // the raw encoded string is a different key
}

#[test]
fn percent_literal_round_trips() {
    let uri = Uri::parse_str("http://h/p")
        .unwrap()
        .with_query_param("p", "100%");
    assert_eq!(uri.query(), Some("p=100%25"));
    assert_eq!(uri.query_param_decoded("p").as_deref(), Some("100%"));
}

#[test]
fn parse_keeps_input_verbatim() {
    // Parsing trusts the already-encoded URI string — it does not re-encode.
    let uri = Uri::parse_str("http://h/p?q=a%20b").unwrap();
    assert_eq!(uri.query_param("q"), Some("a%20b")); // as given
    assert_eq!(uri.query_param_decoded("q").as_deref(), Some("a b"));
    assert_eq!(uri.serialize_bytes(), b"http://h/p?q=a%20b"); // byte-exact round-trip
}

#[test]
fn component_setters_encode() {
    let uri = Uri::parse_str("http://h")
        .unwrap()
        .with_path("/a b/c")
        .with_fragment("sec tion")
        .with_user("john doe");
    assert_eq!(uri.path(), "/a%20b/c"); // space encoded, '/' preserved
    assert_eq!(uri.fragment(), Some("sec%20tion"));
    assert_eq!(uri.user(), Some("john%20doe"));
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn bulk_update_encodes_each_value() {
    let uri = Uri::parse_str("http://h/p")
        .unwrap()
        .with_query_params(&[("a", "1 2"), ("b", "x&y")]);
    assert_eq!(uri.query(), Some("a=1%202&b=x%26y"));
    assert_eq!(uri.query_param_decoded("a").as_deref(), Some("1 2"));
    assert_eq!(uri.query_param_decoded("b").as_deref(), Some("x&y"));
}

#[test]
fn decoded_map_view() {
    let uri = Uri::parse_str("http://h/p")
        .unwrap()
        .with_query_param("name", "a b")
        .with_query_param("tag", "x&y");
    let decoded = uri.query_params_decoded();
    assert_eq!(decoded[0].0.as_ref(), "name");
    assert_eq!(decoded[0].1.as_ref(), "a b");
    assert_eq!(decoded[1].1.as_ref(), "x&y");
}

#[test]
fn malformed_escape_is_left_verbatim_on_decode() {
    // A malformed `%XX` (not two hex digits) is not decoded, only valid ones are.
    let uri = Uri::parse_str("http://h/p?q=50%ZZ%20ok").unwrap();
    assert_eq!(uri.query_param_decoded("q").as_deref(), Some("50%ZZ ok"));
}
