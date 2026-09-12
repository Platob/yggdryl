//! A message restated at its registry's newest version: every child under
//! the dictionary's own field, every retired field and value filling what
//! stands in for it, the wire untouched, and a second pass changing nothing.

use super::OneMessage;
use super::path;

use std::sync::Arc;

use yggdryl::fix::{FixCode, FixFill, FixFillSource, FixLineageEntry, FixPedigree, FixReplacement};
use yggdryl::media::text::{TextBytes, TextLine};
use yggdryl::types::State;
use yggdryl::{
    DataType, Field, FixCategory, FixCodec, FixMsg, FixRegistry, Scalar, VERSION_TAG, Version,
};

fn version(text: &str) -> Version {
    text.parse().expect("a version")
}

/// `LastQty(32)`, which `LastShares` also reaches, and `Symbol(55)`, undated.
fn undated_fields() -> Vec<Field> {
    let mut qty = DataType::Float64.nullable_field("lastqty");
    qty.as_fix_mut().set_tag(32).expect("a tag");
    qty.as_fix_mut()
        .set_aliases(["LastShares"])
        .expect("an alias");
    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).expect("a tag");
    vec![qty, symbol]
}

fn undated_registry() -> Arc<FixRegistry> {
    Arc::new(FixRegistry::from_fields(undated_fields()).expect("a registry"))
}

/// The same two fields, `LastQty` dated: an integer `LastShares` until 4.3.
fn dated_registry() -> Arc<FixRegistry> {
    let mut fields = undated_fields();
    fields[0]
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.0"), None))
                .with_name("lastshares")
                .with_dtype("int32"),
            FixLineageEntry::new(FixPedigree::new(version("4.3"), None))
                .with_name("lastqty")
                .with_dtype("float64"),
        ])
        .expect("a lineage");
    Arc::new(FixRegistry::from_fields(fields).expect("a registry"))
}

/// One message built from a schema and a record, as a converted row is.
fn built(registry: Arc<FixRegistry>, children: Vec<Field>, record: &[(&str, Scalar)]) -> FixMsg {
    let root = DataType::from_fields(children)
        .expect("a struct")
        .required_field("8");
    let value = Scalar::from_record(record.iter().cloned()).expect("a record");
    FixMsg::with_registry(registry, root, value).expect("a message")
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
fn text(message: &FixMsg, tag: i32) -> Option<&str> {
    message.get_by_tag(tag).and_then(Scalar::as_str)
}

/// The integer one tag holds.
fn integer(message: &FixMsg, tag: i32) -> Option<i128> {
    message.get_by_tag(tag).and_then(Scalar::as_i128)
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

/// What a message says about itself that does not add up, rendered.
fn anomalies(message: &FixMsg) -> Vec<String> {
    message
        .anomalies()
        .map(|anomaly| anomaly.to_string())
        .collect()
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
    let latest = message.into_latest().expect("restated");
    // Renamed in place, re-typed to the registry's datatype, the tag now
    // carried so the message answers by it.
    assert_eq!(names(&latest), ["lastqty", "symbol"]);
    assert_eq!(latest.by_tag(32).unwrap(), &Scalar::from(100.0_f64));
    assert_eq!(latest.as_field().fields()[0].dtype(), &DataType::Float64);
    assert!(!latest.as_field().fields()[0].is_nullable());
    assert_eq!(text(&latest, 55), Some("AAPL"));
    assert_eq!(latest.version(), None, "an undated registry stamps nothing");
    assert_eq!(latest.get_by_tag(VERSION_TAG), None);
}

#[test]
fn a_decimal_named_child_is_re_expressed_under_the_registry_field() {
    let message = built(
        undated_registry(),
        vec![DataType::utf8().nullable_field("32")],
        &[("32", Scalar::from("100"))],
    );
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["lastqty"]);
    assert_eq!(latest.by_tag(32).unwrap(), &Scalar::from(100.0_f64));
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
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["lastqty", "symbol"]);
    assert_eq!(latest.by_tag(32).unwrap(), &Scalar::from(100.0_f64));

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
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["lastqty", "LastShares"]);
    assert_eq!(latest.by_tag(32).unwrap(), &Scalar::from(50.0_f64));
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
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["lastqty"]);
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
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["9999", "VenueOwnThing", "lastqty"]);
    assert_eq!(latest.by_tag(9999).unwrap(), &Scalar::from("custom"));
    assert_eq!(
        latest.as_field().fields()[1],
        DataType::utf8().nullable_field("VenueOwnThing")
    );
}

#[test]
fn the_version_is_stamped_from_a_dated_registry_and_a_second_pass_is_equal() {
    let message = built(
        dated_registry(),
        vec![DataType::Int64.required_field("LastShares")],
        &[("LastShares", Scalar::from(100))],
    );
    assert_eq!(message.version(), None);
    let latest = message.into_latest().expect("restated");
    assert_eq!(latest.version(), Some(version("4.3")));
    assert_eq!(text(&latest, VERSION_TAG), Some("4.3"));
    assert_eq!(names(&latest), ["lastqty", "version"]);
    // The lineage's old spelling reaches the field as an alias does.
    assert_eq!(
        latest.by_name("LastShares").unwrap(),
        &Scalar::from(100.0_f64)
    );

    let again = latest.clone().into_latest().expect("a second pass");
    assert_eq!(again, latest);

    // A stated version is replaced in place rather than stood beside.
    let mut stated = DataType::utf8().required_field("version");
    stated.as_fix_mut().set_tag(VERSION_TAG).expect("a tag");
    let message = built(
        dated_registry(),
        vec![stated, DataType::Int64.required_field("LastShares")],
        &[
            ("version", Scalar::from("4.0")),
            ("LastShares", Scalar::from(100)),
        ],
    );
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["version", "lastqty"]);
    assert_eq!(latest.version(), Some(version("4.3")));
}

/// A hand-built rule: `Rule80A(47)` `A` fills `OrderCapacity(528)` `A`,
/// whose code set declares `A` and `P` current and `Z` deprecated at 4.4.
fn ruled_registry() -> Arc<FixRegistry> {
    let mut rule80a = DataType::utf8().nullable_field("rule80a");
    rule80a.as_fix_mut().set_tag(47).expect("a tag");
    rule80a
        .as_fix_mut()
        .set_lineage(&[
            FixLineageEntry::new(FixPedigree::new(version("4.0"), None))
                .with_name("rule80a")
                .with_dtype("utf8"),
            FixLineageEntry::new(FixPedigree::new(version("4.4"), None)).remove(),
        ])
        .expect("a lineage");
    rule80a
        .as_fix_mut()
        .set_replacements(&[FixReplacement::new(version("4.3"))
            .with_when("A")
            .with_fills([FixFill::Field {
                tag: 528,
                value: FixFillSource::Constant("A".into()),
            }])])
        .expect("a rule");
    let mut capacity = DataType::utf8().nullable_field("ordercapacity");
    capacity.as_fix_mut().set_tag(528).expect("a tag");
    capacity
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Agency", "A"),
            FixCode::new("Principal", "P"),
            FixCode::new("Retired", "Z").with_deprecated(version("4.4")),
        ])
        .expect("codes");
    Arc::new(FixRegistry::from_fields([rule80a, capacity]).expect("a registry"))
}

#[test]
fn a_target_takes_a_value_unless_it_states_a_current_code() {
    let rule80a = || DataType::utf8().required_field("rule80a");
    let capacity = || DataType::utf8().nullable_field("ordercapacity");
    // Absent: appended.
    let message = built(
        ruled_registry(),
        vec![rule80a()],
        &[("rule80a", Scalar::from("A"))],
    );
    let latest = message.into_latest().expect("restated");
    assert_eq!(text(&latest, 528), Some("A"));
    assert_eq!(
        text(&latest, 47),
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
    let latest = message.into_latest().expect("restated");
    assert_eq!(names(&latest), ["ordercapacity", "rule80a", "version"]);
    assert_eq!(text(&latest, 528), Some("A"));
    // A stated current code stands, and blocks the rule.
    let message = built(
        ruled_registry(),
        vec![capacity(), rule80a()],
        &[
            ("ordercapacity", Scalar::from("P")),
            ("rule80a", Scalar::from("A")),
        ],
    );
    let latest = message.into_latest().expect("restated");
    assert_eq!(text(&latest, 528), Some("P"));
    // A code the set no longer declares at the newest version is written over.
    let message = built(
        ruled_registry(),
        vec![capacity(), rule80a()],
        &[
            ("ordercapacity", Scalar::from("Z")),
            ("rule80a", Scalar::from("A")),
        ],
    );
    let latest = message.into_latest().expect("restated");
    assert_eq!(text(&latest, 528), Some("A"));
    // A value the rule does not speak for fills nothing.
    let message = built(
        ruled_registry(),
        vec![rule80a()],
        &[("rule80a", Scalar::from("B"))],
    );
    let latest = message.into_latest().expect("restated");
    assert_eq!(latest.get_by_tag(528), None);
    assert_eq!(latest.version(), Some(version("4.4")));
}

// --- The committed dictionary ---

fn reader() -> FixCodec {
    FixCodec::new(super::committed_registry())
}

/// One line read, restated, and restated again to prove the second pass
/// changes nothing; the wire and the anomalies are the same before and after.
fn restated(reader: &FixCodec, line: &[u8]) -> FixMsg {
    let read = reader.one_line(line, false).expect("a readable line");
    let latest = read.clone().into_latest().expect("restated");
    let spelled = String::from_utf8_lossy(line);
    assert_eq!(latest.into_bytes(b'|'), line, "the wire of {spelled}");
    assert_eq!(latest.entries(), read.entries(), "the entries of {spelled}");
    assert_eq!(
        anomalies(&latest),
        anomalies(&read),
        "the anomalies of {spelled}"
    );
    let again = latest.clone().into_latest().expect("a second pass");
    assert_eq!(again, latest, "a second pass over {spelled}");
    assert_eq!(
        latest.version(),
        Some(
            reader
                .registry()
                .newest()
                .expect("a dated dictionary")
                .version()
        ),
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
    let read = reader.one_line(REPORT, false).expect("a readable line");
    assert_eq!(read.version(), Some(version("4.2")));
    assert_eq!(text(&read, 150), Some(state("1").as_str()), "read at 4.2");
    // Tag 47 is a dictionary field at 4.2 and gone by 4.4; the registry
    // holds it whatever the version, and the code sets still date a value.
    let registry = reader.registry();
    assert!(registry.get_field_at(version("4.4"), 47).is_none());
    assert_eq!(
        registry.get_field_at(version("4.2"), 47).map(Field::name),
        Some("rule80a")
    );
    assert_eq!(
        registry
            .field_by_tag(150)
            .unwrap()
            .as_fix()
            .code_value_at(version("4.2"), "1"),
        Some("1")
    );

    let latest = restated(&reader, REPORT);
    // ExecTransType Cancel wrote ExecType TradeCancel over the retired
    // PartiallyFilled before ExecType's own rule read it.
    assert_eq!(text(&latest, 150), Some(state("H").as_str()));
    assert_eq!(text(&latest, 20), Some("1"), "the source stays");
    assert_eq!(
        text(&latest, 39),
        Some(state("1").as_str()),
        "OrdStatus still declares it"
    );
    // Rule80A A is an agency order.
    assert_eq!(text(&latest, 528), Some("A"));
    assert_eq!(text(&latest, 47), Some("A"));
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
        &Scalar::from(3)
    );
    // The fill itself, untouched and under its newest spelling.
    assert_eq!(latest.by_tag(32).unwrap(), &Scalar::from(100.0_f64));
    assert_eq!(
        latest.by_name("LastShares").unwrap(),
        &Scalar::from(100.0_f64)
    );
    assert_eq!(
        text(&latest, 8),
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
        assert_eq!(text(&latest, 150), Some(state("F").as_str()), "{line}");
    }
    // A value the newest version still declares is left alone.
    let latest = restated(&reader, b"8=FIX.4.2|35=8|37=O1|150=0|10=0|");
    assert_eq!(text(&latest, 150), Some(state("0").as_str()));
}

#[test]
fn a_rule_applies_all_or_nothing() {
    let reader = reader();
    // OrdType OnClose is Market at TimeInForce AtTheClose - unless the
    // message stated a TimeInForce of its own, which stands and blocks the
    // whole rule.
    let blocked = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|59=0|10=0|");
    assert_eq!(text(&blocked, 40), Some("A"));
    assert_eq!(text(&blocked, 59), Some("0"));
    let applied = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|10=0|");
    assert_eq!(text(&applied, 40), Some("1"));
    assert_eq!(text(&applied, 59), Some("7"));
    // A TimeInForce already at the rule's own value is no obstacle.
    let agreed = restated(&reader, b"8=FIX.4.2|35=D|11=A|40=A|59=7|10=0|");
    assert_eq!(text(&agreed, 40), Some("1"));
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
        &Scalar::from("CLR")
    );
    assert_eq!(
        latest.by_path(&path("parties[0].partyrole")).unwrap(),
        &Scalar::from(4)
    );
    assert_eq!(
        latest
            .by_path(&path("parties[0].ptyssubgrp[0].partysubid"))
            .unwrap(),
        &Scalar::from("ACCT")
    );
    assert_eq!(
        latest.by_path(&path("parties[0].nopartysubids")).unwrap(),
        &Scalar::from(1)
    );

    // Alone, ClearingAccount makes the role-4 party itself.
    let alone = restated(&reader, b"8=FIX.4.2|35=D|11=A|440=ACCT|10=0|");
    assert_eq!(occurrences(&alone, "parties").len(), 1);
    assert_eq!(
        alone.by_path(&path("parties[0].partyrole")).unwrap(),
        &Scalar::from(4)
    );
    assert_eq!(
        alone
            .by_path(&path("parties[0].ptyssubgrp[0].partysubid"))
            .unwrap(),
        &Scalar::from("ACCT")
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
        &Scalar::from("OTHER")
    );
}

#[test]
fn a_join_and_a_from_read_the_other_tags_at_the_same_level() {
    let reader = reader();
    // MaturityDay completes MaturityMonthYear into MaturityDate, the day
    // spelled with two digits.
    let latest = restated(&reader, b"8=FIX.4.2|35=D|11=A|200=202406|205=5|10=0|");
    let dated = reader
        .one_line(b"8=FIX.4.4|35=D|11=A|541=20240605|10=0|", false)
        .expect("a readable line");
    assert_eq!(latest.by_tag(541).unwrap(), dated.by_tag(541).unwrap());
    assert_eq!(
        latest.by_tag(541).unwrap(),
        &dated.by_tag(541).unwrap().clone()
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
        &Scalar::from("ONBEHALF")
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
    assert_eq!(text(&pegged, 18), Some("R"));
    assert_eq!(integer(&pegged, 835), Some(1));
    assert_eq!(integer(&pegged, 840), Some(1));
    // The token is found among several, and the first rule met answers.
    let several = restated(&reader, b"8=FIX.4.4|35=D|11=A|18=G L|10=0|");
    assert_eq!(integer(&several, 1094), Some(1));
    assert_eq!(text(&several, 18), Some("G L"));
    // A value no rule speaks for is left alone.
    let plain = restated(&reader, b"8=FIX.4.4|35=D|11=A|18=G|10=0|");
    assert_eq!(text(&plain, 18), Some("G"));
    assert_eq!(plain.get_by_tag(1094), None);
}

#[test]
fn a_rule_scoped_to_message_types_and_to_groups_applies_only_there() {
    let reader = reader();
    // OrderID on an OrderMassActionReport is the MassActionReportID.
    let report = restated(&reader, b"8=FIX.5.0|35=r|37=O1|1373=1|10=0|");
    assert_eq!(text(&report, 1369), Some("O1"));
    let execution = restated(&reader, b"8=FIX.5.0|35=8|37=O1|10=0|");
    assert_eq!(execution.get_by_tag(1369), None);

    // SettlCurrAmt inside an allocation is AllocSettlCurrAmt; at the root
    // it is what it was.
    let root = restated(&reader, b"8=FIX.4.2|35=J|70=A1|119=1000|10=0|");
    assert_eq!(root.by_tag(119).unwrap(), &Scalar::from(1000.0_f64));
    assert_eq!(root.get_by_tag(737), None);
    let grouped = restated(
        &reader,
        b"8=FIX.4.2|35=J|70=A1|78=1|NoAllocs[0].79=ACCT|NoAllocs[0].119=1000|10=0|",
    );
    assert_eq!(
        grouped
            .by_path(&path("allocgrp[0].allocsettlcurramt"))
            .unwrap(),
        &Scalar::from(1000.0_f64)
    );
    assert_eq!(
        grouped.by_path(&path("allocgrp[0].settlcurramt")).unwrap(),
        &Scalar::from(1000.0_f64),
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
    assert_eq!(floor.by_tag(1138).unwrap(), &Scalar::from(50.0_f64));
    assert_eq!(floor.by_tag(111).unwrap(), &Scalar::from(50.0_f64));
}

#[test]
fn a_batch_read_lands_at_the_newest_version_when_asked() {
    let registry = super::committed_registry();
    let newest = registry.newest().expect("a dated dictionary").version();
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("the fixed schema");
    // A stage is a call: the restatement composes over the message stream
    // between the parse and the batch.
    let rows = |on: bool| {
        let messages = codec.parse_lines([REPORT]).map(move |held| {
            if on {
                held.and_then(FixMsg::into_latest)
            } else {
                held
            }
        });
        codec
            .arrow_reader(schema.clone(), messages)
            .expect("a reader")
            .map(|batch| batch.expect("a batch"))
            .collect::<Vec<_>>()
    };
    let column = |batches: &[arrow_array::RecordBatch], tag: i32| {
        use arrow_array::cast::AsArray;
        let at = super::tag_index(&batches[0], tag);
        batches[0].column(at).as_string::<i32>().value(0).to_owned()
    };
    let restated = rows(true);
    assert_eq!(column(&restated, VERSION_TAG), newest.to_string());
    let read = rows(false);
    assert_eq!(column(&read, VERSION_TAG), "4.2");

    // The line door composes the same way.
    let line = TextLine::new(0, TextBytes::from_bytes(REPORT).expect("a page"));
    let message = codec
        .parse_text_line(&line)
        .expect("a readable line")
        .next()
        .expect("one message")
        .and_then(FixMsg::into_latest)
        .expect("a message");
    assert_eq!(message.version(), Some(newest));
    assert_eq!(text(&message, 528), Some("A"));
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
        .set_replacements(&[FixReplacement::new(version("4.3"))
            .with_when("A")
            .with_fills([FixFill::Field {
                tag: 528,
                value: FixFillSource::Constant("P".into()),
            }])])
        .expect("a rule");
    registry.update(rule80a).expect("updated");
    assert!(
        registry
            .get_definition(FixCategory::Groups, "parties", None)
            .is_some(),
        "the catalog is untouched"
    );
    let reader = FixCodec::new(Arc::new(registry));
    let latest = restated(&reader, b"8=FIX.4.2|35=D|11=A|47=A|109=C1|10=0|");
    assert_eq!(text(&latest, 528), Some("P"));
    assert_eq!(text(&latest, 47), Some("A"));
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
    assert_eq!(text(&both, 18), Some("G R"));
    assert_eq!(integer(&both, 835), Some(1));
    assert_eq!(integer(&both, 840), Some(1));
    // R is what FIX 5.0 retired for a peg price type, and the chain reaches
    // it in the same pass.
    assert_eq!(integer(&both, 1094), Some(5));
    let trailing = restated(&reader, b"8=FIX.4.2|35=D|11=A|18=T G|10=0|");
    assert_eq!(text(&trailing, 18), Some("R G"));
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
    assert_eq!(anomalies(&latest), Vec::<String>::new());
    // A counter stating none, with a party to make.
    let none = restated(&reader, b"8=FIX.4.4|35=D|11=A|453=0|76=BRKR|10=0|");
    assert_eq!(occurrences(&none, "parties").len(), 1);
    assert_eq!(integer(&none, 453), Some(1));
    assert_eq!(anomalies(&none), Vec::<String>::new());
}

/// The committed `parties` definition as a converted row declares it, holding
/// `value`, beside `ExecBroker(76)` `BRKR`.
fn declared_parties(list: Field, value: Scalar) -> FixMsg {
    let registry = super::committed_registry();
    let broker = registry.field_by_tag(76).expect("ExecBroker").clone();
    let children = vec![list, broker];
    let root = DataType::from_fields(children)
        .expect("a struct")
        .required_field("8");
    let value = Scalar::from_record([("parties", value), ("execbroker", Scalar::from("BRKR"))])
        .expect("a record");
    FixMsg::with_registry(registry, root, value).expect("a message")
}

#[test]
fn a_declared_group_stating_no_occurrence_is_opened_by_a_fill_into_it() {
    let registry = super::committed_registry();
    let mut list = registry
        .get_definition(FixCategory::Groups, "parties", None)
        .expect("parties")
        .clone();
    list.set_nullable(true);
    let latest = declared_parties(list, Scalar::Null)
        .into_latest()
        .expect("restated");
    assert_eq!(
        names(&latest),
        ["parties", "execbroker", "nopartyids", "version"]
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
    let definition = registry
        .get_definition(FixCategory::Groups, "parties", None)
        .expect("parties");
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
    let latest = declared_parties(list, Scalar::from_sequence([occurrence]))
        .into_latest()
        .expect("restated");
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
