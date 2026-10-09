//! `rust/src/fix/idmap.rs`: the `FIX:idmap` document - which identifier map
//! a field's value names a message by, under which key, whether a following
//! operation carries it, and the `Parties` role stating it.

use yggdryl::fix::{FixIdMapKind, FixIdSource};
use yggdryl::{DataType, Field, FixField, FixFieldMut, FixRegistry, IdKey, IdType};

fn order() -> Field {
    let mut order = DataType::utf8().nullable_field("orderid");
    FixFieldMut::new(&mut order).set_tag(37).expect("a tag");
    order
}

fn sources(field: &Field) -> Vec<FixIdSource> {
    FixField::new(field)
        .idmap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("a readable document")
}

#[test]
fn a_source_is_written_once_and_read_back_whole() {
    let mut party = DataType::utf8().nullable_field("partyid");
    FixFieldMut::new(&mut party).set_tag(448).expect("a tag");
    let stated = [
        FixIdSource::new(FixIdMapKind::Identifiers, IdType::CustomerAccount).with_role("24"),
        FixIdSource::new(FixIdMapKind::Identifiers, IdType::EnteringFirm).with_role("7"),
    ];
    FixFieldMut::new(&mut party)
        .set_idmap(&stated)
        .expect("two sources");
    assert_eq!(
        party.get_metadata("FIX:idmap"),
        Some(
            r#"[{"map":"identifiers","key":"customeraccount","role":"24"},{"map":"identifiers","key":"enteringfirm","role":"7"}]"#
        )
    );
    assert_eq!(sources(&party), stated);
    FixFieldMut::new(&mut party).set_idmap(&[]).expect("none");
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
        "IDENTIFIERS".parse::<FixIdMapKind>().expect("folded"),
        FixIdMapKind::Identifiers
    );
    assert!("identifier".parse::<FixIdMapKind>().is_err());
    assert!("altids".parse::<FixIdMapKind>().is_err());
    // The accounts and users a message names are no identifier map.
    assert!("accountids".parse::<FixIdMapKind>().is_err());
    assert!("userids".parse::<FixIdMapKind>().is_err());
}

#[test]
fn a_source_the_document_cannot_state_is_refused_and_the_field_stands() {
    // A key is a typed word the moment a source holds it, so the one thing
    // left to refuse is a role that is no PartyRole code.
    let source =
        FixIdSource::new(FixIdMapKind::Identifiers, IdType::ExecutingTrader).with_role("a role");
    let mut field = order();
    let refusal = FixFieldMut::new(&mut field)
        .set_idmap(std::slice::from_ref(&source))
        .expect_err("refused");
    assert!(
        refusal.to_string().contains("PartyRole code"),
        "{source:?}: {refusal}"
    );
    assert_eq!(field.get_metadata("FIX:idmap"), None, "{source:?}");
    let mut field = order();
    let twice = [
        FixIdSource::new(FixIdMapKind::Identifiers, IdType::OrderId),
        FixIdSource::new(FixIdMapKind::Identifiers, IdType::OrderId).with_follow(true),
    ];
    let refusal = FixFieldMut::new(&mut field)
        .set_idmap(&twice)
        .expect_err("one key twice");
    assert!(refusal.to_string().contains("twice"), "{refusal}");
}

#[test]
fn a_hand_edited_document_is_refused_where_it_stops() {
    for stored in [
        r#"[{"map":"identifiers"}]"#,
        r#"[{"key":"orderid","map":"identifiers"}]"#,
        r#"[{"map":"identifiers","key":"orderid","follow":"yes"}]"#,
        r#"[{"map":"identifiers","key":"orderid","colour":"red"}]"#,
        r#"[{"map":"trades","key":"orderid"}]"#,
        r#"[{"map":"accountids","key":"account"}]"#,
        // A key is the folded word its type spells: neither an upper-case
        // spelling nor an alias of one.
        r#"[{"map":"identifiers","key":"ORDERID"}]"#,
        r#"[{"map":"identifiers","key":"isinnumber"}]"#,
        r#"[{"map":"altids","key":"orderid"}]"#,
    ] {
        let mut field = order();
        field
            .insert_metadata("FIX:idmap", stored)
            .expect("inert text");
        assert!(
            FixField::new(&field).idmap().any(|read| read.is_err()),
            "{stored} is refused"
        );
    }
}

/// A hand-edited word that is not what its key holds is refused naming the
/// key, what it should be and the word it holds - a role that is no
/// `PartyRole(452)` code as the writer refuses it.
#[test]
fn a_hand_edited_word_that_is_not_what_its_key_holds_is_refused_naming_the_key() {
    for (stored, expected) in [
        (
            r#"[{"map":"identifiers","key":"executingtrader","role":"a role"}]"#,
            r#"expected "role" to be a PartyRole code, got "a role""#,
        ),
        (
            r#"[{"map":"trades","key":"orderid"}]"#,
            r#"expected "map" to be an identifier map, got "trades""#,
        ),
        (
            r#"[{"map":"identifiers","key":"isinnumber"}]"#,
            r#"expected "key" to be the folded word of an identifier type, got "isinnumber""#,
        ),
    ] {
        let mut field = DataType::utf8().nullable_field("partyid");
        FixFieldMut::new(&mut field).set_tag(448).expect("a tag");
        field
            .insert_metadata("FIX:idmap", stored)
            .expect("inert text");
        let refused = FixField::new(&field)
            .idmap()
            .find_map(Result::err)
            .expect("refused")
            .to_string();
        assert!(refused.contains(expected), "{stored}: {refused}");
    }
}

#[test]
fn a_registry_takes_a_role_on_partyid_alone() {
    let mut registry = FixRegistry::new();
    let mut field = order();
    FixFieldMut::new(&mut field)
        .set_idmap(&[
            FixIdSource::new(FixIdMapKind::Identifiers, IdType::ExecutingTrader).with_role("12"),
        ])
        .expect("a document");
    let refusal = registry.add_field(field).expect_err("a role off PartyID");
    assert!(refusal.to_string().contains("PartyID(448)"), "{refusal}");
}

#[test]
fn a_store_writes_the_document_as_the_json_it_is_and_reads_it_back() {
    let mut registry = FixRegistry::new();
    let mut field = order();
    let stated = [FixIdSource::new(FixIdMapKind::Identifiers, IdType::OrderId).with_follow(true)];
    FixFieldMut::new(&mut field)
        .set_idmap(&stated)
        .expect("a document");
    registry.add_field(field).expect("a field");
    let json = registry.into_json().expect("a snapshot");
    assert!(
        json.contains(r#""FIX:idmap":[{"map":"identifiers","key":"orderid","follow":true}]"#),
        "{json}"
    );
    let again = FixRegistry::from_json(&json).expect("the snapshot reads back");
    let read = again.field_by_tag(37).expect("the field");
    assert_eq!(sources(read), stated);
}

/// The order's own identities follow a FIX message's chain, never an
/// execution's or a quote's: a message follows its dictionary's `FIX:idmap`
/// flags, where a graph leaf follows every identifier it lacks. The parents
/// of an identifier are no flag of any field: a follower takes them from its
/// chain by the parentage rule, and a bridge's own keys are read off their
/// names.
const FOLLOWED: [&str; 2] = ["orderid", "secondaryorderid"];

#[test]
fn the_committed_dictionary_follows_the_orders_own_identities() {
    let registry = super::committed_registry();
    let mut followed: Vec<&str> = registry
        .idmap_sources()
        .iter()
        .filter(|(_, source)| source.follows())
        .map(|(_, source)| source.key().as_str())
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
    // Twenty-one fields of the dictionary and none of the crate's own, whose
    // bridge keys are read off their names: the ten operation identifiers,
    // the seven secondary ones - SecondaryClOrdID(526),
    // SecondaryExecID(527), SecondaryAllocID(793),
    // SecondaryIndividualAllocID(989), SecondaryTradeID(1040),
    // SecondaryFirmTradeID(1042) and SecondaryQuoteID(1751) - and the four
    // trade lineage fields - TradeReportID(571), TradeReportRefID(572),
    // OrigTradeID(1126) and OrigSecondaryTradeID(1127); no account and no
    // user is an identifier.
    assert_eq!(sources.len(), 21, "{sources:?}");
    for secondary in [526, 527, 793, 989, 1040, 1042, 1751] {
        assert!(
            sources.iter().any(|(tag, _)| *tag == secondary),
            "{secondary} is a source: {sources:?}"
        );
    }
    assert!(
        sources
            .iter()
            .all(|(_, source)| source.map() == FixIdMapKind::Identifiers && source.role().is_none()),
        "{sources:?}"
    );
}

/// A message's parties are each `Parties(453)` occurrence's `PartyID(448)`
/// typed by its `PartyRole(452)`'s name - `partyrole{code}` for a role the
/// set does not name, `party` for none - from
/// its `PartyIDSource(447)`'s name, `base` for none; its regulatory trade
/// identifiers are identifiers typed by their
/// `RegulatoryTradeIDType(1906)`. The leaf a message becomes states both.
#[test]
fn partyids_are_typed_by_role_and_regulatory_trade_ids_are_identifiers() {
    use yggdryl::graph::{Element, MarketData, Operation};
    use yggdryl::{FixEntry, IdSource, Identifier};

    let message = super::fixed_codec(super::committed_registry())
        .parse_fix_line(
            b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|38=5|40=2|44=100|453=5|448=TRADER1|447=D|452=12|448=ACC-9|447=D|452=24|448=CA-1|452=71|448=X-1|452=999|448=NOROLE|1907=2|1903=UTI-1|1906=0|1903=TVT-1|1906=5|10=0|",
        )
        .expect("one order");
    let parties = message.get_partyids();
    assert_eq!(
        parties.get_from(&IdKey::new(IdSource::Proprietary, IdType::ExecutingTrader)),
        Some("TRADER1")
    );
    assert_eq!(
        parties.get_from(&IdKey::new(IdSource::Proprietary, IdType::CustomerAccount)),
        Some("ACC-9")
    );
    // A role's name is its type whatever its length.
    assert_eq!(
        parties.get_from(&IdKey::base(
            "competentauthoritytransactionvenue"
                .parse::<IdType>()
                .unwrap()
        )),
        Some("CA-1")
    );
    assert_eq!(
        parties.get_from(&IdKey::base("partyrole999".parse::<IdType>().unwrap())),
        Some("X-1")
    );
    assert_eq!(
        parties.get_from(&IdKey::base(IdType::Party)),
        Some("NOROLE")
    );
    let identifiers = message.get_identifiers();
    assert_eq!(
        identifiers.get_from(&IdKey::base(IdType::RegTradeId)),
        Some("UTI-1")
    );
    assert_eq!(
        identifiers.get_from(&IdKey::base(IdType::Tvtic)),
        Some("TVT-1")
    );
    assert_eq!(
        identifiers.get_from(&IdKey::base(IdType::ClOrdId)),
        Some("C1")
    );

    // A caller's party is its word: it fills a role and source the message
    // holds none of, a held one stays, and the wire is kept as sent.
    let mut written = message.clone();
    let party = |src: IdSource, kind: IdType, value: &str| {
        Identifier::new(IdKey::new(src, kind), value).expect("a party")
    };
    assert!(
        written
            .insert_partyid(party(IdSource::Base, IdType::ClientId, "C-2"))
            .unwrap()
    );
    assert!(
        !written
            .insert_partyid(party(
                IdSource::Proprietary,
                IdType::ExecutingTrader,
                "OTHER"
            ))
            .unwrap()
    );
    assert_eq!(written.get_partyids().get(&IdType::ClientId), Some("C-2"));
    assert_eq!(
        written.get_partyids().get(&IdType::ExecutingTrader),
        Some("TRADER1")
    );
    assert_eq!(
        written
            .entries()
            .iter()
            .find(|entry| entry.tag() == 453)
            .and_then(FixEntry::value),
        Some("5")
    );

    let leaves = message.into_market_data().expect("an order leaf");
    let [MarketData::OrderEvent(order)] = leaves.as_slice() else {
        panic!("one order event, got {}", leaves.len())
    };
    assert_eq!(
        order.get_partyids().get(&IdType::CustomerAccount),
        Some("ACC-9")
    );
    assert_eq!(order.get_identifiers().get(&IdType::Tvtic), Some("TVT-1"));
    assert!(
        order.get_crosscode().starts_with("10:1:"),
        "{}",
        order.get_crosscode()
    );
}

/// A name a message type declares under `FIX:identifiers` is read at the end
/// of a key on that type alone: an order cancel reject's `OMS_ListID` is its
/// `oms:listid`, captured off the row's `metadata` into `fixentries` under
/// `0:omslistid`; on a new order single, which declares no `listid`, the key
/// names nothing and stays in `metadata` as it arrived.
#[test]
fn a_declared_name_is_read_on_its_message_type_alone_and_captured_off_the_row() {
    use yggdryl::graph::Operation;

    let registry = super::committed_registry();
    let codec = super::fixed_codec(std::sync::Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
    let cell = |row: &yggdryl::Scalar, name: &str| -> Vec<(String, String)> {
        row.as_sequence().expect("a row")[schema.index_of(name).expect(name)]
            .as_mapping()
            .map(|held| {
                held.iter()
                    .map(|(key, value)| {
                        (
                            key.as_str().expect("a text key").to_owned(),
                            value.as_str().expect("a text value").to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let reject = codec
        .parse_fix_line(b"8=FIX.4.4|35=9|11=C1|41=C0|37=O1|39=8|434=1|OMS_ListID=L1|10=0|")
        .expect("a readable line");
    assert_eq!(
        reject.get_identifiers().get_from(&IdKey::new(
            "oms".parse().unwrap(),
            "listid".parse().unwrap()
        )),
        Some("L1")
    );
    let row = reject.into_row(&schema).expect("a row");
    assert!(
        cell(&row, "metadata").is_empty(),
        "{:?}",
        cell(&row, "metadata")
    );
    assert!(
        cell(&row, "fixentries").contains(&("0:omslistid".to_owned(), "L1".to_owned())),
        "{:?}",
        cell(&row, "fixentries")
    );
    let again =
        yggdryl::FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row).expect("again");
    assert_eq!(again.get_identifiers(), reject.get_identifiers());
    assert_eq!(again.into_row(&schema).expect("a row again"), row);

    let order = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=C1|OMS_ListID=L1|10=0|")
        .expect("a readable line");
    assert!(
        !order
            .get_identifiers()
            .contains_kind(&"listid".parse().unwrap()),
        "{}",
        order.get_identifiers()
    );
    let row = order.into_row(&schema).expect("a row");
    assert_eq!(
        cell(&row, "metadata"),
        [("omslistid".to_owned(), "L1".to_owned())]
    );
    assert!(
        cell(&row, "fixentries")
            .iter()
            .all(|(key, _)| !key.starts_with("0:")),
        "{:?}",
        cell(&row, "fixentries")
    );
}
