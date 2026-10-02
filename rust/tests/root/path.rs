//! `rust/src/path.rs`: the value path a difference names its place with.

use yggdryl::internals::path::{Path, Segment};

#[test]
fn root_renders_as_the_root_token() {
    assert_eq!(Path::root().render(), "$");
}

#[test]
fn identifiers_use_dots_and_other_names_use_quoted_brackets() {
    let root = Path::root();
    let plain = root.field("users");
    assert_eq!(plain.render(), "$.users");

    let dotted = root.field("a.b");
    assert_eq!(dotted.render(), "$[\"a.b\"]");

    let empty = root.field("");
    assert_eq!(empty.render(), "$[\"\"]");
}

#[test]
fn nested_segments_render_outermost_first() {
    let root = Path::root();
    let users = root.field("users");
    let third = users.child(Segment::Index(3));
    let address = third.field("address");
    let zip = address.field("zip code");
    assert_eq!(zip.render(), "$.users[3].address[\"zip code\"]");
}

#[test]
fn container_segments_have_stable_spellings() {
    let root = Path::root();
    assert_eq!(root.child(Segment::Item).render(), "$[]");
    assert_eq!(root.child(Segment::MapEntries).render(), "$.entries");
    let entry = root.child(Segment::Index(2));
    assert_eq!(entry.field("key").render(), "$[2].key");
    assert_eq!(entry.field("value").render(), "$[2].value");
    assert_eq!(
        root.child(Segment::DictionaryValue).render(),
        "$.dictionary_value"
    );
    assert_eq!(root.child(Segment::RunEnds).render(), "$.run_ends");
    assert_eq!(
        root.child(Segment::RunEndValues).render(),
        "$.run_end_values"
    );
    assert_eq!(root.child(Segment::UnionType(1)).render(), "$<union:1>");
}

#[test]
fn a_long_field_name_is_bounded() {
    let long = "n".repeat(512);
    let root = Path::root();
    let rendered = root.field(&long).render();
    assert!(rendered.len() < 64, "{}", rendered.len());
    assert!(rendered.contains('\u{2026}'), "{rendered}");
}

#[test]
fn an_explicit_root_token_replaces_the_dollar() {
    let root = Path::root();
    let child = root.field("value");
    assert_eq!(child.render_from("record"), "record.value");
}

#[test]
fn a_record_refusal_located_under_the_root_is_restated_under_a_path() {
    let root = Path::root();
    let column = root.field("securityids");
    let restated = |path: &str| {
        let error = yggdryl::Error::InvalidRecord {
            path: path.into(),
            reason: "expected the key base:isin".into(),
        };
        match column.reroot(error) {
            yggdryl::Error::InvalidRecord { path, reason } => {
                assert_eq!(reason, "expected the key base:isin");
                path
            }
            other => panic!("expected a record refusal, got {other:?}"),
        }
    };
    assert_eq!(restated("$['base:isin']"), "$.securityids['base:isin']");
    assert_eq!(restated("$[2]"), "$.securityids[2]");
    assert_eq!(restated("$"), "$.securityids");
    // Any other refusal is passed on as it was.
    let other = yggdryl::Error::InvalidDataType {
        kind: "identifier",
        reason: "expected a word".into(),
    };
    assert!(matches!(
        column.reroot(other),
        yggdryl::Error::InvalidDataType { kind: "identifier", reason } if reason == "expected a word"
    ));
}
