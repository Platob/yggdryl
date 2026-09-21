//! Message categories and normalized instrument identifiers in the fixed row.

use std::sync::Arc;

use yggdryl::graph::MarketElement;
use yggdryl::{CfiCode, CusipCode, FIGICode, FixMsg, IsinCode, Scalar, SedolCode};

#[test]
fn committed_messages_publish_one_four_byte_category() {
    let registry = super::committed_registry();
    for (msgtype, category) in [
        ("D", "ORDR"),
        ("8", "EXEC"),
        ("V", "BOOK"),
        ("R", "QUOT"),
        ("AE", "TRAD"),
    ] {
        let message = registry.msgtype(msgtype).expect("a committed message");
        assert_eq!(
            message.as_field().get_metadata("FIX:msgcat"),
            Some(category),
            "{msgtype} has its category on its component definition"
        );
        assert_eq!(category.len(), 4, "the stored category is fixed ASCII");
    }
}

#[test]
fn normalized_instrument_codes_are_fixed_row_fields() {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(std::sync::Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
    // CFI keeps standard tag 461. The normalized identifiers this message
    // lifts are derived once by MarketEvent, then the row and tag lookup
    // borrow that same typed fact. CUSIP and SEDOL deliberately stay in
    // FIX's contextual identifier fields and `secaltids`.
    let cases = [
        (
            b"8=FIX.4.4|35=D|11=I|22=4|48=US0378331005|10=0|".as_slice(),
            65_055,
            "US0378331005",
        ),
        (
            b"8=FIX.4.4|35=D|11=C|461=ESXXXX|10=0|".as_slice(),
            461,
            "ESXXXX",
        ),
        (
            b"8=FIX.4.4|35=D|11=B|22=A|48=AAPL US Equity|10=0|".as_slice(),
            65_059,
            "AAPL US Equity",
        ),
        (
            b"8=FIX.4.4|35=D|11=M|207=XNAS|10=0|".as_slice(),
            65_060,
            "XNAS",
        ),
    ];
    for (line, tag, expected) in cases {
        let message = codec.parse_fix_line(line).expect("a typed message");
        let at = yggdryl::fix_column_of(&schema, tag).expect("a normalized code column");
        let row = message.into_row(&schema).expect("a fixed row");
        assert_eq!(
            row.as_sequence().expect("a row")[at].as_str(),
            Some(expected),
            "the row carries tag {tag}"
        );
        assert_eq!(
            message.by_tag(tag).expect("the same lifted fact").as_str(),
            Some(expected),
            "tag lookup and the row share one owner"
        );
    }

    // No raw 461 is present: a lifecycle fact learned by the event remains
    // the standard CFI column's typed value.
    let mut learned = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=L|10=0|")
        .expect("a message without a raw CFI");
    learned.set_cficode(Some(CfiCode::new("ESVUFR").expect("a CFI")));
    assert_eq!(
        learned.get_cficode().map(|value| value.as_str()),
        Some("ESVUFR")
    );
    let at = yggdryl::fix_column_of(&schema, 461).expect("the standard CFI column");
    assert_eq!(
        learned
            .into_row(&schema)
            .expect("a fixed row")
            .as_sequence()
            .expect("a row")[at]
            .as_str(),
        Some("ESVUFR"),
    );

    let message = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=N|10=0|")
        .expect("a message without instrument codes");
    assert!(message.get_isincode().is_none());
    assert!(message.get_cficode().is_none());
    assert!(message.get_cusipcode().is_none());
    assert!(message.get_sedolcode().is_none());
    assert!(message.get_bloombergcode().is_none());
    assert!(message.get_figicode().is_none());
    assert!(message.get_miccode().is_none());
}

#[test]
fn cusip_and_sedol_stay_in_fix_identifiers_without_normalized_lifting() {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");

    let primary = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=C|22=1|48=037833100|10=0|")
        .expect("a CUSIP security identifier");
    let primary_id = primary.get_by_tag(48);
    assert_eq!(
        primary_id.as_ref().and_then(Scalar::as_str),
        Some("037833100")
    );
    assert!(primary.get_cusipcode().is_none());

    let alternates = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=A|454=2|455=037833100|456=1|455=B0YBKJ7|456=2|10=0|")
        .expect("CUSIP and SEDOL alternate identifiers");
    assert!(alternates.get_cusipcode().is_none());
    assert!(alternates.get_sedolcode().is_none());
    let values = super::sequence(
        alternates
            .by_name("secaltids")
            .expect("the alternate identifiers remain FIX content"),
    );
    assert_eq!(values.len(), 2);
    assert_eq!(
        values[0].as_sequence().expect("a CUSIP occurrence")[0].as_str(),
        Some("037833100")
    );
    assert_eq!(
        values[1].as_sequence().expect("a SEDOL occurrence")[0].as_str(),
        Some("B0YBKJ7")
    );

    let row = alternates.into_row(&schema).expect("a fixed row");
    let row = row.as_sequence().expect("a row");
    for tag in [yggdryl::CUSIPCODE_TAG_NAME.0, yggdryl::SEDOLCODE_TAG_NAME.0] {
        let at = yggdryl::fix_column_of(&schema, tag).expect("a normalized code column");
        assert!(row[at].is_null(), "tag {tag} is not lifted");
    }

    let isin = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=I|22=4|48=US0378331005|10=0|")
        .expect("an ISIN carrying an embedded CUSIP");
    assert!(isin.get_cusipcode().is_none());
}

#[test]
fn normalized_codes_stated_by_a_row_survive_market_derivation() {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");

    // A direct normalized write has no raw identifier to derive from. The
    // row must restore the typed event fact instead of clearing it.
    let mut explicit = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=I|10=0|")
        .expect("a message without an ISIN pair");
    explicit
        .set(
            yggdryl::ISINCODE_TAG_NAME.0,
            Scalar::IsinCode(IsinCode::new("US0378331005").expect("an ISIN")),
        )
        .expect("a normalized ISIN fact");
    explicit
        .set(
            yggdryl::CUSIPCODE_TAG_NAME.0,
            Scalar::CusipCode(CusipCode::new("037833100").expect("a CUSIP")),
        )
        .expect("a normalized CUSIP fact");
    explicit
        .set(
            yggdryl::SEDOLCODE_TAG_NAME.0,
            Scalar::SedolCode(SedolCode::new("B0YBKJ7").expect("a SEDOL")),
        )
        .expect("a normalized SEDOL fact");
    let row = explicit.into_row(&schema).expect("a fixed row");
    let rebuilt = FixMsg::from_row(Arc::clone(&registry), &schema, &row).expect("a rebuilt row");
    assert_eq!(
        rebuilt.get_isincode().map(|value| value.as_str()),
        Some("US0378331005")
    );
    assert_eq!(
        rebuilt.get_cusipcode().map(|value| value.as_str()),
        Some("037833100")
    );
    assert_eq!(
        rebuilt.get_sedolcode().map(|value| value.as_str()),
        Some("B0YBKJ7")
    );

    // The first row learns Bloomberg from the ordinary FIX pair. Removing
    // that pair simulates a later fixed row that carries only the normalized
    // fact; reconstruction must keep the stated code.
    let parsed = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=B|22=A|48=AAPL US Equity|10=0|")
        .expect("a message with a Bloomberg source pair");
    let mut columns = parsed
        .into_row(&schema)
        .expect("a fixed row")
        .as_sequence()
        .expect("a row")
        .to_vec();
    for tag in [22, 48] {
        columns[yggdryl::fix_column_of(&schema, tag).expect("a raw identifier column")] =
            Scalar::Null;
    }
    let stored = Scalar::from_sequence(columns);
    let rebuilt = FixMsg::from_row(Arc::clone(&registry), &schema, &stored).expect("a rebuilt row");
    assert_eq!(
        rebuilt.get_bloombergcode().map(|value| value.as_str()),
        Some("AAPL US Equity")
    );
    let bloomberg_at = yggdryl::fix_column_of(&schema, yggdryl::BLOOMBERGCODE_TAG_NAME.0)
        .expect("a normalized Bloomberg column");
    assert_eq!(
        rebuilt
            .into_row(&schema)
            .expect("a rebuilt row")
            .as_sequence()
            .expect("a row")[bloomberg_at]
            .as_str(),
        Some("AAPL US Equity")
    );
}

#[test]
fn figi_sources_lift_to_one_typed_crate_column_without_bloomberg_fallback() {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
    for line in [
        b"8=FIX.4.4|35=D|11=P|22=S|48=BBG000BLNQ16|10=0|".as_slice(),
        b"8=FIX.4.4|35=D|11=A|454=1|455=BBG000BLNQ16|456=S|10=0|".as_slice(),
    ] {
        let message = codec.parse_fix_line(line).expect("a FIGI source");
        assert_eq!(
            message.get_figicode().map(|value| value.as_str()),
            Some("BBG000BLNQ16")
        );
        assert!(message.get_bloombergcode().is_none());
        let at = yggdryl::fix_column_of(&schema, yggdryl::FIGICODE_TAG_NAME.0)
            .expect("the FIGI fixed column");
        assert_eq!(
            message.into_row(&schema).unwrap().as_sequence().unwrap()[at].as_str(),
            Some("BBG000BLNQ16")
        );
    }
    let malformed = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=X|22=S|48=BBG000BLNQ17|10=0|")
        .expect("a malformed FIGI stays raw");
    assert!(malformed.get_figicode().is_none());

    let mut explicit = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=E|10=0|")
        .expect("an empty instrument");
    explicit
        .set(
            yggdryl::FIGICODE_TAG_NAME.0,
            Scalar::FIGICode(FIGICode::new("BBG000BLNQ16").unwrap()),
        )
        .expect("a direct FIGI fact");
    let row = explicit.into_row(&schema).unwrap();
    let rebuilt = FixMsg::from_row(registry, &schema, &row).unwrap();
    assert_eq!(
        rebuilt.get_figicode().map(|value| value.as_str()),
        Some("BBG000BLNQ16")
    );
    assert_eq!(rebuilt.into_row(&schema).unwrap(), row);
}
