//! `rust/src/fix/cfi.rs`: reading a FIX message as an instrument
//! classification - what the value knows, and what a message licenses it to
//! say.

use std::sync::Arc;

use yggdryl::CfiCode;
use yggdryl::graph::Market;
use yggdryl::{FixMsg, FixRegistry};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// One line parsed, which is where the classification is derived: a parse
/// fills what the message implies, and no second pass exists.
fn enriched(line: &[u8]) -> FixMsg {
    super::fixed_codec(registry())
        .parse_fix_line(line)
        .expect("parses")
}

/// Tag 461 as the fixed row reads it back, which is where the column lives.
fn classified(held: &FixMsg) -> Option<String> {
    let schema = yggdryl::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let at = yggdryl::fix_column_of(&schema, 461).expect("a classification column");
    row.as_sequence().expect("a row")[at]
        .as_str()
        .map(str::to_owned)
}

#[test]
fn two_readings_of_one_instrument_merge_where_they_agree_and_forget_where_they_do_not() {
    // Same category and group, so the attributes merge position by position.
    assert_eq!(
        CfiCode::merged("ESXXXX", "ESVUFR").as_deref(),
        Some("ESVUFR")
    );
    assert_eq!(
        CfiCode::merged("ESVUFR", "ESXXXX").as_deref(),
        Some("ESVUFR")
    );
    // Two voices, two answers: picking one would be a guess, so the position
    // says it does not know.
    assert_eq!(
        CfiCode::merged("ESVUFR", "ESVTFR").as_deref(),
        Some("ESVXFR")
    );
    assert_eq!(
        CfiCode::merged("ESVUFR", "ESVUFR").as_deref(),
        Some("ESVUFR")
    );
}

#[test]
fn a_merge_across_a_different_category_or_group_is_no_merge_at_all() {
    // The attributes of `ES` and `DB` mean different things, so merging them
    // position by position would be reading one standard's answer under
    // another's question.
    assert_eq!(CfiCode::merged("ESXXXX", "DBXXXX"), None, "two categories");
    assert_eq!(CfiCode::merged("ESXXXX", "EPXXXX"), None, "two groups");
    // And a code that is not one is not half of one.
    assert_eq!(CfiCode::merged("ESXXXX", "nonsense"), None);
    assert_eq!(CfiCode::merged("", "ESXXXX"), None);
    assert_eq!(CfiCode::merged("ESXXX", "ESXXXX"), None, "five is not six");
}

#[test]
fn a_classification_that_says_nothing_is_not_a_classification() {
    assert!(CfiCode::is_classified("ESVUFR"));
    assert!(
        CfiCode::is_classified("ESXXXX"),
        "a group is still a reading"
    );
    assert!(
        !CfiCode::is_classified("XXXXXX"),
        "six unknowns say nothing"
    );
    assert!(!CfiCode::is_classified("ESXXX"));
    // `X` is the standard's "unknown" and is a category letter nowhere, so a
    // code opening on it is not a reading whatever follows.
    assert!(CfiCode::category_of('X').is_none());
    assert!(CfiCode::category_of('E').is_some());
}

#[test]
fn a_message_stating_its_classification_keeps_exactly_what_it_stated() {
    // A fill never lands over a stated reading, even a coarser one.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESVUFR|167=CS|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESVUFR"));
}

#[test]
fn a_message_that_states_none_is_classified_as_far_as_it_licenses() {
    // `SecurityType=CS` is a common share, which is category `E` group `S`;
    // nothing in the message says more, so nothing more is said.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|167=CS|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESXXXX"));
    // A message naming no product at all is not classified rather than
    // classified as unknown: `XXXXXX` would be a row asserting six facts it
    // does not have.
    let held = enriched(b"8=FIX.4.4|35=0|49=A|56=B|34=1|10=0|");
    assert_eq!(classified(&held), None);
}

#[test]
fn a_partial_stated_code_takes_what_the_rest_of_the_message_adds() {
    // The message states the category and group and leaves the attributes
    // unknown; `PutOrCall` names the group of an option, and the two agree,
    // so the merge keeps what each knew.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=OCXXXX|201=1|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("OCXXXX"));
    // And a stated group is never overwritten by a disagreeing 201: what the
    // message said about itself outranks what the rest of it implies.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=OPXXXX|201=1|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("OPXXXX"));
}

#[test]
fn the_column_states_fixs_code_and_the_event_holds_the_detailed_classification() {
    // Tag 461 is FIX's own field: the column and the lookup by tag answer
    // what the message states or the shipped derivation fills, coarse or
    // not. The market keeps only a detailed classification, so the event
    // answers none for a coarse code and the detailed code where one is
    // stated beside it.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|167=CS|48=US0378331005|22=4|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESXXXX"));
    assert_eq!(
        held.by_tag(461).expect("the classification").as_str(),
        Some("ESXXXX")
    );
    assert_eq!(
        held.get_cficode(),
        None,
        "a coarse code is no classification the market keeps"
    );
    assert!(
        held.as_field().index_of("cficode").is_some(),
        "the classification is FIX's own field and a column like any other"
    );

    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESVTFR|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESVTFR"));
    assert_eq!(
        held.get_cficode().map(ToString::to_string).as_deref(),
        Some("ESVTFR")
    );

    // A bridge states the detailed code beside the coarse one: the column
    // keeps what FIX stated, the event takes the detail.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESXXXX|DETAILEDCFICODE=ESVTFR|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESXXXX"));
    assert_eq!(
        held.get_cficode().map(ToString::to_string).as_deref(),
        Some("ESVTFR")
    );
}
