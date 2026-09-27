//! `rust/src/fix/aliases.rs`: the four word pairs every name lookup reads,
//! with nothing written into the dictionary.

use std::sync::Arc;

use yggdryl::{DataType, Field, FixRegistry, Scalar};

fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

#[test]
fn every_combination_of_the_words_reaches_the_field() {
    let registry = FixRegistry::from_fields([tagged("offersizebidpx", 10_001)]).unwrap();
    for spelling in [
        "asksizebidpx",
        "offerqtybidpx",
        "askqtybidpx",
        "offersizedemandpx",
        "asksizedemandpx",
        "offerqtydemandpx",
        "askqtydemandpx",
        "offersizebidprice",
        "asksizebidprice",
        "offerqtybidprice",
        "askqtybidprice",
        "offersizedemandprice",
        "asksizedemandprice",
        "offerqtydemandprice",
        "Ask_Qty_Demand_Price",
    ] {
        assert_eq!(
            registry.get_field_by_name(spelling).map(Field::name),
            Some("offersizebidpx"),
            "{spelling}"
        );
    }
}

#[test]
fn a_held_name_wins_and_the_lookup_writes_nothing() {
    let mut offer = tagged("offerpx", 10_010);
    offer.as_fix_mut().set_names(["deskoffer"]).unwrap();
    let occupied = tagged("askpx", 10_011);
    let mut alias_owner = tagged("deskprice", 10_012);
    alias_owner.as_fix_mut().set_names(["Ask_Price"]).unwrap();
    let registry = FixRegistry::from_fields([offer, occupied, alias_owner]).unwrap();
    let hash = registry.stable_hash();

    assert_eq!(
        registry.get_field_by_name("askpx").map(Field::name),
        Some("askpx"),
        "a canonical owner is never borrowed"
    );
    assert_eq!(
        registry.get_field_by_name("askprice").map(Field::name),
        Some("deskprice"),
        "an alias a field holds answers before the words"
    );
    // `offerprice` reads as `askprice`, `offerpx` and `askpx`: three
    // fields, so it reaches none.
    assert!(registry.get_field_by_name("offerprice").is_none());
    assert_eq!(
        registry
            .field(10_010)
            .unwrap()
            .as_fix()
            .names()
            .collect::<Vec<_>>(),
        ["deskoffer"],
        "no spelling is lent to the dictionary"
    );
    assert_eq!(registry.stable_hash(), hash);
}

#[test]
fn codec_and_message_reads_and_writes_share_the_word_aliases() {
    let registry = Arc::new(FixRegistry::from_fields([tagged("offerpx", 10_050)]).unwrap());
    let codec = super::fixed_codec(Arc::clone(&registry));
    let mut message = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|10050=ten|10=0|")
        .unwrap();
    assert_eq!(message.by_name("askprice").unwrap().as_str(), Some("ten"));
    message.set("askprice", Scalar::from("eleven")).unwrap();
    assert_eq!(message.by_name("offerpx").unwrap().as_str(), Some("eleven"));
    assert_eq!(
        message.by_name("offerprice").unwrap().as_str(),
        Some("eleven")
    );

    let agreeing = super::sole_message(
        codec
            .parse_line(b"MSGTYPE=D|FIRM.ORIG.OFFERPX=ten|ULLINK.OFFERPRICE=ten|")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(agreeing.by_name("offerpx").unwrap().as_str(), Some("ten"));
    let contradictory = super::sole_message(
        codec
            .parse_line(b"MSGTYPE=D|FIRM.ORIG.OFFERPX=ten|ULLINK.OFFERPRICE=eleven|")
            .unwrap(),
    )
    .unwrap();
    assert!(contradictory.get_by_name("offerpx").is_none());
    assert!(contradictory.get_by_name("askprice").is_none());
}
