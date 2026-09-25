//! `rust/src/fix/msg.rs` `stated_securityids`: the identifiers a message
//! states under the wire's own fields and under the unmapped fields whose
//! names name a source, and what a settle drops of them.

use super::{committed_registry, fixed_codec};
use yggdryl::FixMsg;
use yggdryl::graph::Market;

fn parsed(line: &[u8]) -> FixMsg {
    fixed_codec(committed_registry())
        .parse_fix_line(line)
        .expect("a readable line")
}

fn ids(held: &FixMsg) -> Vec<String> {
    held.get_securityids()
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn anomalies(held: &FixMsg) -> Vec<(&str, &str)> {
    held.anomalies()
        .iter()
        .map(|anomaly| (anomaly.field(), anomaly.reason()))
        .collect()
}

#[test]
fn an_unmapped_field_named_after_a_source_states_an_entry_and_the_isin_derives_its_valor() {
    // A capture spells the instrument under bridge names no dictionary
    // holds: the ISIN is a stated entry, and the Valor a Swiss ISIN embeds
    // is derived beside it.
    let held = parsed(
        b"8=FIX.4.4|35=8|37=O1|55=ABBN|#CFICODE=ESVTFR|#ISINCODE=CH0012221716|#LASTMKT=XSWX|10=0|",
    );
    assert_eq!(ids(&held), ["ISIN:CH0012221716", "VALOR:1222171"]);
    assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
    // A `#`-marked key folds to its bare name, and `isincode` is the crated
    // column the row states the ISIN under: the value is still there.
    assert_eq!(
        held.get_by_name("isincode")
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("CH0012221716".to_owned())
    );
}

#[test]
fn every_spelling_of_a_source_name_states_its_entry() {
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|cusip_code=037833100|#SEDOLCODE=0263494|10=0|");
    assert_eq!(ids(&held), ["CUSIP:037833100", "SEDOL:0263494"]);
}

#[test]
fn an_invalid_value_states_nothing_records_an_anomaly_and_still_re_emits() {
    // A bad check digit is no CUSIP: nothing is guessed, the refusal is
    // kept, and the field stays on the row as it arrived.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|#CUSIPCODE=037833101|10=0|");
    assert!(ids(&held).is_empty());
    let dropped = anomalies(&held);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].0, "cusipcode");
    assert_eq!(
        held.get_by_name("cusipcode")
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("037833101".to_owned())
    );
    // An empty or null-like value states nothing and refuses nothing.
    for line in [
        &b"8=FIX.4.4|35=D|11=A1|55=AAPL|#ISINCODE=|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|#ISINCODE=N/A|10=0|",
    ] {
        let held = parsed(line);
        assert!(ids(&held).is_empty(), "{line:?}");
        assert!(
            anomalies(&held).is_empty(),
            "{line:?}: {:?}",
            anomalies(&held)
        );
    }
}

#[test]
fn a_name_that_names_no_own_source_states_nothing() {
    // A leg's ISIN is another instrument's, and a ticker is never a source.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|LegISIN=US0378331005|#TICKER=AAPL|10=0|");
    assert!(ids(&held).is_empty(), "{:?}", ids(&held));
    assert!(anomalies(&held).is_empty());
}

#[test]
fn the_wire_leads_and_a_conflicting_unmapped_code_is_dropped_with_an_anomaly() {
    // The same code under the wire's field and the bridge's name is one entry.
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|#ISINCODE=US0378331005|10=0|");
    assert_eq!(ids(&held), ["CUSIP:037833100", "ISIN:US0378331005"]);
    assert!(anomalies(&held).is_empty());

    // A different code under a filled key is dropped, and the drop kept.
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|#ISINCODE=US5949181045|10=0|");
    assert_eq!(ids(&held), ["CUSIP:037833100", "ISIN:US0378331005"]);
    let dropped = anomalies(&held);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].0, "isincode");
    assert!(dropped[0].1.contains("US5949181045"), "{}", dropped[0].1);
}

#[test]
fn a_crated_view_ranks_after_the_wire_and_before_an_unmapped_name() {
    // `ISINCODE` is the crated column and `ISIN` a name no dictionary holds:
    // the crate's own statement fills the key whichever arrived first, and
    // the bridge's differing code is dropped with an anomaly.
    for line in [
        &b"8=FIX.4.4|35=D|11=A1|55=AAPL|ISIN=US5949181045|ISINCODE=US0378331005|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|ISINCODE=US0378331005|ISIN=US5949181045|10=0|",
    ] {
        let held = parsed(line);
        assert_eq!(
            ids(&held),
            ["CUSIP:037833100", "ISIN:US0378331005"],
            "{line:?}"
        );
        let dropped = anomalies(&held);
        assert_eq!(dropped.len(), 1, "{line:?}: {dropped:?}");
        assert_eq!(dropped[0].0, "isin", "{line:?}");
        assert!(dropped[0].1.contains("US5949181045"), "{}", dropped[0].1);
    }
}
