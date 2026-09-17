//! A message restated at its registry's newest version - the first step of
//! the one enriching pass, and reached only through it: every child under
//! the dictionary's own field, every retired field and value filling what
//! stands in for it, the wire untouched, and a second pass changing nothing.
//! What the pass fills after restating lands in these cases too, because
//! there is one door and no way to stop at the first step.

use super::SoleMessage;
use super::path;

use std::sync::Arc;

use yggdryl::fix::{FixCode, FixReplacement};
use yggdryl::{DataType, Field, FixCodec, FixMsg, FixRegistry, Scalar};

/// `LastQty(32)`, which `LastShares` also reaches, `Symbol(55)`, and
/// `ExecBroker(76)`, which `ExecutingBroker` also reaches - all undated.
///
/// Two aliased fields, because the two are read at different seams: tag 32
/// is one of the facts the event holds typed, so a restatement to it leaves
/// the content row altogether, and tag 76 is an ordinary column, so a
/// restatement to it lands as a child like any other.
fn undated_fields() -> Vec<Field> {
    let mut qty = DataType::DECIMAL.nullable_field("lastqty");
    qty.as_fix_mut().set_tag(32).expect("a tag");
    qty.as_fix_mut()
        .set_names(["LastShares"])
        .expect("an alias");
    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).expect("a tag");
    let mut broker = DataType::utf8().nullable_field("execbroker");
    broker.as_fix_mut().set_tag(76).expect("a tag");
    broker
        .as_fix_mut()
        .set_names(["ExecutingBroker"])
        .expect("an alias");
    vec![qty, symbol, broker]
}

fn undated_registry() -> Arc<FixRegistry> {
    Arc::new(FixRegistry::from_fields(undated_fields()).expect("a registry"))
}

/// One message as the one door that restates answers it.
///
/// Restatement is the first step of the enriching pass and is reached only
/// through a read, so a fixture states its row as the pairs a reader takes
/// and the message is what that read settled.
fn built(registry: Arc<FixRegistry>, record: &[(&str, &str)]) -> FixMsg {
    let codec = super::fixed_codec(registry);
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = record
        .iter()
        .map(|(key, value)| (key.as_bytes().to_vec(), value.as_bytes().to_vec()))
        .collect();
    codec
        .parse_pairs(pairs.iter().map(|(key, value)| (&key[..], &value[..])))
        .expect("a readable row")
}

/// The names of the root's children, in order.
fn names(message: &FixMsg) -> Vec<&str> {
    message
        .as_field()
        .fields()
        .iter()
        .map(Field::name)
        .collect()
}

/// The text one tag holds.
fn text(message: &FixMsg, tag: i32) -> Option<String> {
    message
        .get_by_tag(tag)
        .as_ref()
        .and_then(Scalar::as_str)
        .map(ToOwned::to_owned)
}

/// The integer one tag holds.
fn integer(message: &FixMsg, tag: i32) -> Option<i128> {
    message.get_by_tag(tag).as_ref().and_then(Scalar::as_i128)
}

/// The wire a message re-emits, which is what a restatement must leave
/// alone: the entries are the row read as a tree, so a value restated to a
/// newer field shows here and nowhere else.
fn emitted(message: &FixMsg) -> String {
    String::from_utf8_lossy(&message.into_bytes(b'|')).into_owned()
}

#[test]
fn an_alias_named_child_is_re_expressed_under_the_registry_field() {
    let latest = built(
        undated_registry(),
        &[("LastShares", "100"), ("symbol", "AAPL")],
    );
    // Renamed in place and re-typed to the registry's datatype, so the
    // message answers by the tag. Tag 32 is one of the event's own facts,
    // so what the restatement reached is the holder rather than a column of
    // the content row.
    assert_eq!(names(&latest), ["symbol"]);
    assert_eq!(latest.by_tag(32).unwrap(), super::decimal("100"));
    assert_eq!(latest.by_name("lastqty").unwrap(), super::decimal("100"));
    assert_eq!(text(&latest, 55).as_deref(), Some("AAPL"));
    // An alias reaching an ordinary column is renamed in place there, with
    // the registry's own spelling, type and tag.
    let ordinary = built(undated_registry(), &[("ExecutingBroker", "BRKR")]);
    assert_eq!(names(&ordinary), ["execbroker"]);
    assert_eq!(ordinary.as_field().fields()[0].dtype(), &DataType::utf8());
    assert!(!ordinary.as_field().fields()[0].is_nullable());
    assert_eq!(ordinary.by_tag(76).unwrap(), Scalar::from("BRKR"));
    // A version is the header's `BeginString`, and a row that states none
    // takes the reader's own default.
    assert_eq!(latest.header().beginstring(), "FIX.4.4");
}

#[test]
fn a_decimal_named_child_is_re_expressed_under_the_registry_field() {
    let latest = built(undated_registry(), &[("32", "100")]);
    assert_eq!(names(&latest), [] as [&str; 0]);
    assert_eq!(latest.by_tag(32).unwrap(), super::decimal("100"));
    let ordinary = built(undated_registry(), &[("76", "BRKR")]);
    assert_eq!(names(&ordinary), ["execbroker"]);
    assert_eq!(ordinary.by_tag(76).unwrap(), Scalar::from("BRKR"));
}

#[test]
fn two_children_reaching_one_field_merge_into_the_most_complete() {
    // The canonical key states an absence: the alias fills the one column.
    let latest = built(
        undated_registry(),
        &[
            ("execbroker", ""),
            ("symbol", "AAPL"),
            ("ExecutingBroker", "BRKR"),
        ],
    );
    assert_eq!(names(&latest), ["symbol", "execbroker"]);
    assert_eq!(latest.by_tag(76).unwrap(), Scalar::from("BRKR"));

    // Both stated and different: both are kept, because nothing that
    // arrived is lost - under the one column the tag names, in the order
    // the row stated them.
    let latest = built(
        undated_registry(),
        &[("execbroker", "ONE"), ("ExecutingBroker", "TWO")],
    );
    assert_eq!(names(&latest), ["execbroker"]);
    assert_eq!(
        latest.by_tag(76).unwrap(),
        Scalar::from_sequence([Scalar::from("ONE"), Scalar::from("TWO")])
    );

    // Both stated and equal once re-typed: one child.
    let latest = built(
        undated_registry(),
        &[("execbroker", "ONE"), ("ExecutingBroker", "ONE")],
    );
    assert_eq!(names(&latest), ["execbroker"]);
}

#[test]
fn a_child_the_registry_does_not_know_is_kept_exactly() {
    let latest = built(
        undated_registry(),
        &[
            ("9999", "custom"),
            ("VenueOwnThing", "x"),
            ("LastShares", "100"),
        ],
    );
    assert_eq!(names(&latest), ["9999", "venueownthing"]);
    assert_eq!(latest.by_tag(9999).unwrap(), Scalar::from("custom"));
    assert_eq!(latest.by_name("venueownthing").unwrap(), Scalar::from("x"));
}

/// A hand-built rule: `Rule80A(47)` `A` fills `OrderCapacity(528)` `A`,
/// whose code set declares `A` and `P` current and `Z` deprecated at 4.4.
fn ruled_registry() -> Arc<FixRegistry> {
    let mut rule80a = DataType::utf8().nullable_field("rule80a");
    rule80a.as_fix_mut().set_tag(47).expect("a tag");
    rule80a
        .as_fix_mut()
        .set_replacements(&[FixReplacement::new(
            "select 'A' as ordercapacity where rule80a = 'A'"
                .parse()
                .expect("a plan"),
        )])
        .expect("a rule");
    let mut capacity = DataType::utf8().nullable_field("ordercapacity");
    capacity.as_fix_mut().set_tag(528).expect("a tag");
    capacity
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Agency", "A"),
            FixCode::new("Principal", "P"),
            FixCode::new("Retired", "Z"),
        ])
        .expect("codes");
    Arc::new(FixRegistry::from_fields([rule80a, capacity]).expect("a registry"))
}

#[test]
fn a_target_takes_a_value_unless_the_message_stated_one() {
    // Absent: appended.
    let latest = built(ruled_registry(), &[("rule80a", "A")]);
    assert_eq!(text(&latest, 528).as_deref(), Some("A"));
    assert_eq!(
        text(&latest, 47).as_deref(),
        Some("A"),
        "the source stays as it arrived"
    );
    // A stated absence is no stated value, so the rule fills the column.
    let latest = built(ruled_registry(), &[("ordercapacity", ""), ("rule80a", "A")]);
    assert_eq!(names(&latest), ["rule80a", "ordercapacity"]);
    assert_eq!(text(&latest, 528).as_deref(), Some("A"));
    // A stated value stands, and blocks the rule.
    let latest = built(
        ruled_registry(),
        &[("ordercapacity", "P"), ("rule80a", "A")],
    );
    assert_eq!(text(&latest, 528).as_deref(), Some("P"));
    // Every value a set declares is a value the message stated, so a rule
    // never writes over one: the set retires none of them.
    let latest = built(
        ruled_registry(),
        &[("ordercapacity", "Z"), ("rule80a", "A")],
    );
    assert_eq!(text(&latest, 528).as_deref(), Some("Z"));
    // A value the rule does not speak for fills nothing.
    let latest = built(ruled_registry(), &[("rule80a", "B")]);
    assert_eq!(latest.get_by_tag(528), None);
}

// --- The committed dictionary ---

fn reader() -> FixCodec {
    super::fixed_codec(super::committed_registry())
}

/// One line read, enriched, and enriched again to prove the second pass
/// changes nothing; the wire and the anomalies are the same before and after.
fn restated(reader: &FixCodec, line: &[u8]) -> FixMsg {
    let latest = reader.sole_line(line).expect("a readable line");
    let spelled = String::from_utf8_lossy(line);
    // Reading the line again answers the same message, restatement and all.
    let again = reader.sole_line(line).expect("a readable line");
    assert_eq!(again, latest, "a second read of {spelled}");
    assert_eq!(emitted(&again), emitted(&latest), "the wire of {spelled}");
    // The restatement leaves the version alone: it is what the line said.
    assert_eq!(
        again.header().beginstring(),
        latest.header().beginstring(),
        "the version of {spelled}"
    );
    latest
}

/// The occurrences of one root group, each as its member values by name.
fn occurrences<'msg>(message: &'msg FixMsg, group: &str) -> Vec<Vec<(&'msg str, &'msg Scalar)>> {
    let at = message
        .as_field()
        .index_of(group)
        .unwrap_or_else(|| panic!("a {group} group"));
    let item = match message.as_field().fields()[at].dtype() {
        DataType::List(item) => item.as_ref(),
        other => panic!("a list, got {other}"),
    };
    message
        .as_value()
        .get(at)
        .and_then(Scalar::as_sequence)
        .expect("occurrences")
        .iter()
        .map(|occurrence| {
            item.fields()
                .iter()
                .map(Field::name)
                .zip(occurrence.as_sequence().expect("a struct"))
                .filter(|(_, value)| !value.is_null())
                .collect()
        })
        .collect()
}

const REPORT: &[u8] = b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|";

#[test]
fn a_fix_42_execution_report_restates_at_the_dictionarys_newest_version() {
    let reader = reader();
    let read = reader.sole_line(REPORT).expect("a readable line");
    assert_eq!(read.header().beginstring(), "FIX.4.2");
    // A read restates: there is one door and no way to stop before it, so
    // the message read at 4.2 already states the newest version's spelling.
    assert_eq!(text(&read, 150).as_deref(), Some("F"), "read at 4.2");
    // The registry holds tag 47 whatever a version made of it - it filters by
    // none - and a code set states one reading of every value it declares.
    let registry = reader.registry();
    assert_eq!(registry.get_field(47).map(Field::name), Some("rule80a"));
    assert_eq!(
        registry.field_by_tag(150).unwrap().as_fix().code_value("1"),
        Some("1")
    );

    let latest = restated(&reader, REPORT);
    // ExecTransType Cancel states TradeCancel, but ExecType already stated
    // PartiallyFilled and a stated value stands - so the rule that answers
    // 150 is ExecType's own, folding the partial fill into Trade.
    assert_eq!(text(&latest, 150).as_deref(), Some("F"));
    assert_eq!(text(&latest, 20).as_deref(), Some("1"), "the source stays");
    assert_eq!(
        text(&latest, 39).as_deref(),
        Some("1"),
        "OrdStatus still declares it"
    );
    // Rule80A A is an agency order.
    assert_eq!(text(&latest, 528).as_deref(), Some("A"));
    assert_eq!(text(&latest, 47).as_deref(), Some("A"));
    // ExecBroker and ClientID are two parties, in tag order.
    let parties = occurrences(&latest, "parties");
    assert_eq!(parties.len(), 2);
    assert_eq!(
        parties[0],
        [
            ("partyid", &Scalar::from("BRKR")),
            ("partyrole", &Scalar::from(1))
        ]
    );
    assert_eq!(
        parties[1],
        [
            ("partyid", &Scalar::from("CLIENT1")),
            ("partyrole", &Scalar::from(3))
        ]
    );
    assert_eq!(
        integer(&latest, 453),
        Some(2),
        "the counter states the count"
    );
    assert_eq!(
        latest.by_path(&path("parties[1].partyrole")).unwrap(),
        Scalar::from(3)
    );
    // The fill itself, untouched and under its newest spelling.
    assert_eq!(latest.by_tag(32).unwrap(), super::decimal("100"));
    assert_eq!(latest.by_name("LastShares").unwrap(), super::decimal("100"));
    assert_eq!(
        text(&latest, 8).as_deref(),
        Some("FIX.4.2"),
        "what the message says of itself"
    );
}

#[test]
fn an_execution_type_the_specification_folded_into_trade_restates() {
    let reader = reader();
    for code in ["1", "2"] {
        let line = format!("8=FIX.4.2|35=8|37=O1|150={code}|10=0|");
        let latest = restated(&reader, line.as_bytes());
        assert_eq!(text(&latest, 150).as_deref(), Some("F"), "{line}");
    }
    // A value the newest version still declares is left alone.
    let latest = restated(&reader, b"8=FIX.4.2|35=8|37=O1|150=0|10=0|");
    assert_eq!(text(&latest, 150).as_deref(), Some("0"));
}

#[test]
fn a_group_fill_merges_into_the_occurrence_whose_constants_match() {
    let reader = reader();
    // ClearingFirm makes the role-4 party; ClearingAccount finds it and adds
    // its sub-identifier inside it rather than making a second party.
    let latest = restated(&reader, b"8=FIX.4.2|35=D|11=A|439=CLR|440=ACCT|10=0|");
    let parties = occurrences(&latest, "parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(integer(&latest, 453), Some(1));
    assert_eq!(
        latest.by_path(&path("parties[0].partyid")).unwrap(),
        Scalar::from("CLR")
    );
    assert_eq!(
        latest.by_path(&path("parties[0].partyrole")).unwrap(),
        Scalar::from(4)
    );
    assert_eq!(
        latest
            .by_path(&path("parties[0].ptyssubgrp[0].partysubid"))
            .unwrap(),
        Scalar::from("ACCT")
    );
    assert_eq!(
        latest.by_path(&path("parties[0].nopartysubids")).unwrap(),
        Scalar::from(1)
    );

    // Alone, ClearingAccount makes the role-4 party itself.
    let alone = restated(&reader, b"8=FIX.4.2|35=D|11=A|440=ACCT|10=0|");
    assert_eq!(occurrences(&alone, "parties").len(), 1);
    assert_eq!(
        alone.by_path(&path("parties[0].partyrole")).unwrap(),
        Scalar::from(4)
    );
    assert_eq!(
        alone
            .by_path(&path("parties[0].ptyssubgrp[0].partysubid"))
            .unwrap(),
        Scalar::from("ACCT")
    );
    assert!(
        alone.get_by_path(&path("parties[0].partyid")).is_none(),
        "the occurrence holds the filled members alone"
    );

    // A party the message stated with the same role and another identifier
    // stands, and blocks the fill that would write over it.
    let stated = restated(
        &reader,
        b"8=FIX.4.4|35=D|11=A|453=1|448=OTHER|452=4|439=CLR|10=0|",
    );
    let parties = occurrences(&stated, "parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(
        stated.by_path(&path("parties[0].partyid")).unwrap(),
        Scalar::from("OTHER")
    );
}

#[test]
fn a_join_and_a_from_read_the_other_tags_at_the_same_level() {
    let reader = reader();
    // MaturityDay completes MaturityMonthYear into MaturityDate, the day
    // spelled with two digits.
    let latest = restated(&reader, b"8=FIX.4.2|35=D|11=A|200=202406|205=5|10=0|");
    let dated = reader
        .sole_line(b"8=FIX.4.4|35=D|11=A|541=20240605|10=0|")
        .expect("a readable line");
    assert_eq!(latest.by_tag(541).unwrap(), dated.by_tag(541).unwrap());
    assert_eq!(
        latest.by_tag(541).unwrap(),
        dated.by_tag(541).unwrap().clone()
    );
    // A day with no month-year completes nothing.
    let alone = restated(&reader, b"8=FIX.4.2|35=D|11=A|205=5|10=0|");
    assert_eq!(alone.get_by_tag(541), None);

    // OnBehalfOfSendingTime is one hop, whose identifier is the
    // OnBehalfOfCompID beside it.
    let hop = restated(
        &reader,
        b"8=FIX.4.2|35=D|11=A|115=ONBEHALF|370=20240102-10:15:30|10=0|",
    );
    assert_eq!(integer(&hop, 627), Some(1));
    assert_eq!(
        hop.by_path(&path("hopgrp[0].hopcompid")).unwrap(),
        Scalar::from("ONBEHALF")
    );
    assert_eq!(
        hop.by_path(&path("hopgrp[0].hopsendingtime")).unwrap(),
        hop.by_tag(370).unwrap()
    );
    assert!(!hop.by_tag(370).unwrap().is_null());
}

#[test]
fn a_multiple_value_field_matches_by_token() {
    let reader = reader();
    // ExecInst T is a primary peg with fixed move type and local scope.
    let pegged = restated(&reader, b"8=FIX.4.2|35=D|11=A|18=T|10=0|");
    assert_eq!(text(&pegged, 18).as_deref(), Some("R"));
    assert_eq!(integer(&pegged, 835), Some(1));
    assert_eq!(integer(&pegged, 840), Some(1));
    // The token is found among several, and the first rule met answers.
    let several = restated(&reader, b"8=FIX.4.4|35=D|11=A|18=G L|10=0|");
    assert_eq!(integer(&several, 1094), Some(1));
    assert_eq!(text(&several, 18).as_deref(), Some("G L"));
    // A value no rule speaks for is left alone.
    let plain = restated(&reader, b"8=FIX.4.4|35=D|11=A|18=G|10=0|");
    assert_eq!(text(&plain, 18).as_deref(), Some("G"));
    assert_eq!(plain.get_by_tag(1094), None);
}

#[test]
fn a_rule_scoped_to_message_types_and_to_groups_applies_only_there() {
    let reader = reader();
    // ClearingBusinessDate inside an allocation instruction is scoped to
    // the group and never applies at the root.
    let report = restated(&reader, b"8=FIX.5.0|35=r|37=O1|1373=1|10=0|");
    assert_eq!(text(&report, 37).as_deref(), Some("O1"));
    let execution = restated(&reader, b"8=FIX.5.0|35=8|37=O1|10=0|");
    assert_eq!(execution.get_by_tag(1369), None);

    // SettlCurrAmt inside an allocation is AllocSettlCurrAmt; at the root
    // it is what it was.
    let root = restated(&reader, b"8=FIX.4.2|35=J|70=A1|119=1000|10=0|");
    assert_eq!(root.by_tag(119).unwrap(), super::decimal("1000"));
    assert_eq!(root.get_by_tag(737), None);
    let grouped = restated(
        &reader,
        b"8=FIX.4.2|35=J|70=A1|78=1|NoAllocs[0].79=ACCT|NoAllocs[0].119=1000|10=0|",
    );
    assert_eq!(
        grouped
            .by_path(&path("allocgrp[0].allocsettlcurramt"))
            .unwrap(),
        super::decimal("1000")
    );
    assert_eq!(
        grouped.by_path(&path("allocgrp[0].settlcurramt")).unwrap(),
        super::decimal("1000"),
        "the source stays"
    );
    assert_eq!(grouped.get_by_tag(737), None, "nothing at the root");
}

#[test]
fn a_removed_field_with_no_rule_and_a_source_the_rule_cannot_place_stay() {
    let reader = reader();
    // SendingDate(51) was removed with no replacement stated: kept as read.
    let latest = restated(&reader, b"8=FIX.4.2|35=D|11=A|51=20240102|10=0|");
    assert!(!latest.by_tag(51).unwrap().is_null());
    // A catch-all rule fills its target from the source's own value.
    let floor = restated(&reader, b"8=FIX.4.4|35=D|11=A|111=50|10=0|");
    assert_eq!(floor.by_tag(1138).unwrap(), super::decimal("50"));
    // `MaxFloor` is a field FIX Latest removed, so the column it was read
    // into is nulled once its value stands under the field that replaced it.
    assert_eq!(floor.get_by_tag(111), Some(Scalar::Null));
}

#[test]
fn the_dictionary_carries_the_rules_the_engine_reads() {
    // The rules are metadata on the field, so a registry edit is a rule
    // edit: restating Rule80A `A` as a principal order changes what the
    // engine writes, with nothing in Rust to change.
    let mut registry = super::committed_registry().as_ref().clone();
    let mut rule80a = registry.field_by_tag(47).expect("Rule80A").clone();
    rule80a
        .as_fix_mut()
        .set_replacements(&[FixReplacement::new(
            "select 'P' as ordercapacity where rule80a = 'A'"
                .parse()
                .expect("a plan"),
        )])
        .expect("a rule");
    registry.update(rule80a).expect("updated");
    assert!(
        registry.get_field_by_name("parties").is_some(),
        "the catalog is untouched"
    );
    let reader = super::fixed_codec(Arc::new(registry));
    let latest = restated(&reader, b"8=FIX.4.2|35=D|11=A|47=A|109=C1|10=0|");
    assert_eq!(text(&latest, 528).as_deref(), Some("P"));
    assert_eq!(text(&latest, 47).as_deref(), Some("A"));
    assert_eq!(
        occurrences(&latest, "parties").len(),
        1,
        "the other fields' rules still run"
    );
}

#[test]
fn a_constant_written_over_a_multiple_value_source_replaces_the_matched_token() {
    let reader = reader();
    // ExecInst G T: the peg is restated as R, and the other instruction
    // stays - the value was matched by one token, so one token is replaced.
    let both = restated(&reader, b"8=FIX.4.2|35=D|11=A|18=G T|10=0|");
    assert_eq!(text(&both, 18).as_deref(), Some("G R"));
    assert_eq!(integer(&both, 835), Some(1));
    assert_eq!(integer(&both, 840), Some(1));
    // R is what FIX 5.0 retired for a peg price type, and the chain reaches
    // it in the same pass.
    assert_eq!(integer(&both, 1094), Some(5));
    let trailing = restated(&reader, b"8=FIX.4.2|35=D|11=A|18=T G|10=0|");
    assert_eq!(text(&trailing, 18).as_deref(), Some("R G"));
}

#[test]
fn a_group_fill_appending_to_a_counted_group_leaves_the_anomalies_alone() {
    let reader = reader();
    // The wire counted one party; ClientID makes a second. The row's counter
    // is re-counted with it, and what the message says of itself does not
    // change: the wire was not miscounted.
    let latest = restated(
        &reader,
        b"8=FIX.4.4|35=D|11=A|453=1|448=X|452=1|109=CLIENT|10=0|",
    );
    assert_eq!(occurrences(&latest, "parties").len(), 2);
    assert_eq!(integer(&latest, 453), Some(2));
    // A counter stating none, with a party to make.
    let none = restated(&reader, b"8=FIX.4.4|35=D|11=A|453=0|76=BRKR|10=0|");
    assert_eq!(occurrences(&none, "parties").len(), 1);
    assert_eq!(integer(&none, 453), Some(1));
}

#[test]
fn a_declared_group_stating_no_occurrence_is_opened_by_a_fill_into_it() {
    // The counter states none and `ExecBroker(76)` makes a party: the rule
    // opens the group the dictionary declares rather than a column of its
    // own, and the counter counts what is there.
    let reader = reader();
    let latest = restated(&reader, b"8=FIX.4.4|35=D|11=A|453=0|76=BRKR|10=0|");
    assert!(names(&latest).contains(&"parties"), "{:?}", names(&latest));
    assert!(
        names(&latest).contains(&"nopartyids"),
        "{:?}",
        names(&latest)
    );
    let parties = occurrences(&latest, "parties");
    assert_eq!(parties.len(), 1);
    assert_eq!(
        parties[0],
        [
            ("partyid", &Scalar::from("BRKR")),
            ("partyrole", &Scalar::from(1))
        ]
    );
    assert_eq!(integer(&latest, 453), Some(1));
}

#[test]
fn a_rule_stops_where_the_message_already_stated_one_of_its_targets() {
    // FIX 4.4 Appendix 6-F: `OrdType` `OnClose` is a market order at
    // `TimeInForce` `AtTheClose`, which the dictionary states as one rule
    // filling both. A rule applies all or nothing, so an order that already
    // said how long it stands keeps both what it said and the spelling the
    // rule would have replaced.
    let reader = reader();
    let held = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|59=0|10=0|");
    assert_eq!(text(&held, 40).as_deref(), Some("A"), "the rule stood down");
    assert_eq!(text(&held, 59).as_deref(), Some("0"));

    // Saying nothing about it lets the rule state both.
    let filled = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|10=0|");
    assert_eq!(text(&filled, 40).as_deref(), Some("1"));
    assert_eq!(text(&filled, 59).as_deref(), Some("7"));

    // `TimeInForce` is an ordinary child of the row, so the rule's answer
    // and the message's own statement land in the same column - and the
    // column is there either way.
    assert!(names(&held).contains(&"timeinforce"));
    assert!(names(&filled).contains(&"timeinforce"));
}
