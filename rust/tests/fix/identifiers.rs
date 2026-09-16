//! Decision 21: identifiers are a declaration, and their values a Map group.

use std::sync::Arc;

use super::SoleMessage;
use yggdryl::{
    ALTIDS_TAG_NAME, DataType, Error, Field, FixCategory, FixCodec, FixMsg, FixRegistry, Scalar,
    fix_schema,
};

fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn component() -> Field {
    let mut order = tagged("clordid", 11);
    order.as_fix_mut().set_names(["ClientOrder"]).unwrap();
    order.as_fix_mut().set_tags(&[9001]).unwrap();
    let nested = DataType::list(
        DataType::from_fields([tagged("execid", 17)])
            .unwrap()
            .required_field("item"),
    )
    .nullable_field("executions");
    DataType::from_fields([order, tagged("orderid", 37), nested])
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
            field.get_metadata("fix:identifiers"),
            Some("clordid,orderid")
        );
    }
    field.as_fix_mut().set_identifiers([] as [&str; 0]).unwrap();
    assert!(field.get_metadata("fix:identifiers").is_none());
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
            matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "fix:identifiers"),
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
        .set_dtype(
            DataType::from_fields(field.fields().iter().cloned().chain([ambiguous])).unwrap(),
        )
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
        let mut stored = DataType::from_fields(members.clone())
            .unwrap()
            .required_field("order");
        stored.as_fix_mut().set_identifiers(["11"]).unwrap();
        registry
            .create_definition(FixCategory::Components, stored)
            .unwrap();
        members.reverse();
        let mut incoming = DataType::from_fields(members)
            .unwrap()
            .required_field("order");
        incoming.as_fix_mut().set_identifiers(["11", "37"]).unwrap();
        assert_eq!(
            incoming.as_fix().identifiers().collect::<Vec<_>>(),
            ["orderid", "clordid"]
        );
        registry
            .add_definition(FixCategory::Components, incoming)
            .unwrap();
        let merged = registry
            .definition(FixCategory::Components, "order")
            .unwrap();
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
    registry
        .create_definition(FixCategory::Components, definition)
        .unwrap();
    let registry = Arc::new(registry);
    let field = DataType::from_fields([
        tagged("venue_order", 37),
        tagged("clordid", 11),
        DataType::from_fields([tagged("execid", 17)])
            .unwrap()
            .nullable_field("executions"),
    ])
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
    assert!(std::ptr::eq(held[0].1, message.by_tag(37).unwrap()));
    assert_eq!(held[0].1.as_str(), Some("O-01"));
}

#[test]
fn compiled_selection_keeps_member_identity_when_several_fields_share_a_tag() {
    let mut definition = DataType::from_fields([tagged("clordid", 11), tagged("venueid", 11)])
        .unwrap()
        .required_field("order");
    definition
        .as_fix_mut()
        .set_identifiers(["clordid", "venueid"])
        .unwrap();
    definition.as_fix_mut().set_msgtype("D").unwrap();
    let mut registry = FixRegistry::new();
    registry
        .create_definition(FixCategory::Components, definition)
        .unwrap();
    let registry = Arc::new(registry);
    let field = DataType::from_fields([
        tagged("first_tag_holder", 11),
        tagged("venueid", 11),
        tagged("clordid", 11),
    ])
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
        .map(|(field, value)| (field.name(), value.as_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(selected, [("clordid", "C-1"), ("venueid", "V-1")]);

    let unnamed = FixMsg::with_registry(
        Arc::clone(&registry),
        DataType::from_fields([tagged("unresolved", 11)])
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
    let mut definition = DataType::from_fields([tagged("clordid", 11)])
        .unwrap()
        .required_field("order");
    definition
        .as_fix_mut()
        .set_identifiers(["clordid"])
        .unwrap();
    definition.as_fix_mut().set_msgtype("D").unwrap();
    let mut registry = FixRegistry::new();
    registry
        .create_definition(FixCategory::Components, definition)
        .unwrap();
    let registry = Arc::new(registry);
    let field = DataType::from_fields([tagged("first", 11), tagged("second", 11)])
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
            .update_metadata([("fix:identifiers", text.to_owned())])
            .unwrap();
        let mut registry = FixRegistry::new();
        assert!(
            registry
                .create_definition(FixCategory::Components, field)
                .is_err(),
            "{text}"
        );
        assert!(registry.get_msgtype("D").is_none());
    }
}

#[test]
fn raw_component_and_occurrence_identifiers_are_refused_before_create_or_merge() {
    for group in [false, true] {
        let category = if group {
            FixCategory::Groups
        } else {
            FixCategory::Components
        };
        let definition = |field: Field| {
            if group {
                let mut group = DataType::list(field).nullable_field("orders");
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
            raw.update_metadata([("fix:identifiers", malformed.to_owned())])
                .unwrap();
            let raw = definition(raw);
            let mut counter = DataType::Int32.nullable_field("noorders");
            counter.as_fix_mut().set_tag(9001).unwrap();
            let mut registry = FixRegistry::from_fields([counter]).unwrap();
            let before = registry.clone();
            let error = registry
                .create_definition(category, raw.clone())
                .unwrap_err();
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
            registry
                .create_definition(category, definition(valid))
                .unwrap();
            let before = registry.clone();
            let error = registry.add_definition(category, raw).unwrap_err();
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
        let mut raw = DataType::from_fields(members)
            .unwrap()
            .required_field("order");
        raw.update_metadata([("fix:identifiers", "37,ClientOrder")])
            .unwrap();
        registry
            .create_definition(FixCategory::Components, raw.clone())
            .unwrap();
        assert_eq!(
            registry
                .definition(FixCategory::Components, "order")
                .unwrap()
                .as_fix()
                .identifiers()
                .collect::<Vec<_>>(),
            ["clordid", "orderid"]
        );
        raw.update_metadata([("fix:identifiers", "37,11")]).unwrap();
        registry
            .add_definition(FixCategory::Components, raw)
            .unwrap();
        assert_eq!(
            registry
                .definition(FixCategory::Components, "order")
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
            let mut component = DataType::from_fields(members.clone())
                .unwrap()
                .required_field("order");
            component
                .update_metadata([("fix:identifiers", declaration)])
                .unwrap();
            // The store's own shape: a field's `fix:names` is the array it is
            // there, never the escaped text a native document holds.
            let snapshot = Scalar::from_record([
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
            let component = registry
                .definition(FixCategory::Components, "order")
                .unwrap();
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

fn mapping(pairs: &[(&str, &str)]) -> Scalar {
    Scalar::from_mapping(
        pairs
            .iter()
            .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
    )
    .unwrap()
}

fn custom_identifier_message(dtype: DataType, value: Scalar) -> (FixCodec, FixMsg) {
    let msgtype = tagged("msgtype", 35);
    let mut identifier = dtype.nullable_field("customid");
    identifier.as_fix_mut().set_tag(9001).unwrap();
    let mut registry = FixRegistry::from_fields([msgtype.clone(), identifier.clone()]).unwrap();
    let mut field = DataType::from_fields([msgtype, identifier])
        .unwrap()
        .required_field("custom");
    field.as_fix_mut().set_msgtype("Z9").unwrap();
    field.as_fix_mut().set_identifiers(["customid"]).unwrap();
    registry
        .create_definition(FixCategory::Components, field.clone())
        .unwrap();
    let registry = Arc::new(registry);
    let message = FixMsg::with_registry(
        Arc::clone(&registry),
        field,
        Scalar::from_sequence([Scalar::from("Z9"), value]),
    )
    .unwrap();
    (super::fixed_codec(registry), message)
}

#[test]
fn scalar_identifiers_render_through_the_existing_utf8_value_contract() {
    let value = Scalar::from(-42_i32);
    let expected = DataType::utf8().scalar(value.clone()).unwrap();
    let (codec, message) = custom_identifier_message(DataType::Int32, value.clone());
    let enriched = codec.enrich_message(message).unwrap();
    assert_eq!(enriched.by_tag(9001).unwrap(), &value);
    assert_eq!(
        enriched.by_tag(ALTIDS_TAG_NAME.0).unwrap(),
        &mapping(&[("customid", "-42")])
    );
    assert_eq!(
        enriched
            .by_tag(ALTIDS_TAG_NAME.0)
            .unwrap()
            .get_key_str("customid"),
        Some(&expected)
    );
    assert_eq!(codec.enrich_message(enriched.clone()).unwrap(), enriched);
}

#[test]
fn a_binary_identifier_that_will_not_spell_text_names_nothing() {
    let value = Scalar::from(vec![b'A', 0xff]);
    let (codec, message) = custom_identifier_message(DataType::binary(), value.clone());
    let enriched = codec.enrich_message(message).unwrap();
    // The declared identifier is unreadable as text, so it names nothing and
    // is left out of the map. It does not take the message with it: the
    // message enriches, and the bytes it stated are still its own.
    assert_eq!(enriched.by_tag(ALTIDS_TAG_NAME.0).unwrap(), &mapping(&[]));
    assert_eq!(enriched.by_tag(9001).unwrap(), &value);
    assert_eq!(codec.enrich_message(enriched.clone()).unwrap(), enriched);
}

/// One unreadable identifier costs that identifier and not the ones beside it.
#[test]
fn a_readable_identifier_beside_an_unreadable_one_still_names_itself() {
    let mut registry = FixRegistry::new();
    let mut readable = DataType::utf8().nullable_field("readableid");
    readable.as_fix_mut().set_tag(9002).unwrap();
    registry.insert(readable.clone()).unwrap();
    let mut binary = DataType::binary().nullable_field("customid");
    binary.as_fix_mut().set_tag(9001).unwrap();
    registry.insert(binary.clone()).unwrap();
    let mut msgtype = DataType::utf8().nullable_field("msgtype");
    msgtype.as_fix_mut().set_tag(35).unwrap();
    let mut field = DataType::from_fields([msgtype, binary, readable])
        .unwrap()
        .required_field("zmessage");
    field.as_fix_mut().set_msgtype("Z9").unwrap();
    field
        .as_fix_mut()
        .set_identifiers(["customid", "readableid"])
        .unwrap();
    registry
        .create_definition(FixCategory::Components, field.clone())
        .unwrap();
    let registry = Arc::new(registry);
    let message = FixMsg::with_registry(
        Arc::clone(&registry),
        field,
        Scalar::from_sequence([
            Scalar::from("Z9"),
            Scalar::from(vec![b'A', 0xff]),
            Scalar::from("R-1"),
        ]),
    )
    .unwrap();
    let enriched = super::fixed_codec(registry)
        .enrich_message(message)
        .unwrap();
    assert_eq!(
        enriched.by_tag(ALTIDS_TAG_NAME.0).unwrap(),
        &mapping(&[("readableid", "R-1")]),
    );
}

#[test]
fn enrichment_fills_sorted_identifier_text_and_preserves_arrival_and_second_pass() {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let line = b"8=FIX.4.4|35=8|37=O-01|11=C-001|17=E-09|10=0|";
    let original = codec.sole_line(line, false).unwrap();
    let enriched = codec.enrich_message(original.clone()).unwrap();
    assert_eq!(
        enriched.by_tag(ALTIDS_TAG_NAME.0).unwrap(),
        &mapping(&[
            ("clordid", "C-001"),
            ("execid", "E-09"),
            ("orderid", "O-01"),
        ])
    );
    assert_eq!(enriched.entries(), original.entries());
    assert_eq!(enriched.digest(), original.digest());
    assert_eq!(enriched.into_bytes(b'|'), line);
    assert_eq!(codec.enrich_message(enriched.clone()).unwrap(), enriched);

    let schema = fix_schema(&registry, "fix").unwrap();
    let row = enriched.into_row(&schema).unwrap();
    let rebuilt = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(
        rebuilt.by_tag(ALTIDS_TAG_NAME.0).unwrap(),
        enriched.by_tag(ALTIDS_TAG_NAME.0).unwrap()
    );
    let array = yggdryl::arrow::scalar_array(&schema, &row).unwrap();
    let roundtrip = yggdryl::arrow::scalar_value(&schema, array.as_ref()).unwrap();
    assert_eq!(roundtrip, row);
}

#[test]
fn stated_maps_including_empty_are_preserved_and_unknown_types_have_none() {
    let codec = super::fixed_codec(super::committed_registry());
    for stated in [mapping(&[("venue", "001")]), mapping(&[])] {
        let mut message = codec
            .sole_line(b"8=FIX.4.4|35=D|11=C-1|10=0|", false)
            .unwrap();
        message.set(ALTIDS_TAG_NAME.0, stated.clone()).unwrap();
        let message = codec.enrich_message(message).unwrap();
        assert_eq!(message.by_tag(ALTIDS_TAG_NAME.0).unwrap(), &stated);
        assert_eq!(codec.enrich_message(message.clone()).unwrap(), message);
    }
    for line in [b"8=FIX.4.4|35=0|10=0|".as_slice(), b"8=FIX.4.4|35=D|10=0|"] {
        let message = codec.sole_line(line, true).unwrap();
        assert_eq!(message.by_tag(ALTIDS_TAG_NAME.0).unwrap(), &mapping(&[]));
    }
    let unknown = codec
        .sole_line(b"8=FIX.4.4|35=ZZ|11=C-1|10=0|", true)
        .unwrap();
    assert!(unknown.get_by_tag(ALTIDS_TAG_NAME.0).is_none());
}

#[test]
fn enrichment_does_not_promote_an_identifier_from_a_nested_occurrence() {
    let codec = super::fixed_codec(super::committed_registry());
    let nested = codec.sole_line(
        b"MSGTYPE=E|#LISTID=L-1|#NOORDERS=1|#NOORDERS[0]=CLORDID=C-nested\x04\x03SYMBOL=EXAMPLE",
        true,
    ).unwrap();
    assert_eq!(
        nested.by_tag(ALTIDS_TAG_NAME.0).unwrap(),
        &mapping(&[("listid", "L-1")])
    );
}
