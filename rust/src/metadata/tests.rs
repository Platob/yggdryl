//! Metadata value and protocol-view tests.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::Metadata;
use crate::Scheme;

#[test]
fn empty_and_cloned_metadata_share_their_backing_map() {
    let empty = Metadata::new();
    let other_empty = Metadata::new();
    assert!(Arc::ptr_eq(&empty.0, &other_empty.0));

    let metadata = Metadata::from_entries([("source", "orders")]).unwrap();
    let clone = metadata.clone();
    assert!(Arc::ptr_eq(&metadata.0, &clone.0));
}

#[test]
fn unique_arrow_projection_moves_string_allocations() {
    let key = "protocol-property-key-longer-than-inline-storage";
    let value = "protocol-property-value-longer-than-inline-storage";
    let metadata = Metadata::from_entries([(key, value)]).unwrap();
    let (stored_key, stored_value) = metadata.iter().next().unwrap();
    let key_pointer = stored_key.as_ptr();
    let value_pointer = stored_value.as_ptr();

    let arrow = metadata.into_arrow();
    let (arrow_key, arrow_value) = arrow.iter().next().unwrap();
    assert_eq!(arrow_key.as_ptr(), key_pointer);
    assert_eq!(arrow_value.as_ptr(), value_pointer);
}

#[test]
fn protocol_iteration_is_exact_sorted_double_ended_and_cursor_compatible() {
    let metadata = Metadata::from_entries([
        ("postgre", "before"),
        ("postgres", "plain"),
        ("postgres-prefix", "before-colon"),
        ("postgres:alpha", "a"),
        ("postgres:middle", "m"),
        ("postgres:omega", "z"),
        ("postgres0", "before-colon"),
        ("postgresql:alpha", "different-scheme"),
        ("z:last", "after"),
    ])
    .unwrap();

    let mut properties = metadata.property_iter(&Scheme::POSTGRES);
    assert_eq!(properties.next(), Some(("alpha", "a")));
    assert_eq!(properties.next_back(), Some(("omega", "z")));
    assert_eq!(properties.next(), Some(("middle", "m")));
    assert_eq!(properties.next(), None);
    assert_eq!(properties.next_back(), None);

    assert_eq!(
        metadata.next_property_entry(&Scheme::POSTGRES, Some("alpha")),
        Some(("middle", "m"))
    );
    assert_eq!(
        metadata.next_property_entry(&Scheme::POSTGRES, Some("omega")),
        None
    );
    assert_eq!(metadata.get_property(&Scheme::POSTGRES, "alpha"), Some("a"));
    assert_eq!(metadata.get_property(&Scheme::POSTGRES, "missing"), None);
}

#[test]
fn protocol_cursor_visits_every_wide_property_once() {
    let metadata = Metadata::from_entries(
        (0..1_024).map(|index| (format!("postgres:key-{index:04}"), index.to_string())),
    )
    .unwrap();
    let mut after = None;
    let mut count = 0;
    while let Some((name, value)) = metadata.next_property_entry(&Scheme::POSTGRES, after) {
        assert_eq!(name, format!("key-{count:04}"));
        assert_eq!(value, count.to_string());
        after = Some(name);
        count += 1;
    }
    assert_eq!(count, 1_024);
}

#[test]
fn protocol_views_order_and_hash_the_properties_they_expose() {
    fn hash(value: &impl Hash) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    let first = Metadata::from_entries([("postgres:a", "1")]).unwrap();
    let equal = Metadata::from_entries([("postgres:a", "1"), ("s3:bucket", "ignored")]).unwrap();
    let later = Metadata::from_entries([("postgres:b", "1")]).unwrap();
    assert_eq!(first.as_postgres(), equal.as_postgres());
    assert_eq!(hash(&first.as_postgres()), hash(&equal.as_postgres()));
    assert!(first.as_postgres() < later.as_postgres());
}

#[test]
fn http_keys_are_canonical_case_insensitive_and_collision_safe() {
    let mut metadata = Metadata::from_entries([
        ("HTTP:Content-Type", "text/plain; charset=utf-8"),
        ("HtTpS:X-Custom", "preserved"),
        ("http:Content-Length", "00042"),
    ])
    .unwrap();

    assert_eq!(
        metadata.iter().collect::<Vec<_>>(),
        [
            ("http:content-length", "42"),
            ("http:content-type", "text/plain; charset=utf-8"),
            ("http:x-custom", "preserved"),
        ]
    );
    assert_eq!(
        metadata.get("HTTP:CONTENT-TYPE"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        metadata.get("HTTPS:CONTENT-TYPE"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        metadata.get_property(&Scheme::HTTPS, "X-CUSTOM"),
        Some("preserved")
    );
    assert_eq!(
        metadata.property_iter(&Scheme::HTTPS).collect::<Vec<_>>(),
        [
            ("content-length", "42"),
            ("content-type", "text/plain; charset=utf-8"),
            ("x-custom", "preserved"),
        ]
    );
    assert_eq!(metadata.get("http:content-length"), Some("42"));
    assert_eq!(
        metadata.remove("HTTPS:CONTENT-TYPE").as_deref(),
        Some("text/plain; charset=utf-8")
    );
    assert!(!metadata.contains_key("http:content-type"));

    assert!(
        Metadata::from_entries([
            ("HTTPS:Content-Type", "text/plain"),
            ("HTTP:content-type", "application/json"),
        ])
        .is_err()
    );
}

#[test]
fn http_values_reject_injection_but_allow_horizontal_tab() {
    let metadata = Metadata::from_entries([("HTTPS:X-Trace", "one\ttwo")]).unwrap();
    assert_eq!(metadata.get("http:x-trace"), Some("one\ttwo"));

    for value in ["a\0b", "a\nb", "a\rb", "a\u{1f}b", "a\u{7f}b"] {
        assert!(
            Metadata::from_entries([("https:x-trace", value)]).is_err(),
            "accepted HTTP control value {value:?}"
        );
    }
    for key in ["http:", "http:bad name", "HTTP:bad:name", "http:café"] {
        assert!(
            Metadata::from_entries([(key, "value")]).is_err(),
            "accepted invalid HTTP field name {key:?}"
        );
    }
}

#[test]
fn content_length_requires_ascii_digits_and_u64_range() {
    assert_eq!(
        Metadata::from_entries([("http:content-length", u64::MAX.to_string())])
            .unwrap()
            .get("http:content-length"),
        Some("18446744073709551615")
    );
    for value in ["", "+1", "-1", " 1", "1 ", "١", "18446744073709551616"] {
        assert!(
            Metadata::from_entries([("http:content-length", value)]).is_err(),
            "accepted invalid Content-Length {value:?}"
        );
    }
}
