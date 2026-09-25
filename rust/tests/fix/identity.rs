//! `rust/src/fix/identity.rs`: the typed facts a message holds beside its row -
//! the identifier declaration, the names a parse fills from it, and the
//! category and instrument codes the fixed row carries.

use super::SoleMessage;
use super::committed_registry;
use super::fixed_codec;
use super::sequence;

mod categories {
    use std::sync::Arc;
    use yggdryl::graph::Market;
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
        // CFI keeps standard tag 461. The normalized identifiers this message
        // lifts are derived once as the message settles, then the row and tag lookup
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
        assert!(message.get_securityids().is_empty());
        assert!(message.get_cficode().is_none());
        assert!(message.get_miccode().is_none());
    }

    #[test]
    fn cusip_and_sedol_are_security_identifiers_without_columns_of_their_own() {
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
        assert_eq!(primary.get_securityids().get("CUSIP"), Some("037833100"));
        assert_eq!(primary.get_securityids().get("1"), Some("037833100"));

        let alternates = codec
            .parse_fix_line(
                b"8=FIX.4.4|35=D|11=A|454=2|455=037833100|456=1|455=B0YBKJ7|456=2|10=0|",
            )
            .expect("CUSIP and SEDOL alternate identifiers");
        assert_eq!(alternates.get_securityids().get("CUSIP"), Some("037833100"));
        assert_eq!(alternates.get_securityids().get("SEDOL"), Some("B0YBKJ7"));
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

        // The crate lifts neither into a column of its own: the retired tags
        // 65057 and 65058 name no column, and the set travels as
        // `securityids` on the graph side.
        for tag in [65_057, 65_058] {
            assert!(
                yggdryl::fix_column_of(&schema, tag).is_none(),
                "tag {tag} is retired and never reused"
            );
        }

        // An ISIN carries its country's national number as a derived
        // identifier: read off the set, never written to the wire.
        let isin = codec
            .parse_fix_line(b"8=FIX.4.4|35=D|11=I|22=4|48=US0378331005|10=0|")
            .expect("an ISIN carrying an embedded CUSIP");
        assert_eq!(isin.get_securityids().get("ISIN"), Some("US0378331005"));
        assert_eq!(isin.get_securityids().get("CUSIP"), Some("037833100"));
        let wire = String::from_utf8(isin.into_bytes(b'|')).expect("ASCII");
        assert!(
            !wire.contains("455="),
            "no alternate identifier was written: {wire}"
        );
        assert!(
            !wire.contains("22=1"),
            "and the primary still names the ISIN: {wire}"
        );
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
        let rebuilt =
            FixMsg::from_row(Arc::clone(&registry), &schema, &row).expect("a rebuilt row");
        assert_eq!(rebuilt.get_securityids().get("ISIN"), Some("US0378331005"));
        assert_eq!(
            rebuilt.get_securityids().get("CUSIP"),
            Some("037833100"),
            "the national number the ISIN carries is derived again on the way back"
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
        let rebuilt =
            FixMsg::from_row(Arc::clone(&registry), &schema, &stored).expect("a rebuilt row");
        assert_eq!(
            rebuilt.get_securityids().get("BLOOMBERG"),
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
}

mod identifiers {
    use std::sync::Arc;

    use super::SoleMessage;
    use yggdryl::graph::Operation;
    use yggdryl::{DataType, Error, Field, FixMsg, FixRegistry, Scalar, StructType, fix_schema};

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        field.as_fix_mut().set_tag(tag).unwrap();
        field
    }

    fn component() -> Field {
        let mut order = tagged("clordid", 11);
        order.as_fix_mut().set_names(["ClientOrder"]).unwrap();
        order.as_fix_mut().set_tags(&[9001]).unwrap();
        let nested = DataType::serie(
            StructType::from_fields([tagged("execid", 17)])
                .map(DataType::from)
                .unwrap()
                .required_field("item"),
        )
        .nullable_field("executions");
        StructType::from_fields([order, tagged("orderid", 37), nested])
            .map(DataType::from)
            .unwrap()
            .required_field("order")
    }

    #[test]
    fn identifier_intake_resolves_members_once_and_stores_component_order() {
        let mut field = component();
        for spellings in [["OrderID", "ClientOrder"], ["37", "11"], ["37", "9001"]] {
            field.as_fix_mut().set_identifiers(spellings).unwrap();
            assert_eq!(
                field.as_fix().identifiers().collect::<Vec<_>>(),
                ["clordid", "orderid"]
            );
            assert_eq!(
                field.get_metadata("FIX:identifiers"),
                Some("clordid,orderid")
            );
        }
        field.as_fix_mut().set_identifiers([] as [&str; 0]).unwrap();
        assert!(field.get_metadata("FIX:identifiers").is_none());
        assert_eq!(field.as_fix().identifiers().count(), 0);
    }

    #[test]
    fn identifier_refusals_are_located_atomic_and_do_not_accept_paths() {
        let mut field = component();
        field.as_fix_mut().set_identifiers(["11"]).unwrap();
        let before = field.clone();
        for bad in [
            vec![""],
            vec!["clordid,orderid"],
            vec!["unknown"],
            vec!["executions"],
            vec!["executions[0].execid"],
            vec!["11", "ClientOrder"],
            vec!["ClOrdID", "cl_ord_id"],
        ] {
            let error = field.as_fix_mut().set_identifiers(&bad).unwrap_err();
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:identifiers"),
                "{error}"
            );
            assert!(
                error.to_string().contains("order.fix:identifiers["),
                "{error}"
            );
            assert_eq!(field, before, "{bad:?}");
        }
        let mut ambiguous = tagged("another", 100);
        ambiguous.as_fix_mut().set_names(["ClientOrder"]).unwrap();
        field
            .set_dtype(DataType::from(
                StructType::from_fields(field.fields().iter().cloned().chain([ambiguous])).unwrap(),
            ))
            .unwrap();
        let before = field.clone();
        assert!(field.as_fix_mut().set_identifiers(["ClientOrder"]).is_err());
        assert_eq!(field, before);
        let mut scalar = tagged("scalar", 11);
        assert!(scalar.as_fix_mut().set_identifiers(["11"]).is_err());
    }

    #[test]
    fn incoming_identifiers_replace_the_whole_declaration_on_merge() {
        let mut stored = component();
        stored.as_fix_mut().set_identifiers(["11", "37"]).unwrap();
        let mut incoming = component();
        incoming.as_fix_mut().set_identifiers(["37"]).unwrap();
        incoming.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
        assert_eq!(
            incoming.as_fix().identifiers().collect::<Vec<_>>(),
            ["orderid"]
        );
        let mut absent = component();
        absent.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
        assert_eq!(
            absent.as_fix().identifiers().collect::<Vec<_>>(),
            ["clordid", "orderid"]
        );
    }

    #[test]
    fn registry_merge_orders_the_incoming_selection_by_the_final_members() {
        for references in [false, true] {
            let first = tagged("clordid", 11);
            let second = tagged("orderid", 37);
            let mut registry = FixRegistry::from_fields([first.clone(), second.clone()]).unwrap();
            let mut members = [first, second];
            if references {
                for child in &mut members {
                    let name = child.name().to_owned();
                    child.as_fix_mut().set_field_ref(&name).unwrap();
                }
            }
            let mut stored = StructType::from_fields(members.clone())
                .map(DataType::from)
                .unwrap()
                .required_field("order");
            stored.as_fix_mut().set_identifiers(["11"]).unwrap();
            registry.insert(stored).unwrap();
            members.reverse();
            let mut incoming = StructType::from_fields(members)
                .map(DataType::from)
                .unwrap()
                .required_field("order");
            incoming.as_fix_mut().set_identifiers(["11", "37"]).unwrap();
            assert_eq!(
                incoming.as_fix().identifiers().collect::<Vec<_>>(),
                ["orderid", "clordid"]
            );
            registry.add_field(incoming).unwrap();
            let merged = registry.field_by_name("order").unwrap();
            assert_eq!(
                merged.as_fix().identifiers().collect::<Vec<_>>(),
                ["clordid", "orderid"],
                "references={references}"
            );
        }
    }

    #[test]
    fn compiled_selection_borrows_tagged_reordered_values_and_skips_nulls_and_groups() {
        let mut definition = component();
        definition
            .as_fix_mut()
            .set_identifiers(["37", "11"])
            .unwrap();
        definition.as_fix_mut().set_msgtype("D").unwrap();
        let mut registry = FixRegistry::new();
        registry.insert(definition).unwrap();
        let registry = Arc::new(registry);
        let field = StructType::from_fields([
            tagged("venue_order", 37),
            tagged("clordid", 11),
            StructType::from_fields([tagged("execid", 17)])
                .map(DataType::from)
                .unwrap()
                .nullable_field("executions"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            field,
            Scalar::from_sequence([
                Scalar::from("O-01"),
                Scalar::Null,
                Scalar::from_sequence([Scalar::from("E-ignored")]),
            ]),
        )
        .unwrap();
        let definition = registry.msgtype("D").unwrap();
        let held = definition.identifier_values(&message).collect::<Vec<_>>();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].0.name(), "orderid");
        assert_eq!(held[0].1, message.by_tag(37).unwrap());
        assert_eq!(held[0].1.as_str(), Some("O-01"));
    }

    #[test]
    fn compiled_selection_keeps_member_identity_when_several_fields_share_a_tag() {
        let mut definition =
            StructType::from_fields([tagged("clordid", 11), tagged("venueid", 11)])
                .map(DataType::from)
                .unwrap()
                .required_field("order");
        definition
            .as_fix_mut()
            .set_identifiers(["clordid", "venueid"])
            .unwrap();
        definition.as_fix_mut().set_msgtype("D").unwrap();
        let mut registry = FixRegistry::new();
        registry.insert(definition).unwrap();
        let registry = Arc::new(registry);
        let field = StructType::from_fields([
            tagged("first_tag_holder", 11),
            tagged("venueid", 11),
            tagged("clordid", 11),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            field,
            Scalar::from_sequence([
                Scalar::from("unrelated"),
                Scalar::from("V-1"),
                Scalar::from("C-1"),
            ]),
        )
        .unwrap();
        let selected = registry
            .msgtype("D")
            .unwrap()
            .identifier_values(&message)
            .map(|(field, value)| {
                (
                    field.name().to_owned(),
                    value.as_str().expect("text").to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            selected,
            [
                ("clordid".to_owned(), "C-1".to_owned()),
                ("venueid".to_owned(), "V-1".to_owned())
            ]
        );

        let unnamed = FixMsg::with_registry(
            Arc::clone(&registry),
            StructType::from_fields([tagged("unresolved", 11)])
                .map(DataType::from)
                .unwrap()
                .required_field("row"),
            Scalar::from_sequence([Scalar::from("which-member")]),
        )
        .unwrap();
        assert_eq!(
            registry
                .msgtype("D")
                .unwrap()
                .identifier_values(&unnamed)
                .count(),
            0,
            "one row tag cannot choose between two definition members"
        );
    }

    #[test]
    fn compiled_selection_skips_a_tag_shared_by_unnamed_row_children() {
        let mut definition = StructType::from_fields([tagged("clordid", 11)])
            .map(DataType::from)
            .unwrap()
            .required_field("order");
        definition
            .as_fix_mut()
            .set_identifiers(["clordid"])
            .unwrap();
        definition.as_fix_mut().set_msgtype("D").unwrap();
        let mut registry = FixRegistry::new();
        registry.insert(definition).unwrap();
        let registry = Arc::new(registry);
        let field = StructType::from_fields([tagged("first", 11), tagged("second", 11)])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            field,
            Scalar::from_sequence([Scalar::from("A"), Scalar::from("B")]),
        )
        .unwrap();
        assert_eq!(
            registry
                .msgtype("D")
                .unwrap()
                .identifier_values(&message)
                .count(),
            0,
            "a declaration cannot choose between two unnamed tag holders"
        );
    }

    #[test]
    fn malformed_stored_declarations_are_refused_at_message_registration() {
        for text in [
            "clordid,,orderid",
            "clordid,",
            "clordid,11",
            "executions",
            "unknown",
        ] {
            let mut field = component();
            field.as_fix_mut().set_msgtype("D").unwrap();
            field
                .update_metadata([("FIX:identifiers", text.to_owned())])
                .unwrap();
            let mut registry = FixRegistry::new();
            assert!(registry.insert(field).is_err(), "{text}");
            assert!(registry.get_msgtype("D").is_none());
        }
    }

    #[test]
    fn raw_component_and_occurrence_identifiers_are_refused_before_create_or_merge() {
        for group in [false, true] {
            let definition = |field: Field| {
                if group {
                    let mut group = DataType::serie(field).nullable_field("orders");
                    group.as_fix_mut().set_counter(9001).unwrap();
                    group
                } else {
                    field
                }
            };
            for malformed in [
                "",
                "clordid,,orderid",
                "clordid,",
                "missing",
                "executions",
                "clordid,11",
            ] {
                let mut raw = component();
                raw.update_metadata([("FIX:identifiers", malformed.to_owned())])
                    .unwrap();
                let raw = definition(raw);
                let mut counter = DataType::Int32.nullable_field("noorders");
                counter.as_fix_mut().set_tag(9001).unwrap();
                let mut registry = FixRegistry::from_fields([counter]).unwrap();
                let before = registry.clone();
                let error = registry.insert(raw.clone()).unwrap_err();
                assert!(
                    matches!(error, Error::InvalidMetadataValue { .. }),
                    "{error}"
                );
                assert_eq!(
                    registry, before,
                    "create group={group}, declaration={malformed:?}"
                );

                let mut valid = component();
                valid.as_fix_mut().set_identifiers(["clordid"]).unwrap();
                registry.insert(definition(valid)).unwrap();
                let before = registry.clone();
                let error = registry.add_field(raw).unwrap_err();
                assert!(
                    matches!(error, Error::InvalidMetadataValue { .. }),
                    "{error}"
                );
                assert_eq!(
                    registry, before,
                    "merge group={group}, declaration={malformed:?}"
                );
            }
        }
    }

    #[test]
    fn raw_identifier_spellings_normalize_on_create_and_merge_before_references_compact() {
        for references in [false, true] {
            let mut first = tagged("clordid", 11);
            first.as_fix_mut().set_names(["ClientOrder"]).unwrap();
            let second = tagged("orderid", 37);
            let mut registry = FixRegistry::from_fields([first.clone(), second.clone()]).unwrap();
            let mut members = [first, second];
            if references {
                for child in &mut members {
                    let name = child.name().to_owned();
                    child.as_fix_mut().set_field_ref(&name).unwrap();
                }
            }
            let mut raw = StructType::from_fields(members)
                .map(DataType::from)
                .unwrap()
                .required_field("order");
            raw.update_metadata([("FIX:identifiers", "37,ClientOrder")])
                .unwrap();
            registry.insert(raw.clone()).unwrap();
            assert_eq!(
                registry
                    .field_by_name("order")
                    .unwrap()
                    .as_fix()
                    .identifiers()
                    .collect::<Vec<_>>(),
                ["clordid", "orderid"]
            );
            raw.update_metadata([("FIX:identifiers", "37,11")]).unwrap();
            registry.add_field(raw).unwrap();
            assert_eq!(
                registry
                    .field_by_name("order")
                    .unwrap()
                    .as_fix()
                    .identifiers()
                    .collect::<Vec<_>>(),
                ["clordid", "orderid"]
            );
        }
    }

    #[test]
    fn raw_identifier_spellings_normalize_after_inline_or_compact_json_children_resolve() {
        for references in [false, true] {
            let mut first = tagged("clordid", 11);
            first.as_fix_mut().set_names(["ClientOrder"]).unwrap();
            let second = tagged("orderid", 37);
            let definitions = [first.clone(), second.clone()];
            let members = if references {
                ["clordid", "orderid"].map(|name| {
                    let mut field = DataType::Null.nullable_field(name);
                    field.as_fix_mut().set_field_ref(name).unwrap();
                    field
                })
            } else {
                [first, second]
            };
            for declaration in ["37,11", "OrderID,ClientOrder", "orderid,clordid"] {
                let mut component = StructType::from_fields(members.clone())
                    .map(DataType::from)
                    .unwrap()
                    .required_field("order");
                component
                    .update_metadata([("FIX:identifiers", declaration)])
                    .unwrap();
                // The store's own shape: a field's `FIX:names` is the array it is
                // there, never the escaped text a native document holds.
                let snapshot = Scalar::from_struct([
                    (
                        "fields",
                        Scalar::from_sequence(
                            definitions
                                .iter()
                                .cloned()
                                .map(|field| yggdryl::into_fix_document(field).unwrap()),
                        ),
                    ),
                    (
                        "components",
                        Scalar::from_sequence([yggdryl::into_fix_document(component).unwrap()]),
                    ),
                    ("groups", Scalar::from_sequence([])),
                ])
                .unwrap();
                let registry =
                    FixRegistry::from_json(&yggdryl::into_json_scalar(&snapshot).unwrap()).unwrap();
                let component = registry.field_by_name("order").unwrap();
                assert_eq!(
                    component.as_fix().identifiers().collect::<Vec<_>>(),
                    ["clordid", "orderid"],
                    "references={references}, declaration={declaration}"
                );
                assert_eq!(
                    FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
                    registry
                );
            }
        }
    }

    /// The names a message goes by, as the event holds them.
    ///
    /// A parse fills them from the message component's identifier declaration:
    /// every declared member the message states, under the member's canonical
    /// name, in sorted order. The map is the event's, so it is read through the
    /// graph trait rather than out of the row.
    #[test]
    fn a_parse_fills_the_names_a_message_goes_by_in_sorted_order() {
        let registry = super::committed_registry();
        let codec = super::fixed_codec(Arc::clone(&registry));
        let line = b"8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|";
        let read = codec.sole_line(line).unwrap();
        assert_eq!(
            read.get_altids().iter().collect::<Vec<_>>(),
            [
                ("CLORDID", "C-001"),
                ("EXECID", "E-09"),
                ("ORDERID", "O-01")
            ]
        );
        // Filling them is not an arrival: the identifiers are the event's own
        // fact, so the wire is the line's own pairs beside what the dictionary
        // derived for it.
        assert_eq!(
            String::from_utf8(read.into_bytes(b'|')).unwrap(),
            "8=FIX.4.4|35=8|11=C-001|17=E-09|37=O-01|59=0|10=0|"
        );

        // And the row carries them, so a message read back off one goes by the
        // same names.
        let schema = fix_schema(&registry, "fix").unwrap();
        let row = read.into_row(&schema).unwrap();
        let rebuilt = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
        assert_eq!(rebuilt.get_altids(), read.get_altids());
        let array = yggdryl::Serie::from_scalars(schema.clone(), [row.clone()])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let roundtrip = yggdryl::Serie::from_arrow_array(
            Some(&schema),
            array,
            yggdryl::ArrowCastOptions::default(),
        )
        .unwrap()
        .scalar(0)
        .unwrap();
        assert_eq!(roundtrip, row);
    }

    /// A message goes by every identifier a source field states, whatever
    /// its type declares - the source table is the crate's, never the
    /// component's - and by nothing where it states none.
    #[test]
    fn a_message_goes_by_the_identifiers_its_source_fields_state() {
        let codec = super::fixed_codec(super::committed_registry());
        let named = codec.sole_line(b"8=FIX.4.4|35=ZZ|11=C-1|10=0|").unwrap();
        assert_eq!(
            named.get_altids().iter().collect::<Vec<_>>(),
            [("CLORDID", "C-1")]
        );
        let unnamed = codec.sole_line(b"8=FIX.4.4|35=ZZ|10=0|").unwrap();
        assert!(unnamed.get_altids().is_empty());
    }

    /// An identifier inside a repeating group's occurrence is that occurrence's,
    /// never the message's: only a direct member of the declaring component
    /// names the message.
    #[test]
    fn a_nested_occurrence_never_names_the_message_it_rides_in() {
        let codec = super::fixed_codec(super::committed_registry());
        let nested = codec
            .sole_line(
                b"MSGTYPE=E|#LISTID=L-1|#NOORDERS=1|#NOORDERS[0]=CLORDID=C-nested\x04\x03SYMBOL=EXAMPLE",
            )
            .unwrap();
        assert_eq!(
            nested.get_altids().get("CLORDID"),
            None,
            "the occurrence's ClOrdID names the occurrence"
        );
        // ListID(66) has no source in the crate's identifier table today, so
        // the list goes by no alternate identifier until the dictionary's
        // `FIX:idmap` names one.
        assert_eq!(nested.get_altids().get("LISTID"), None);
    }
}
