//! `rust/fix/src/msg.rs` `stated_securityids`: the identifiers a message
//! states under the wire's own fields and under the unmapped fields whose
//! names name a source, and what a settle drops of them. The set holds them
//! in the order of their keys as `src:type` spells them.

use super::{committed_registry, fixed_codec};
use yggdryl_fix::FixMsg;
use yggdryl_market::graph::Market;
use yggdryl_market::{IdKey, IdType};

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
    crate::install::installed();
    // A capture spells the instrument under bridge names no dictionary
    // holds: the ISIN is a stated entry, and the Valor a Swiss ISIN embeds
    // is derived beside it.
    let held = parsed(
        b"8=FIX.4.4|35=8|37=O1|55=ABBN|#CFICODE=ESVTFR|#ISINCODE=CH0012221716|#LASTMKT=XSWX|10=0|",
    );
    assert_eq!(
        ids(&held),
        [
            "derived:valor=1222171",
            "isin=CH0012221716",
            "valor=1222171"
        ]
    );
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
    crate::install::installed();
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|cusip_code=037833100|#SEDOLCODE=0263494|10=0|");
    assert_eq!(ids(&held), ["cusip=037833100", "sedol=0263494"]);
}

#[test]
fn an_invalid_value_states_nothing_records_an_anomaly_and_still_re_emits() {
    crate::install::installed();
    // A bad check digit is a CUSIP of rank zero: the value is stated as it
    // is, nothing is refused, and the field stays on the row as it arrived.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|#CUSIPCODE=037833101|10=0|");
    assert_eq!(ids(&held), ["cusip=037833101"]);
    assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
    assert_eq!(
        held.get_by_name("cusipcode")
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("037833101".to_owned())
    );
    // A value of the wrong shape is no CUSIP: nothing is guessed, the
    // refusal is kept, and the field stays on the row as it arrived.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|#CUSIPCODE=03783310|10=0|");
    assert!(ids(&held).is_empty());
    let dropped = anomalies(&held);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].0, "cusipcode");
    assert_eq!(
        held.get_by_name("cusipcode")
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("03783310".to_owned())
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

/// Only a key naming a security type is this set's to refuse: an
/// operation's or a party's identifier whose value its type refuses stays
/// on the wire, unread, and is no anomaly.
#[test]
fn an_unmapped_operation_or_party_key_whose_value_its_type_refuses_is_no_anomaly() {
    crate::install::installed();
    use yggdryl_market::graph::Operation;

    let long = "R".repeat(70);
    let held = parsed(
        format!(
            "8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|BRIDGEORDERID={long}|firm.x.UserID={long}|10=0|"
        )
        .as_bytes(),
    );
    assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
    assert!(ids(&held).is_empty(), "{:?}", ids(&held));
    for set in [held.get_identifiers(), held.get_partyids()] {
        assert!(set.iter().all(|id| id.value() != long), "{set}");
    }
    assert_eq!(
        held.get_by_name("bridgeorderid")
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some(long)
    );
}

#[test]
fn a_name_that_names_no_own_source_states_nothing() {
    crate::install::installed();
    // A leg's ISIN is another instrument's, and a ticker is never a source.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|LegISIN=US0378331005|#TICKER=AAPL|10=0|");
    assert!(ids(&held).is_empty(), "{:?}", ids(&held));
    assert!(anomalies(&held).is_empty());
}

/// Another instrument's word refuses a security type wherever it stands
/// before the type, a namespace in front of it or not: a bridge's
/// underlying, leg or contra code is no code of this instrument.
#[test]
fn another_instruments_code_behind_a_namespace_states_nothing() {
    crate::install::installed();
    for entry in [
        "OMS_UnderlyingISIN=US0378331005",
        "FIX.LegISIN=US0378331005",
        "firm.x.ContraCUSIP=037833100",
        // The wire's own underlying group states no security of this
        // instrument either.
        "711=1|311=AAPL|309=US0378331005|305=4",
    ] {
        let line = format!("8=FIX.4.4|35=D|11=A1|55=AAPL|{entry}|10=0|");
        let held = parsed(line.as_bytes());
        assert!(ids(&held).is_empty(), "{entry}: {:?}", ids(&held));
        assert!(
            anomalies(&held).is_empty(),
            "{entry}: {:?}",
            anomalies(&held)
        );
    }
}

/// SIX's source spelling `X-SWX-VALOR` names the Valor number, and its
/// symbol is the exchange symbol: each a type of this instrument's own.
#[test]
fn a_vendors_source_spelling_and_a_six_symbol_name_this_instruments_own_types() {
    crate::install::installed();
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=HOLN|22=X-SWX-VALOR|48=1221405|SIXSymbol=HOLN|10=0|");
    assert_eq!(ids(&held), ["exchsymb=HOLN", "valor=1221405"]);
    assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
}

#[test]
fn the_wire_leads_and_an_unmapped_code_is_a_statement_of_its_own_source() {
    crate::install::installed();
    // The wire's field and the bridge's name are two sources, each stating
    // its own entry, and the wire's answers the type.
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|ISIN=US0378331005|10=0|");
    assert_eq!(
        ids(&held),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    assert!(anomalies(&held).is_empty());
    assert_eq!(
        held.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    // `#ISINCODE` is the crated column, a view of the set: the code the
    // wire's entry holds states nothing new.
    let viewed =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|#ISINCODE=US0378331005|10=0|");
    assert_eq!(
        ids(&viewed),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    assert!(anomalies(&viewed).is_empty());

    // A different code the view states under the wire's own key - the
    // base key - is dropped beside an anomaly: the wire states the type's
    // answer first, and the codes derived from it.
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|#ISINCODE=US5949181045|10=0|");
    assert_eq!(
        held.get_securityids().get(&IdType::Isin),
        Some("US0378331005"),
        "the wire's code answers the type, and the codes derived from it"
    );
    assert_eq!(
        ids(&held),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    let dropped = anomalies(&held);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(dropped[0].1.contains("US5949181045"), "{dropped:?}");

    // A bridge's namespace stands beside the wire as a source of its own,
    // and the wire's code still answers the type.
    for line in [
        &b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|ABC.ISIN=US5949181045|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|DBI_ISIN=US5949181045|10=0|",
    ] {
        let held = parsed(line);
        assert!(
            ids(&held)
                .iter()
                .any(|id| id.ends_with(":isin=US5949181045")),
            "{:?}",
            ids(&held)
        );
        assert_eq!(
            held.get_securityids().get(&IdType::Isin),
            Some("US0378331005"),
            "{:?}",
            ids(&held)
        );
        assert_eq!(held.get_isincode(), Some("US0378331005"));
        assert_eq!(
            held.get_securityids().get_from(&IdKey::new(
                yggdryl_market::IdSource::Derived,
                IdType::Cusip
            )),
            Some("037833100"),
            "{:?}",
            ids(&held)
        );
    }
}

/// `SecurityID(48)` fills its type first: a `SecAltIDGrp(454)` occurrence
/// restating its code is the same entry, and one stating another code under
/// that type states nothing and is kept as an anomaly.
#[test]
fn an_alternate_restating_the_primarys_type_fills_nothing_and_a_different_code_is_an_anomaly() {
    crate::install::installed();
    let held = parsed(
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|454=3|455=US0378331005|456=4|455=CH0012221716|456=4|455=US5949181045|456=4|10=0|",
    );
    assert_eq!(
        ids(&held),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    assert_eq!(
        anomalies(&held),
        [
            (
                "secaltids",
                "states isin=CH0012221716 where isin=US0378331005 is already stated"
            ),
            (
                "secaltids",
                "states isin=US5949181045 where isin=US0378331005 is already stated"
            )
        ]
    );
    let wire = String::from_utf8(held.into_bytes(b'|')).expect("a text wire");
    assert!(
        wire.contains(
            "|454=3|455=US0378331005|456=4|455=CH0012221716|456=4|455=US5949181045|456=4|"
        ),
        "every occurrence stays on the wire: {wire}"
    );
}

/// The fixed row narrowed to its identity, `columns` and the entries.
fn narrow_root(columns: &[&str]) -> yggdryl::Field {
    let schema = yggdryl_fix::fix_schema(&committed_registry(), "fix").expect("the fixed row");
    yggdryl::StructType::from_fields(
        [
            "beginstring",
            "msgtype",
            "transunix",
            "creaunix",
            "hashcode",
            "crosshashcode",
            "uuid",
            "crossuuid",
        ]
        .iter()
        .chain(columns)
        .chain(&["fixentries"])
        .map(|name| schema.fields()[schema.index_of(name).expect(name)].clone()),
    )
    .map(yggdryl::DataType::from)
    .expect("a narrow root")
    .required_field("fix")
}

/// `held` written to the fixed row narrowed to its identity, `views` and
/// the entries - no `securityids` column, so a view is all the row says of
/// the set - and read back.
fn narrow_round_trip(held: &FixMsg, views: &[&str]) -> FixMsg {
    let narrow = narrow_root(views);
    let row = held.into_row(&narrow).expect("the narrow row");
    FixMsg::from_row(committed_registry(), &narrow, &row).expect("the row reads")
}

/// A row narrowed to the `isincode` view without the `securityids` column
/// reads the view back as the identifier it viewed, not as another source's.
#[test]
fn a_narrow_row_reads_the_isin_view_back_as_the_identifier_it_viewed() {
    crate::install::installed();
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=1|22=4|48=US0378331005|10=0|");
    assert_eq!(
        ids(&held),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    let back = narrow_round_trip(&held, &["isincode"]);
    assert_eq!(ids(&back), ids(&held));
    assert!(anomalies(&back).is_empty(), "{:?}", anomalies(&back));
}

/// A view is checked against everything the reading holds: the code a
/// bridge's own key states, and the pair a symbol derives, are no new
/// statement of the view's own.
#[test]
fn a_narrow_row_reads_a_view_of_a_bridged_or_derived_code_back_as_that_code() {
    crate::install::installed();
    let bridged = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=1|DBI_ISIN=US0378331005|10=0|");
    assert_eq!(
        ids(&bridged),
        [
            "cusip=037833100",
            "dbi:isin=US0378331005",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    let back = narrow_round_trip(&bridged, &["isincode", "metadata"]);
    assert_eq!(ids(&back), ids(&bridged));
    assert!(anomalies(&back).is_empty(), "{:?}", anomalies(&back));

    let detected = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR/USD|54=1|38=1|10=0|");
    assert_eq!(ids(&detected), ["derived:forex=EUR/USD", "forex=EUR/USD"]);
    let back = narrow_round_trip(&detected, &["forexcode"]);
    assert_eq!(ids(&back), ids(&detected));
    assert!(anomalies(&back).is_empty(), "{:?}", anomalies(&back));
}

/// A view of the code `Symbol(55)`'s own shape derives - a FIGI, an ISIN, an
/// instrument key's ISIN, a Bloomberg identifier - is a view of that
/// derivation, as the detected pair's is: it reads back as derived.
#[test]
fn a_narrow_row_reads_a_view_of_a_code_the_symbol_derives_back_as_derived() {
    crate::install::installed();
    for (line, view, expected) in [
        (
            &b"8=FIX.4.4|35=D|11=A1|55=BBG000BLNNH6|54=1|38=1|10=0|"[..],
            "figicode",
            &["derived:figi=BBG000BLNNH6", "figi=BBG000BLNNH6"][..],
        ),
        (
            b"8=FIX.4.4|35=D|11=A1|55=US0378331005|54=1|38=1|10=0|",
            "isincode",
            &[
                "cusip=037833100",
                "derived:cusip=037833100",
                "derived:isin=US0378331005",
                "isin=US0378331005",
            ],
        ),
        (
            b"8=FIX.4.4|35=D|11=A1|55=CH0012214059_XSWX_CHF|54=1|38=1|10=0|",
            "isincode",
            &[
                "derived:isin=CH0012214059",
                "derived:valor=1221405",
                "isin=CH0012214059",
                "valor=1221405",
            ],
        ),
        (
            b"8=FIX.4.4|35=D|11=A1|55=HOLN SW Equity|54=1|38=1|10=0|",
            "bloombergcode",
            &[
                "bloomberg=HOLN SW Equity",
                "derived:bloomberg=HOLN SW Equity",
            ],
        ),
    ] {
        let held = parsed(line);
        assert_eq!(ids(&held), expected, "{view}");
        let back = narrow_round_trip(&held, &[view]);
        assert_eq!(ids(&back), ids(&held), "{view}");
        assert!(
            anomalies(&back).is_empty(),
            "{view}: {:?}",
            anomalies(&back)
        );
    }
}

/// A view a caller writes beside a bridge's code of its type replaces the
/// type's answer and keeps the bridge's code as evidence: the narrow row
/// reads it back as that answer, the bridge's key read from the metadata
/// again beside it, so `get` answers the row's cell and a second row writes
/// it again.
#[test]
fn a_view_a_caller_writes_beside_a_bridges_code_replaces_the_answer_and_keeps_the_source() {
    crate::install::installed();
    let dbi = IdKey::new("dbi".parse().expect("a word"), IdType::Isin);
    let held = parsed(DROPPED_BRIDGE_ISIN);
    assert_eq!(held.get_securityids().get_from(&dbi), Some("CH0012214059"));
    assert_eq!(
        held.get_isincode(),
        Some("CH0012214059"),
        "the bridge's code fills the answer"
    );
    assert_eq!(anomalies(&held).len(), 1, "{:?}", anomalies(&held));
    let held = viewed_by_a_caller(DROPPED_BRIDGE_ISIN);
    assert_eq!(held.get_isincode(), Some("US0378331005"));
    assert_eq!(
        held.get_securityids().get_from(&dbi),
        Some("CH0012214059"),
        "the source stays"
    );
    let narrow = narrow_root(&["isincode", "metadata"]);
    let row = held.into_row(&narrow).expect("the narrow row");
    let back = FixMsg::from_row(committed_registry(), &narrow, &row).expect("the row reads");
    assert_eq!(
        ids(&back),
        [
            "cusip=037833100",
            "dbi:isin=CH0012214059",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    assert_eq!(ids(&back), ids(&held));
    assert_eq!(back.get_isincode(), Some("US0378331005"));
    assert_eq!(back.into_row(&narrow).expect("the second row"), row);
    assert_eq!(anomalies(&back), anomalies(&held), "the same code dropped");
}

/// A view no reading holds is the row's statement of its type's answer: it
/// reads back as the caller's write of it, and a write reaching
/// `SecurityIDSource(22)` answers on the read-back as on the message it was
/// written from - the statement stands and the fields fill beside it.
#[test]
fn a_view_no_reading_holds_is_stated_once_and_a_restatement_keeps_it() {
    crate::install::installed();
    let held = viewed_by_a_caller(DROPPED_BRIDGE_ISIN);
    let mut back = narrow_round_trip(&held, &["isincode", "metadata"]);
    assert_eq!(ids(&back), ids(&held));
    let mut stated = held.clone();
    for message in [&mut stated, &mut back] {
        message
            .set(22, yggdryl::Scalar::from("4"))
            .expect("the source writes");
    }
    assert_eq!(
        ids(&back),
        ids(&stated),
        "the statement stands and the fields fill beside it"
    );
    assert_eq!(back.get_isincode(), Some("US0378331005"));
    assert_eq!(anomalies(&back), anomalies(&held), "the same code dropped");
}

/// A view of the code `Symbol(55)` derives, where nothing else states its
/// type, is that derivation and never a statement: a later write stating
/// another code of its type stands alone, as it does on the parse.
#[test]
fn a_view_of_the_symbols_derivation_is_never_stated_beside_a_later_code_of_its_type() {
    crate::install::installed();
    let mut held = parsed(b"8=FIX.4.4|35=D|11=A1|55=BBG000BLNNH6|54=1|38=1|10=0|");
    let mut back = narrow_round_trip(&held, &["figicode"]);
    for message in [&mut held, &mut back] {
        message
            .set_many([
                (22, yggdryl::Scalar::from("S")),
                (48, yggdryl::Scalar::from("BBG000BPH459")),
            ])
            .expect("the security writes");
    }
    assert_eq!(ids(&held), ["figi=BBG000BPH459"]);
    assert_eq!(ids(&back), ids(&held));
    assert!(anomalies(&back).is_empty(), "{:?}", anomalies(&back));
}

/// What `Symbol(55)` derives follows the symbol: a written symbol takes the
/// derivation a view read back as with it - the pair at once, as detection
/// takes a pair back, a code at the next restatement of the identifiers -
/// exactly as the same writes do on the parse.
#[test]
fn a_derivation_a_view_read_back_as_follows_a_written_symbol() {
    crate::install::installed();
    let mut held = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR/USD|54=1|38=1|10=0|");
    let mut back = narrow_round_trip(&held, &["forexcode"]);
    for message in [&mut held, &mut back] {
        message
            .set(55, yggdryl::Scalar::from("AAPL"))
            .expect("the symbol writes");
    }
    assert!(ids(&held).is_empty(), "{:?}", ids(&held));
    assert_eq!(ids(&back), ids(&held));

    let mut held = parsed(b"8=FIX.4.4|35=D|11=A1|55=BBG000BLNNH6|54=1|38=1|10=0|");
    let mut back = narrow_round_trip(&held, &["figicode"]);
    for message in [&mut held, &mut back] {
        message
            .set(55, yggdryl::Scalar::from("AAPL"))
            .expect("the symbol writes");
        message
            .set_many([
                (22, yggdryl::Scalar::from("4")),
                (48, yggdryl::Scalar::from("US0378331005")),
            ])
            .expect("the security writes");
    }
    assert_eq!(
        ids(&held),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    assert_eq!(ids(&back), ids(&held));
}

/// One message of the differential corpus: the message a narrow row is
/// written from, the view it is about, and what that row documents losing.
/// A view the row's own reading answers, or the derivation of its symbol,
/// loses nothing; any other reads back as the row's statement, from `base`,
/// leading its type.
struct Viewed {
    /// What the message is, for a failure to name.
    name: &'static str,
    /// The message: a parse, and what a caller or a walk did to it.
    message: fn() -> FixMsg,
    view: &'static str,
    /// Each identifier only a view carried, as the message holds it, which
    /// reads back as its type's answer: a named source's key is lost and the
    /// base key it filled stays, and a derivation reads back as the
    /// statement of its code.
    lost: &'static [&'static str],
}

const WIRE_ISIN: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=1|22=4|48=US0378331005|10=0|";
const DROPPED_BRIDGE_ISIN: &[u8] =
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=1|DBI.ISIN=CH0012214059|DBI_ISIN=US0378331005|10=0|";
const NO_ISIN: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=1|10=0|";

/// `held` settled, as the next write settles a caller's verbs: restating
/// `ClOrdID(11)` reaches no fact.
fn settled(mut held: FixMsg) -> FixMsg {
    let clordid = held.get_by_tag(11).expect("a ClOrdID");
    held.set(11, clordid).expect("11 writes");
    held
}

/// The parse of `line` with `US0378331005` inserted from `src` - an ISIN no
/// field states, which a row narrowed to its views carries only as
/// `isincode`.
fn inserted(line: &[u8], src: &str) -> FixMsg {
    let mut held = parsed(line);
    assert!(
        held.insert_securityid(
            yggdryl_market::Identifier::new(
                IdKey::new(src.parse().expect("a source"), IdType::Isin),
                "US0378331005"
            )
            .expect("an ISIN")
        )
        .expect("inserts")
    );
    settled(held)
}

/// The parse of `line` with `US0378331005` written to its `isincode` view,
/// replacing the answer of its type.
fn viewed_by_a_caller(line: &[u8]) -> FixMsg {
    let mut held = parsed(line);
    held.set("isincode", yggdryl::Scalar::from("US0378331005"))
        .expect("the view writes");
    settled(held)
}

/// An amend stating the unknown `ZZ0000000008` walked after the order
/// stating `US0378331005`: the lifecycle takes the `ZZ` ISIN out and
/// inserts its predecessor's, which no field of the amend states.
fn walked_past_an_unknown_isin() -> FixMsg {
    let codec = fixed_codec(committed_registry());
    let walked: Vec<FixMsg> = codec
        .lifecycle([
            codec.parse_fix_line(
                b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=1|22=4|48=US0378331005|10=0|",
            ),
            codec.parse_fix_line(
                b"8=FIX.4.4|35=G|49=S|56=T|34=2|52=20260102-10:15:31|11=A2|41=A1|55=AAPL|54=1|38=2|22=4|48=ZZ0000000008|10=0|",
            ),
        ])
        .collect::<yggdryl::Result<_>>()
        .expect("the walk");
    let amend = walked.into_iter().last().expect("the amend");
    assert_eq!(
        ids(&amend),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ],
        "the walk replaced the unknown ISIN"
    );
    amend
}

/// The wire's ISIN replaced by a caller: the `fix` code removed and
/// another inserted under the same key.
fn replaced_by_a_caller() -> FixMsg {
    let mut held = parsed(WIRE_ISIN);
    assert!(
        held.remove_securityid(&IdKey::base(IdType::Isin))
            .expect("removes")
    );
    assert!(
        held.insert_securityid(
            yggdryl_market::Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")
                .expect("an ISIN")
        )
        .expect("inserts")
    );
    settled(held)
}

/// A FIGI derived on the parse, as a lifecycle's `SecurityIdRegistry`
/// derives what it learned.
fn derived_figi() -> FixMsg {
    let mut held = parsed(WIRE_ISIN);
    assert!(held.derive_securityid(&IdType::Figi, "BBG000B9XRY4"));
    held
}

/// The messages a view must read back as the parse holds them, or as the
/// documented losses say: a wire ISIN, a FIGI- and an ISIN-shaped symbol, a
/// detected pair, a venue's instrument key on the wire and in a bridge's
/// key, a bridge code dropped against an earlier one, an ISIN only a caller
/// stated - under the wire's source and under a bridge's - and beside a
/// bridge's other code, the wire's ISIN replaced by a walk and by a caller,
/// and a FIGI derived.
const VIEWED: [Viewed; 13] = [
    Viewed {
        name: "a wire ISIN",
        message: || parsed(WIRE_ISIN),
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "a FIGI symbol",
        message: || parsed(b"8=FIX.4.4|35=D|11=A1|55=BBG000BLNNH6|54=1|38=1|10=0|"),
        view: "figicode",
        lost: &[],
    },
    Viewed {
        name: "an ISIN symbol",
        message: || parsed(b"8=FIX.4.4|35=D|11=A1|55=US0378331005|54=1|38=1|10=0|"),
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "a detected pair",
        message: || parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR/USD|54=1|38=1|10=0|"),
        view: "forexcode",
        lost: &[],
    },
    Viewed {
        name: "an instrument key on the wire",
        message: || {
            parsed(b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|38=5|40=2|22=ULLINKINSTRUMENTID|48=dbi;CH0012214059_XSWX_CHF|10=0|")
        },
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "an instrument key in a bridge's key",
        message: || {
            parsed(b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|38=5|40=2|OMS_InstrumentID=dbi;CH0012214059_XSWX_CHF|10=0|")
        },
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "a dropped bridge code",
        message: || parsed(DROPPED_BRIDGE_ISIN),
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "a caller's ISIN",
        message: || inserted(NO_ISIN, "base"),
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "a caller's ISIN from dbi",
        message: || inserted(NO_ISIN, "dbi"),
        view: "isincode",
        lost: &["dbi:isin=US0378331005"],
    },
    Viewed {
        name: "a caller's ISIN beside a bridge's",
        message: || viewed_by_a_caller(DROPPED_BRIDGE_ISIN),
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "the wire's unknown ISIN replaced by a walk",
        message: walked_past_an_unknown_isin,
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "the wire's ISIN replaced by a caller",
        message: replaced_by_a_caller,
        view: "isincode",
        lost: &[],
    },
    Viewed {
        name: "a derived FIGI",
        message: derived_figi,
        view: "figicode",
        lost: &["derived:figi=BBG000B9XRY4"],
    },
];

/// The four views of the security identifiers.
const VIEWS: [&str; 4] = ["isincode", "bloombergcode", "figicode", "forexcode"];

/// A write sequence, applied alike to a parse and to its read-back, and
/// whether it writes nothing but fields the identifiers are read off - the
/// wire's security fields, the symbol.
type Writes = (&'static str, bool, fn(&mut FixMsg));

/// The writes a read-back must answer as the parse does: none, the wire's
/// code and source, the symbol, and a caller's verbs on the set - the
/// lifecycle's replacement of an unknown ISIN among them - each followed
/// by a write that restates the identifiers.
const WRITES: [Writes; 10] = [
    ("nothing", false, |_| {}),
    ("48 another ISIN", true, |held| {
        held.set(48, yggdryl::Scalar::from("CH0012214059"))
            .expect("48 writes");
    }),
    ("48 null", true, |held| {
        held.set(48, yggdryl::Scalar::Null).expect("48 clears");
    }),
    ("22", true, |held| {
        held.set(22, yggdryl::Scalar::from("4")).expect("22 writes");
    }),
    ("55 another symbol", true, |held| {
        held.set(55, yggdryl::Scalar::from("MSFT"))
            .expect("55 writes");
    }),
    ("55 another pair", true, |held| {
        held.set(55, yggdryl::Scalar::from("GBP/USD"))
            .expect("55 writes");
    }),
    ("the ISIN removed, then 22", false, |held| {
        if held.get_securityids().contains_kind(&IdType::Isin) {
            assert!(
                held.remove_securityid(&IdKey::base(IdType::Isin))
                    .expect("removes")
            );
        }
        held.set(22, yggdryl::Scalar::from("4")).expect("22 writes");
    }),
    ("the ISIN replaced, then 22", false, |held| {
        if held.get_securityids().contains_kind(&IdType::Isin) {
            assert!(
                held.remove_securityid(&IdKey::base(IdType::Isin))
                    .expect("removes")
            );
            held.insert_securityid(
                yggdryl_market::Identifier::new(IdKey::base(IdType::Isin), "CH0012221716")
                    .expect("an ISIN"),
            )
            .expect("inserts");
        }
        held.set(22, yggdryl::Scalar::from("4")).expect("22 writes");
    }),
    ("the set cleared, then 22", false, |held| {
        held.set_securityids(yggdryl_market::Identifiers::new(), true)
            .expect("clears");
        held.set(22, yggdryl::Scalar::from("4")).expect("22 writes");
    }),
    ("the set cleared, then 48", false, |held| {
        held.set_securityids(yggdryl_market::Identifiers::new(), true)
            .expect("clears");
        held.set(48, yggdryl::Scalar::from("CH0012214059"))
            .expect("48 writes");
    }),
];

/// One identifier spelled `src:type=value`.
fn identifier(spelled: &str) -> yggdryl_market::Identifier {
    let (src, rest) = spelled.split_once(':').expect("src:type=value");
    let (kind, value) = rest.split_once('=').expect("type=value");
    yggdryl_market::Identifier::new(
        IdKey::new(
            src.parse().expect("a source"),
            kind.parse().expect("a type"),
        ),
        value,
    )
    .expect("an identifier")
}

/// `held`'s identifiers sorted.
fn sorted_ids(held: &FixMsg) -> Vec<String> {
    let mut held = ids(held);
    held.sort();
    held
}

/// `held`'s identifiers sorted, as a narrow row of `viewed` reads them
/// back: each one lost gone, the base key it filled standing.
fn read_back_ids(held: &FixMsg, viewed: &Viewed) -> Vec<String> {
    let mut read: Vec<String> = ids(held)
        .into_iter()
        .filter(|id| !viewed.lost.contains(&id.as_str()))
        .collect();
    read.sort();
    read
}

/// `held` as its narrow row reads it back, stated through a caller's verbs:
/// each named source lost removed as `remove_securityid` removes it, the
/// base key it filled staying, each derivation lost stated as its code -
/// inserted under the base key, which takes the derivation back - and the
/// message settled. What a write does to it is what the same write does to
/// the read-back.
fn as_read_back(held: &FixMsg, viewed: &Viewed) -> FixMsg {
    let mut held = held.clone();
    if viewed.lost.is_empty() {
        return held;
    }
    for id in viewed.lost.iter().map(|id| identifier(id)) {
        if id.src() == &yggdryl_market::IdSource::Derived {
            assert!(
                held.insert_securityid(
                    yggdryl_market::Identifier::new(IdKey::base(id.kind().clone()), id.value())
                        .expect("an identifier")
                )
                .expect("inserts")
            );
        } else {
            assert!(held.remove_securityid(id.key()).expect("removes"));
        }
    }
    settled(held)
}

/// What `held` answers for the type a view column views: `get_isincode`
/// for the ISIN, the set's `get` for the others.
fn answered(held: &FixMsg, view: &str) -> Option<String> {
    let kind = match view {
        "isincode" => return held.get_isincode().map(ToOwned::to_owned),
        "bloombergcode" => IdType::Bloomberg,
        "figicode" => IdType::Figi,
        "forexcode" => IdType::Forex,
        other => panic!("{other} is no view"),
    };
    held.get_securityids().get(&kind).map(ToOwned::to_owned)
}

/// A row narrowed to its views reads back as the parse it was written from,
/// and answers every write the parse answers alike: one view, every view,
/// every view beside the `securityids` column, whichever comes first. A
/// view the row's own reading answers, or its symbol derives, loses
/// nothing; any other reads back as the row's statement of its type's
/// answer - a named source only the view carried lost, the base key it
/// filled standing, and a derivation stated as its code - so `get` answers
/// every view cell after the round trip, a second row writes the same
/// cells, and every write answers as on the parse with exactly those
/// losses stated through a caller's verbs. A `securityids` column is a crate column the row
/// states, so the set it carries is the row's word, which no write of a
/// field the identifiers are read off restates - the wire's security
/// fields, the symbol: there the read-back keeps the set the parse held
/// before the write.
#[test]
fn a_narrow_row_answers_every_write_as_the_parse_it_was_written_from() {
    crate::install::installed();
    let every: Vec<&str> = VIEWS.iter().copied().chain(["metadata"]).collect();
    let before: Vec<&str> = VIEWS
        .iter()
        .copied()
        .chain(["securityids", "metadata"])
        .collect();
    let after: Vec<&str> = ["securityids"]
        .into_iter()
        .chain(VIEWS)
        .chain(["metadata"])
        .collect();
    let mut failures = Vec::new();
    for viewed in &VIEWED {
        let name = viewed.name;
        let held = (viewed.message)();
        let stated_back = as_read_back(&held, viewed);
        if sorted_ids(&stated_back) != read_back_ids(&held, viewed) {
            failures.push(format!(
                "{name}: the losses stated\n  stated {:?}\n  losses {:?}",
                sorted_ids(&stated_back),
                read_back_ids(&held, viewed)
            ));
        }
        let one = [viewed.view, "metadata"];
        for (columns, stated) in [
            (&one[..], false),
            (&every[..], false),
            (&before[..], true),
            (&after[..], true),
        ] {
            let narrow = narrow_root(columns);
            let row = held.into_row(&narrow).expect("the narrow row");
            let read =
                FixMsg::from_row(committed_registry(), &narrow, &row).expect("the row reads");
            let again = read.into_row(&narrow).expect("the second row");
            let cells = row.as_sequence().expect("a row");
            for view in VIEWS.iter().filter(|view| columns.contains(view)) {
                let at = narrow
                    .fields()
                    .iter()
                    .position(|child| child.name() == *view)
                    .expect("the view column");
                let cell = cells[at].as_str().map(ToOwned::to_owned);
                if answered(&read, view) != cell
                    || again.as_sequence().expect("a row")[at] != cells[at]
                {
                    failures.push(format!(
                        "{name} {columns:?} {view}: row {cell:?}, read back {:?}, again {:?}",
                        answered(&read, view),
                        again.as_sequence().expect("a row")[at]
                    ));
                }
            }
            let expected = if stated {
                sorted_ids(&held)
            } else {
                read_back_ids(&held, viewed)
            };
            if sorted_ids(&read) != expected {
                failures.push(format!(
                    "{name} {columns:?} read back:\n  losses {expected:?}\n  back   {:?}",
                    sorted_ids(&read)
                ));
            }
            for (write, fields, apply) in WRITES {
                let parse = if stated { &held } else { &stated_back };
                let (mut written, mut back) = (parse.clone(), read.clone());
                apply(&mut written);
                apply(&mut back);
                let answered = if stated && fields { parse } else { &written };
                let (expected, actual) = (sorted_ids(answered), sorted_ids(&back));
                if expected != actual {
                    failures.push(format!(
                        "{name} {columns:?} {write}:\n  parse {expected:?}\n  back  {actual:?}"
                    ));
                }
            }
        }
        // Where the views stand beside the set makes no difference.
        let (first, last) = (
            narrow_round_trip(&held, &before),
            narrow_round_trip(&held, &after),
        );
        for (write, _, apply) in WRITES {
            let (mut first, mut last) = (first.clone(), last.clone());
            apply(&mut first);
            apply(&mut last);
            if ids(&first) != ids(&last) {
                failures.push(format!(
                    "{name} column order, {write}:\n  views first {:?}\n  views last  {:?}",
                    ids(&first),
                    ids(&last)
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A caller writing a view states the set through its type's base key: a
/// code the set holds moves nothing, another replaces the type's answer -
/// taking back what the replaced ISIN derived - and a removed view removes
/// the type, the set staying the caller's word, which a later write of the
/// wire's fields only fills where it states nothing.
#[test]
fn a_written_view_states_the_set_through_the_callers_verbs() {
    crate::install::installed();
    let mut held = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=1|22=4|48=US0378331005|10=0|");
    let wire = [
        "cusip=037833100",
        "derived:cusip=037833100",
        "isin=US0378331005",
    ];
    held.set("isincode", yggdryl::Scalar::from("US0378331005"))
        .expect("the view writes");
    assert_eq!(ids(&held), wire, "a code the set holds moves nothing");
    held.set("isincode", yggdryl::Scalar::from("CH0012214059"))
        .expect("the view writes");
    assert_eq!(
        ids(&held),
        [
            "derived:valor=1221405",
            "isin=CH0012214059",
            "valor=1221405"
        ]
    );
    held.set(48, yggdryl::Scalar::from("US5949181045"))
        .expect("48 writes");
    assert_eq!(
        ids(&held),
        [
            "derived:valor=1221405",
            "isin=CH0012214059",
            "valor=1221405"
        ],
        "the wire's code under the key the caller stated fills nothing"
    );
    held.remove("isincode").expect("the view clears");
    assert!(ids(&held).is_empty(), "{:?}", ids(&held));
}

/// The cells FX detection wrote are content of the row - `fixentries` -
/// and read back as the row's own word: a written symbol takes the pair
/// with it, as on the parse, and leaves the cells as the row stated them,
/// where the parse takes back or rewrites the cells its detection owns.
#[test]
fn a_detected_pairs_cells_read_back_as_the_rows_word_and_only_the_pair_follows_the_symbol() {
    crate::install::installed();
    let cells = |held: &FixMsg| -> Vec<Option<String>> {
        [167, 460, 461, 15, 120]
            .into_iter()
            .map(|tag| {
                let held = held.get_by_tag(tag)?;
                held.as_str()
                    .map(ToOwned::to_owned)
                    .or_else(|| held.as_i64().map(|value| value.to_string()))
            })
            .collect()
    };
    let detected = |values: [&str; 5]| -> Vec<Option<String>> {
        values
            .into_iter()
            .map(|value| Some(value.to_owned()))
            .collect()
    };
    let held = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR/USD|54=1|38=1|10=0|");
    let stated = detected(["FXSPOT", "4", "IFXXXP", "EUR", "USD"]);
    assert_eq!(cells(&held), stated);
    let back = narrow_round_trip(&held, &["forexcode", "metadata"]);
    assert_eq!(cells(&back), stated);
    assert_eq!(ids(&back), ids(&held));

    let (mut parse, mut read) = (held.clone(), back.clone());
    for message in [&mut parse, &mut read] {
        message
            .set(55, yggdryl::Scalar::from("AAPL"))
            .expect("the symbol writes");
    }
    assert!(ids(&parse).is_empty(), "{:?}", ids(&parse));
    assert_eq!(ids(&read), ids(&parse), "the pair follows the symbol");
    assert_eq!(cells(&parse), vec![None; 5], "detection's own, taken back");
    assert_eq!(cells(&read), stated, "the row's word");

    let (mut parse, mut read) = (held.clone(), back.clone());
    for message in [&mut parse, &mut read] {
        message
            .set(55, yggdryl::Scalar::from("GBP/USD"))
            .expect("the symbol writes");
    }
    assert_eq!(ids(&parse), ["derived:forex=GBP/USD", "forex=GBP/USD"]);
    assert_eq!(ids(&read), ids(&parse), "the pair follows the symbol");
    assert_eq!(
        cells(&parse),
        detected(["FXSPOT", "4", "IFXXXP", "GBP", "USD"]),
        "detection's own, rewritten"
    );
    assert_eq!(cells(&read), stated, "the row's word");
    assert_eq!(parse.get_currency().as_str(), "GBP");
    assert_eq!(read.get_currency().as_str(), "EUR");
}

/// A source spelled `{NAMESPACE}INSTRUMENTID` states a venue's instrument
/// key from that namespace rather than from `fix`, on the primary and an
/// alternate alike, and the source crosses the row and the leaf.
#[test]
fn a_namespaced_instrument_source_states_its_namespace_through_the_row_and_the_leaf() {
    crate::install::installed();
    let registry = committed_registry();
    let schema = yggdryl_fix::fix_schema(&registry, "fix").expect("the fixed row");
    let ullink: yggdryl_market::IdSource = "ullink".parse().expect("a word");
    for line in [
        &b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|38=5|40=2|22=ULLINKINSTRUMENTID|48=dbi;CH0012214059_XSWX_CHF|10=0|"[..],
        b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|38=5|40=2|454=1|455=dbi;CH0012214059_XSWX_CHF|456=ULLINKINSTRUMENTID|10=0|",
    ] {
        let held = parsed(line);
        assert_eq!(
            ids(&held),
            [
                "derived:valor=1221405", "instrumentid=dbi;CH0012214059_XSWX_CHF", "isin=CH0012214059", "ullink:instrumentid=dbi;CH0012214059_XSWX_CHF", "ullink:isin=CH0012214059", "valor=1221405"
            ],
            "{}",
            String::from_utf8_lossy(line)
        );
        let row = held.into_row(&schema).expect("its row");
        let back = FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row)
            .expect("the row reads");
        assert_eq!(ids(&back), ids(&held));
        let leaves = held.into_market_data().expect("an order leaf");
        let [yggdryl_market::graph::MarketData::OrderEvent(order)] = leaves.as_slice() else {
            panic!("one order event, got {}", leaves.len())
        };
        assert_eq!(
            order
                .get_securityids()
                .get_from(&IdKey::new(ullink.clone(), IdType::InstrumentId)),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
    }
}

/// The namespace is read as a key's source is: folded, its separators at
/// either end dropped, so every spelling of one venue is one source.
#[test]
fn a_namespaced_instrument_source_names_the_namespace_without_its_separator() {
    crate::install::installed();
    let ullink: yggdryl_market::IdSource = "ullink".parse().expect("a word");
    for source in [
        "ULLINKINSTRUMENTID",
        "ULLINK_INSTRUMENTID",
        "ULLINK.INSTRUMENTID",
        "Ullink Instrument ID",
        "ullink-instrumentid",
    ] {
        for line in [
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={source}|48=dbi;CH0012214059_XSWX_CHF|10=0|"),
            format!(
                "8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455=dbi;CH0012214059_XSWX_CHF|456={source}|10=0|"
            ),
        ] {
            let held = parsed(line.as_bytes());
            assert_eq!(
                held.get_securityids()
                    .get_from(&IdKey::new(ullink.clone(), IdType::InstrumentId)),
                Some("dbi;CH0012214059_XSWX_CHF"),
                "{line}: {:?} {:?}",
                ids(&held),
                anomalies(&held)
            );
            assert_eq!(
                held.get_securityids()
                    .get_from(&IdKey::new(ullink.clone(), IdType::Isin)),
                Some("CH0012214059"),
                "{line}"
            );
        }
    }
}

/// A namespace that folds to a source the crate reserves - `base`,
/// `derived`, `fix` - names no venue: the source is the wire's own
/// instrument key, from `fix`, and never a statement filed as the crate's.
#[test]
fn a_reserved_namespace_reads_as_no_namespace() {
    crate::install::installed();
    for source in [
        "DERIVEDINSTRUMENTID",
        "Derived Instrument ID",
        "DERIVED.INSTRUMENTID",
        "BASE_INSTRUMENTID",
        "FIX.INSTRUMENTID",
    ] {
        for line in [
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={source}|48=dbi;X|10=0|"),
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455=dbi;X|456={source}|10=0|"),
        ] {
            let held = parsed(line.as_bytes());
            assert_eq!(ids(&held), ["instrumentid=dbi;X"], "{line}");
            assert!(
                anomalies(&held).is_empty(),
                "{line}: {:?}",
                anomalies(&held)
            );
        }
    }
}

/// The same rule reads an unmapped key: a reserved source spelled before
/// the name a key ends with names no namespace, so the entry is the wire's
/// statement under its type's base key - never filed under `derived` as
/// though the crate derived it, and `FIX` naming the base source itself.
#[test]
fn a_reserved_namespace_before_an_unmapped_keys_name_reads_as_no_namespace() {
    crate::install::installed();
    for (entry, expected) in [
        (
            "Derived_ISIN=US0378331005",
            &[
                "cusip=037833100",
                "derived:cusip=037833100",
                "isin=US0378331005",
            ][..],
        ),
        (
            "FIX.ISIN=US0378331005",
            &[
                "cusip=037833100",
                "derived:cusip=037833100",
                "isin=US0378331005",
            ],
        ),
        ("DERIVED.INSTRUMENTID=dbi;X", &["instrumentid=dbi;X"]),
    ] {
        let line = format!("8=FIX.4.4|35=D|11=A1|55=AAPL|{entry}|10=0|");
        let held = parsed(line.as_bytes());
        assert_eq!(ids(&held), expected, "{line}");
        assert!(
            anomalies(&held).is_empty(),
            "{line}: {:?}",
            anomalies(&held)
        );
    }
}

/// The namespace is bounded as a source is, a word of its own, never
/// together with the `INSTRUMENTID` it is spelled before.
#[test]
fn a_namespaced_instrument_source_names_a_namespace_as_wide_as_a_source() {
    crate::install::installed();
    let namespace = "v".repeat(64);
    let held = parsed(
        format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={namespace}INSTRUMENTID|48=dbi;X|10=0|")
            .as_bytes(),
    );
    assert_eq!(
        ids(&held),
        [
            "instrumentid=dbi;X".to_owned(),
            format!("{namespace}:instrumentid=dbi;X")
        ],
        "{:?}",
        anomalies(&held)
    );
}

#[test]
fn a_crated_view_ranks_after_the_wire_and_an_unmapped_name() {
    crate::install::installed();
    // `ISINCODE` is the crated column and `ISIN` a name no dictionary holds:
    // the unmapped name is read with the fields, so it states the type's
    // answer whichever arrived first, and the view's differing code is
    // dropped with an anomaly naming the view.
    for line in [
        &b"8=FIX.4.4|35=D|11=A1|55=AAPL|ISIN=US5949181045|ISINCODE=US0378331005|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|ISINCODE=US0378331005|ISIN=US5949181045|10=0|",
    ] {
        let held = parsed(line);
        assert_eq!(
            ids(&held),
            [
                "cusip=594918104",
                "derived:cusip=594918104",
                "isin=US5949181045"
            ],
            "{line:?}"
        );
        let dropped = anomalies(&held);
        assert_eq!(dropped.len(), 1, "{line:?}: {dropped:?}");
        assert_eq!(dropped[0].0, "isincode", "{line:?}");
        assert!(dropped[0].1.contains("US0378331005"), "{}", dropped[0].1);
    }
}

/// `SecurityIDSource(22)` beside `SecurityID(48)` states one identifier of
/// the type the source's code names, from the `fix` source, however the
/// pair arrives: by tag on the wire, by field name in a bridge row, by the
/// FIX 4 name `IDSource` of the same tag - and whether the source is the
/// code or the type's own name.
#[test]
fn the_security_source_and_id_fields_state_an_entry_however_they_are_spelled() {
    crate::install::installed();
    const WIRE: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|10=0|";
    let by_tag = parsed(WIRE);
    assert_eq!(
        ids(&by_tag),
        [
            "cusip=037833100",
            "derived:cusip=037833100",
            "isin=US0378331005"
        ]
    );
    assert!(anomalies(&by_tag).is_empty(), "{:?}", anomalies(&by_tag));
    for line in [
        &b"8=FIX.4.4|35=D|11=A1|55=AAPL|SecurityIDSource=4|SecurityID=US0378331005|10=0|"[..],
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|IDSource=4|SecurityID=US0378331005|10=0|",
        b"MSGTYPE=D|CLORDID=A1|SECURITYIDSOURCE=4|SECURITYID=US0378331005",
        b"MSGTYPE=D|CLORDID=A1|IDSOURCE=4|SECURITYID=US0378331005",
    ] {
        let held = parsed(line);
        assert_eq!(
            ids(&held),
            ids(&by_tag),
            "{}",
            String::from_utf8_lossy(line)
        );
        assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
        // Each reaches the field the tag names: the same cells, so the wire
        // it writes states the tag's own pair.
        assert_eq!(held.get_by_tag(22), by_tag.get_by_tag(22));
        assert_eq!(held.get_by_tag(48), by_tag.get_by_tag(48));
    }
    // A source stated as the type's own name is the same entry, and the
    // field keeps the word as it arrived.
    let spelled =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|SecurityIDSource=ISIN|SecurityID=US0378331005|10=0|");
    assert_eq!(ids(&spelled), ids(&by_tag));
    assert!(anomalies(&spelled).is_empty(), "{:?}", anomalies(&spelled));
    assert_eq!(
        spelled
            .get_by_tag(22)
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("ISIN".to_owned())
    );
}

/// The type the source code names is the type the entry takes: `1` a CUSIP,
/// `A` a Bloomberg symbol, each checked by its own rule.
#[test]
fn the_security_source_code_picks_the_type_of_the_entry() {
    crate::install::installed();
    let cusip = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=1|48=037833100|10=0|");
    assert_eq!(ids(&cusip), ["cusip=037833100"]);
    let bloomberg =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|IDSource=A|SecurityID=AAPL US EQUITY|10=0|");
    assert_eq!(ids(&bloomberg), ["bloomberg=AAPL US EQUITY"]);
    // A number its check digit does not close is held as the value it is,
    // of a lower rank, and is no anomaly; a value the type's shape refuses
    // states nothing and is kept as one, the field staying on the row as
    // it arrived.
    let typo = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331006|10=0|");
    assert_eq!(ids(&typo), ["isin=US0378331006"]);
    assert!(anomalies(&typo).is_empty(), "{:?}", anomalies(&typo));
    let refused = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US037833100|10=0|");
    assert!(ids(&refused).is_empty(), "{:?}", ids(&refused));
    assert_eq!(
        refused
            .get_by_tag(48)
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("US037833100".to_owned())
    );
    assert!(!anomalies(&refused).is_empty());
}

/// Every `SecurityIDSource(22)` code FIX 4.4 and FIX 5.0 name types the
/// entry `SecurityID(48)` states, the primary and an alternate alike, each
/// value held to its type's width; a code no member names - a private one,
/// a letter FIX gives nothing - is kept as it was stated.
#[test]
fn every_fix_security_source_code_types_its_entry() {
    crate::install::installed();
    for (code, kind, value) in [
        ("1", "cusip", "037833100"),
        ("2", "sedol", "0263494"),
        ("3", "quik", "12345"),
        ("4", "isin", "US0378331005"),
        ("5", "ric", "AAPL.OQ"),
        ("6", "isoccy", "EUR"),
        ("7", "isoctry", "US"),
        ("8", "exchsymb", "AAPL"),
        ("9", "cta", "AAPL"),
        ("A", "bloomberg", "AAPL US Equity"),
        ("B", "wkn", "865985"),
        ("C", "dutch", "123456"),
        ("D", "valor", "908440"),
        ("E", "sicovam", "12000"),
        ("F", "belgian", "123456"),
        ("G", "common", "012345678"),
        ("H", "clearinghouse", "CH-123"),
        ("I", "fpmlspec", "InterestRate:IRSwap:FixedFloat"),
        ("J", "opra", "AAPL  240119C00150000"),
        (
            "K",
            "fpmlurl",
            "http://www.fpml.org/coding-scheme/product-taxonomy",
        ),
        ("L", "loc", "LC-2024-000123"),
        ("100", "100", "HOUSE-1"),
        ("Z", "z", "VENUE-7"),
    ] {
        let kind = kind.parse::<IdType>().expect("a type");
        let primary =
            parsed(format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={code}|48={value}|10=0|").as_bytes());
        assert_eq!(
            primary
                .get_securityids()
                .get_from(&IdKey::base(kind.clone())),
            Some(value),
            "22={code}: {:?} {:?}",
            ids(&primary),
            anomalies(&primary)
        );
        let alternate = parsed(
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455={value}|456={code}|10=0|").as_bytes(),
        );
        assert_eq!(
            alternate
                .get_securityids()
                .get_from(&IdKey::base(kind.clone())),
            Some(value),
            "456={code}: {:?} {:?}",
            ids(&alternate),
            anomalies(&alternate)
        );
    }
}

/// Every name the code set writes, its punctuation and remarks included,
/// types the entry `SecurityID(48)` states as its code does, the primary and
/// an alternate alike.
#[test]
fn a_source_named_in_full_types_its_entry_as_its_code_does() {
    crate::install::installed();
    for (name, code, value) in [
        ("ISIN number", '4', "US0378331005"),
        ("Exchange Symbol", '8', "AAPL"),
        (
            "Consolidated Tape Association (CTA) Symbol (SIAC CTS/CQS line format)",
            '9',
            "AAPL",
        ),
        ("\"Common\" (Clearstream and Euroclear)", 'G', "012345678"),
        ("Clearing House / Clearing Organization", 'H', "CH-123"),
        (
            "ISDA/FpML Product Specification (XML in EncodedSecurityDesc <351>)",
            'I',
            "InterestRate:IRSwap:FixedFloat",
        ),
        (
            "Option Price Reporting Authority",
            'J',
            "AAPL  240119C00150000",
        ),
        (
            "ISDA/FpML Product URL (URL in SecurityID)",
            'K',
            "http://www.fpml.org/coding-scheme/product-taxonomy",
        ),
        ("Letter of Credit", 'L', "LC-2024-000123"),
    ] {
        let kind = IdType::from_fix_security_source(code).expect("a FIX code");
        let fix = yggdryl_market::IdSource::Base;
        let primary =
            parsed(format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={name}|48={value}|10=0|").as_bytes());
        assert_eq!(
            primary
                .get_securityids()
                .get_from(&IdKey::new(fix.clone(), kind.clone())),
            Some(value),
            "22={name}: {:?} {:?}",
            ids(&primary),
            anomalies(&primary)
        );
        let alternate = parsed(
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455={value}|456={name}|10=0|").as_bytes(),
        );
        assert_eq!(
            alternate
                .get_securityids()
                .get_from(&IdKey::new(fix.clone(), kind.clone())),
            Some(value),
            "456={name}: {:?} {:?}",
            ids(&alternate),
            anomalies(&alternate)
        );
    }
}

/// A derivation reads a source by its name as the identifiers do: an ISIN
/// named in full opens the country it was issued in, an exchange symbol
/// named in full is the symbol, and an ISIN named as an alternate is the
/// security identifier.
#[test]
fn a_derivation_reads_a_source_by_its_name() {
    crate::install::installed();
    let text = |message: &FixMsg, tag: i32| {
        message
            .get_by_tag(tag)
            .as_ref()
            .and_then(yggdryl::Scalar::as_str)
            .map(str::to_owned)
    };
    for source in ["4", "ISIN number", "isin"] {
        let message = parsed(
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={source}|48=US0378331005|10=0|").as_bytes(),
        );
        assert_eq!(text(&message, 470).as_deref(), Some("US"), "22={source}");
    }
    for source in ["8", "Exchange Symbol", "A", "Bloomberg Symbol"] {
        let message = parsed(format!("8=FIX.4.4|35=D|11=A1|22={source}|48=AAPL|10=0|").as_bytes());
        assert_eq!(text(&message, 55).as_deref(), Some("AAPL"), "22={source}");
    }
    for source in ["4", "ISIN number"] {
        let message = parsed(
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455=US0378331005|456={source}|10=0|")
                .as_bytes(),
        );
        assert_eq!(
            text(&message, 48).as_deref(),
            Some("US0378331005"),
            "456={source}"
        );
    }
}

/// A source that names no security type - a ticker, an order's or a party's
/// identifier, a spelling no word holds - states no identifier: an anomaly
/// names it, on the primary and an alternate alike.
#[test]
fn a_source_naming_no_security_type_is_an_anomaly() {
    crate::install::installed();
    for source in ["ticker", "ClOrdID", "Exchange", "House/Key"] {
        let primary =
            parsed(format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={source}|48=HK-1|10=0|").as_bytes());
        assert!(
            ids(&primary).iter().all(|id| !id.starts_with("fix:")),
            "22={source}: {:?}",
            ids(&primary)
        );
        assert!(
            anomalies(&primary)
                .iter()
                .any(|(field, _)| *field == "securityid"),
            "22={source}: {:?}",
            anomalies(&primary)
        );
        let alternate = parsed(
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455=HK-1|456={source}|10=0|").as_bytes(),
        );
        assert!(
            anomalies(&alternate)
                .iter()
                .any(|(field, _)| *field == "secaltids"),
            "456={source}: {:?}",
            anomalies(&alternate)
        );
    }
}

/// A source no member names is kept as it was stated, the word it folds to,
/// through the row and back and onto the market leaf; the wire keeps the
/// spelling that arrived.
#[test]
fn a_source_kept_as_stated_crosses_the_row_and_the_leaf() {
    crate::install::installed();
    let registry = committed_registry();
    let schema = yggdryl_fix::fix_schema(&registry, "fix").expect("a schema");
    let fix = yggdryl_market::IdSource::Base;
    for (source, word) in [("100", "100"), ("Z", "z")] {
        let kind: IdType = word.parse().expect("a word");
        let held =
            parsed(format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={source}|48=HOUSE-1|10=0|").as_bytes());
        assert_eq!(
            held.get_securityids()
                .get_from(&IdKey::new(fix.clone(), kind.clone())),
            Some("HOUSE-1")
        );
        let row = held.into_row(&schema).expect("a row");
        let again = FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row)
            .expect("the row's message");
        assert_eq!(
            again
                .get_securityids()
                .get_from(&IdKey::new(fix.clone(), kind.clone())),
            Some("HOUSE-1")
        );
        assert_eq!(
            again
                .get_by_tag(22)
                .as_ref()
                .and_then(yggdryl::Scalar::as_str),
            Some(source),
            "the wire keeps the source as it arrived"
        );
        let leaves = held.into_market_data().expect("an order leaf");
        let [yggdryl_market::graph::MarketData::OrderEvent(order)] = leaves.as_slice() else {
            panic!("one order event, got {}", leaves.len())
        };
        assert_eq!(
            order
                .get_securityids()
                .get_from(&IdKey::new(fix.clone(), kind.clone())),
            Some("HOUSE-1")
        );
    }
}

/// A source no word folds from states no identifier: an anomaly names it and
/// the fields stay on the wire as they arrived.
#[test]
fn a_source_no_type_reads_is_an_anomaly_and_stays_on_the_wire() {
    crate::install::installed();
    let message = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=House/Key|48=HK-1|10=0|");
    assert!(ids(&message).is_empty(), "{:?}", ids(&message));
    assert!(
        anomalies(&message)
            .iter()
            .any(|(field, reason)| *field == "securityid" && reason.contains("House/Key")),
        "{:?}",
        anomalies(&message)
    );
    assert_eq!(
        message
            .get_by_tag(22)
            .as_ref()
            .and_then(yggdryl::Scalar::as_str),
        Some("House/Key")
    );
    assert_eq!(
        message
            .get_by_tag(48)
            .as_ref()
            .and_then(yggdryl::Scalar::as_str),
        Some("HK-1")
    );
}

/// An ISO currency or country code is one its standard names, upper-cased;
/// any other value states nothing and is an anomaly.
#[test]
fn an_iso_currency_or_country_source_reads_a_code_its_standard_names() {
    crate::install::installed();
    let lower = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR|22=6|48=eur|454=1|455=ch|456=7|10=0|");
    assert_eq!(ids(&lower), ["isoccy=EUR", "isoctry=CH"]);
    let refused = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR|22=6|48=EURO|10=0|");
    assert!(ids(&refused).is_empty(), "{:?}", ids(&refused));
    assert!(!anomalies(&refused).is_empty());
}

/// A code-suffixed spelling behind a namespace states its source's entry -
/// `OMS_RICCODE` is `oms:ric`, `ULLINK.ISINCODE` `ullink:isin`,
/// `firm.x.BBGSymbol` `firm.x:bloomberg`, `OMS_FIGICODE` `oms:figi` - and
/// the base key of each type takes the value where nothing else states it.
#[test]
fn a_code_suffixed_alias_behind_a_namespace_states_its_sources_entry() {
    crate::install::installed();
    let held = parsed(
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|OMS_RICCODE=AAPL.O|ULLINK.ISINCODE=US0378331005|\
          firm.x.BBGSymbol=AAPL US Equity|OMS_FIGICODE=BBG000B9XRY4|10=0|",
    );
    let ids = held.get_securityids();
    let src = |name: &str| name.parse::<yggdryl_market::IdSource>().expect("a source");
    for (source, kind, value) in [
        ("oms", IdType::Ric, "AAPL.O"),
        ("ullink", IdType::Isin, "US0378331005"),
        ("firm.x", IdType::Bloomberg, "AAPL US Equity"),
        ("oms", IdType::Figi, "BBG000B9XRY4"),
    ] {
        assert_eq!(
            ids.get_from(&IdKey::new(src(source), kind.clone())),
            Some(value),
            "{source}:{kind} in {ids}"
        );
        assert_eq!(ids.get(&kind), Some(value), "{kind} in {ids}");
    }
    assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
}
