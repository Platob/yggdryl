//! Message categories and normalized instrument identifiers in the fixed row.

use std::sync::Arc;

use yggdryl::graph::MarketElement;
use yggdryl::{CfiCode, FixMsg, IsinCode, Scalar};

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
    // Five crate tags follow MsgCat(65054); CFI keeps standard tag 461.
    // Each is derived once by MarketEvent, then the row and tag lookup
    // borrow that same typed fact.
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
            b"8=FIX.4.4|35=D|11=U|22=1|48=037833100|10=0|".as_slice(),
            65_057,
            "037833100",
        ),
        (
            b"8=FIX.4.4|35=D|11=S|22=2|48=B0YBKJ7|10=0|".as_slice(),
            65_058,
            "B0YBKJ7",
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
    assert!(message.get_miccode().is_none());
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
    let row = explicit.into_row(&schema).expect("a fixed row");
    let rebuilt = FixMsg::from_row(Arc::clone(&registry), &schema, &row).expect("a rebuilt row");
    assert_eq!(
        rebuilt.get_isincode().map(|value| value.as_str()),
        Some("US0378331005")
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
