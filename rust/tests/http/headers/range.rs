//! `rust/src/http/headers/range.rs`: the `Range` a request spells and the
//! `Content-Range` an answer states, the refusals first.

use yggdryl::Error;
use yggdryl::http::{ContentRange, range_header};

fn refusal(text: &str) -> (usize, String) {
    match ContentRange::from_str(text) {
        Err(Error::Parse {
            target: "http header",
            position,
            reason,
        }) => (position, reason.to_string()),
        other => panic!("{text:?} should be refused as a Content-Range, got {other:?}"),
    }
}

#[test]
fn another_unit_than_bytes_is_refused() {
    let (position, reason) = refusal("items 0-9/10");
    assert_eq!(position, 0);
    assert!(
        reason.starts_with("Content-Range: expected the bytes unit"),
        "{reason}"
    );
    assert!(refusal("bytes0-9/10").1.contains("a unit and a range"));
    assert!(refusal("").1.contains("a unit and a range"));
}

#[test]
fn a_range_that_is_not_first_last_over_a_total_is_refused() {
    assert!(refusal("bytes 0-9").1.contains("a range and a total"));
    assert!(refusal("bytes 9/10").1.contains("first-last"));
    assert!(refusal("bytes a-9/10").1.contains("decimal byte position"));
    assert!(refusal("bytes 0-b/10").1.contains("decimal byte position"));
    assert!(refusal("bytes 0-9/c").1.contains("decimal byte position"));
    assert!(refusal("bytes -9/10").1.contains("decimal byte position"));
    assert!(refusal("bytes 0-/10").1.contains("decimal byte position"));
    assert!(refusal("bytes */*").1.contains("decimal byte position"));
    assert!(
        refusal("bytes 0-99999999999999999999/*")
            .1
            .contains("a u64 holds")
    );
}

#[test]
fn a_last_before_its_first_or_a_total_within_the_range_is_refused() {
    let (position, reason) = refusal("bytes 9-0/10");
    assert_eq!(position, 6);
    assert!(reason.contains("at or after the first"), "{reason}");
    let (position, reason) = refusal("bytes 0-9/9");
    assert_eq!(position, 10);
    assert!(reason.contains("beyond the last byte"), "{reason}");
    assert!(refusal("bytes 0-9/5").1.contains("beyond the last byte"));
}

#[test]
fn the_request_header_spells_an_open_or_a_closed_range() {
    assert_eq!(range_header(0, None), "bytes=0-");
    assert_eq!(range_header(0, Some(0)), "bytes=0-0");
    assert_eq!(range_header(4096, Some(8191)), "bytes=4096-8191");
    assert_eq!(range_header(u64::MAX, None), format!("bytes={}-", u64::MAX));
}

#[test]
fn a_satisfied_range_reads_its_positions_and_total() {
    let range = ContentRange::from_str("bytes 0-1023/4096").unwrap();
    assert_eq!(
        range,
        ContentRange::Bytes {
            start: 0,
            end: 1023,
            total: Some(4096)
        }
    );
    assert_eq!(range.total(), Some(4096));
    assert_eq!(range.len(), 1024);
    assert!(!range.is_empty());
    assert_eq!(range.to_string(), "bytes 0-1023/4096");

    let one = ContentRange::from_str("bytes 7-7/8").unwrap();
    assert_eq!(one.len(), 1);
}

#[test]
fn a_hand_built_range_the_parser_refuses_still_answers_a_length() {
    let backwards = ContentRange::Bytes {
        start: 9,
        end: 2,
        total: None,
    };
    assert_eq!(backwards.len(), 0);
    assert!(backwards.is_empty());
    let whole = ContentRange::Bytes {
        start: 0,
        end: u64::MAX,
        total: None,
    };
    assert_eq!(whole.len(), u64::MAX);
}

#[test]
fn an_unknown_total_is_the_star() {
    let range = ContentRange::from_str("bytes 100-199/*").unwrap();
    assert_eq!(
        range,
        ContentRange::Bytes {
            start: 100,
            end: 199,
            total: None
        }
    );
    assert_eq!(range.total(), None);
    assert_eq!(range.to_string(), "bytes 100-199/*");
}

#[test]
fn the_unsatisfied_form_carries_the_total_alone() {
    let range = ContentRange::from_str("bytes */4096").unwrap();
    assert_eq!(range, ContentRange::Unsatisfied { total: 4096 });
    assert_eq!(range.total(), Some(4096));
    assert_eq!(range.len(), 0);
    assert!(range.is_empty());
    assert_eq!(range.to_string(), "bytes */4096");
}

#[test]
fn the_unit_folds_case_and_surrounding_whitespace_is_dropped() {
    let range = ContentRange::from_str(" \tBYTES 0-9/10\t").unwrap();
    assert_eq!(
        range,
        ContentRange::Bytes {
            start: 0,
            end: 9,
            total: Some(10)
        }
    );
    let parsed: ContentRange = "bytes 0-9/10".parse().unwrap();
    assert_eq!(parsed, range);
}

#[test]
fn display_reads_back_as_the_same_range() {
    for text in ["bytes 0-1023/4096", "bytes 100-199/*", "bytes */4096"] {
        let range = ContentRange::from_str(text).unwrap();
        assert_eq!(ContentRange::from_str(&range.to_string()).unwrap(), range);
    }
}
