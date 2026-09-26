//! `rust/src/http/headers/etag.rs`: entity tags, strong and weak, and the
//! two comparisons; the refusals first.

use yggdryl::Error;
use yggdryl::http::ETag;

fn refusal(text: &str) -> (usize, String) {
    match ETag::from_str(text) {
        Err(Error::Parse {
            target: "http header",
            position,
            reason,
        }) => (position, reason.to_string()),
        other => panic!("{text:?} should be refused as an ETag, got {other:?}"),
    }
}

#[test]
fn an_unquoted_tag_is_refused() {
    let (position, reason) = refusal("abc");
    assert_eq!(position, 0);
    assert!(
        reason.starts_with("ETag: expected a quoted tag"),
        "{reason}"
    );
    assert!(refusal("").1.contains("quoted tag"));
    assert!(refusal("\"abc").1.contains("quoted tag"));
    assert!(refusal("abc\"").1.contains("quoted tag"));
    // The weakness marker without the quotes, and a wrong-case marker.
    assert_eq!(refusal("W/abc").0, 2);
    assert!(refusal("w/\"abc\"").1.contains("quoted tag"));
    // Text past the closing quote.
    assert!(refusal("\"abc\" x").1.contains("quoted tag"));
}

#[test]
fn a_quote_a_space_or_a_control_byte_inside_the_tag_is_refused() {
    let (position, reason) = refusal("\"a\"b\"");
    assert_eq!(position, 2);
    assert!(reason.contains("inside the tag"), "{reason}");
    assert_eq!(refusal("\"a b\"").0, 2);
    assert_eq!(refusal("\"a\tb\"").0, 2);
    assert_eq!(refusal("W/\"\u{7f}\"").0, 3);
}

#[test]
fn a_strong_tag_reads_its_opaque_text() {
    let tag = ETag::from_str("\"xyzzy\"").unwrap();
    assert_eq!(
        tag,
        ETag {
            opaque: "xyzzy".to_owned(),
            weak: false
        }
    );
    assert!(!tag.is_weak());
    assert_eq!(tag.to_string(), "\"xyzzy\"");
    // The empty tag is a tag.
    assert_eq!(ETag::from_str("\"\"").unwrap().opaque, "");
    // Surrounding whitespace is a field's, not the tag's.
    assert_eq!(ETag::from_str(" \"xyzzy\"\t").unwrap().opaque, "xyzzy");
    // Every etagc byte, obs-text included.
    assert_eq!(
        ETag::from_str("\"!#$%&'()*+,-./:;<=>?@[\\]^_`{|}~é\"")
            .unwrap()
            .opaque,
        "!#$%&'()*+,-./:;<=>?@[\\]^_`{|}~é"
    );
}

#[test]
fn a_weak_tag_carries_its_marker() {
    let tag: ETag = "W/\"xyzzy\"".parse().unwrap();
    assert!(tag.is_weak());
    assert_eq!(tag.opaque, "xyzzy");
    assert_eq!(tag.to_string(), "W/\"xyzzy\"");
    assert_eq!(ETag::from_str(&tag.to_string()).unwrap(), tag);
}

#[test]
fn the_strong_comparison_needs_two_strong_tags_and_the_weak_one_does_not() {
    let strong = ETag::from_str("\"1\"").unwrap();
    let strong_again = ETag::from_str("\"1\"").unwrap();
    let weak = ETag::from_str("W/\"1\"").unwrap();
    let weak_again = ETag::from_str("W/\"1\"").unwrap();
    let other = ETag::from_str("\"2\"").unwrap();

    // RFC 9110 section 8.8.3.2, the table.
    assert!(!weak.strong_eq(&weak_again));
    assert!(weak.weak_eq(&weak_again));
    assert!(!weak.strong_eq(&other));
    assert!(!weak.weak_eq(&other));
    assert!(!weak.strong_eq(&strong));
    assert!(weak.weak_eq(&strong));
    assert!(!strong.strong_eq(&weak));
    assert!(strong.weak_eq(&weak));
    assert!(strong.strong_eq(&strong_again));
    assert!(strong.weak_eq(&strong_again));
    assert!(!strong.strong_eq(&other));
    assert!(!strong.weak_eq(&other));

    // Equality sees the marker; the comparisons are the protocol's.
    assert_ne!(strong, weak);
    assert_eq!(weak, weak_again);
}
