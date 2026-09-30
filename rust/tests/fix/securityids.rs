//! `rust/src/fix/msg.rs` `stated_securityids`: the identifiers a message
//! states under the wire's own fields and under the unmapped fields whose
//! names name a source, and what a settle drops of them. The set holds them
//! in the order of their keys as `src:type` spells them.

use super::{committed_registry, fixed_codec};
use yggdryl::graph::Market;
use yggdryl::{FixMsg, IdType};

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
    assert_eq!(
        ids(&held),
        ["base:isin=CH0012221716", "derived:valor=1222171"]
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
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|cusip_code=037833100|#SEDOLCODE=0263494|10=0|");
    assert_eq!(ids(&held), ["base:cusip=037833100", "base:sedol=0263494"]);
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
fn the_wire_leads_and_an_unmapped_code_is_a_statement_of_its_own_source() {
    // The wire's field and the bridge's name are two sources, each stating
    // its own entry, and the wire's answers the type.
    let held =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|#ISINCODE=US0378331005|10=0|");
    assert_eq!(
        ids(&held),
        [
            "base:isin=US0378331005",
            "derived:cusip=037833100",
            "fix:isin=US0378331005"
        ]
    );
    assert!(anomalies(&held).is_empty());
    assert_eq!(
        held.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );

    // A different code under another source stands beside the wire's,
    // which still answers the type: no source overrides another.
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
            "base:isin=US5949181045",
            "derived:cusip=037833100",
            "fix:isin=US0378331005"
        ]
    );
    assert!(anomalies(&held).is_empty(), "{:?}", anomalies(&held));
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
            ["base:isin=US0378331005", "derived:cusip=037833100"],
            "{line:?}"
        );
        let dropped = anomalies(&held);
        assert_eq!(dropped.len(), 1, "{line:?}: {dropped:?}");
        assert_eq!(dropped[0].0, "isin", "{line:?}");
        assert!(dropped[0].1.contains("US5949181045"), "{}", dropped[0].1);
    }
}

/// `SecurityIDSource(22)` beside `SecurityID(48)` states one identifier of
/// the type the source's code names, from the `fix` source, however the
/// pair arrives: by tag on the wire, by field name in a bridge row, by the
/// FIX 4 name `IDSource` of the same tag - and whether the source is the
/// code or the type's own name.
#[test]
fn the_security_source_and_id_fields_state_an_entry_however_they_are_spelled() {
    const WIRE: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331005|10=0|";
    let by_tag = parsed(WIRE);
    assert_eq!(
        ids(&by_tag),
        ["derived:cusip=037833100", "fix:isin=US0378331005"]
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
    let cusip = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=1|48=037833100|10=0|");
    assert_eq!(ids(&cusip), ["fix:cusip=037833100"]);
    let bloomberg =
        parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|IDSource=A|SecurityID=AAPL US EQUITY|10=0|");
    assert_eq!(ids(&bloomberg), ["fix:bloomberg=AAPL US EQUITY"]);
    // A value the type refuses states nothing and is kept as an anomaly,
    // the field staying on the row as it arrived.
    let refused = parsed(b"8=FIX.4.4|35=D|11=A1|55=AAPL|22=4|48=US0378331006|10=0|");
    assert!(ids(&refused).is_empty(), "{:?}", ids(&refused));
    assert_eq!(
        refused
            .get_by_tag(48)
            .and_then(|held| held.as_str().map(str::to_owned)),
        Some("US0378331006".to_owned())
    );
    assert!(!anomalies(&refused).is_empty());
}

/// Every `SecurityIDSource(22)` code FIX 4.4 and FIX 5.0 name types the
/// entry `SecurityID(48)` states, the primary and an alternate alike, each
/// value held to its type's width; a code no member names - a private one,
/// a letter FIX gives nothing - is kept as it was stated.
#[test]
fn every_fix_security_source_code_types_its_entry() {
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
                .get_from(&yggdryl::IdSource::Fix, &kind),
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
                .get_from(&yggdryl::IdSource::Fix, &kind),
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
        let fix = yggdryl::IdSource::Fix;
        let primary =
            parsed(format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={name}|48={value}|10=0|").as_bytes());
        assert_eq!(
            primary.get_securityids().get_from(&fix, &kind),
            Some(value),
            "22={name}: {:?} {:?}",
            ids(&primary),
            anomalies(&primary)
        );
        let alternate = parsed(
            format!("8=FIX.4.4|35=D|11=A1|55=AAPL|454=1|455={value}|456={name}|10=0|").as_bytes(),
        );
        assert_eq!(
            alternate.get_securityids().get_from(&fix, &kind),
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
    let registry = committed_registry();
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a schema");
    let fix = yggdryl::IdSource::Fix;
    for (source, word) in [("100", "100"), ("Z", "z")] {
        let kind: IdType = word.parse().expect("a word");
        let held =
            parsed(format!("8=FIX.4.4|35=D|11=A1|55=AAPL|22={source}|48=HOUSE-1|10=0|").as_bytes());
        assert_eq!(
            held.get_securityids().get_from(&fix, &kind),
            Some("HOUSE-1")
        );
        let row = held.into_row(&schema).expect("a row");
        let again = FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row)
            .expect("the row's message");
        assert_eq!(
            again.get_securityids().get_from(&fix, &kind),
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
        let [yggdryl::graph::MarketData::OrderEvent(order)] = leaves.as_slice() else {
            panic!("one order event, got {}", leaves.len())
        };
        assert_eq!(
            order.get_securityids().get_from(&fix, &kind),
            Some("HOUSE-1")
        );
    }
}

/// A source no word folds from states no identifier: an anomaly names it and
/// the fields stay on the wire as they arrived.
#[test]
fn a_source_no_type_reads_is_an_anomaly_and_stays_on_the_wire() {
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
    let lower = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR|22=6|48=eur|454=1|455=ch|456=7|10=0|");
    assert_eq!(ids(&lower), ["fix:isoccy=EUR", "fix:isoctry=CH"]);
    let refused = parsed(b"8=FIX.4.4|35=D|11=A1|55=EUR|22=6|48=EURO|10=0|");
    assert!(ids(&refused).is_empty(), "{:?}", ids(&refused));
    assert!(!anomalies(&refused).is_empty());
}
