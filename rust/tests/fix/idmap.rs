//! `rust/src/fix/idmap.rs`: the `FIX:idmap` document - which identifier map
//! a field's value names a message by, under which key, whether a following
//! operation carries it, and the `Parties` role stating it.

use yggdryl::fix::{FixIdMapKind, FixIdSource};
use yggdryl::{DataType, Field, FixRegistry};

fn order() -> Field {
    let mut order = DataType::utf8().nullable_field("orderid");
    order.as_fix_mut().set_tag(37).expect("a tag");
    order
}

fn sources(field: &Field) -> Vec<FixIdSource> {
    field
        .as_fix()
        .idmap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("a readable document")
}

#[test]
fn a_source_is_written_once_and_read_back_whole() {
    let mut party = DataType::utf8().nullable_field("partyid");
    party.as_fix_mut().set_tag(448).expect("a tag");
    let stated = [
        FixIdSource::new(FixIdMapKind::Accounts, "CUSTOMERACCOUNT").with_role("24"),
        FixIdSource::new(FixIdMapKind::Users, "ENTERINGTRADER").with_role("36"),
    ];
    party.as_fix_mut().set_idmap(&stated).expect("two sources");
    assert_eq!(
        party.get_metadata("FIX:idmap"),
        Some(
            r#"[{"map":"accountids","key":"CUSTOMERACCOUNT","role":"24"},{"map":"userids","key":"ENTERINGTRADER","role":"36"}]"#
        )
    );
    assert_eq!(sources(&party), stated);
    party.as_fix_mut().set_idmap(&[]).expect("none");
    assert_eq!(party.get_metadata("FIX:idmap"), None);
    assert!(sources(&party).is_empty());
}

#[test]
fn a_map_reads_by_its_name_and_nothing_else() {
    for kind in FixIdMapKind::ALL {
        assert_eq!(kind.as_str().parse::<FixIdMapKind>().expect("a map"), kind);
        assert_eq!(kind.to_string(), kind.as_str());
    }
    assert_eq!(
        "ALTIDS".parse::<FixIdMapKind>().expect("folded"),
        FixIdMapKind::Alts
    );
    assert!("altid".parse::<FixIdMapKind>().is_err());
}

#[test]
fn a_source_the_document_cannot_state_is_refused_and_the_field_stands() {
    for (source, says) in [
        (
            FixIdSource::new(FixIdMapKind::Alts, "orderid"),
            "upper-case",
        ),
        (FixIdSource::new(FixIdMapKind::Alts, ""), "upper-case"),
        (
            FixIdSource::new(FixIdMapKind::Alts, "ORDER_ID"),
            "upper-case",
        ),
        (
            FixIdSource::new(FixIdMapKind::Alts, "X".repeat(33)),
            "upper-case",
        ),
        (
            FixIdSource::new(FixIdMapKind::Accounts, "ACCOUNT").with_follow(true),
            "altids only",
        ),
        (
            FixIdSource::new(FixIdMapKind::Users, "TRADER").with_role("a role"),
            "PartyRole code",
        ),
    ] {
        let mut field = order();
        let refusal = field
            .as_fix_mut()
            .set_idmap(std::slice::from_ref(&source))
            .expect_err("refused");
        assert!(refusal.to_string().contains(says), "{source:?}: {refusal}");
        assert_eq!(field.get_metadata("FIX:idmap"), None, "{source:?}");
    }
    let mut field = order();
    let twice = [
        FixIdSource::new(FixIdMapKind::Alts, "ORDERID"),
        FixIdSource::new(FixIdMapKind::Alts, "ORDERID").with_follow(true),
    ];
    let refusal = field
        .as_fix_mut()
        .set_idmap(&twice)
        .expect_err("one key twice");
    assert!(refusal.to_string().contains("twice"), "{refusal}");
}

#[test]
fn a_hand_edited_document_is_refused_where_it_stops() {
    for stored in [
        r#"[{"map":"altids"}]"#,
        r#"[{"key":"ORDERID","map":"altids"}]"#,
        r#"[{"map":"altids","key":"ORDERID","follow":"yes"}]"#,
        r#"[{"map":"altids","key":"ORDERID","colour":"red"}]"#,
        r#"[{"map":"trades","key":"ORDERID"}]"#,
    ] {
        let mut field = order();
        field
            .insert_metadata("FIX:idmap", stored)
            .expect("inert text");
        assert!(
            field.as_fix().idmap().any(|read| read.is_err()),
            "{stored} is refused"
        );
    }
}

#[test]
fn a_registry_takes_a_role_on_partyid_alone() {
    let mut registry = FixRegistry::new();
    let mut field = order();
    field
        .as_fix_mut()
        .set_idmap(&[FixIdSource::new(FixIdMapKind::Users, "TRADER").with_role("12")])
        .expect("a document");
    let refusal = registry.add_field(field).expect_err("a role off PartyID");
    assert!(refusal.to_string().contains("PartyID(448)"), "{refusal}");
}

#[test]
fn a_store_writes_the_document_as_the_json_it_is_and_reads_it_back() {
    let mut registry = FixRegistry::new();
    let mut field = order();
    let stated = [FixIdSource::new(FixIdMapKind::Alts, "ORDERID").with_follow(true)];
    field.as_fix_mut().set_idmap(&stated).expect("a document");
    registry.add_field(field).expect("a field");
    let json = registry.into_json().expect("a snapshot");
    assert!(
        json.contains(r#""FIX:idmap":[{"map":"altids","key":"ORDERID","follow":true}]"#),
        "{json}"
    );
    let again = FixRegistry::from_json(&json).expect("the snapshot reads back");
    let read = again.field_by_tag(37).expect("the field");
    assert_eq!(sources(read), stated);
}

#[test]
fn the_committed_dictionary_follows_exactly_what_a_plain_holder_does() {
    // `FOLLOWED_ALTIDS` is the answer a holder with no dictionary gives; the
    // shipped dictionary's follow flags are the same keys, so a FIX message
    // and a plain operation chain alike.
    let registry = super::committed_registry();
    let mut followed: Vec<&str> = registry
        .idmap_sources()
        .iter()
        .filter(|(_, source)| source.follows())
        .map(|(_, source)| source.key())
        .collect();
    followed.sort_unstable();
    let mut constant = yggdryl::graph::FOLLOWED_ALTIDS.to_vec();
    constant.sort_unstable();
    assert_eq!(followed, constant);
    // Every source the message rebuilds from, one key once per map.
    let sources = registry.idmap_sources();
    for (index, (_, source)) in sources.iter().enumerate() {
        assert!(
            !sources[..index]
                .iter()
                .any(|(_, held)| held.map() == source.map() && held.key() == source.key()),
            "{source:?} is stated once"
        );
    }
    assert_eq!(sources.len(), 24, "{sources:?}");
}
