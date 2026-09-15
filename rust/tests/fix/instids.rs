//! Every identifier one instrument is known by, filled from what the message said.

use std::sync::Arc;

use yggdryl::{FixMsg, FixRegistry, Scalar};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn enriched(line: &[u8]) -> FixMsg {
    let codec = super::fixed_codec(registry());
    codec
        .enrich_message(codec.parse_fix_line(line).expect("parses"))
        .expect("enriches")
}

fn text(message: &FixMsg, tag: i32) -> Option<String> {
    message
        .get_by_tag(tag)
        .filter(|held| !held.is_null())
        .and_then(Scalar::as_str)
        .map(str::to_owned)
}

#[test]
fn the_identifier_source_says_which_code_the_identifier_is() {
    // One sentence, four letters: `SecurityID` is whatever its source says.
    for (source, code, tag) in [
        ("4", "US0378331005", yggdryl::ISINCODE_TAG_NAME.0),
        ("1", "037833100", yggdryl::CUSIPCODE_TAG_NAME.0),
        ("2", "B0YBKJ7", yggdryl::SEDOLCODE_TAG_NAME.0),
        ("A", "AAPL US EQUITY", yggdryl::BLOOMBERGCODE_TAG_NAME.0),
    ] {
        let line = format!("8=FIX.4.4|35=D|11=A1|48={code}|22={source}|10=0|");
        let held = enriched(line.as_bytes());
        assert_eq!(text(&held, tag).as_deref(), Some(code), "source {source}");
    }
}

#[test]
fn an_alt_identifier_fills_the_code_its_own_source_names() {
    // The same rule one level down: a SecurityAltID occurrence carries its
    // own source, and the column reads the occurrence that names it.
    let held = enriched(
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|454=2|455=US0378331005|456=4|455=B0YBKJ7|456=2|10=0|",
    );
    assert_eq!(
        text(&held, yggdryl::ISINCODE_TAG_NAME.0).as_deref(),
        Some("US0378331005")
    );
    assert_eq!(
        text(&held, yggdryl::SEDOLCODE_TAG_NAME.0).as_deref(),
        Some("B0YBKJ7")
    );
}

#[test]
fn a_figi_is_a_bloomberg_identifier_too() {
    // Source `S` is the Financial Instrument Global Identifier, which
    // Bloomberg issues, so it fills the same column as source `A`.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|48=BBG000B9XRY4|22=S|10=0|");
    assert_eq!(
        text(&held, yggdryl::BLOOMBERGCODE_TAG_NAME.0).as_deref(),
        Some("BBG000B9XRY4")
    );
}

#[test]
fn every_identifier_the_instrument_is_known_by_lands_in_one_column() {
    let held = enriched(
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|167=CS|48=US0378331005|22=4|454=1|455=037833100|456=1|10=0|",
    );
    let schema = yggdryl::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let values = row.as_sequence().expect("a row");
    let instids = &values[schema.index_of("instids").expect("instids")];
    let members = instids.as_sequence().expect("a struct value");

    // The order a reader reads them in: what the instrument is, then what
    // each registry calls it.
    assert_eq!(members.len(), 5);
    assert_eq!(members[0].as_str(), Some("ESXXXX"), "cficode");
    assert_eq!(members[1].as_str(), Some("US0378331005"), "isincode");
    assert!(members[2].is_null(), "no Bloomberg identifier was stated");
    assert_eq!(members[3].as_str(), Some("037833100"), "cusipcode");
    assert!(members[4].is_null(), "no SEDOL was stated");
}

#[test]
fn a_message_naming_the_instrument_in_no_way_has_no_identifier_column() {
    // A struct of five nulls is not a fact.
    let held = enriched(b"8=FIX.4.4|35=0|49=A|56=B|34=1|10=0|");
    let schema = yggdryl::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let values = row.as_sequence().expect("a row");
    assert!(values[schema.index_of("instids").expect("instids")].is_null());
}

#[test]
fn one_identifier_alone_is_still_a_fact() {
    // A struct of five nulls is not a fact, but a struct of one is: a venue
    // that publishes only a SEDOL still names the instrument.
    let held = enriched(b"8=FIX.4.4|35=D|11=A1|48=B0YBKJ7|22=2|10=0|");
    let schema = yggdryl::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let values = row.as_sequence().expect("a row");
    let members = values[schema.index_of("instids").expect("instids")]
        .as_sequence()
        .expect("a struct value");
    assert_eq!(members[4].as_str(), Some("B0YBKJ7"), "sedolcode");
    for (at, name) in [
        (0, "cficode"),
        (1, "isincode"),
        (2, "bloombergcode"),
        (3, "cusipcode"),
    ] {
        assert!(members[at].is_null(), "{name}");
    }
}

#[test]
fn a_bloomberg_identifier_fits_its_width_or_is_not_one() {
    // The column is a code of bounded width, so a value the width will not
    // hold is not that code - the message keeps what it said in its arrival
    // record and the column stays empty rather than holding a truncation.
    let width = yggdryl::DataTypeId::Bloomberg
        .code_width()
        .expect("a bounded code");
    assert_eq!(width, 32);
    let exact = "B".repeat(width);
    let held = enriched(format!("8=FIX.4.4|35=D|11=A1|48={exact}|22=A|10=0|").as_bytes());
    assert_eq!(
        text(&held, yggdryl::BLOOMBERGCODE_TAG_NAME.0).as_deref(),
        Some(exact.as_str()),
        "exactly the width is the width"
    );
    let over = "B".repeat(width + 1);
    let held = enriched(format!("8=FIX.4.4|35=D|11=A1|48={over}|22=A|10=0|").as_bytes());
    assert!(
        text(&held, yggdryl::BLOOMBERGCODE_TAG_NAME.0).is_none(),
        "a value wider than the code is not that code"
    );
    // And what the message said is still readable, because the arrival
    // record is never what a derivation decided.
    assert_eq!(
        held.get_by_tag(48).and_then(Scalar::as_str),
        Some(over.as_str())
    );
}
