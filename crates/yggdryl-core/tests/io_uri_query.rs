//! Query-parameter map access + CRUD on [`Uri`] / [`Url`]: read (first / all / pairs /
//! contains), create-or-update (`set` / `with`), delete (`remove` / `without`), and the
//! edge cases — bare keys, empty values, repeated keys, creating/clearing the query,
//! values that embed `=`, and building a real map from the pairs.

use std::collections::HashMap;

use yggdryl_core::uri::{Uri, Url};

#[test]
fn read_first_all_pairs_contains() {
    let uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    assert_eq!(uri.query_param("a"), Some("1")); // first occurrence wins
    assert_eq!(uri.query_param("b"), Some("2"));
    assert_eq!(uri.query_param("missing"), None);

    assert_eq!(uri.query_param_all("a"), vec!["1", "3"]); // every value, in order
    assert!(uri.query_param_all("missing").is_empty());

    assert_eq!(uri.query_params(), vec![("a", "1"), ("b", "2"), ("a", "3")]);
    assert!(uri.has_query_param("a"));
    assert!(!uri.has_query_param("missing"));
}

#[test]
fn no_query_reads_empty() {
    let uri = Uri::parse_str("http://h/p").unwrap();
    assert_eq!(uri.query_param("a"), None);
    assert!(uri.query_param_all("a").is_empty());
    assert!(uri.query_params().is_empty());
    assert!(!uri.has_query_param("a"));
}

#[test]
fn set_updates_in_place_and_drops_duplicates() {
    let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    uri.set_query_param("a", "9");
    assert_eq!(uri.query(), Some("a=9&b=2")); // first updated in place, later dup dropped
    assert_eq!(uri.query_param("a"), Some("9"));
}

#[test]
fn set_appends_absent_key_and_creates_query() {
    let mut uri = Uri::parse_str("http://h/p?a=1").unwrap();
    uri.set_query_param("c", "7");
    assert_eq!(uri.query(), Some("a=1&c=7")); // absent -> appended at the end

    let mut none = Uri::parse_str("http://h/p").unwrap();
    assert_eq!(none.query(), None);
    none.set_query_param("first", "yes");
    assert_eq!(none.query(), Some("first=yes")); // creates the query
    assert_eq!(none.to_string(), "http://h/p?first=yes");
}

#[test]
fn remove_drops_all_occurrences_and_clears_empty_query() {
    let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    assert!(uri.remove_query_param("a")); // returns true when something was removed
    assert_eq!(uri.query(), Some("b=2"));

    assert!(uri.remove_query_param("b"));
    assert_eq!(uri.query(), None); // last param gone -> query cleared entirely
    assert_eq!(uri.to_string(), "http://h/p");

    assert!(!uri.remove_query_param("anything")); // no-op on an absent key
}

#[test]
fn builder_variants_chain() {
    let uri = Uri::parse_str("http://h/p?a=1")
        .unwrap()
        .with_query_param("b", "2")
        .with_query_param("a", "9")
        .without_query_param("a");
    assert_eq!(uri.query(), Some("b=2"));
    assert_eq!(uri.to_string(), "http://h/p?b=2");
}

#[test]
fn bare_key_and_empty_value_edge_cases() {
    // A bare `key` (no `=`) reads as an empty value and is present.
    let uri = Uri::parse_str("http://h/p?flag&a=1").unwrap();
    assert_eq!(uri.query_param("flag"), Some(""));
    assert!(uri.has_query_param("flag"));

    // An explicit empty value.
    let uri = Uri::parse_str("http://h/p?a=").unwrap();
    assert_eq!(uri.query_param("a"), Some(""));

    // Updating a bare key normalizes it to `key=value`.
    let mut uri = Uri::parse_str("http://h/p?flag").unwrap();
    uri.set_query_param("flag", "on");
    assert_eq!(uri.query(), Some("flag=on"));
}

#[test]
fn value_may_embed_equals() {
    // Only the first `=` splits key from value, so a value keeps any inner `=`.
    let uri = Uri::parse_str("http://h/p?token=a=b=c").unwrap();
    assert_eq!(uri.query_param("token"), Some("a=b=c"));
}

#[test]
fn params_build_a_map() {
    let uri = Uri::parse_str("http://h/p?a=1&b=2&c=3").unwrap();
    let map: HashMap<&str, &str> = uri.query_params().into_iter().collect();
    assert_eq!(map.get("b"), Some(&"2"));
    assert_eq!(map.len(), 3);
}

#[test]
fn crud_survives_a_byte_round_trip() {
    let mut uri = Uri::parse_str("http://h/p?a=1#frag").unwrap();
    uri.set_query_param("b", "2");
    uri.set_query_param("a", "9");
    assert_eq!(uri.query(), Some("a=9&b=2"));
    // The fragment is untouched and the whole thing round-trips.
    assert_eq!(uri.to_string(), "http://h/p?a=9&b=2#frag");
    assert_eq!(Uri::deserialize_bytes(&uri.serialize_bytes()).unwrap(), uri);
}

#[test]
fn url_mirrors_the_query_param_surface() {
    let mut url = Url::parse_str("https://h/p?a=1&a=2").unwrap();
    assert_eq!(url.query_param("a"), Some("1"));
    assert_eq!(url.query_param_all("a"), vec!["1", "2"]);
    assert!(url.has_query_param("a"));

    url.set_query_param("a", "9");
    assert_eq!(url.query_param("a"), Some("9"));
    assert!(url.remove_query_param("a"));
    assert_eq!(url.query_params(), Vec::<(&str, &str)>::new());
    assert_eq!(
        url.with_query_param("k", "v").to_string(),
        "https://h/p?k=v"
    );
}

#[test]
fn set_query_params_bulk_update() {
    let mut uri = Uri::parse_str("http://h/p?a=1&b=2&a=3").unwrap();
    uri.set_query_params(&[("a", "9"), ("c", "7"), ("d", "0")]);
    assert_eq!(uri.query(), Some("a=9&b=2&c=7&d=0")); // a updated (dup dropped), c/d appended

    // A key repeated in the input takes its last value.
    let mut u = Uri::parse_str("http://h/p?x=1").unwrap();
    u.set_query_params(&[("x", "a"), ("x", "b"), ("y", "c")]);
    assert_eq!(u.query(), Some("x=b&y=c"));

    // Empty input is a no-op; a bulk set on a query-less URI creates the query.
    let mut none = Uri::parse_str("http://h/p").unwrap();
    none.set_query_params(&[]);
    assert_eq!(none.query(), None);
    none.set_query_params(&[("k", "v")]);
    assert_eq!(none.query(), Some("k=v"));
}

#[test]
fn with_query_params_chains() {
    let uri = Uri::parse_str("http://h/p?a=1")
        .unwrap()
        .with_query_params(&[("a", "9"), ("b", "2")]);
    assert_eq!(uri.to_string(), "http://h/p?a=9&b=2");
}

#[test]
fn normalize_query_sorts_and_cleans() {
    let mut uri = Uri::parse_str("http://h/p?c=3&a=1&b=2&a=0").unwrap();
    uri.normalize_query();
    assert_eq!(uri.query(), Some("a=1&a=0&b=2&c=3")); // sorted by key, stable within 'a'

    // Empty tokens are cleaned out.
    let mut messy = Uri::parse_str("http://h/p?b=2&&a=1&").unwrap();
    messy.normalize_query();
    assert_eq!(messy.query(), Some("a=1&b=2"));

    // Normalizing away everything clears the query entirely.
    let mut only_empty = Uri::parse_str("http://h/p?&&").unwrap();
    only_empty.normalize_query();
    assert_eq!(only_empty.query(), None);
    assert_eq!(only_empty.to_string(), "http://h/p");

    // No query -> no-op.
    let mut plain = Uri::parse_str("http://h/p").unwrap();
    plain.normalize_query();
    assert_eq!(plain.query(), None);
}

#[test]
fn bulk_and_normalize_on_url() {
    let url = Url::parse_str("https://h/p?b=2&a=1")
        .unwrap()
        .with_query_params(&[("c", "3"), ("a", "9")])
        .with_normalized_query();
    assert_eq!(url.query(), Some("a=9&b=2&c=3"));
    assert_eq!(url.to_string(), "https://h/p?a=9&b=2&c=3");
}
