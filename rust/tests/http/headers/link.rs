//! `rust/src/http/headers/link.rs`: RFC 8288 link values - targets,
//! relation types, quoted parameters - the refusals first.

use yggdryl::Error;
use yggdryl::http::{Link, parse_links};

fn refusal(text: &str) -> (usize, String) {
    match parse_links(text) {
        Err(Error::Parse {
            target: "http header",
            position,
            reason,
        }) => (position, reason.to_string()),
        other => panic!("{text:?} should be refused as a Link, got {other:?}"),
    }
}

#[test]
fn a_link_value_without_a_target_is_refused() {
    let (position, reason) = refusal("rel=\"next\"");
    assert_eq!(position, 0);
    assert!(reason.starts_with("Link: expected a <target>"), "{reason}");
    // The second value is the one missing its target.
    assert_eq!(refusal("</a>; rel=\"prev\", rel=\"next\"").0, 18);
}

#[test]
fn a_target_never_closed_is_refused() {
    let (position, reason) = refusal("</items?page=2; rel=\"next\"");
    assert_eq!(position, 0);
    assert!(reason.contains("closed by >"), "{reason}");
}

#[test]
fn a_parameter_that_is_no_token_or_a_quote_never_closed_is_refused() {
    assert!(refusal("</a>; =\"next\"").1.contains("parameter name"));
    assert!(
        refusal("</a>; rel=\"next")
            .1
            .contains("quoted value closed")
    );
    assert!(
        refusal("</a>; rel=\"next\\")
            .1
            .contains("quoted value closed")
    );
    assert!(
        refusal("</a> rel=\"next\"")
            .1
            .contains("expected ; , or the end")
    );
    assert!(
        refusal("</a>; rel=\"next\" </b>")
            .1
            .contains("expected ; , or the end")
    );
}

#[test]
fn an_empty_value_holds_no_link() {
    assert!(parse_links("").unwrap().is_empty());
    assert!(parse_links(" \t").unwrap().is_empty());
    // Empty list elements are ignored, as RFC 9110 section 5.6.1 asks.
    assert!(parse_links(", ,").unwrap().is_empty());
}

#[test]
fn several_links_read_in_order_with_their_relations() {
    let links = parse_links(
        "<https://api.example/items?page=3>; rel=\"next\", \
         <https://api.example/items?page=1>; rel=\"prev\",\
         <https://api.example/items?page=9>; rel=\"last\"",
    )
    .unwrap();
    assert_eq!(links.len(), 3);
    assert_eq!(links[0].target, "https://api.example/items?page=3");
    assert!(links[0].has_rel("next"));
    assert!(!links[0].has_rel("prev"));
    assert_eq!(links[1].target, "https://api.example/items?page=1");
    assert!(links[1].has_rel("prev"));
    assert_eq!(links[2].target, "https://api.example/items?page=9");
    assert_eq!(links[2].rel, ["last"]);
    assert!(links.iter().all(|link| link.parameters.is_empty()));
}

#[test]
fn a_rel_holding_several_tokens_is_every_one_of_them() {
    let links = parse_links("</feed>; rel=\"alternate  next\tlast\"").unwrap();
    assert_eq!(links[0].rel, ["alternate", "next", "last"]);
    assert!(links[0].has_rel("next"));
    assert!(links[0].has_rel("LAST"));
    assert!(links[0].has_rel("Alternate"));
    // An unquoted rel is one token.
    let links = parse_links("</feed>; rel=next").unwrap();
    assert_eq!(links[0].rel, ["next"]);
    assert!(links[0].has_rel("NEXT"));
}

#[test]
fn a_target_may_hold_commas_and_semicolons() {
    let links = parse_links("<https://api.example/q?ids=1,2,3;sort=asc>; rel=\"next\"").unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "https://api.example/q?ids=1,2,3;sort=asc");
}

#[test]
fn a_quoted_parameter_keeps_its_commas_and_unescapes_its_pairs() {
    let links = parse_links(
        "</a>; rel=\"next\"; title=\"Next, please\"; anchor=\"#\\\"top\\\"\"; type=\"text/html\"",
    )
    .unwrap();
    let link = &links[0];
    assert_eq!(link.parameter("title"), Some("Next, please"));
    assert_eq!(link.parameter("anchor"), Some("#\"top\""));
    assert_eq!(link.parameter("type"), Some("text/html"));
    assert_eq!(link.parameter("TITLE"), Some("Next, please"));
    assert_eq!(link.parameter("rel"), None);
    let parameters: Vec<(&str, &str)> = link
        .parameters
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        parameters,
        [
            ("title", "Next, please"),
            ("anchor", "#\"top\""),
            ("type", "text/html"),
        ]
    );
}

#[test]
fn parameter_names_fold_to_lower_case_and_a_bare_one_is_the_empty_value() {
    let links =
        parse_links("</a>; REL=\"next\"; CrossOrigin; Title*=UTF-8'de'N%c3%a4chste").unwrap();
    let link = &links[0];
    assert_eq!(link.rel, ["next"]);
    assert_eq!(link.parameter("crossorigin"), Some(""));
    assert_eq!(link.parameter("title*"), Some("UTF-8'de'N%c3%a4chste"));
    assert_eq!(link.parameters[0].0, "crossorigin");
}

#[test]
fn a_second_rel_is_ignored_and_a_link_may_state_none() {
    let links = parse_links("</a>; rel=\"next\"; rel=\"prev\"").unwrap();
    assert_eq!(links[0].rel, ["next"]);
    assert!(!links[0].has_rel("prev"));
    assert!(links[0].parameters.is_empty());

    let links = parse_links("</a>; title=\"Plain\"").unwrap();
    assert!(links[0].rel.is_empty());
    assert!(!links[0].has_rel("next"));
}

#[test]
fn whitespace_around_the_separators_is_dropped() {
    let links = parse_links(" </a> ;\trel = \"next\" ,\t</b>\t; rel=prev ").unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].target, "/a");
    assert!(links[0].has_rel("next"));
    assert_eq!(links[1].target, "/b");
    assert!(links[1].has_rel("prev"));
}

#[test]
fn a_link_built_by_hand_answers_the_same_questions() {
    let link = Link {
        target: "/next".to_owned(),
        rel: vec!["next".into()],
        parameters: vec![("title".into(), "More".to_owned())],
    };
    assert!(link.has_rel("Next"));
    assert_eq!(link.parameter("title"), Some("More"));
    assert_eq!(
        link,
        parse_links("</next>; rel=next; title=More").unwrap()[0]
    );
}
