//! `rust/src/fix/crated.rs`: the definitions this crate invents above every
//! published tag.

use super::committed_registry;
use super::fixed_codec;

mod categories {
    use std::sync::Arc;
    use yggdryl::graph::Market;
    use yggdryl::{Figi, FixMsg, IdType, Scalar};

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
                message.get_securityids().get(&IdType::Figi),
                Some("BBG000BLNQ16")
            );
            assert!(message.get_securityids().get(&IdType::Bloomberg).is_none());
            let at = yggdryl::fix_column_of(&schema, yggdryl::FIGICODE_TAG_NAME.0)
                .expect("the FIGI fixed column");
            assert_eq!(
                message.into_row(&schema).unwrap().as_sequence().unwrap()[at].as_str(),
                Some("BBG000BLNQ16")
            );
        }
        // A FIGI whose check digit does not close is a value of rank 0, held
        // until a closing one replaces it; one of no FIGI shape - no closing
        // digit at all - stays raw.
        let unclosed = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=X|22=S|48=BBG000BLNQ17|10=0|")
            .expect("an unclosed FIGI is a value");
        assert_eq!(
            unclosed.get_securityids().get(&IdType::Figi),
            Some("BBG000BLNQ17")
        );
        assert_eq!(IdType::Figi.rank("BBG000BLNQ17"), 0);
        let malformed = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=Y|22=S|48=BBG000BLNQ1X|10=0|")
            .expect("a malformed FIGI stays raw");
        assert!(malformed.get_securityids().get(&IdType::Figi).is_none());

        let mut explicit = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=E|10=0|")
            .expect("an empty instrument");
        explicit
            .set(
                yggdryl::FIGICODE_TAG_NAME.0,
                Scalar::Figi(Figi::new("BBG000BLNQ16").unwrap()),
            )
            .expect("a direct FIGI fact");
        let row = explicit.into_row(&schema).unwrap();
        let rebuilt = FixMsg::from_row(registry, &schema, &row).unwrap();
        assert_eq!(
            rebuilt.get_securityids().get(&IdType::Figi),
            Some("BBG000BLNQ16")
        );
        assert_eq!(rebuilt.into_row(&schema).unwrap(), row);
    }
}

mod table {
    use yggdryl::graph::{ElementColumn, EventColumn, MarketColumn, OperationColumn};

    /// Every graph element and event column has a crate definition, and
    /// takes that column's datatype, display and name.
    ///
    /// The table in `rust/src/fix/crated.rs` replaced an exhaustive match on
    /// the columns, which the compiler used to check. This is that check: a
    /// column added to the graph and not to the table would otherwise have
    /// no crate tag, and a FIX row would answer it under no column at all.
    #[test]
    fn every_element_and_event_column_is_one_crate_definition_under_the_columns_own_name() {
        let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
        let columns = ElementColumn::ALL
            .into_iter()
            .map(|column| (column.name(), column.datatype(), column.display()))
            .chain(
                EventColumn::ALL
                    .into_iter()
                    .map(|column| (column.name(), column.datatype(), column.display())),
            );
        for (name, dtype, display) in columns {
            let found: Vec<_> = held.iter().filter(|field| field.name() == name).collect();
            assert_eq!(found.len(), 1, "one definition of {name}");
            let field = found[0];
            assert_eq!(
                field.dtype(),
                &dtype,
                "{name} is read at the column's datatype"
            );
            assert_eq!(
                field.display(),
                Some(display),
                "{name} displays as the column does"
            );
            let tag = field
                .as_fix()
                .tag()
                .expect("a tag reading")
                .expect("a stated tag");
            assert!(
                yggdryl::is_crate_tag(tag),
                "{name} sits in the crate's own block"
            );
        }
    }

    /// Every market and operation column is one column of the fixed row:
    /// the dictionary field that already carries its name, or one crate
    /// definition at the column's datatype. A crate definition FIX states
    /// under a name of its own is derived, and no registry holds it, so the
    /// dictionary's word aliases keep answering their spellings.
    #[test]
    fn every_market_and_operation_column_is_one_column_of_the_fixed_row() {
        let registry = super::committed_registry();
        let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
        let columns = MarketColumn::ALL
            .into_iter()
            .map(|column| (column.name(), column.datatype()))
            .chain(
                OperationColumn::ALL
                    .into_iter()
                    .map(|column| (column.name(), column.datatype())),
            );
        let mut derived = Vec::new();
        for (name, dtype) in columns {
            assert!(schema.index_of(name).is_some(), "{name} is a fixed column");
            let crated: Vec<_> = held.iter().filter(|field| field.name() == name).collect();
            match crated.as_slice() {
                [field] => {
                    assert_eq!(
                        field.dtype(),
                        &dtype,
                        "{name} is read at the column's datatype"
                    );
                    let tag = field.as_fix().tag().unwrap().unwrap();
                    assert_eq!(
                        registry.get_field_by_tag(tag).is_none()
                            && registry.get_field_by_counter(tag).is_none(),
                        yggdryl::is_derived_tag(tag),
                        "{name}: a registry holds exactly the columns no row derives"
                    );
                    if yggdryl::is_derived_tag(tag) {
                        derived.push(name);
                    }
                }
                [] => {
                    assert!(
                        registry.get_field_by_name(name).is_some_and(|field| {
                            !yggdryl::is_crate_tag(field.as_fix().tag().unwrap().unwrap())
                        }),
                        "{name} is a dictionary field"
                    )
                }
                _ => panic!("two definitions of {name}"),
            }
        }
        assert_eq!(
            derived,
            [
                "hiddenqty",
                "unit",
                "securityids",
                "prevpx",
                "prevqty",
                "spotrate",
                "forwardpoints",
                "bidqty",
                "bidccy",
                "askpx",
                "askqty",
                "askccy",
                "fxrates",
                "ticker",
                "strikepx",
                "ordqty",
                "tradable",
                "identifiers",
                "partyids",
            ]
        );
        // A spelling the dictionary reaches through its word aliases still
        // reaches the dictionary's field, not the derived column.
        for (spelled, tag) in [("askpx", 133), ("bidqty", 134), ("askqty", 135)] {
            assert_eq!(
                registry.get_field_by_name(spelled).and_then(|field| field
                    .as_fix()
                    .tag()
                    .ok()
                    .flatten()),
                Some(tag),
                "{spelled}"
            );
        }
    }

    /// The eight element and event columns FIX says more about than the
    /// graph does keep their own wording; the other seven take the column's. `execunix` is
    /// a market column, which states no wording, so it always says its own.
    ///
    /// Which is which is a judgement, so it is pinned rather than argued: a
    /// description that drifts back into restating the column is caught here
    /// and in the committed dictionary hash.
    #[test]
    fn a_crate_column_restates_the_graphs_wording_only_where_fix_adds_nothing() {
        let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
        let mut own = Vec::new();
        let columns = ElementColumn::ALL
            .into_iter()
            .map(|column| (column.name(), column.description()))
            .chain(
                EventColumn::ALL
                    .into_iter()
                    .map(|column| (column.name(), column.description())),
            );
        for (name, description) in columns {
            let field = held
                .iter()
                .find(|field| field.name() == name)
                .expect("a definition");
            if field.description() != Some(description) {
                own.push(name);
            }
        }
        own.sort_unstable();
        assert_eq!(
            own,
            [
                "creaunix",
                "crosscode",
                "currhashcode",
                "exprunix",
                "recdunix",
                "seqnum",
                "srcuuids",
                "state",
            ]
        );
        let execunix = held
            .iter()
            .find(|field| field.name() == "execunix")
            .expect("a definition");
        assert_eq!(
            execunix.display(),
            Some(yggdryl::graph::MarketColumn::ExecUnix.display())
        );
        assert!(
            execunix
                .description()
                .is_some_and(|text| text.contains("ExecutionTimestamp")),
            "names the FIX fields it is read off"
        );
    }
}

mod inferred {
    //! The bridge keys no dictionary maps: each is read as the identifier
    //! its key names - a security in `securityids`, a party in `partyids`,
    //! any other in `identifiers` - and stays as it arrived, a child of the
    //! row or, for a namespaced key, an entry of the metadata.

    use std::sync::Arc;
    use yggdryl::graph::{Market, Operation};
    use yggdryl::{FixMsg, IdKey, IdSource, IdType};

    fn parsed(line: &[u8]) -> FixMsg {
        super::fixed_codec(super::committed_registry())
            .parse_fix_line(line)
            .expect("a readable line")
    }

    fn wire(held: &FixMsg) -> String {
        String::from_utf8(held.into_bytes(b'|')).expect("a text wire")
    }

    fn src(word: &str) -> IdSource {
        word.parse().expect("a source")
    }

    fn kind(word: &str) -> IdType {
        word.parse().expect("a type")
    }

    /// What the message still states under `key`, as it arrived: the
    /// untagged child no dictionary field maps, else the namespaced entry
    /// of its metadata.
    fn kept(held: &FixMsg, key: &str) -> Option<String> {
        held.get_by_name(key)
            .and_then(|value| value.as_str().map(str::to_owned))
            .or_else(|| held.metadata().get(key).map(|value| value.to_string()))
    }

    #[test]
    fn a_bridge_instrument_is_a_security_identifier_read_off_its_key_and_kept_as_it_arrived() {
        let held = parsed(
            b"8=FIX.4.4|35=8|17=E1|55=HOLN|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|\
              ULLINKINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|",
        );
        // The source is what the rest of the key spells; the type the
        // identifier name it ends with.
        let ids = held.get_securityids();
        assert_eq!(
            ids.get_from(&IdKey::new(src("oms"), IdType::InstrumentId)),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert_eq!(
            ids.get_from(&IdKey::new(src("ullink"), IdType::InstrumentId)),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert!(held.get_identifiers().get(&IdType::InstrumentId).is_none());
        // The entries are no field of the dictionary: the message keeps
        // both as they arrived.
        for key in ["omsinstrumentid", "ullinkinstrumentid"] {
            assert_eq!(
                kept(&held, key).as_deref(),
                Some("dbi;CH0012214059_XSWX_CHF"),
                "{key}"
            );
        }
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        // Inferred, never restated: no occurrence of the wire's own states
        // either source, and the wire it writes reads back to the same
        // identifiers.
        let once = wire(&held);
        assert!(!once.contains("456="), "{once}");
        assert_eq!(wire(&parsed(once.as_bytes())), once);
        assert_eq!(parsed(once.as_bytes()).get_securityids(), ids);
    }

    /// A key reads alike however it is spelled: the fold drops case, `_`, `-`
    /// and `#`, so every spelling of one key states the one entry.
    #[test]
    fn every_spelling_of_a_key_states_the_same_entry() {
        for key in [
            "OMSINSTRUMENTID",
            "OMS_InstrumentID",
            "oms-instrumentid",
            "#OMSInstrumentID",
        ] {
            let line = format!(
                "8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|{key}=dbi;CH0012214059_XSWX_CHF|10=0|"
            );
            let held = parsed(line.as_bytes());
            assert_eq!(
                held.get_securityids()
                    .get_from(&IdKey::new(src("oms"), IdType::InstrumentId)),
                Some("dbi;CH0012214059_XSWX_CHF"),
                "{key}: {}",
                held.get_securityids()
            );
            assert!(held.anomalies().is_empty(), "{key}: {:?}", held.anomalies());
        }
        // A key naming another instrument's identifier states nothing.
        for key in ["underlyingisin", "LegSecurityIsin", "transversalkey"] {
            let line = format!("8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|{key}=US0378331005|10=0|");
            let held = parsed(line.as_bytes());
            assert!(held.get_securityids().is_empty(), "{key}");
            assert!(held.anomalies().is_empty(), "{key}: {:?}", held.anomalies());
        }
    }

    /// A namespaced key - a dot inside it - is the bridge's own spelling:
    /// it is read the same way, and it stays in the metadata, which no wire
    /// carries.
    #[test]
    fn a_namespaced_instrument_key_is_read_and_stays_in_the_metadata() {
        let held = parsed(
            b"8=FIX.4.4|35=8|17=E1|55=HOLN|ULLINK.INSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|",
        );
        assert_eq!(
            held.get_securityids()
                .get_from(&IdKey::new(src("ullink"), IdType::InstrumentId)),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert_eq!(
            held.metadata()
                .get("ullink.instrumentid")
                .map(ToString::to_string)
                .as_deref(),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert!(held.get_by_name("ullink.instrumentid").is_none());
        assert!(!wire(&held).contains("dbi;"), "{}", wire(&held));
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
    }

    /// The leaf holds what the message read in its sets and drops the keys it
    /// lifted from its metadata: a key whose set holds another value under
    /// its key is the one that stays, as it arrived.
    #[test]
    fn a_leaf_lifts_the_keys_into_its_sets_and_keeps_the_one_its_set_disagrees_with() {
        let held = parsed(
            b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|\
              OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|\
              ULLINK.INSTRUMENTID=dbi;CH0012214059_XSWX_CHF|ULLINKINSTRUMENTID=dbi;ZZ|\
              VENUEUSERID=trader1|10=0|",
        );
        let leaves = held.market_data().expect("an order");
        let [leaf] = leaves.as_slice() else {
            panic!("one order, got {}", leaves.len())
        };
        let yggdryl::graph::MarketData::OrderEvent(order) = leaf else {
            panic!("an order, got {}", leaf.kind().as_str())
        };
        assert_eq!(
            order
                .get_securityids()
                .get_from(&IdKey::new(src("oms"), IdType::InstrumentId)),
            Some("dbi;CH0012214059_XSWX_CHF")
        );
        assert_eq!(
            order
                .get_securityids()
                .get_from(&IdKey::new(src("ullink"), IdType::InstrumentId)),
            Some("dbi;CH0012214059_XSWX_CHF"),
            "the namespaced key was read first and stands"
        );
        assert_eq!(
            order
                .get_partyids()
                .get_from(&IdKey::new(src("venue"), IdType::UserId)),
            Some("trader1")
        );
        // What was lifted left the metadata; the one disagreeing key stays.
        let metadata: Vec<(String, String)> = order
            .get_metadata()
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        assert_eq!(
            metadata,
            [("ullinkinstrumentid".to_owned(), "dbi;ZZ".to_owned())]
        );
        // The message says so too, naming the key it refused.
        let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        assert_eq!(anomalies, ["ullinkinstrumentid"]);
    }

    /// The wire's own statement of a key ranks before an inferred one: the
    /// disagreeing entry is no overwrite but an anomaly naming both, and it
    /// stays as it arrived.
    #[test]
    fn a_wire_statement_under_the_same_key_ranks_first_and_the_disagreement_is_an_anomaly() {
        let held = parsed(
            b"8=FIX.4.4|35=8|17=E1|55=HOLN|454=1|455=OTHER|456=OMSINSTRUMENTID|\
              OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF|10=0|",
        );
        assert_eq!(
            held.get_securityids()
                .get_from(&IdKey::new(src("oms"), IdType::InstrumentId)),
            Some("OTHER"),
            "the wire's own statement ranks first"
        );
        let written = wire(&held);
        assert_eq!(
            written.matches("456=OMSINSTRUMENTID").count(),
            1,
            "{written}"
        );
        assert!(written.contains("455=OTHER"), "{written}");
        assert_eq!(
            kept(&held, "omsinstrumentid").as_deref(),
            Some("dbi;CH0012214059_XSWX_CHF"),
            "the entry stays as it arrived"
        );
        let anomalies: Vec<(&str, &str)> = held
            .anomalies()
            .iter()
            .map(|held| (held.field(), held.reason()))
            .collect();
        assert_eq!(
            anomalies,
            [(
                "omsinstrumentid",
                "states oms:instrumentid=dbi;CH0012214059_XSWX_CHF where oms:instrumentid=OTHER is already stated"
            )]
        );
    }

    /// A value past what an identifier holds - 64 bytes - is refused by its
    /// type: it is kept as an anomaly named by its key, and the message
    /// still re-emits it as it arrived.
    #[test]
    fn a_value_past_64_bytes_is_an_anomaly_and_still_re_emits() {
        let long = format!("dbi;CH0012214059_XSWX_CHF_{}", "0123456789".repeat(4));
        assert!(long.len() > 64);
        let line = format!("8=FIX.4.4|35=8|17=E1|55=HOLN|OMSINSTRUMENTID={long}|10=0|");
        let held = parsed(line.as_bytes());
        assert!(held.get_securityids().get(&IdType::InstrumentId).is_none());
        let written = wire(&held);
        assert!(!written.contains("456="), "{written}");
        assert!(
            written.contains(&long),
            "the value still re-emits: {written}"
        );
        let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        assert_eq!(anomalies, ["omsinstrumentid"], "{:?}", held.anomalies());
    }

    /// One past 32 bytes and within the 64 an identifier holds is a
    /// security identifier like any other: no second width stands under it.
    #[test]
    fn a_value_within_64_bytes_is_an_identifier_however_long() {
        let long = "dbi;CH0012214059_XSWX_CHF_0123456789";
        assert!(long.len() > 32 && long.len() <= 64);
        let line = format!("8=FIX.4.4|35=8|17=E1|55=HOLN|OMSINSTRUMENTID={long}|10=0|");
        let held = parsed(line.as_bytes());
        assert_eq!(
            held.get_securityids()
                .get_from(&IdKey::new(src("oms"), IdType::InstrumentId)),
            Some(long)
        );
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
        assert!(wire(&held).contains(long), "the value still re-emits");
    }

    /// An operation's identifier a bridge names is no security: it lands in
    /// `identifiers` under the source and the type its key spells, with a
    /// parentage word kept inside the type, and a parent the message states
    /// without its base states the base. A key naming no identifier is
    /// content alone.
    #[test]
    fn a_bridge_order_identifier_lands_in_identifiers_under_the_source_and_type_its_key_spells() {
        let held = parsed(
            b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|\
              PARENTORDERID=P1|OMSDEALERPARENTORDERID=OP1|EXCHANGEORDERID=X1|\
              TRADERCLORDID=U1|TRANSVERSALKEY=T1|10=0|",
        );
        let ids = held.get_identifiers();
        for (source, type_, value) in [
            ("base", "parentorderid", "P1"),
            ("omsdealer", "parentorderid", "OP1"),
            ("exchange", "orderid", "X1"),
            ("trader", "clordid", "U1"),
            // A parent states what it is a parent of, under its own source,
            // where nothing answers it: the exchange's order identifier
            // filled the base key first, so the base parent fills nothing.
            ("base", "orderid", "X1"),
            ("omsdealer", "orderid", "OP1"),
        ] {
            assert_eq!(
                ids.get_from(&IdKey::new(src(source), kind(type_))),
                Some(value),
                "{source}:{type_} in {ids}"
            );
        }
        // Each stays as it arrived.
        for (key, value) in [
            ("parentorderid", "P1"),
            ("omsdealerparentorderid", "OP1"),
            ("exchangeorderid", "X1"),
            ("traderclordid", "U1"),
        ] {
            assert_eq!(kept(&held, key).as_deref(), Some(value), "{key}");
        }
        // `TransversalKey` ends with no identifier name: nothing reads it.
        assert_eq!(kept(&held, "transversalkey").as_deref(), Some("T1"));
        assert!(
            !held.get_identifiers().iter().any(|id| id.value() == "T1"),
            "{}",
            held.get_identifiers()
        );
        assert!(
            !held.get_securityids().iter().any(|id| id.value() == "T1"),
            "{}",
            held.get_securityids()
        );
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
    }

    /// A user or an account a bridge names is a party: `partyids`, under
    /// the source its key spells.
    #[test]
    fn a_bridge_user_and_account_are_parties() {
        let held = parsed(
            b"8=FIX.4.4|35=D|11=C1|55=HOLN|54=1|40=2|\
              DEALERACCOUNT=YNHD5|DEALERUSERID=trader1|10=0|",
        );
        let parties = held.get_partyids();
        assert_eq!(
            parties.get_from(&IdKey::new(src("dealer"), IdType::UserId)),
            Some("trader1"),
            "{parties}"
        );
        assert_eq!(
            parties.get_from(&IdKey::new(src("dealer"), IdType::Account)),
            Some("YNHD5"),
            "{parties}"
        );
        assert!(
            held.get_identifiers()
                .get_from(&IdKey::new(src("dealer"), IdType::UserId))
                .is_none()
        );
        assert_eq!(kept(&held, "dealeruserid").as_deref(), Some("trader1"));
        assert_eq!(kept(&held, "dealeraccount").as_deref(), Some("YNHD5"));
    }

    /// Four bridge spellings are the dictionary's names of FIX's own
    /// fields, so they build and dump as those fields: the dealer account
    /// is `Account(1)` and so the `account` party, the trader's client
    /// order identifier `ClOrdID(11)`, the exchange's client order
    /// identifier `SecondaryClOrdID(526)` and the user `Username(553)`.
    #[test]
    fn the_bridge_names_of_fix_fields_build_and_dump_as_those_fields() {
        let held = parsed(
            b"8=FIX.4.4|35=D|55=HOLN|54=1|40=2|OMSDEALERACCOUNT=YNHD5|ULTRADERCLORDID=U1|\
              EXCHANGECLIENTORDERID=X1|OMSUSERID=trader1|10=0|",
        );
        let text = |tag: i32| {
            held.get_by_tag(tag)
                .and_then(|value| value.as_str().map(str::to_owned))
        };
        assert_eq!(text(1).as_deref(), Some("YNHD5"));
        assert_eq!(text(11).as_deref(), Some("U1"));
        assert_eq!(text(526).as_deref(), Some("X1"));
        assert_eq!(text(553).as_deref(), Some("trader1"));
        assert_eq!(
            held.get_partyids()
                .get_from(&IdKey::new(src("base"), IdType::Account)),
            Some("YNHD5")
        );
        assert_eq!(
            held.get_identifiers()
                .get_from(&IdKey::new(src("fix"), IdType::ClOrdId)),
            Some("U1")
        );
        assert_eq!(
            held.get_identifiers()
                .get_from(&IdKey::new(src("fix"), kind("secondaryclordid"))),
            Some("X1")
        );
        let wire = wire(&held);
        for pair in ["|1=YNHD5|", "|11=U1|", "|526=X1|", "|553=trader1|"] {
            assert!(wire.contains(pair), "{pair} in {wire}");
        }
        assert!(held.metadata().is_empty(), "{:?}", held.metadata());
        assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
    }

    #[test]
    fn a_bridge_key_is_no_column_of_the_fixed_row_and_reads_back_from_the_row() {
        let registry = super::committed_registry();
        let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
        for retired in [
            "omsinstrumentid",
            "ullinkinstrumentid",
            "parentorderid",
            "transversalkey",
        ] {
            assert!(schema.index_of(retired).is_none(), "{retired}");
            assert!(registry.get_field_by_name(retired).is_none(), "{retired}");
        }
        // A row keeps each among its residual entries, and reads it back
        // to the same identifiers.
        let held = parsed(
            b"8=FIX.4.4|35=D|11=C1|55=HOLN|OMSINSTRUMENTID=dbi;X_Y_Z|PARENTORDERID=P1|\
              VENUEUSERID=trader1|10=0|",
        );
        let row = held.into_row(&schema).expect("a row");
        let again = FixMsg::from_row(Arc::clone(&registry), &schema, &row).expect("the message");
        assert_eq!(
            kept(&again, "omsinstrumentid").as_deref(),
            Some("dbi;X_Y_Z")
        );
        assert_eq!(
            again
                .get_securityids()
                .get_from(&IdKey::new(src("oms"), IdType::InstrumentId)),
            Some("dbi;X_Y_Z")
        );
        assert_eq!(
            again
                .get_identifiers()
                .get_from(&IdKey::new(src("base"), kind("parentorderid"))),
            Some("P1")
        );
        assert_eq!(
            again
                .get_partyids()
                .get_from(&IdKey::new(src("venue"), IdType::UserId)),
            Some("trader1")
        );
        assert_eq!(again.into_row(&schema).unwrap(), row);
    }
}

/// The currency pair a message is about is one of the crate's instrument
/// fields: a forex column, displayed `ForexCode`, under tag 65048, the
/// first of the fixed row's instrument band.
#[test]
fn the_currency_pair_is_an_instrument_field_after_the_isin() {
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    assert_eq!(held.len(), 51);
    let at = |name: &str| {
        held.iter()
            .position(|field| field.name() == name)
            .expect("a crate field")
    };
    let pair = &held[at("forexcode")];
    // The first of the instrument fields the message's own band states,
    // the ISIN being a market column of the shared prefix.
    assert_eq!(at("forexcode"), at("conversationid") + 1);
    assert_eq!(pair.dtype(), &yggdryl::DataType::Forex);
    assert_eq!(pair.display(), Some("Forex Code"));
    assert!(pair.is_nullable());
    assert_eq!(yggdryl::FOREXCODE_TAG_NAME, (65_048, "forexcode"));
    assert_eq!(
        pair.as_fix().tag().expect("a tag reading"),
        Some(yggdryl::FOREXCODE_TAG_NAME.0)
    );
}

/// The option strike a message identifies is the market fact `strikepx`,
/// derived from the dictionary's own `StrikePrice(202)` as `prevpx` is from
/// `PrevClosePx(140)`: a crate column no registry holds, tagged after the
/// ticker in the shared prefix, while the dictionary's field keeps its place
/// in the instrument band and its name. A row stating the column is the
/// row's word, a write of the field states it again and a null clears it,
/// and a strike that reads as no decimal is an anomaly the field names.
#[test]
fn strikepx_is_a_derived_market_column_beside_the_dictionarys_strikeprice() {
    use std::sync::Arc;
    use yggdryl::graph::Market;
    use yggdryl::{Decimal, FixMsg, Scalar};

    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    let strike = held
        .iter()
        .find(|field| field.name() == "strikepx")
        .expect("the crate's strike column");
    assert_eq!(yggdryl::STRIKEPX_TAG_NAME, (65_035, "strikepx"));
    assert_eq!(
        strike.as_fix().tag().expect("a tag reading"),
        Some(yggdryl::STRIKEPX_TAG_NAME.0)
    );
    assert!(yggdryl::is_derived_tag(yggdryl::STRIKEPX_TAG_NAME.0));
    assert_eq!(strike.dtype(), &yggdryl::DataType::Decimal);
    assert_eq!(strike.display(), Some("Strike Price"));
    assert!(strike.is_nullable());
    let tags = yggdryl::fix_schema_tags();
    let place = |tag: i32| tags.iter().position(|held| *held == tag).expect("a band");
    assert_eq!(
        place(yggdryl::STRIKEPX_TAG_NAME.0),
        place(yggdryl::TICKER_TAG_NAME.0) + 1,
        "the strike follows the ticker in the shared prefix"
    );
    assert_eq!(
        place(yggdryl::METADATA_TAG_NAME.0),
        place(yggdryl::STRIKEPX_TAG_NAME.0) + 1
    );
    assert_eq!(
        place(202),
        place(yggdryl::FIGICODE_TAG_NAME.0) + 1,
        "the dictionary's strike follows the FIGI in the instrument band"
    );
    assert!(!yggdryl::is_crate_tag(202));

    let registry = committed_registry();
    assert!(
        registry
            .get_field_by_tag(yggdryl::STRIKEPX_TAG_NAME.0)
            .is_none(),
        "no registry holds a derived column"
    );
    // The column's spelling still reaches StrikePrice(202) through the
    // `px`/`price` words: a bridge writing `StrikePx` lands on the field.
    assert_eq!(
        registry
            .get_field_by_name("strikepx")
            .map(|field| field.as_fix().tag().expect("a tag reading")),
        Some(Some(202))
    );
    let codec = fixed_codec(Arc::clone(&registry));
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
    let derived = schema.index_of("strikepx").expect("the derived column");
    assert_eq!(
        yggdryl::fix_column_of(&schema, yggdryl::STRIKEPX_TAG_NAME.0),
        Some(derived)
    );
    assert_eq!(
        schema.fields()[derived].dtype(),
        &yggdryl::DataType::Decimal
    );
    let at = yggdryl::fix_column_of(&schema, 202).expect("the strike field's column");
    assert_eq!(schema.index_of("strikeprice"), Some(at));
    let column = &schema.fields()[at];
    assert_eq!(
        column.dtype(),
        &yggdryl::DataType::decimal128(38, 18).expect("the dictionary's price type")
    );
    assert_eq!(column.display(), Some("StrikePrice"));

    let option = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=O|55=XAU|201=1|202=4600.5|10=0|")
        .expect("an option order");
    let strike: Decimal = "4600.5".parse().unwrap();
    assert_eq!(option.get_strikepx(), Some(strike));
    // The leaf a message is states the strike once: as the fact, never
    // beside it as the field's metadata key.
    let leaf = option.clone().into_market_leaf().expect("an order leaf");
    assert_eq!(leaf.get_strikepx(), Some(strike));
    assert!(
        !leaf.get_metadata().contains_key("strikeprice"),
        "{:?}",
        leaf.get_metadata()
    );
    assert_eq!(
        option.by_name("strikepx").unwrap(),
        option.by_tag(202).unwrap()
    );
    let plain = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=P|55=XAU|10=0|")
        .expect("an order");
    assert_eq!(plain.get_strikepx(), None);
    let cells = plain.into_row(&schema).unwrap();
    let cells = cells.as_sequence().unwrap();
    assert!(
        cells[at].is_null() && cells[derived].is_null(),
        "no strike stated, none written"
    );

    let row = option.into_row(&schema).expect("a fixed row");
    let mut cells = row.as_sequence().expect("a row").to_vec();
    assert_eq!(Decimal::from_scalar(&cells[at]), Some(strike));
    assert_eq!(Decimal::from_scalar(&cells[derived]), Some(strike));
    // A row stating another strike under the column is the row's word; the
    // field keeps what the message sent.
    cells[derived] = Scalar::from(Decimal::from_int(4_700));
    let mut restated = FixMsg::with_registry(
        Arc::clone(&registry),
        schema.clone(),
        Scalar::from_sequence(cells),
    )
    .expect("the row read back");
    assert_eq!(restated.get_strikepx(), Some(Decimal::from_int(4_700)));
    assert_eq!(restated.by_tag(202).unwrap(), Scalar::from(strike));
    // A write of the field moves the field and leaves the row's word.
    restated
        .set(202, Scalar::from("13"))
        .expect("a strike the message can state");
    assert_eq!(restated.get_strikepx(), Some(Decimal::from_int(4_700)));
    // On a message whose strike is the field's, a write of the field states
    // the fact again, and a null clears it.
    let mut option = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=O|55=XAU|201=1|202=4600.5|10=0|")
        .expect("an option order");
    option
        .set(202, Scalar::from("13"))
        .expect("a strike the message can state");
    assert_eq!(option.get_strikepx(), Some(Decimal::from_int(13)));
    option
        .set(202, Scalar::Null)
        .expect("a strike the message can clear");
    assert_eq!(option.get_strikepx(), None);

    // A strike that reads as no decimal states none, and says so by the
    // field's name.
    let unread = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=U|55=XAU|201=1|202=abc|10=0|")
        .expect("a readable line");
    assert_eq!(unread.get_strikepx(), None);
    assert!(
        unread
            .anomalies()
            .iter()
            .any(|anomaly| anomaly.field() == "strikeprice"),
        "{:?}",
        unread.anomalies()
    );
}

/// The plugin's role is one crate field right after the plugin that logged
/// the line: required, typed as the `pluginside` enum, reading by the
/// intrinsic set, a column of the fixed row's message band, and a column
/// no FIX tag is read into - the codec stamps it from its source.
#[test]
fn the_plugin_side_is_a_required_crate_field_after_the_plugin_id_reading_the_intrinsic_set() {
    use yggdryl::{DataType, PluginSide, Scalar};

    assert_eq!(yggdryl::MSGPLUGINSIDE_TAG_NAME, (65_042, "msgpluginside"));
    assert_eq!(yggdryl::MSGPLUGINID_TAG_NAME, (65_041, "msgpluginid"));
    assert_eq!(
        yggdryl::MSGORIGINATOR_TAG_NAME,
        (65_043, "msgoriginator"),
        "every later crate tag moved up by one"
    );
    assert_eq!(yggdryl::FIXMSG_TAG_NAME, (65_052, "fixmsg"));
    let held = yggdryl::fix_crate_fields().expect("the crate's own fields");
    assert_eq!(held.len(), 51);
    let at = held
        .iter()
        .position(|field| field.name() == "msgpluginside")
        .expect("the definition");
    assert_eq!(held[at - 1].name(), "msgpluginid");
    assert_eq!(held[at + 1].name(), "msgoriginator");
    let field = &held[at];
    assert_eq!(field.dtype(), &DataType::PluginSide);
    assert!(!field.is_nullable(), "every row states it");
    assert_eq!(field.display(), Some("Message Plugin Side"));
    assert_eq!(field.as_fix().codeset(), Some("msgpluginsidecodeset"));
    assert_eq!(field.as_fix().tag().unwrap(), Some(65_042));
    assert!(
        field
            .description()
            .is_some_and(|text| text.contains("BUYS") && text.contains("Side(54)")),
        "{:?}",
        field.description()
    );
    assert_eq!(
        field.scalar("sell-side").unwrap(),
        Scalar::PluginSide(PluginSide::SellSide)
    );
    assert!(field.scalar(Scalar::Null).is_err(), "required");

    // The fixed row states it in the message band, right after the plugin
    // id, required, and the band's tags run the same way.
    let registry = super::committed_registry();
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a fixed schema");
    let columns: Vec<&str> = schema.fields().iter().map(|column| column.name()).collect();
    let column = columns
        .iter()
        .position(|name| *name == "msgpluginside")
        .expect("a column");
    assert_eq!(columns[column - 1], "msgpluginid");
    assert_eq!(columns[column + 1], "msgoriginator");
    assert!(!schema.fields()[column].is_nullable());
    assert_eq!(schema.fields()[column].dtype(), &DataType::PluginSide);
    let tags = yggdryl::fix_schema_tags();
    let tag = tags.iter().position(|tag| *tag == 65_042).expect("the tag");
    assert_eq!(tags[tag - 1], 65_041);
    assert_eq!(tags[tag + 1], 65_043);
    assert_eq!(yggdryl::fix_column_of(&schema, 65_042), Some(column));
    // Every registry holds it, reading by the intrinsic set it holds too.
    assert_eq!(
        registry.field_by_tag(65_042).unwrap().name(),
        "msgpluginside"
    );
    assert_eq!(
        registry
            .codeset("msgpluginsidecodeset")
            .expect("the intrinsic set")
            .codes()
            .count(),
        PluginSide::ALL.len()
    );
}
