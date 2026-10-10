//! `rust/fix/src/cfi.rs`: reading a FIX message as an instrument
//! classification - what the value knows, and what a message licenses it to
//! say.

use std::sync::Arc;

use yggdryl::Cfi;
use yggdryl_fix::{FixMsg, FixRegistry};
use yggdryl_market::graph::Market;

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
    let schema = yggdryl_fix::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let at = yggdryl_fix::fix_column_of(&schema, 461).expect("a classification column");
    row.as_sequence().expect("a row")[at]
        .as_str()
        .map(str::to_owned)
}

/// The wire a message re-emits, `|`-separated.
fn wire(held: &FixMsg) -> String {
    String::from_utf8(held.into_bytes(b'|')).expect("a text wire")
}

#[test]
fn a_bridges_detailed_code_is_a_name_of_461_and_fills_the_unknowns_it_left() {
    crate::install::installed();
    // `DETAILEDCFICODE` is one of `CFICode(461)`'s names: beside a coarse
    // 461 it fills the `X` the code left, whichever arrives first, `#`-marked
    // or not, and a coarse 461 repeated with the detail is the same fold. One
    // statement stands - on the wire once, nothing kept in the metadata, no
    // anomaly - and the market keeps it.
    for line in [
        &b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESXXXX|DETAILEDCFICODE=ESVTFR|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|DETAILEDCFICODE=ESVTFR|461=ESXXXX|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESXXXX|#DETAILEDCFICODE=ESVTFR|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESXXXX|461=ESVTFR|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=XXXXXX|DETAILEDCFICODE=ESVTFR|10=0|",
    ] {
        let held = enriched(line);
        assert_eq!(
            held.by_tag(461).expect("the classification").as_str(),
            Some("ESVTFR"),
            "{line:?}"
        );
        assert_eq!(classified(&held).as_deref(), Some("ESVTFR"), "{line:?}");
        assert_eq!(
            held.get_cficode().map(ToString::to_string).as_deref(),
            Some("ESVTFR"),
            "{line:?}"
        );
        assert_eq!(held.metadata().get("detailedcficode"), None, "{line:?}");
        assert!(
            held.anomalies().is_empty(),
            "{line:?}: {:?}",
            held.anomalies()
        );
        let wire = wire(&held);
        assert_eq!(wire.matches("|461=").count(), 1, "{wire}");
        assert!(wire.contains("|461=ESVTFR|"), "{wire}");
        assert!(
            !wire.to_ascii_lowercase().contains("detailedcficode"),
            "{wire}"
        );
    }
}

#[test]
fn a_row_nested_in_a_data_field_refines_the_lines_code_and_replaces_only_a_contradiction() {
    crate::install::installed();
    // The row a frame carries in its `XmlData(213)` restates the message's
    // fields; its statements of the classification fold into the line's as
    // the line's own would, so a re-emitted capture reads back as it was
    // read, and only a code contradicting the line's replaces it.
    for (line, code) in [
        (
            &b"8=FIX.4.2|35=8|461=ESVTFR|213=#CFICODE=ESXXXX|#DETAILEDCFICODE=ESVTFR|CFICODE=ESXXXX|10=0|"[..],
            "ESVTFR",
        ),
        (
            b"8=FIX.4.2|35=8|461=ESXXXX|213=#DETAILEDCFICODE=ESVTFR|CFICODE=ESXXXX|10=0|",
            "ESVTFR",
        ),
        (b"8=FIX.4.2|35=8|461=ESVUFR|213=CFICODE=ESNUFR|10=0|", "ESNUFR"),
    ] {
        let held = enriched(line);
        assert_eq!(
            held.by_tag(461).expect("the classification").as_str(),
            Some(code),
            "{line:?}"
        );
    }
}

#[test]
fn a_detailed_code_contradicting_461_stays_beside_it_under_the_alias_rule() {
    crate::install::installed();
    // Two different letters at one position are two instruments' attributes:
    // the code of record stands, and the name that lost stays in the
    // metadata beside an anomaly naming it.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESVUFR|DETAILEDCFICODE=ESNUFR|10=0|");
    assert_eq!(
        held.by_tag(461).expect("the classification").as_str(),
        Some("ESVUFR")
    );
    assert_eq!(
        held.metadata()
            .get("detailedcficode")
            .map(ToString::to_string)
            .as_deref(),
        Some("ESNUFR")
    );
    let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
    assert_eq!(anomalies, ["detailedcficode"]);
    // Another group is no refinement either.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESXXXX|DETAILEDCFICODE=DBFNFB|10=0|");
    assert_eq!(
        held.by_tag(461).expect("the classification").as_str(),
        Some("ESXXXX")
    );
    assert_eq!(held.anomalies().len(), 1, "{:?}", held.anomalies());
}

#[test]
fn a_classification_that_says_nothing_is_not_a_classification() {
    crate::install::installed();
    assert!(Cfi::is_classified("ESVUFR"));
    assert!(Cfi::is_classified("ESXXXX"), "a group is still a reading");
    assert!(!Cfi::is_classified("XXXXXX"), "six unknowns say nothing");
    assert!(!Cfi::is_classified("ESXXX"));
    // `X` is the standard's "unknown" and is a category letter nowhere, so a
    // code opening on it is not a reading whatever follows.
    assert!(Cfi::category_of('X').is_none());
    assert!(Cfi::category_of('E').is_some());
}

#[test]
fn a_message_stating_its_classification_keeps_exactly_what_it_stated() {
    crate::install::installed();
    // A fill never lands over a stated reading, even a coarser one.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESVUFR|167=CS|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESVUFR"));
}

#[test]
fn a_message_that_states_none_is_classified_as_far_as_it_licenses() {
    crate::install::installed();
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
    crate::install::installed();
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
    crate::install::installed();
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

    // A bridge states the detailed code beside the coarse one: one of 461's
    // names, folded into it, so the column and the event both hold the
    // detail.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|55=AAPL|461=ESXXXX|DETAILEDCFICODE=ESVTFR|10=0|");
    assert_eq!(classified(&held).as_deref(), Some("ESVTFR"));
    assert_eq!(
        held.get_cficode().map(ToString::to_string).as_deref(),
        Some("ESVTFR")
    );
}
