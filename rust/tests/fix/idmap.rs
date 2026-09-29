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
        FixIdSource::new(FixIdMapKind::Alts, "CUSTOMERID").with_role("24"),
        FixIdSource::new(FixIdMapKind::Alts, "ENTERINGFIRM").with_role("7"),
    ];
    party.as_fix_mut().set_idmap(&stated).expect("two sources");
    assert_eq!(
        party.get_metadata("FIX:idmap"),
        Some(
            r#"[{"map":"altids","key":"CUSTOMERID","role":"24"},{"map":"altids","key":"ENTERINGFIRM","role":"7"}]"#
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
    // The accounts and users a message names are no identifier map.
    assert!("accountids".parse::<FixIdMapKind>().is_err());
    assert!("userids".parse::<FixIdMapKind>().is_err());
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
            FixIdSource::new(FixIdMapKind::Alts, "TRADER").with_role("a role"),
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
        r#"[{"map":"accountids","key":"ACCOUNT"}]"#,
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
        .set_idmap(&[FixIdSource::new(FixIdMapKind::Alts, "TRADER").with_role("12")])
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

/// The order's own identities follow a FIX message's chain, never an
/// execution's or a quote's: a message follows its dictionary's `FIX:idmap`
/// flags, where a graph leaf follows every identifier it lacks.
const FOLLOWED: [&str; 7] = [
    "EXCHANGECLIENTORDERID",
    "OMSDEALERPARENTORDERID",
    "ORDERID",
    "PARENTCLORDID",
    "PARENTORDERID",
    "SECONDARYORDERID",
    "TRANSVERSALKEY",
];

#[test]
fn the_committed_dictionary_follows_the_orders_own_identities() {
    let registry = super::committed_registry();
    let mut followed: Vec<&str> = registry
        .idmap_sources()
        .iter()
        .filter(|(_, source)| source.follows())
        .map(|(_, source)| source.key())
        .collect();
    followed.sort_unstable();
    assert_eq!(followed, FOLLOWED);
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
    // Ten fields of the dictionary and six of the crate's own; no account
    // and no user is an identifier.
    assert_eq!(sources.len(), 16, "{sources:?}");
    assert!(
        sources
            .iter()
            .all(|(_, source)| source.map() == FixIdMapKind::Alts && source.role().is_none()),
        "{sources:?}"
    );
}

/// A message's parties are its accounts: each `Parties(453)` occurrence's
/// `PartyID(448)` under its `PartyRole(452)`'s name - `PARTYROLE{code}` for
/// a role the set does not name or one longer than a key holds, `PARTY` for
/// none - and its regulatory trade identifiers are alternate identifiers
/// keyed by their `RegulatoryTradeIDType(1906)`. The leaf a message becomes
/// states both.
#[test]
fn parties_are_accounts_and_regulatory_trade_ids_are_alternate_identifiers() {
    use yggdryl::graph::{Element, MarketData, Operation};

    let message = super::fixed_codec(super::committed_registry())
        .parse_fix_line(
            b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|40=2|44=100|453=5|448=TRADER1|447=D|452=12|448=ACC-9|447=D|452=24|448=CA-1|452=71|448=X-1|452=999|448=NOROLE|1907=2|1903=UTI-1|1906=0|1903=TVT-1|1906=5|10=0|",
        )
        .expect("one order");
    let accounts = message.get_accountids();
    assert_eq!(accounts.get("EXECUTINGTRADER"), Some("TRADER1"));
    assert_eq!(accounts.get("CUSTOMERACCOUNT"), Some("ACC-9"));
    // `CompetentAuthorityTransactionVenue` is longer than a key holds.
    assert_eq!(accounts.get("PARTYROLE71"), Some("CA-1"));
    assert_eq!(accounts.get("PARTYROLE999"), Some("X-1"));
    assert_eq!(accounts.get("PARTY"), Some("NOROLE"));
    let altids = message.get_altids();
    assert_eq!(altids.get("REGTRADEID"), Some("UTI-1"));
    assert_eq!(altids.get("TVTIC"), Some("TVT-1"));
    assert_eq!(altids.get("CLORDID"), Some("C1"));

    // The accounts are the parties': a direct write is refused by name.
    let mut written = message.clone();
    let refused = written
        .insert_accountid("CLIENTID", "C-2")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("accountids"), "{refused}");
    assert!(
        !written
            .insert_accountid("EXECUTINGTRADER", "OTHER")
            .unwrap()
    );

    let leaves = message.into_market_data().expect("an order leaf");
    let [MarketData::OrderEvent(order)] = leaves.as_slice() else {
        panic!("one order event, got {}", leaves.len())
    };
    assert_eq!(order.get_accountids().get("CUSTOMERACCOUNT"), Some("ACC-9"));
    assert_eq!(order.get_altids().get("TVTIC"), Some("TVT-1"));
    assert!(order.get_crosscode().starts_with("BUYS:"));
}
