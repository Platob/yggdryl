//! Focused edge cases for the one path grammar.

use super::{FieldPath, FieldSegment};
use crate::Scalar;

fn parse(text: &str) -> FieldPath {
    FieldPath::from_str(text).expect("path parses")
}

fn render(text: &str) -> String {
    parse(text).to_string()
}

#[test]
fn a_bare_name_is_one_field_segment() {
    let path = parse("price");
    assert_eq!(path.len(), 1);
    assert_eq!(path.as_name(), Some("price"));
    assert_eq!(path.segments(), [FieldSegment::field("price")]);
}

#[test]
fn a_leading_dot_is_optional_and_later_dots_are_not() {
    assert_eq!(parse(".price"), parse("price"));
    assert_eq!(parse("order.line.price").len(), 3);
    assert!(FieldPath::from_str("order line").is_err());
}

#[test]
fn positions_are_bracketed_and_may_count_back() {
    assert_eq!(
        parse("legs[0].price").segments(),
        [
            FieldSegment::field("legs"),
            FieldSegment::index(0),
            FieldSegment::field("price"),
        ]
    );
    assert_eq!(
        parse("legs[-1]").segments(),
        [FieldSegment::field("legs"), FieldSegment::index(-1),]
    );
}

#[test]
fn a_quoted_key_is_a_key_and_never_a_position() {
    let numeric = parse("tags['7']");
    assert_eq!(
        numeric.segments().last().and_then(FieldSegment::as_index),
        None
    );
    assert_eq!(
        numeric.segments().last().and_then(FieldSegment::as_name),
        Some("7")
    );
    assert_eq!(
        parse("legs[7]")
            .segments()
            .last()
            .and_then(FieldSegment::as_index),
        Some(7)
    );
}

#[test]
fn a_name_carrying_a_dot_has_one_spelling_and_is_one_segment() {
    // The ambiguity the plain splitters could not resolve: this is one child
    // named `a.b`, not two levels.
    let path = parse("\"a.b\"");
    assert_eq!(path.len(), 1);
    assert_eq!(path.as_name(), Some("a.b"));
    assert_eq!(parse("a.b").len(), 2);
}

#[test]
fn quotes_double_to_mean_themselves() {
    assert_eq!(parse("\"say \"\"hi\"\"\"").as_name(), Some("say \"hi\""));
    assert_eq!(
        parse("tags['it''s']")
            .segments()
            .last()
            .and_then(FieldSegment::as_name),
        Some("it's")
    );
}

#[test]
fn rendering_round_trips_through_the_parser() {
    for text in [
        "price",
        "order.line.price",
        "legs[0].price",
        "legs[-1]",
        "tags['k']",
        "\"a.b\"",
        "\"say \"\"hi\"\"\"",
        "tags['it''s']",
        "_private.x9",
    ] {
        let once = parse(text);
        let rendered = once.to_string();
        assert_eq!(
            FieldPath::from_str(&rendered).expect("rendered path parses"),
            once,
            "{text} rendered as {rendered}"
        );
    }
}

#[test]
fn a_bare_name_renders_without_a_leading_dot() {
    assert_eq!(render("price"), "price");
    assert_eq!(render(".price"), "price");
    assert_eq!(render("order.line"), "order.line");
    assert_eq!(render("legs[0]"), "legs[0]");
}

#[test]
fn the_empty_path_is_the_root() {
    let root = parse("");
    assert!(root.is_root());
    assert!(root.is_empty());
    assert_eq!(root.to_string(), "");
    assert_eq!(root, FieldPath::root());
    assert_eq!(parse("   "), FieldPath::root());
}

#[test]
fn parents_strip_one_segment_at_a_time() {
    let path = parse("order.line[2].price");
    let parent = path.parent().expect("a four-segment path has a parent");
    assert_eq!(parent.to_string(), "order.line[2]");
    assert_eq!(
        parent.parent().map(|held| held.to_string()).as_deref(),
        Some("order.line")
    );
    assert!(FieldPath::root().parent().is_none());
}

#[test]
fn joining_adds_one_segment_without_reparsing() {
    let path = FieldPath::root()
        .join(FieldSegment::field("order"))
        .join(FieldSegment::index(1));
    assert_eq!(path.to_string(), "order[1]");
    assert_eq!(path, parse("order[1]"));
}

#[test]
fn malformed_paths_name_where_they_stopped() {
    for text in [
        "order.",
        "order[",
        "order[]",
        "order[1",
        "order['k",
        "\"unterminated",
        "order..price",
        "[",
        "order[1.5]",
    ] {
        let error = FieldPath::from_str(text).expect_err(&format!("{text} must be refused"));
        let rendered = error.to_string();
        assert!(
            rendered.contains("field path"),
            "{text} refused as {rendered}"
        );
    }
}

#[test]
fn a_position_wider_than_sixty_four_bits_is_refused() {
    let error = FieldPath::from_str("legs[99999999999999999999]").expect_err("too wide");
    assert!(error.to_string().contains("64 bits"), "{error}");
}

#[test]
fn segments_order_by_kind_then_by_value() {
    let mut segments = [
        FieldSegment::index(2),
        FieldSegment::field("b"),
        FieldSegment::key(Scalar::from("k")).expect("a text key"),
        FieldSegment::field("a"),
        FieldSegment::index(1),
    ];
    segments.sort();
    assert_eq!(
        segments,
        [
            FieldSegment::field("a"),
            FieldSegment::field("b"),
            FieldSegment::index(1),
            FieldSegment::index(2),
            FieldSegment::key(Scalar::from("k")).expect("a text key"),
        ]
    );
}

#[test]
fn equal_paths_hash_alike_whatever_built_them() {
    let parsed = parse("order[1]");
    let built = FieldPath::new([FieldSegment::field("order"), FieldSegment::index(1)]);
    assert_eq!(parsed, built);
    assert_eq!(parsed.stable_hash(), built.stable_hash());
}

#[test]
fn serde_round_trips_through_the_canonical_text() {
    let path = parse("order.line[0]['k']");
    let json = serde_json::to_string(&path).expect("serializes");
    assert_eq!(json, "\"order.line[0]['k']\"");
    let back: FieldPath = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, path);
}

#[test]
fn whitespace_around_steps_is_ignored() {
    assert_eq!(parse(" order . line [ 0 ] "), parse("order.line[0]"));
}
