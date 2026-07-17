//! Additional edge cases for [`Uri`] beyond `io_uri.rs`: the empty URI, port boundaries,
//! multi-`?`/`#` component splitting, percent-encoding kept verbatim, Unicode, long paths
//! (which exercise the pre-sized canonical buffer), an unterminated IPv6 host, scheme case
//! preservation, and a parse → serialize → parse round-trip property over a broad corpus.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use yggdryl_core::io::uri::{Uri, UriError};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn empty_string_is_the_empty_uri() {
    let uri = Uri::parse_str("").unwrap();
    assert_eq!(uri.scheme(), None);
    assert_eq!(uri.authority(), None);
    assert_eq!(uri.path(), "");
    assert_eq!(uri.name(), None);
    assert_eq!(uri.serialize_bytes(), b"");
    assert_eq!(Uri::deserialize_bytes(b"").unwrap(), uri);
}

#[test]
fn port_boundaries() {
    assert_eq!(Uri::parse_str("//h:0").unwrap().port(), Some(0));
    assert_eq!(Uri::parse_str("//h:1").unwrap().port(), Some(1));
    assert_eq!(Uri::parse_str("//h:65535").unwrap().port(), Some(65535));
    // 65536 is one past `u16::MAX` — a guided error naming the offending value.
    let err = Uri::parse_str("//h:65536").unwrap_err();
    assert!(matches!(err, UriError::InvalidPort { .. }));
    assert!(err.to_string().contains("65536"));
}

#[test]
fn multiple_question_and_hash_split_on_the_first() {
    // The first `#` opens the fragment; the first `?` before it opens the query. Inner
    // `?`/`#` are literal content of the query/fragment.
    let uri = Uri::parse_str("p?a?b#c#d").unwrap();
    assert_eq!(uri.path(), "p");
    assert_eq!(uri.query(), Some("a?b"));
    assert_eq!(uri.fragment(), Some("c#d"));
}

#[test]
fn percent_encoding_is_kept_verbatim() {
    // The parser is a component split, not a decoder — percent-escapes pass through.
    let uri = Uri::parse_str("http://h/a%20b?x=%3D#f%23").unwrap();
    assert_eq!(uri.path(), "/a%20b");
    assert_eq!(uri.query(), Some("x=%3D"));
    assert_eq!(uri.fragment(), Some("f%23"));
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn consecutive_slashes_in_path_are_preserved() {
    let uri = Uri::parse_str("http://h//a///b").unwrap();
    assert_eq!(uri.host(), Some("h"));
    assert_eq!(uri.path(), "//a///b");
    assert_eq!(uri.name(), Some("b"));
}

#[test]
fn unicode_path_is_percent_encoded_on_store() {
    // `from_path` percent-encodes non-ASCII (UTF-8) bytes for storage.
    let uri = Uri::from_path("/café/naïve.tar.gz");
    assert_eq!(uri.path(), "/caf%C3%A9/na%C3%AFve.tar.gz");
    assert_eq!(uri.name(), Some("na%C3%AFve.tar.gz"));
    assert_eq!(uri.stem(), Some("na%C3%AFve.tar")); // dot logic unaffected by the escapes
    assert_eq!(uri.extension(), Some("gz"));
    assert_eq!(uri.extensions(), vec!["tar", "gz"]);
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn long_path_round_trips_through_the_presized_buffer() {
    // A long URI forces the canonical buffer past several `String`-growth points; the
    // pre-sized `encoded_len` must be an upper bound so the round-trip stays byte-exact.
    let long = format!(
        "https://user:pw@host.example.com:8443/{}archive.backup.tar.gz?a=1&b=2#end",
        "segment/".repeat(64),
    );
    let uri = Uri::parse_str(&long).unwrap();
    assert_eq!(uri.serialize_bytes(), long.as_bytes());
    assert_eq!(uri.extension(), Some("gz"));
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn unterminated_ipv6_bracket_is_kept_as_the_host() {
    // No closing `]`: the remainder is taken as the host verbatim, with no port. It still
    // renders back to the same string, so it round-trips.
    let uri = Uri::parse_str("http://[::1/p").unwrap();
    assert_eq!(uri.host(), Some("[::1"));
    assert_eq!(uri.port(), None);
    assert_eq!(uri.path(), "/p");
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn scheme_and_host_case_is_preserved() {
    // The parser is case-preserving; it does not lowercase the scheme or host.
    let uri = Uri::parse_str("HTTPS://Example.COM/Path").unwrap();
    assert_eq!(uri.scheme(), Some("HTTPS"));
    assert_eq!(uri.host(), Some("Example.COM"));
    assert_eq!(uri.path(), "/Path");
}

#[test]
fn empty_query_and_fragment_are_present_but_empty() {
    let q = Uri::parse_str("http://h/p?").unwrap();
    assert_eq!(q.query(), Some(""));
    assert_eq!(q.fragment(), None);

    let f = Uri::parse_str("http://h/p#").unwrap();
    assert_eq!(f.fragment(), Some(""));
    assert_eq!(f.query(), None);
    assert_eq!(Uri::deserialize_bytes(&f.serialize_bytes()).unwrap(), f);
}

#[test]
fn parse_serialize_parse_round_trip_property() {
    // For every input, parse → serialize → parse must reproduce an equal `Uri` with an
    // equal hash. For inputs already in canonical form, serialize must be byte-identical.
    let canonical = [
        "https://user:pw@example.com:8080/a/b/c.txt?q=1&x=2#frag",
        "http://example.com/",
        "ftp://files.example.org:21/pub/readme",
        "http://[::1]:8080/v1/status",
        "mailto:person@example.com",
        "file:///etc/hosts",
        "s3://bucket/keys/object.parquet",
        "/a/b/c",
        "?q=1",
        "#frag",
        "",
    ];
    for s in canonical {
        let uri = Uri::parse_str(s).unwrap();
        let bytes = uri.serialize_bytes();
        assert_eq!(
            bytes,
            s.as_bytes(),
            "canonical input {s:?} must serialize verbatim"
        );
        let round = Uri::deserialize_bytes(&bytes).unwrap();
        assert_eq!(round, uri, "round-trip for {s:?}");
        assert_eq!(hash_of(&round), hash_of(&uri), "hash agrees for {s:?}");
    }

    // Non-canonical inputs (back-slash paths) normalize, but still round-trip once parsed.
    for s in [r"C:\Users\x\a.txt", r"\\server\share\f", r"a\b\c"] {
        let uri = Uri::parse_str(s).unwrap();
        let round = Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap();
        assert_eq!(round, uri, "normalized round-trip for {s:?}");
        assert_eq!(hash_of(&round), hash_of(&uri));
    }
}

#[test]
fn distinct_uris_are_distinct_map_keys() {
    let entries = [
        "http://h/a",
        "http://h/b",
        "https://h/a",
        "http://h:80/a",
        "http://h/a?q",
        "http://h/a#f",
    ];
    let map: HashMap<Uri, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, s)| (Uri::parse_str(s).unwrap(), i))
        .collect();
    assert_eq!(
        map.len(),
        entries.len(),
        "all six URIs must be distinct keys"
    );
    for (i, s) in entries.iter().enumerate() {
        assert_eq!(map.get(&Uri::parse_str(s).unwrap()), Some(&i));
    }
}
