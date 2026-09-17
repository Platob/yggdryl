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
use yggdryl::types::State;
use yggdryl::{DataType, Field, FixCodec, FixMsg, FixRegistry, Scalar};

/// `LastQty(32)`, which `LastShares` also reaches, and `Symbol(55)`, undated.
fn undated_fields() -> Vec<Field> {
    let mut qty = DataType::Float64.nullable_field("lastqty");
    qty.as_fix_mut().set_tag(32).expect("a tag");
    qty.as_fix_mut()
        .set_names(["LastShares"])
        .expect("an alias");
    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).expect("a tag");
    vec![qty, symbol]
}

fn undated_registry() -> Arc<FixRegistry> {
    Arc::new(FixRegistry::from_fields(undated_fields()).expect("a registry"))
}

/// One message built from a schema and a record, as a converted row is.
fn built(
    registry: Arc<FixRegistry>,
    mut children: Vec<Field>,
    record: &[(&str, Scalar)],
) -> FixMsg {
    let sending = registry.field_by_tag(52).unwrap().clone();
    let fixed = super::fixed_codec(Arc::clone(&registry))
        .default_sending_time()
        .unwrap()
        .clone();
    let value = Scalar::from_record(record.iter().cloned().chain([(sending.name(), fixed)]))
        .expect("a record");
    children.push(sending);
    let root = DataType::from_fields(children)
        .expect("a struct")
        .required_field("8");
    FixMsg::with_registry(registry, root, value).expect("a message")
}

/// One message as a parse settles it: the restatement is the first step of
/// the read, so a message a caller built is already what it restates to.
fn enriched(message: FixMsg) -> FixMsg {
    message
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

/// Initial business columns, the settled bundle, then facts restatement adds.
///
fn with_bundle<'a>(before: &[&'a str], after: &[&'a str]) -> Vec<&'a str> {
    before
        .iter()
        .copied()
        .chain([
            "sendingtime",
            "updatedat",
            "createdat",
            "msghash",
            "msgphash",
            "code",
            "snapshotat",
        ])
        .chain(after.iter().copied())
        // Last, because it is enrichment's own fill rather than part of the
        // settled bundle or of what restatement adds: it derives from
        // `SendingTime`, which every settled message has, so every enriched
        // message ends on it.
        .chain(["recordedat"])
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

/// The ranked spelling a `state` column holds for one wire code.
fn state(code: &str) -> String {
    State::from_spelling(code)
        .expect("a lifecycle code")
        .as_str()
        .to_owned()
}

/// The value one root child holds, by the child's own spelling alone.
fn child<'msg>(message: &'msg FixMsg, name: &str) -> &'msg Scalar {
    let at = message
        .as_field()
        .index_of(name)
        .unwrap_or_else(|| panic!("a child named {name}"));
    message.as_value().get(at).expect("a value")
}

/// The wire a message re-emits, which is what a restatement must leave
/// alone: the entries are the row read as a tree, so a value restated to a
/// newer field shows here and nowhere else.
fn emitted(message: &FixMsg) -> String {
    String::from_utf8_lossy(&message.into_bytes(b'|')).into_owned()
}

#[test]
fn an_alias_named_child_is_re_expressed_under_the_registry_field() {
    let message = built(
        undated_registry(),
        vec![
            DataType::Int64.required_field("LastShares"),
            DataType::utf8().required_field("symbol"),
        ],
        &[
            ("LastShares", Scalar::from(100)),
            ("symbol", Scalar::from("AAPL")),
        ],
    );
    let latest = enriched(message);
    // Renamed in place, re-typed to the registry's datatype, the tag now
    // carried so the message answers by it.
    assert_eq!(names(&latest), with_bundle(&["lastqty", "symbol"], &[]));
    assert_eq!(latest.by_tag(32).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(latest.as_field().fields()[0].dtype(), &DataType::Float64);
    assert!(!latest.as_field().fields()[0].is_nullable());
    assert_eq!(text(&latest, 55).as_deref(), Some("AAPL"));
    // A version is the header's `BeginString`, and an undated registry
    // states none of its own.
    assert_eq!(latest.header().beginstring(), "");
}

#[test]
fn a_decimal_named_child_is_re_expressed_under_the_registry_field() {
    let message = built(
        undated_registry(),
        vec![DataType::utf8().nullable_field("32")],
        &[("32", Scalar::from("100"))],
    );
    let latest = enriched(message);
    assert_eq!(names(&latest), with_bundle(&["lastqty"], &[]));
    assert_eq!(latest.by_tag(32).unwrap(), Scalar::from(100.0_f64));
}

#[test]
fn two_children_reaching_one_field_merge_into_the_most_complete() {
    // The canonical child is null: the alias fills it and is dropped.
    let message = built(
        undated_registry(),
        vec![
            DataType::Float64.nullable_field("lastqty"),
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("LastShares"),
        ],
        &[
            ("lastqty", Scalar::Null),
            ("symbol", Scalar::from("AAPL")),
            ("LastShares", Scalar::from(100)),
        ],
    );
    let latest = enriched(message);
    assert_eq!(names(&latest), with_bundle(&["lastqty", "symbol"], &[]));
    assert_eq!(latest.by_tag(32).unwrap(), Scalar::from(100.0_f64));

    // Both stated and different: both are kept, because nothing that
    // arrived is lost.
    let message = built(
        undated_registry(),
        vec![
            DataType::Float64.nullable_field("lastqty"),
            DataType::Int64.required_field("LastShares"),
        ],
        &[
            ("lastqty", Scalar::from(50.0_f64)),
            ("LastShares", Scalar::from(100)),
        ],
    );
    let latest = enriched(message);
    assert_eq!(names(&latest), with_bundle(&["lastqty", "LastShares"], &[]));
    assert_eq!(latest.by_tag(32).unwrap(), Scalar::from(50.0_f64));
    assert_eq!(child(&latest, "LastShares"), &Scalar::from(100));

    // Both stated and equal once re-typed: one child.
    let message = built(
        undated_registry(),
        vec![
            DataType::Float64.nullable_field("lastqty"),
            DataType::Int64.required_field("LastShares"),
        ],
        &[
            ("lastqty", Scalar::from(100.0_f64)),
            ("LastShares", Scalar::from(100)),
        ],
    );
    let latest = enriched(message);
    assert_eq!(names(&latest), with_bundle(&["lastqty"], &[]));
}

#[test]
fn a_child_the_registry_does_not_know_is_kept_exactly() {
    let message = built(
        undated_registry(),
        vec![
            DataType::utf8().nullable_field("9999"),
            DataType::utf8().nullable_field("VenueOwnThing"),
            DataType::Int64.required_field("LastShares"),
        ],
        &[
            ("9999", Scalar::from("custom")),
            ("VenueOwnThing", Scalar::from("x")),
            ("LastShares", Scalar::from(100)),
        ],
    );
    let latest = enriched(message);
    assert_eq!(
        names(&latest),
        with_bundle(&["9999", "VenueOwnThing", "lastqty"], &[])
    );
    assert_eq!(latest.by_tag(9999).unwrap(), Scalar::from("custom"));
    assert_eq!(
        latest.as_field().fields()[1],
        DataType::utf8().nullable_field("VenueOwnThing")
    );
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
    let rule80a = || DataType::utf8().required_field("rule80a");
    let capacity = || DataType::utf8().nullable_field("ordercapacity");
    // Absent: appended.
    let message = built(
        ruled_registry(),
        vec![rule80a()],
        &[("rule80a", Scalar::from("A"))],
    );
    let latest = enriched(message);
    assert_eq!(text(&latest, 528).as_deref(), Some("A"));
    assert_eq!(
        text(&latest, 47).as_deref(),
        Some("A"),
        "the source stays as it arrived"
    );
    // Null: filled in place.
    let message = built(
        ruled_registry(),
        vec![capacity(), rule80a()],
        &[
            ("ordercapacity", Scalar::Null),
            ("rule80a", Scalar::from("A")),
        ],
    );
    let latest = enriched(message);
    assert_eq!(
        names(&latest),
        with_bundle(&["ordercapacity", "rule80a"], &[])
    );
    assert_eq!(text(&latest, 528).as_deref(), Some("A"));
    // A stated value stands, and blocks the rule.
    let message = built(
        ruled_registry(),
        vec![capacity(), rule80a()],
        &[
            ("ordercapacity", Scalar::from("P")),
            ("rule80a", Scalar::from("A")),
        ],
    );
    let latest = enriched(message);
    assert_eq!(text(&latest, 528).as_deref(), Some("P"));
    // Every value a set declares is a value the message stated, so a rule
    // never writes over one: the set retires none of them.
    let message = built(
        ruled_registry(),
        vec![capacity(), rule80a()],
        &[
            ("ordercapacity", Scalar::from("Z")),
            ("rule80a", Scalar::from("A")),
        ],
    );
    let latest = enriched(message);
    assert_eq!(text(&latest, 528).as_deref(), Some("Z"));
    // A value the rule does not speak for fills nothing.
    let message = built(
        ruled_registry(),
        vec![rule80a()],
        &[("rule80a", Scalar::from("B"))],
    );
    let latest = enriched(message);
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
    assert_eq!(
        text(&read, 150).as_deref(),
        Some(state("1").as_str()),
        "read at 4.2"
    );
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
    assert_eq!(text(&latest, 150).as_deref(), Some(state("F").as_str()));
    assert_eq!(text(&latest, 20).as_deref(), Some("1"), "the source stays");
    assert_eq!(
        text(&latest, 39).as_deref(),
        Some(state("1").as_str()),
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
    assert_eq!(latest.by_tag(32).unwrap(), Scalar::from(100.0_f64));
    assert_eq!(
        latest.by_name("LastShares").unwrap(),
        Scalar::from(100.0_f64)
    );
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
        assert_eq!(
            text(&latest, 150).as_deref(),
            Some(state("F").as_str()),
            "{line}"
        );
    }
    // A value the newest version still declares is left alone.
    let latest = restated(&reader, b"8=FIX.4.2|35=8|37=O1|150=0|10=0|");
    assert_eq!(text(&latest, 150).as_deref(), Some(state("0").as_str()));
}

#[test]
fn a_rule_applies_all_or_nothing() {
    let reader = reader();
    // OrdType OnClose is Market at TimeInForce AtTheClose - unless the
    // message stated a TimeInForce of its own, which stands and blocks the
    // whole rule.
    let blocked = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|59=0|10=0|");
    assert_eq!(text(&blocked, 40).as_deref(), Some("A"));
    assert_eq!(text(&blocked, 59).as_deref(), Some("0"));
    let applied = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|10=0|");
    assert_eq!(text(&applied, 40).as_deref(), Some("1"));
    assert_eq!(text(&applied, 59).as_deref(), Some("7"));
    // A TimeInForce already at the rule's own value is no obstacle.
    let agreed = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|59=7|10=0|");
    assert_eq!(text(&agreed, 40).as_deref(), Some("1"));
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
    // OrderID on an OrderMassActionReport is the MassActionReportID.
    let report = restated(&reader, b"8=FIX.5.0|35=r|37=O1|1373=1|10=0|");
    assert_eq!(text(&report, 1369).as_deref(), Some("O1"));
    let execution = restated(&reader, b"8=FIX.5.0|35=8|37=O1|10=0|");
    assert_eq!(execution.get_by_tag(1369), None);

    // SettlCurrAmt inside an allocation is AllocSettlCurrAmt; at the root
    // it is what it was.
    let root = restated(&reader, b"8=FIX.4.2|35=J|70=A1|119=1000|10=0|");
    assert_eq!(root.by_tag(119).unwrap(), Scalar::from(1000.0_f64));
    assert_eq!(root.get_by_tag(737), None);
    let grouped = restated(
        &reader,
        b"8=FIX.4.2|35=J|70=A1|78=1|NoAllocs[0].79=ACCT|NoAllocs[0].119=1000|10=0|",
    );
    assert_eq!(
        grouped
            .by_path(&path("allocgrp[0].allocsettlcurramt"))
            .unwrap(),
        Scalar::from(1000.0_f64)
    );
    assert_eq!(
        grouped.by_path(&path("allocgrp[0].settlcurramt")).unwrap(),
        Scalar::from(1000.0_f64),
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
    assert_eq!(floor.by_tag(1138).unwrap(), Scalar::from(50.0_f64));
    assert_eq!(floor.by_tag(111).unwrap(), Scalar::from(50.0_f64));
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

/// The committed `parties` definition as a converted row declares it, holding
/// `value`, beside `ExecBroker(76)` `BRKR`.
fn declared_parties(list: Field, value: Scalar) -> FixMsg {
    let registry = super::committed_registry();
    let broker = registry.field_by_tag(76).expect("ExecBroker").clone();
    built(
        registry,
        vec![list, broker],
        &[("parties", value), ("execbroker", Scalar::from("BRKR"))],
    )
}

#[test]
fn a_declared_group_stating_no_occurrence_is_opened_by_a_fill_into_it() {
    let registry = super::committed_registry();
    let mut list = registry
        .get_field_by_name("parties")
        .expect("parties")
        .clone();
    list.set_nullable(true);
    let latest = enriched(declared_parties(list, Scalar::Null));
    assert_eq!(
        names(&latest),
        with_bundle(&["parties", "execbroker"], &["nopartyids"])
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
fn a_group_that_arrived_as_a_large_list_keeps_its_shape() {
    let registry = super::committed_registry();
    let definition = registry.get_field_by_name("parties").expect("parties");
    let item = match definition.dtype() {
        DataType::List(item) => item.as_ref().clone(),
        other => panic!("a list, got {other}"),
    };
    let occurrence =
        Scalar::from_sequence(item.fields().iter().map(|member| match member.name() {
            "partyid" => Scalar::from("OTHER"),
            "partyrole" => Scalar::from(4),
            _ => Scalar::Null,
        }));
    let mut list = DataType::large_list(item).nullable_field("parties");
    list.set_metadata(definition.as_metadata().iter())
        .expect("the definition's metadata");
    let latest = enriched(declared_parties(list, Scalar::from_sequence([occurrence])));
    let at = latest.as_field().index_of("parties").expect("parties");
    assert!(
        matches!(
            latest.as_field().fields()[at].dtype(),
            DataType::LargeList(_)
        ),
        "the shape it arrived as"
    );
    assert_eq!(
        latest
            .as_value()
            .get(at)
            .and_then(Scalar::as_sequence)
            .map(<[Scalar]>::len),
        Some(2),
        "the broker's party appended beside the stated one"
    );
}
