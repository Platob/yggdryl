//! `rust/src/fix/anomaly.rs`: what a message states that its reading could
//! not take as it stands, read off the message beside the row.

use super::{committed_registry, fixed_codec};
use yggdryl::{FixAnomaly, FixMsg};

fn parsed(line: &[u8]) -> FixMsg {
    fixed_codec(committed_registry())
        .parse_fix_line(line)
        .expect("a readable line")
}

fn pairs(held: &FixMsg) -> Vec<(&str, &str)> {
    held.anomalies()
        .iter()
        .map(|anomaly| (anomaly.field(), anomaly.reason()))
        .collect()
}

#[test]
fn a_value_that_will_not_type_is_null_in_the_row_and_an_anomaly_beside_it() {
    // `StopPx(99)` is a decimal; `abc` is not one. The row types null, the
    // arrival record keeps the text, and the refusal is readable rather than
    // a null nobody can explain.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=abc|10=0|");
    assert!(held.get_by_tag(99).is_none_or(|value| value.is_null()));
    let anomalies = pairs(&held);
    assert_eq!(anomalies.len(), 1, "{anomalies:?}");
    assert_eq!(anomalies[0].0, "stoppx");
    assert!(anomalies[0].1.contains("abc"), "{}", anomalies[0].1);
    assert_eq!(
        held.anomalies()[0].to_string(),
        format!("stoppx: {}", anomalies[0].1)
    );

    // A line that types whole has none.
    let clean = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=10.5|10=0|");
    assert!(clean.anomalies().is_empty());
}

#[test]
fn a_counter_disagreeing_with_its_group_is_an_anomaly_and_the_group_is_the_row() {
    // `NoPartyIDs(453)` says two, one occurrence follows: the group holds
    // one, the counter reads one, and the disagreement is kept.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=2|448=BROKER|447=D|452=1|10=0|");
    let anomalies = pairs(&held);
    assert_eq!(anomalies.len(), 1, "{anomalies:?}");
    assert_eq!(anomalies[0].0, "nopartyids");
    assert_eq!(anomalies[0].1, "states 2, the group holds 1");
    assert_eq!(
        held.get_by_tag(453).and_then(|value| value.as_i64()),
        Some(1)
    );

    // A counter that agrees is no anomaly.
    let agreed = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=1|448=BROKER|447=D|452=1|10=0|");
    assert!(agreed.anomalies().is_empty());
}

#[test]
fn a_settle_keeps_what_the_parse_recorded_and_a_clone_carries_it() {
    let mut held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=abc|10=0|");
    let before = held.anomalies().to_vec();
    held.set(44, yggdryl::Scalar::from("101.5"))
        .expect("a price the message can state");
    assert_eq!(
        held.anomalies(),
        before.as_slice(),
        "a write settles; the parse's refusals stay"
    );
    let copy = held.clone();
    assert_eq!(copy.anomalies(), before.as_slice());
    assert_eq!(
        before,
        vec![FixAnomaly::new("stoppx", before[0].reason())],
        "one refusal, exactly as recorded"
    );
}
