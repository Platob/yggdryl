//! Generated FIX aliases retain the registry's one-name ownership rules.

use std::sync::Arc;

use yggdryl::{DataType, Field, FixRegistry, Scalar, StructType};

fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn aliases(field: &Field) -> Vec<&str> {
    field.as_fix().names().collect()
}

#[test]
fn every_applicable_substitution_combines_once() {
    let registry = FixRegistry::from_fields([tagged("offersizebidpx", 10_001)])
        .unwrap()
        .with_default_aliases()
        .unwrap();
    let expected = [
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
        "askqtydemandprice",
    ];

    let field = registry.field(10_001).unwrap();
    assert_eq!(aliases(field), expected);
    for spelling in expected {
        assert_eq!(
            registry.get_field_by_name(spelling).map(Field::name),
            Some("offersizebidpx"),
            "{spelling}"
        );
    }
}

#[test]
fn canonical_and_existing_owners_win_and_generated_claims_follow_scalar_order() {
    let mut offer = tagged("offerpx", 10_010);
    offer.as_fix_mut().set_names(["deskoffer"]).unwrap();
    let occupied = tagged("askpx", 10_011);
    let mut alias_owner = tagged("deskprice", 10_012);
    alias_owner.as_fix_mut().set_names(["Ask_Price"]).unwrap();
    let registry = FixRegistry::from_fields([offer, occupied, alias_owner])
        .unwrap()
        .with_default_aliases()
        .unwrap();

    assert_eq!(
        registry.get_field_by_name("askpx").map(Field::name),
        Some("askpx"),
        "a canonical owner is never borrowed"
    );
    let offer = registry.field(10_010).unwrap();
    assert_eq!(aliases(offer), ["deskoffer", "offerprice"]);
    assert_eq!(
        registry.get_field_by_name("deskoffer").map(Field::name),
        Some("offerpx"),
        "an existing alias stays first and owned"
    );
    assert_eq!(
        registry.get_field_by_name("askprice").map(Field::name),
        Some("deskprice"),
        "a folded existing alias stays with its owner"
    );
    assert_eq!(aliases(registry.field(10_012).unwrap()), ["Ask_Price"]);

    for (bidpx_tag, bidprice_tag, expected) in
        [(10_020, 10_021, "bidpx"), (10_023, 10_022, "bidprice")]
    {
        let registry = FixRegistry::from_fields([
            tagged("bidpx", bidpx_tag),
            tagged("bidprice", bidprice_tag),
        ])
        .unwrap()
        .with_default_aliases()
        .unwrap();
        assert_eq!(
            registry.get_field_by_name("demandprice").map(Field::name),
            Some(expected),
            "the first scalar in tag order keeps the generated spelling"
        );
        assert_eq!(
            registry.get_field_by_name("bidprice").map(Field::name),
            Some("bidprice"),
            "the canonical spelling wins regardless of tag order"
        );
    }

    let shared_tag = 10_024;
    let registry =
        FixRegistry::from_fields([tagged("offerpx", shared_tag), tagged("bidsize", shared_tag)])
            .unwrap()
            .with_default_aliases()
            .unwrap();
    assert_eq!(registry.field(shared_tag).unwrap().name(), "offerpx");
    let holders: Vec<&str> = registry
        .iter()
        .filter(|field| field.as_fix().tag().ok().flatten() == Some(shared_tag))
        .map(Field::name)
        .collect();
    assert_eq!(holders, ["offerpx", "bidsize"]);
}

#[test]
fn fields_without_substitutions_stay_equal_and_a_second_pass_is_idempotent() {
    let unchanged = tagged("symbol", 10_030);
    let once = FixRegistry::from_fields([unchanged.clone(), tagged("offerpx", 10_031)])
        .unwrap()
        .with_default_aliases()
        .unwrap();
    assert_eq!(once.field(10_030).unwrap(), &unchanged);

    let hash = once.stable_hash();
    let twice = once.clone().with_default_aliases().unwrap();
    assert_eq!(twice, once);
    assert_eq!(twice.stable_hash(), hash);
    assert_eq!(
        aliases(twice.field(10_031).unwrap()),
        ["askpx", "offerprice", "askprice"]
    );
}

#[test]
fn generated_aliases_refresh_referenced_component_metadata() {
    let mut registry = FixRegistry::from_fields([tagged("offerpx", 10_040)]).unwrap();
    let mut member = registry.field(10_040).unwrap().clone();
    member.as_fix_mut().set_field_ref("offerpx").unwrap();
    let component = StructType::from_fields([member])
        .map(DataType::from)
        .unwrap()
        .required_field("quote");
    registry.insert(component).unwrap();

    let registry = registry.with_default_aliases().unwrap();
    let scalar = registry.field(10_040).unwrap();
    let referenced = &registry.field_by_name("quote").unwrap().fields()[0];
    assert_eq!(aliases(scalar), ["askpx", "offerprice", "askprice"]);
    assert_eq!(aliases(referenced), aliases(scalar));
    assert_eq!(referenced.as_fix().field_ref(), Some("offerpx"));
}

#[test]
fn codec_and_message_reads_and_writes_share_generated_aliases() {
    let registry = Arc::new(
        FixRegistry::from_fields([tagged("offerpx", 10_050)])
            .unwrap()
            .with_default_aliases()
            .unwrap(),
    );
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
