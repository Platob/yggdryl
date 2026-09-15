//! FIX category storage and atomic catalog mutations.

use super::path as fpath;

use std::path::PathBuf;
use yggdryl::holder::local::Folder;
use yggdryl::{DataType, Field, FixCategory, FixCode, FixId, FixRegistry, IOBase, Scalar};

fn scratch(label: &str) -> PathBuf {
    let path = Folder::temporary().unwrap().path().unwrap().join(format!(
        "yggdryl-fix-catalog-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn catalog() -> FixRegistry {
    let counter = tagged("NoPartyIDs", 453, DataType::Int32);
    let mut partyid = tagged("PartyID", 448, DataType::utf8());
    partyid
        .as_fix_mut()
        .set_codes(&[FixCode::new("Broker", "B")])
        .unwrap();
    let mut registry = FixRegistry::from_fields([counter, partyid]).unwrap();
    let mut member = registry.field(448).unwrap().clone();
    member.as_fix_mut().set_field_ref("PartyID").unwrap();
    let component = DataType::from_fields([member])
        .unwrap()
        .required_field("Party");
    registry
        .insert_definition(FixCategory::Components, component)
        .unwrap();
    // The occurrence is the stored definition, derived tag and all, the way
    // the message below takes the stored group rather than the field it was
    // built from.
    let component = registry
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    let mut group = DataType::list(component).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component("Party").unwrap();
    registry
        .insert_definition(FixCategory::Groups, group)
        .unwrap();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    group.as_fix_mut().set_group("Parties").unwrap();
    let mut counter = registry.field(453).unwrap().clone();
    counter.as_fix_mut().set_field_ref("NoPartyIDs").unwrap();
    let mut message = DataType::from_fields([counter, group])
        .unwrap()
        .required_field("NewOrderSingle");
    message.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .insert_definition(FixCategory::Components, message)
        .unwrap();
    registry
}

#[test]
fn crate_map_groups_are_inherited_instead_of_stored_or_overridden() {
    let registry = FixRegistry::new();
    let map = registry.get_group_by_tag(65_020).unwrap();
    let mut stated = map.clone();
    stated.set_comment("not the crate's declaration").unwrap();
    let snapshot = registry.into_json().unwrap();
    assert!(!snapshot.contains("altids"));
    assert_eq!(FixRegistry::from_json(&snapshot).unwrap(), registry);

    let document = Scalar::from_record([
        ("fields", Scalar::from_sequence([])),
        ("components", Scalar::from_sequence([])),
        (
            "groups",
            Scalar::from_sequence([stated.clone().into_value()]),
        ),
    ])
    .unwrap();
    let loaded = FixRegistry::from_json(&yggdryl::into_json_scalar(&document).unwrap()).unwrap();
    assert_eq!(loaded.get_group_by_tag(65_020), Some(map));

    let root = scratch("crate-map");
    let mut folder = Folder::new(&root).unwrap();
    registry.write_into(&mut folder).unwrap();
    assert!(!root.join("groups/altids.json").exists());
    folder
        .child_by_path("groups/altids.json")
        .unwrap()
        .write_all_bytes(&stated.into_json_bytes().unwrap())
        .unwrap();
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded.get_group_by_tag(65_020), Some(map));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn builtin_map_group_references_resolve_after_snapshot_and_directory_roundtrips() {
    let mut registry = FixRegistry::new();
    let mut map = registry.get_group_by_tag(65_020).unwrap().clone();
    map.as_fix_mut().set_group("altids").unwrap();
    let component = DataType::from_fields([map])
        .unwrap()
        .required_field("identified");
    registry
        .create_definition(FixCategory::Components, component)
        .unwrap();
    let snapshot = registry.into_json().unwrap();
    let loaded = FixRegistry::from_json(&snapshot).unwrap();
    assert_eq!(loaded, registry);

    let root = scratch("crate-map-reference");
    let mut folder = Folder::new(&root).unwrap();
    registry.write_into(&mut folder).unwrap();
    assert!(!root.join("groups/altids.json").exists());
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn map_key_and_value_references_round_trip_and_refresh_from_their_owners() {
    for sorted in [false, true] {
        let mut key = tagged("lookupkey", 9001, DataType::utf8());
        key.set_nullable(false);
        let value = tagged("lookupvalue", 9002, DataType::utf8());
        let mut registry = FixRegistry::from_fields([key.clone(), value.clone()]).unwrap();
        key.as_fix_mut().set_field_ref("lookupkey").unwrap();
        let mut value = value;
        value.as_fix_mut().set_field_ref("lookupvalue").unwrap();
        let entries = DataType::from_fields([key, value])
            .unwrap()
            .required_field("entries");
        let mapping = DataType::map(entries, sorted)
            .unwrap()
            .nullable_field("pairs");
        let component = DataType::from_fields([mapping])
            .unwrap()
            .required_field("lookup");
        registry
            .create_definition(FixCategory::Components, component)
            .unwrap();

        let json = registry.into_json().unwrap();
        let snapshot = yggdryl::from_json_scalar(&json).unwrap();
        let component = Field::from_value(
            snapshot.as_record().unwrap()["components"]
                .get(0)
                .unwrap()
                .clone(),
        )
        .unwrap();
        let DataType::Map(map) = component.fields()[0].dtype() else {
            panic!("compacting references preserves the Map")
        };
        assert_eq!(map.keys_sorted(), sorted);
        assert!(!map.entries().is_nullable());
        for (index, name) in ["lookupkey", "lookupvalue"].into_iter().enumerate() {
            let child = &map.entries().fields()[index];
            assert_eq!(child.dtype(), &DataType::Null);
            assert_eq!(child.as_fix().field_ref(), Some(name));
            assert_eq!(child.is_nullable(), index == 1);
        }
        assert_eq!(FixRegistry::from_json(&json).unwrap(), registry);

        // A metadata-only update recompacts and resolves the graph. Each
        // Map child inherits its owner's update without relaxing the key.
        for tag in [9001, 9002] {
            let mut changed = registry.field_by_tag(tag).unwrap().clone();
            changed
                .as_fix_mut()
                .set_description("Reviewed lookup member")
                .unwrap();
            registry
                .update_definition(FixCategory::Fields, changed)
                .unwrap();
        }
        let component = registry
            .definition(FixCategory::Components, "lookup")
            .unwrap();
        let DataType::Map(map) = component.fields()[0].dtype() else {
            panic!("reference refresh preserves the Map")
        };
        assert_eq!(map.keys_sorted(), sorted);
        assert!(component.fields()[0].is_nullable());
        assert!(!map.entries().is_nullable());
        for (index, tag) in [9001, 9002].into_iter().enumerate() {
            let child = &map.entries().fields()[index];
            let owner = registry.field_by_tag(tag).unwrap();
            assert_eq!(child.dtype(), owner.dtype());
            assert_eq!(child.as_fix().description(), owner.as_fix().description());
            assert_eq!(child.as_fix().field_ref(), Some(owner.name()));
            assert_eq!(child.is_nullable(), index == 1);
        }
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );
        let root = scratch(if sorted {
            "sorted-map-members"
        } else {
            "map-members"
        });
        let mut folder = Folder::new(&root).unwrap();
        registry.write_into(&mut folder).unwrap();
        assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn map_entries_component_references_refresh_without_losing_the_storage_contract() {
    for sorted in [false, true] {
        let mut component = DataType::from_fields([
            DataType::utf8().required_field("key"),
            DataType::utf8().nullable_field("value"),
        ])
        .unwrap()
        .required_field("lookupentry");
        component.set_comment("Original entries").unwrap();
        let mut registry = FixRegistry::new();
        registry
            .create_definition(FixCategory::Components, component)
            .unwrap();
        let mut entries = registry
            .definition(FixCategory::Components, "lookupentry")
            .unwrap()
            .clone();
        entries.as_fix_mut().set_component("lookupentry").unwrap();
        let mapping = DataType::map(entries, sorted)
            .unwrap()
            .nullable_field("nativepairs");
        let containing = DataType::from_fields([mapping])
            .unwrap()
            .required_field("mappedlookup");
        registry
            .create_definition(FixCategory::Components, containing)
            .unwrap();

        let json = registry.into_json().unwrap();
        let document = yggdryl::from_json_scalar(&json).unwrap();
        let mut stored = Field::from_value(
            document.as_record().unwrap()["components"]
                .get(1)
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(stored.name(), "mappedlookup");
        let DataType::Map(map) = stored.fields()[0].dtype() else {
            panic!("a persisted Map keeps its entries Struct")
        };
        assert_eq!(map.keys_sorted(), sorted);
        assert!(!map.entries().is_nullable());
        assert!(!map.entries().fields()[0].is_nullable());
        assert_eq!(map.entries().as_fix().component(), Some("lookupentry"));
        assert_eq!(map.entries().comment(), None);
        assert_eq!(map.entries().as_fix().tag().unwrap(), None);
        assert_eq!(FixRegistry::from_json(&json).unwrap(), registry);

        // The storage envelope permits a Struct, not an occurrence override.
        let mut overridden = map.entries().clone();
        overridden.set_comment("An occurrence override").unwrap();
        let mut mapping = stored.fields()[0].clone();
        mapping
            .set_dtype(DataType::map(overridden, sorted).unwrap())
            .unwrap();
        stored
            .set_dtype(DataType::from_fields([mapping]).unwrap())
            .unwrap();
        let refused =
            Scalar::from_record(document.as_record().unwrap().iter().map(|(key, value)| {
                (
                    key.clone(),
                    if key == "components" {
                        Scalar::from_sequence([
                            value.get(0).unwrap().clone(),
                            stored.clone().into_value(),
                        ])
                    } else {
                        value.clone()
                    },
                )
            }))
            .unwrap();
        let error =
            FixRegistry::from_json(&yggdryl::into_json_scalar(&refused).unwrap()).unwrap_err();
        assert!(matches!(error, yggdryl::Error::Conflict { .. }));
        assert!(error.to_string().contains("occurrence metadata override"));

        let mut changed = registry
            .definition(FixCategory::Components, "lookupentry")
            .unwrap()
            .clone();
        changed.set_comment("Reviewed entries").unwrap();
        registry
            .update_definition(FixCategory::Components, changed)
            .unwrap();
        let containing = registry
            .definition(FixCategory::Components, "mappedlookup")
            .unwrap();
        let mapping = &containing.fields()[0];
        let DataType::Map(map) = mapping.dtype() else {
            panic!("reference refresh preserves Map")
        };
        assert!(mapping.is_nullable());
        assert_eq!(map.keys_sorted(), sorted);
        assert!(!map.entries().is_nullable());
        assert!(!map.entries().fields()[0].is_nullable());
        assert!(map.entries().fields()[1].is_nullable());
        assert_eq!(map.entries().comment(), Some("Reviewed entries"));
        assert_eq!(map.entries().as_fix().component(), Some("lookupentry"));
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );

        // A component fold cannot turn the entries into a three-member Struct.
        let before = registry.clone();
        let mut extended = registry
            .definition(FixCategory::Components, "lookupentry")
            .unwrap()
            .clone();
        extended
            .set_dtype(
                DataType::from_fields(
                    extended
                        .fields()
                        .iter()
                        .cloned()
                        .chain([DataType::utf8().nullable_field("extra")]),
                )
                .unwrap(),
            )
            .unwrap();
        assert!(
            registry
                .add_definition(FixCategory::Components, extended)
                .is_err()
        );
        assert_eq!(registry, before);
        assert_eq!(registry.stable_hash(), before.stable_hash());

        let root = scratch(if sorted {
            "sorted-map-component"
        } else {
            "map-component"
        });
        let mut folder = Folder::new(&root).unwrap();
        registry.write_into(&mut folder).unwrap();
        assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn ordinary_stored_component_references_still_require_null_placeholders() {
    let component = DataType::from_fields([DataType::utf8().nullable_field("value")])
        .unwrap()
        .required_field("ordinary");
    let mut occurrence = component.clone();
    occurrence.as_fix_mut().set_component("ordinary").unwrap();
    let containing = DataType::from_fields([occurrence])
        .unwrap()
        .required_field("containing");
    let document = Scalar::from_record([
        ("fields", Scalar::from_sequence([])),
        (
            "components",
            Scalar::from_sequence([component.into_value(), containing.into_value()]),
        ),
        ("groups", Scalar::from_sequence([])),
    ])
    .unwrap();
    let error = FixRegistry::from_json(&yggdryl::into_json_scalar(&document).unwrap()).unwrap_err();
    assert!(error.to_string().contains("Null placeholder datatype"));
}

#[test]
fn a_stored_builtin_group_name_cannot_be_redefined_under_another_tag() {
    let registry = FixRegistry::new();
    let map = registry.get_group_by_tag(65_020).unwrap();
    let mut substituted = map.clone();
    substituted.as_fix_mut().set_tag(9001).unwrap();
    substituted.as_fix_mut().set_counter(9001).unwrap();
    let document = Scalar::from_record([
        ("fields", Scalar::from_sequence([])),
        ("components", Scalar::from_sequence([])),
        (
            "groups",
            Scalar::from_sequence([substituted.clone().into_value()]),
        ),
    ])
    .unwrap();
    let loaded = FixRegistry::from_json(&yggdryl::into_json_scalar(&document).unwrap()).unwrap();
    assert_eq!(loaded.get_group_by_tag(65_020), Some(map));
    assert!(loaded.get_group_by_tag(9001).is_none());

    let root = scratch("crate-map-substitution");
    let folder = Folder::new(&root).unwrap();
    folder
        .child_by_path("groups/altids.json")
        .unwrap()
        .write_all_bytes(&substituted.into_json_bytes().unwrap())
        .unwrap();
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded.get_group_by_tag(65_020), Some(map));
    assert!(loaded.get_group_by_tag(9001).is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn registry_json_snapshots_preserve_the_graph_and_every_membership() {
    let mut registry = catalog();
    let mut venue = tagged("VenueTrade", 5001, DataType::utf8());
    venue
        .as_fix_mut()
        .set_branches(["CME", "merc", "cme"])
        .unwrap();
    registry
        .create_definition(FixCategory::Fields, venue)
        .unwrap();
    let mut pending = registry
        .definition(FixCategory::Components, "NewOrderSingle")
        .unwrap()
        .clone();
    pending.as_fix_mut().set_branches(["pending"]).unwrap();
    registry
        .update_definition(FixCategory::Components, pending)
        .unwrap();

    let json = registry.into_json().unwrap();
    let document = yggdryl::from_json_scalar(&json).unwrap();
    let record = document.as_record().unwrap();
    assert_eq!(record.len(), FixCategory::ALL.len());
    assert!(record.get("branches").is_none());
    for category in FixCategory::ALL {
        assert!(record[category.as_str()].as_sequence().is_some());
    }
    let group = Field::from_value(record["groups"].get(0).unwrap().clone()).unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("the native group list")
    };
    assert_eq!(item.dtype(), &DataType::Null);
    assert_eq!(item.as_fix().component(), Some("party"));

    let loaded = FixRegistry::from_json(&json).unwrap();
    assert_eq!(loaded, registry);
    assert_eq!(loaded.into_json().unwrap(), json);
    assert_eq!(loaded.stable_hash(), registry.stable_hash());
    // Membership is stamped on the field and the definition it was declared
    // on, folded once, deduplicated and sorted, and read back as written.
    let venue = loaded.field(5001).unwrap().as_fix();
    assert_eq!(venue.branches().collect::<Vec<_>>(), ["cme", "merc"]);
    assert!(venue.has_branch("Cme"));
    assert!(!venue.has_branch("pending"));
    assert!(
        loaded
            .field(448)
            .unwrap()
            .as_fix()
            .branches()
            .next()
            .is_none()
    );
    let message = loaded.msgtype("D").unwrap();
    assert!(message.as_field().as_fix().has_branch("pending"));
    assert_eq!(loaded.dialects(), ["cme", "merc", "pending"]);
    assert_eq!(loaded.field(453).unwrap().dtype(), &DataType::Int32);
    assert_eq!(
        loaded.field(448).unwrap().as_fix().code_name("B"),
        Some("Broker")
    );
    let message = loaded.msgtype("D").unwrap();
    assert_eq!(message.name(), "NewOrderSingle");
    assert_eq!(message.get_group_by_tag(453).unwrap().name(), "Parties");

    let reversed = Scalar::from_record(record.iter().map(|(key, value)| {
        (
            key.clone(),
            Scalar::from_sequence(value.as_sequence().unwrap().iter().rev().cloned()),
        )
    }))
    .unwrap();
    let reordered = FixRegistry::from_json(&yggdryl::into_json_scalar(&reversed).unwrap()).unwrap();
    assert_eq!(reordered, registry);
    assert_eq!(reordered.stable_hash(), registry.stable_hash());
    assert_eq!(reordered.into_json().unwrap(), json);
}

#[test]
fn canonical_fields_supersede_aliases_in_every_creation_and_snapshot_order() {
    let mut old = tagged("quotestatus", 297, DataType::Int32);
    old.as_fix_mut().set_aliases(["quoteackstatus"]).unwrap();
    old.as_fix_mut().set_tags(&[1865]).unwrap();
    let current = tagged("quoteackstatus", 1865, DataType::Int32);
    for fields in [
        [old.clone(), current.clone()],
        [current.clone(), old.clone()],
    ] {
        let mut registry = FixRegistry::new();
        for field in fields {
            registry
                .create_definition(FixCategory::Fields, field)
                .unwrap();
        }
        assert_eq!(registry.field("quoteackstatus").unwrap(), &current);
        assert_eq!(registry.field(1865).unwrap(), &current);
        assert_eq!(registry.len(), 2 + super::seeded_fields());
        let loaded = FixRegistry::from_json(&registry.into_json().unwrap()).unwrap();
        assert_eq!(loaded, registry);
        let document = yggdryl::from_json_scalar(registry.into_json().unwrap()).unwrap();
        let reversed =
            Scalar::from_record(document.as_record().unwrap().iter().map(|(key, value)| {
                (
                    key.clone(),
                    if key == "fields" {
                        Scalar::from_sequence(value.as_sequence().unwrap().iter().rev().cloned())
                    } else {
                        value.clone()
                    },
                )
            }))
            .unwrap();
        assert_eq!(
            FixRegistry::from_json(&yggdryl::into_json_scalar(&reversed).unwrap()).unwrap(),
            registry
        );
        let before = registry.clone();
        for duplicate in [
            current.clone(),
            tagged("quoteackstatus", 1866, DataType::Int32),
        ] {
            assert!(
                registry
                    .create_definition(FixCategory::Fields, duplicate)
                    .is_err()
            );
            assert_eq!(registry, before);
        }
        // Another name on the held tag is a field of its own beside the
        // holder, which lends nothing but learns the name; the bare tag
        // keeps answering the holder.
        registry
            .create_definition(FixCategory::Fields, tagged("other", 1865, DataType::Int32))
            .unwrap();
        assert_eq!(registry.len(), 3 + super::seeded_fields());
        assert_eq!(registry.field(1865).unwrap().name(), "quoteackstatus");
        assert_eq!(registry.field("other").unwrap().name(), "other");
        assert_eq!(
            registry
                .field(FixId::of(1865, "other").unwrap())
                .unwrap()
                .name(),
            "other"
        );
        assert!(
            registry
                .field(1865)
                .unwrap()
                .as_fix()
                .aliases()
                .any(|alias| alias == "other")
        );
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );
    }
}

#[test]
fn the_complete_committed_catalog_round_trips_through_one_snapshot() {
    let registry = super::committed_registry();
    let document = registry.into_json().unwrap();
    let loaded = FixRegistry::from_json(&document).unwrap();
    assert_eq!(&loaded, registry.as_ref());
    assert_eq!(loaded.stable_hash(), registry.stable_hash());
    assert_eq!(loaded.into_json().unwrap(), document);
    assert_eq!(loaded.len(), 6241 + super::crated_fields());
    assert_eq!(
        loaded.definitions(FixCategory::Components).count(),
        928 + super::crated_messages()
    );
    assert_eq!(loaded.msgtypes().count(), 181 + super::crated_messages());
    assert_eq!(loaded.definitions(FixCategory::Groups).count(), 581);
}

#[test]
fn registry_hashes_include_named_definitions_and_membership() {
    let original = catalog();
    let mut changed = original.clone();
    let old_message = original.msgtype("D").unwrap();
    let mut field = old_message.as_field().clone();
    field.as_fix_mut().set_msgtype("D2").unwrap();
    changed
        .update_definition(FixCategory::Components, field)
        .unwrap();
    assert_ne!(changed, original);
    assert_ne!(changed.stable_hash(), original.stable_hash());
    assert_ne!(
        changed.msgtype("D2").unwrap().stable_hash(),
        old_message.stable_hash()
    );
    assert_eq!(old_message.stable_hash(), old_message.clone().stable_hash());

    // Membership is metadata a field carries, so it is part of what the
    // registry hashes; its order is not, since the store sorts it.
    let mut declared = original.clone();
    let mut member = declared.field(448).unwrap().clone();
    member.as_fix_mut().set_branches(["pending"]).unwrap();
    declared.update(member).unwrap();
    let before = declared.stable_hash();
    assert_ne!(before, original.stable_hash());
    let mut member = declared.field(448).unwrap().clone();
    member.as_fix_mut().add_branch("venue").unwrap();
    declared.update(member).unwrap();
    assert_ne!(declared.stable_hash(), before);
    let mut reversed = original.clone();
    let mut member = reversed.field(448).unwrap().clone();
    member
        .as_fix_mut()
        .set_branches(["Venue", "pending"])
        .unwrap();
    reversed.update(member).unwrap();
    assert_eq!(reversed, declared);
    assert_eq!(reversed.stable_hash(), declared.stable_hash());
}

#[test]
fn registry_snapshots_reject_missing_categories_and_unresolved_references() {
    for json in [
        "[]",
        r#"{"fields":[],"components":[]}"#,
        r#"{"fields":[],"components":[],"groups":[],"branches":[],"codesets":[]}"#,
        // A message is a component (decision 13): a snapshot written with a
        // fourth category is refused by that key's name.
        r#"{"fields":[],"messages":[],"components":[],"groups":[]}"#,
    ] {
        assert!(FixRegistry::from_json(json).is_err(), "{json}");
    }
    let refused =
        FixRegistry::from_json(r#"{"fields":[],"messages":[],"components":[],"groups":[]}"#)
            .unwrap_err()
            .to_string();
    assert!(
        refused.contains("messages") && refused.contains("fields, components, or groups"),
        "{refused}"
    );
    // The three categories are the whole of a snapshot: nothing else is
    // declared beside them, so an empty one is a registry of the crate's own
    // fields and nothing more.
    let empty = FixRegistry::from_json(r#"{"fields":[],"components":[],"groups":[]}"#).unwrap();
    assert_eq!(empty, FixRegistry::new());
    assert_eq!(empty.len(), super::seeded_fields());
    assert!(empty.dialects().is_empty());
    let document = yggdryl::from_json_scalar(catalog().into_json().unwrap()).unwrap();
    let missing = Scalar::from_record(document.as_record().unwrap().iter().map(|(key, value)| {
        (
            key.clone(),
            if key == "components" {
                Scalar::from_sequence([])
            } else {
                value.clone()
            },
        )
    }))
    .unwrap();
    let error = FixRegistry::from_json(&yggdryl::into_json_scalar(&missing).unwrap()).unwrap_err();
    assert!(matches!(error, yggdryl::Error::Absent { .. }));
    assert!(error.to_string().contains("party"));
}

#[test]
fn categories_round_trip_compact_references_and_counter_fields() {
    let root = scratch("roundtrip");
    let mut folder = Folder::new(&root).unwrap();
    let registry = catalog();
    registry.write_into(&mut folder).unwrap();
    for file in [
        "fields/4.json",
        "components/Party.json",
        "groups/Parties.json",
        "components/NewOrderSingle.json",
    ] {
        assert!(root.join(file).is_file(), "{file}");
    }
    // A message is a component with a marker, so nothing is written beside
    // the three folders.
    assert!(!root.join("messages").exists());
    let document =
        Field::from_json_bytes(&std::fs::read(root.join("groups/Parties.json")).unwrap()).unwrap();
    let DataType::List(item) = document.dtype() else {
        panic!("a group list")
    };
    assert_eq!(item.dtype(), &DataType::Null);
    assert_eq!(item.as_fix().component(), Some("party"));
    // The stored group carries the identity the catalog derived for its name,
    // which is never the wire tag of the counter it heads.
    let derived = document
        .as_fix()
        .tag()
        .unwrap()
        .expect("a derived definition tag");
    assert!(FixId::is_definition_tag(derived), "{derived}");
    assert_ne!(derived, 453);
    assert_eq!(document.as_fix().counter().unwrap(), Some(453));
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded, registry);
    assert_eq!(
        loaded
            .definition(FixCategory::Groups, "Parties")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(derived),
        "the derived tag survives the round trip"
    );
    assert_eq!(loaded.field(453).unwrap().dtype(), &DataType::Int32);
    assert_eq!(loaded.group_by_tag(453).unwrap().name(), "Parties");
    assert_eq!(loaded.field(448).unwrap().as_fix().codes().count(), 1);
    assert_eq!(
        loaded.field(448).unwrap().as_fix().code_name("B"),
        Some("Broker")
    );
    assert!(loaded.field(448).unwrap().as_fix().get("codes").is_some());
    assert!(loaded.get_field("Parties").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_mutations_refuse_dangling_or_stale_resolved_references_atomically() {
    let mut registry = catalog();
    let before = registry.clone();
    assert!(registry.remove(453).is_none());
    assert_eq!(registry, before);
    for (category, name) in [
        (FixCategory::Fields, "NoPartyIDs"),
        (FixCategory::Fields, "PartyID"),
        (FixCategory::Components, "Party"),
        (FixCategory::Groups, "Parties"),
    ] {
        assert!(
            registry.remove_definition(category, name).is_err(),
            "{category}/{name}"
        );
        assert_eq!(registry, before);
    }
    let changed = tagged("PartyID", 448, DataType::Int32);
    assert!(
        registry
            .update_definition(FixCategory::Fields, changed)
            .is_err()
    );
    assert_eq!(registry, before);
    // The held name on another tag is the same field spelled with another
    // number, which a creation refuses: only a merge folds it in.
    assert!(
        registry
            .create_definition(
                FixCategory::Fields,
                tagged("PartyID", 5001, DataType::utf8())
            )
            .is_err()
    );
    assert_eq!(registry, before);
    assert!(
        registry
            .insert(tagged("party_id", 5001, DataType::utf8()))
            .is_err()
    );
    assert_eq!(registry, before);
    assert!(
        registry
            .update_definition(
                FixCategory::Components,
                DataType::from_fields([]).unwrap().required_field("Missing")
            )
            .is_err()
    );
    assert_eq!(registry, before);
    registry
        .remove_definition(FixCategory::Components, "NewOrderSingle")
        .unwrap()
        .unwrap();
    registry
        .remove_definition(FixCategory::Groups, "Parties")
        .unwrap()
        .unwrap();
    registry
        .remove_definition(FixCategory::Components, "Party")
        .unwrap()
        .unwrap();
    registry
        .remove_definition(FixCategory::Fields, "PartyID")
        .unwrap()
        .unwrap();
}

#[test]
fn enum_codes_belong_to_each_field() {
    let mut registry = catalog();
    // A venue's field on the standard tag, under its own name: a second
    // field beside the holder, each with the codes it declared.
    let mut field = tagged("VenuePartyID", 448, DataType::utf8());
    field.as_fix_mut().set_branches(["venue"]).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("VenueBroker", "V")])
        .unwrap();
    registry.insert(field).unwrap();
    let venue = registry
        .field(FixId::of(448, "VenuePartyID").unwrap())
        .unwrap()
        .as_fix();
    assert_eq!(venue.code_value("VenueBroker"), Some("V"));
    assert_eq!(venue.code_value("Broker"), None);
    assert!(venue.has_branch("venue"));
    assert_eq!(registry.field(448).unwrap().name(), "PartyID");
    assert_eq!(
        registry
            .field(448)
            .unwrap()
            .as_fix()
            .code_value("VenueBroker"),
        None
    );
    assert_eq!(
        registry.field(448).unwrap().as_fix().code_value("Broker"),
        Some("B")
    );
    assert_eq!(FixCategory::ALL.len(), 3);
    assert!(FixCategory::from_str("codesets").is_err());
}

#[test]
fn two_fields_on_one_tag_round_trip_through_the_snapshot_and_the_store() {
    // A dictionary's own name over a tag the standard holds is a second
    // field: it shares the holder's shard, the holder keeps the bare tag and
    // the alias it learnt, and both survive every order a store reads.
    let mut registry = catalog();
    let mut venue = tagged("VenuePartyID", 448, DataType::utf8());
    venue.as_fix_mut().set_branches(["venue"]).unwrap();
    registry.insert(venue).unwrap();
    let holder = FixId::of(448, "PartyID").unwrap();
    let newcomer = FixId::of(448, "VenuePartyID").unwrap();
    assert_ne!(holder, newcomer);
    let check = |registry: &FixRegistry| {
        assert_eq!(registry.len(), 3 + super::seeded_fields());
        assert_eq!(registry.field(448).unwrap().name(), "PartyID");
        assert_eq!(registry.field(holder).unwrap().name(), "PartyID");
        assert_eq!(registry.field(newcomer).unwrap().name(), "VenuePartyID");
        assert_eq!(
            registry.field("venue_party_id").unwrap().name(),
            "VenuePartyID"
        );
        assert!(
            registry
                .field(newcomer)
                .unwrap()
                .as_fix()
                .has_branch("venue")
        );
        assert_eq!(
            registry
                .field(448)
                .unwrap()
                .as_fix()
                .aliases()
                .collect::<Vec<_>>(),
            ["VenuePartyID"]
        );
        assert!(
            registry
                .field(448)
                .unwrap()
                .as_fix()
                .branches()
                .next()
                .is_none()
        );
        // Tag-major, the holder first: the two share a tag and sit side by
        // side, the one the bare tag answers leading, which is what a store
        // writes first and a reader loads first.
        let tags: Vec<i32> = registry
            .iter()
            .filter_map(|field| field.as_fix().tag().unwrap())
            .filter(|tag| *tag < yggdryl::CRATE_TAG_MIN)
            .collect();
        assert_eq!(tags, [52, 60, 448, 448, 453]);
        let on_448: Vec<FixId> = registry
            .iter()
            .filter(|field| field.as_fix().tag().unwrap() == Some(448))
            .map(|field| field.as_fix().id().unwrap().unwrap())
            .collect();
        assert_eq!(
            on_448[0],
            registry.field(448).unwrap().as_fix().id().unwrap().unwrap(),
            "{on_448:?}"
        );
        assert_eq!(
            registry
                .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
                .unwrap()
                .as_fix()
                .code_name("B"),
            Some("Broker")
        );
    };
    check(&registry);

    let json = registry.into_json().unwrap();
    let loaded = FixRegistry::from_json(&json).unwrap();
    assert_eq!(loaded, registry);
    check(&loaded);
    // The array order is the arrival order, and the holder of a shared tag
    // is decided by arrival: a document that lists the two the other way
    // round describes the other field as the holder, with the alias lent
    // the other way - a different dictionary, loaded as written.
    let document = yggdryl::from_json_scalar(&json).unwrap();
    let reversed = Scalar::from_record(document.as_record().unwrap().iter().map(|(key, value)| {
        (
            key.clone(),
            Scalar::from_sequence(value.as_sequence().unwrap().iter().rev().cloned()),
        )
    }))
    .unwrap();
    let reordered = FixRegistry::from_json(&yggdryl::into_json_scalar(&reversed).unwrap()).unwrap();
    assert_ne!(reordered, registry);
    assert_eq!(reordered.field(448).unwrap().name(), "VenuePartyID");
    assert_eq!(
        reordered
            .field(448)
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["PartyID"]
    );
    assert_eq!(reordered.len(), registry.len());
    assert_eq!(
        FixRegistry::from_json(&reordered.into_json().unwrap()).unwrap(),
        reordered,
        "what a store writes, it reads back exactly, whichever field holds the tag"
    );

    let root = scratch("two-on-one-tag");
    let mut folder = Folder::new(&root).unwrap();
    registry.write_into(&mut folder).unwrap();
    assert!(root.join("fields/4.json").is_file());
    let shard =
        yggdryl::from_json_scalar(std::fs::read(root.join("fields/4.json")).unwrap()).unwrap();
    assert_eq!(shard.as_sequence().unwrap().len(), 3);
    let stored = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(stored, registry);
    check(&stored);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn contexts_sharing_a_counter_are_explicitly_ambiguous() {
    let mut registry = catalog();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    group.set_name("TradeParties");
    registry
        .insert_definition(FixCategory::Groups, group)
        .unwrap();
    assert!(registry.get_group_by_tag(453).is_none());
    assert!(registry.group_by_tag(453).is_err());
    assert_eq!(registry.definitions(FixCategory::Groups).count(), 3);
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
}

#[test]
fn store_removes_empty_shards_and_named_documents() {
    let root = scratch("cleanup");
    let mut folder = Folder::new(&root).unwrap();
    let mut registry = catalog();
    registry
        .insert(tagged("Distant", 10000, DataType::utf8()))
        .unwrap();
    registry.write_into(&mut folder).unwrap();
    registry.remove(10000).unwrap();
    registry
        .remove_definition(FixCategory::Components, "NewOrderSingle")
        .unwrap();
    registry.write_into(&mut folder).unwrap();
    assert!(!root.join("fields/100.json").exists());
    assert!(!root.join("components/NewOrderSingle.json").exists());
    assert!(root.join("components/Party.json").exists());
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unresolved_and_cyclic_compact_references_name_the_failure() {
    for (label, reference) in [("missing", "Missing"), ("cycle", "Cycle")] {
        let root = scratch(label);
        let folder = Folder::new(&root).unwrap();
        let mut child = DataType::Null.nullable_field("child");
        child.as_fix_mut().set_component(reference).unwrap();
        let field = DataType::from_fields([child])
            .unwrap()
            .required_field("Cycle");
        folder
            .child_by_path("components/Cycle.json")
            .unwrap()
            .write_all_bytes(&field.into_json_bytes().unwrap())
            .unwrap();
        let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
        assert!(
            error
                .to_ascii_lowercase()
                .contains(&reference.to_ascii_lowercase()),
            "{error}"
        );
        assert!(error.contains("Cycle.json"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn malformed_shards_are_located_and_nested_folders_are_passed_over() {
    let root = scratch("malformed");
    let folder = Folder::new(&root).unwrap();
    for bytes in [
        b"not json".to_vec(),
        b"{}".to_vec(),
        yggdryl::text::json::into_bytes(&Scalar::from_sequence([tagged(
            "Misplaced",
            150,
            DataType::utf8(),
        )
        .into_value()]))
        .unwrap(),
    ] {
        folder
            .child_by_path("fields/0.json")
            .unwrap()
            .write_all_bytes(&bytes)
            .unwrap();
        let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
        assert!(error.contains("0.json"), "{error}");
    }
    folder
        .child_by_path("fields/0.json")
        .unwrap()
        .remove(false)
        .unwrap();
    // A shard whose number disagrees with the tags inside is located by
    // its own name.
    let field = tagged("Misplaced", 5001, DataType::utf8());
    let bytes =
        yggdryl::text::json::into_bytes(&Scalar::from_sequence([field.clone().into_value()]))
            .unwrap();
    folder
        .child_by_path("fields/49.json")
        .unwrap()
        .write_all_bytes(&bytes)
        .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(error.contains("49.json"), "{error}");
    folder
        .child_by_path("fields/49.json")
        .unwrap()
        .remove(false)
        .unwrap();
    // There are no per-dictionary folders: a folder inside a category is
    // not a store's layout, so what it holds is passed over rather than read
    // as a dictionary's own shard.
    folder
        .child_by_path("fields/venue/50.json")
        .unwrap()
        .write_all_bytes(&bytes)
        .unwrap();
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert!(loaded.get_field(5001).is_none());
    assert_eq!(loaded.len(), super::seeded_fields());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tracked_seed_resolves_every_category_and_native_reference_graph() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap();
    assert_eq!(registry.len(), 6241 + super::crated_fields());
    // The census decision 13 rests on: 747 components and 181 messages fold
    // to 928 distinct names, so no message and component share one.
    for (category, count) in [
        (FixCategory::Components, 928 + super::crated_messages()),
        (FixCategory::Groups, 581),
    ] {
        assert_eq!(registry.definitions(category).count(), count, "{category}");
    }
    assert_eq!(registry.msgtypes().count(), 181 + super::crated_messages());
    assert_eq!(
        registry
            .definitions(FixCategory::Components)
            .filter(|field| field.as_fix().msgtype().is_some())
            .count(),
        181 + super::crated_messages()
    );
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
    let group = registry.definition(FixCategory::Groups, "Parties").unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("parties list")
    };
    assert_eq!(item.name(), "party");
    assert_eq!(
        item.get_field("partyid").unwrap().as_fix().tag().unwrap(),
        Some(448)
    );
    assert_eq!(
        registry.field(54).unwrap().as_fix().code_name("1"),
        Some("Buy")
    );
}

#[test]
fn named_definition_equality_does_not_depend_on_insertion_order() {
    let mut left = FixRegistry::new();
    let mut right = FixRegistry::new();
    for name in ["A", "B"] {
        left.insert_definition(
            FixCategory::Components,
            DataType::from_fields([]).unwrap().required_field(name),
        )
        .unwrap();
    }
    for name in ["B", "A"] {
        right
            .insert_definition(
                FixCategory::Components,
                DataType::from_fields([]).unwrap().required_field(name),
            )
            .unwrap();
    }
    assert_eq!(left, right);
    assert!(!left.is_empty());
    assert_eq!(
        left.definition_at(FixCategory::Components, 0)
            .unwrap()
            .name(),
        "A"
    );
    assert_eq!(
        left.definition_at(FixCategory::Components, 1)
            .unwrap()
            .name(),
        "B"
    );
    // The two this test added, and behind them the one every registry starts
    // with.
    assert!(
        left.definition_at(FixCategory::Components, 2 + super::crated_messages())
            .is_none()
    );
    assert!(
        left.definition_at(FixCategory::Components, usize::MAX)
            .is_none()
    );
}

#[test]
fn merging_a_complete_catalog_commits_references_together() {
    let source = catalog();
    let mut target = FixRegistry::new();
    // SendingTime and TransactTime are real stored definitions, not crate fields.
    assert_eq!(target.merge_with(&source).unwrap(), (2, 2));
    assert_eq!(target, source);
    assert_eq!(target.merge_with(&source).unwrap(), (0, 4));
    assert_eq!(target, source);
}

#[test]
fn merging_folded_named_definitions_preserves_canonical_names_and_references() {
    let original = catalog();
    // Every spelling the fold reads as one name: another case, and a
    // separator the fold drops. Built one definition at a time, so in
    // dependency order: the component, the group over it, then the message
    // over the group - a message is a component (decision 13), so name order
    // alone would put `NewOrderSingle` before the `Parties` it references.
    let source = |respell: fn(&str) -> String| {
        let mut source = FixRegistry::from_fields(original.iter().cloned()).unwrap();
        let plain = original
            .definitions(FixCategory::Components)
            .filter(|field| field.as_fix().msgtype().is_none())
            .map(|field| (FixCategory::Components, field));
        let groups = original
            .definitions(FixCategory::Groups)
            .filter(|field| {
                !field
                    .as_fix()
                    .tag()
                    .unwrap()
                    .is_some_and(yggdryl::is_crate_tag)
            })
            .map(|field| (FixCategory::Groups, field));
        let messages = original
            .msgtypes()
            // The crate's own is already in the source, which starts from a
            // registry of its own: respelling it would restate it.
            .filter(|held| held.name() != yggdryl::PLUGINCONFIG_CODE_NAME.1)
            .map(|held| (FixCategory::Components, held.as_field()));
        for (category, field) in plain.chain(groups).chain(messages) {
            let mut field = field.clone();
            field.set_name(respell(field.name()));
            source.create_definition(category, field).unwrap();
        }
        source
    };
    for respell in [str::to_ascii_uppercase as fn(&str) -> String, |name| {
        format!("_{name}")
    }] {
        let mut target = original.clone();
        let incoming = source(respell);
        assert_eq!(target.merge_with(&incoming).unwrap(), (0, 4));
        assert_eq!(target, original);
        for (category, name) in [
            (FixCategory::Components, "Party"),
            (FixCategory::Groups, "Parties"),
            (FixCategory::Components, "NewOrderSingle"),
        ] {
            assert_eq!(target.definition(category, name).unwrap().name(), name);
        }
        let group = target.msgtype("D").unwrap().get_group_by_tag(453).unwrap();
        assert_eq!(group.name(), "Parties");
        assert_eq!(
            target
                .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
                .unwrap()
                .as_fix()
                .code_name("B"),
            Some("Broker")
        );
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );
    }
}

#[test]
fn merging_catalogs_resolves_imported_references_against_the_inline_code_union() {
    let mut target = catalog();
    let mut source = catalog();
    let mut coded = source.field(448).unwrap().clone();
    coded
        .as_fix_mut()
        .set_codes(&[FixCode::new("Client", "C")])
        .unwrap();
    source.insert(coded).unwrap();
    let mut message = source
        .definition(FixCategory::Components, "NewOrderSingle")
        .unwrap()
        .clone();
    message.set_name("IncomingOrder");
    message.as_fix_mut().set_msgtype("I").unwrap();
    source
        .create_definition(FixCategory::Components, message)
        .unwrap();
    let before_source = source.clone();

    assert_eq!(target.merge_with(&source).unwrap(), (0, 4));
    for path in [
        "PartyID",
        "Party.PartyID",
        "Parties.PartyID",
        "NewOrderSingle.Parties.PartyID",
        "IncomingOrder.Parties.PartyID",
    ] {
        let field = target.field_by_path(&fpath(path)).unwrap();
        assert_eq!(field.as_fix().code_name("B"), Some("Broker"), "{path}");
        assert_eq!(field.as_fix().code_name("C"), Some("Client"), "{path}");
    }
    let group = target.msgtype("I").unwrap().get_group_by_tag(453).unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("the resolved group list")
    };
    assert_eq!(
        item.get_field("PartyID")
            .unwrap()
            .as_fix()
            .code_value("Broker"),
        Some("B")
    );
    assert_eq!(source, before_source);
    assert_eq!(
        FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
        target
    );
}

#[test]
fn merging_catalogs_extends_referenced_definitions_and_refuses_a_changed_member_atomically() {
    let mut target = catalog();
    let mut coded = target.field(448).unwrap().clone();
    coded
        .as_fix_mut()
        .set_codes(&[FixCode::new("Client", "C")])
        .unwrap();
    coded.as_fix_mut().set_branches(["incoming"]).unwrap();
    let mut source = FixRegistry::from_fields([coded]).unwrap();
    let mut member = source.field(448).unwrap().clone();
    member.as_fix_mut().set_field_ref("PartyID").unwrap();
    let extended = DataType::from_fields([member, DataType::Int32.nullable_field("Extra")])
        .unwrap()
        .required_field("Party");
    source
        .create_definition(FixCategory::Components, extended)
        .unwrap();
    let before_source = source.clone();

    // The member the source adds to the component reaches the group and the
    // message that restate it, through the references they keep.
    assert_eq!(target.merge_with(&source).unwrap(), (0, 3));
    for path in [
        "Party.Extra",
        "Parties.Extra",
        "NewOrderSingle.Parties.Extra",
    ] {
        assert_eq!(
            target.field_by_path(&fpath(path)).unwrap().dtype(),
            &DataType::Int32,
            "{path}"
        );
    }
    assert_eq!(
        target
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
            .unwrap()
            .as_fix()
            .code_name("C"),
        Some("Client")
    );
    // The membership the source stamped unions onto the standard field the
    // target already held, and reaches its occurrences the same way.
    assert!(target.field(448).unwrap().as_fix().has_branch("incoming"));
    assert!(
        target
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
            .unwrap()
            .as_fix()
            .has_branch("incoming")
    );
    assert_eq!(target.dialects(), ["incoming"]);
    assert_eq!(source, before_source);
    assert_eq!(
        FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
        target
    );

    // A member both hold under another datatype refuses the whole merge.
    let before = target.clone();
    let mut member = source.field(448).unwrap().clone();
    member.as_fix_mut().set_field_ref("PartyID").unwrap();
    let changed = DataType::from_fields([member, DataType::Int64.nullable_field("Extra")])
        .unwrap()
        .required_field("Party");
    source
        .insert_definition(FixCategory::Components, changed)
        .unwrap();
    let error = target.merge_with(&source).unwrap_err().to_string();
    assert!(error.contains("Party.Extra"), "{error}");
    assert_eq!(target, before);
    assert_eq!(target.stable_hash(), before.stable_hash());
}

#[test]
fn referenced_metadata_updates_cascade_and_occurrence_overrides_fail_without_loss() {
    let mut registry = catalog();
    let mut component = registry
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    component
        .as_fix_mut()
        .set_description("A changed description")
        .unwrap();
    registry
        .update_definition(FixCategory::Components, component)
        .unwrap();
    let DataType::List(item) = registry
        .field_by_path(&fpath("NewOrderSingle.Parties"))
        .unwrap()
        .dtype()
    else {
        panic!("a group list")
    };
    assert_eq!(item.as_fix().description(), Some("A changed description"));
    let mut scalar = registry.field(448).unwrap().clone();
    scalar
        .as_fix_mut()
        .set_description("A party identifier")
        .unwrap();
    registry
        .update_definition(FixCategory::Fields, scalar)
        .unwrap();
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
            .unwrap()
            .as_fix()
            .description(),
        Some("A party identifier")
    );
    let mut group = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    group
        .as_fix_mut()
        .set_description("Reviewed parties")
        .unwrap();
    registry
        .update_definition(FixCategory::Groups, group)
        .unwrap();
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties"))
            .unwrap()
            .as_fix()
            .description(),
        Some("Reviewed parties")
    );
    let root = scratch("metadata-refresh");
    let mut folder = Folder::new(&root).unwrap();
    registry.write_into(&mut folder).unwrap();
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();
    let before = registry.clone();
    let mut child = registry.field(448).unwrap().clone();
    child.as_fix_mut().set_field_ref("PartyID").unwrap();
    child
        .as_fix_mut()
        .set_description("An occurrence override")
        .unwrap();
    let component = DataType::from_fields([child])
        .unwrap()
        .required_field("OverriddenParty");
    assert!(
        registry
            .insert_definition(FixCategory::Components, component)
            .is_err()
    );
    assert_eq!(registry, before);
}

#[test]
fn case_only_replacements_keep_canonical_spelling_and_refresh_every_category() {
    let mut registry = catalog();
    let root = scratch("canonical-case");
    let mut folder = Folder::new(&root).unwrap();
    registry.write_into(&mut folder).unwrap();
    for (category, name) in [
        (FixCategory::Fields, "PartyID"),
        (FixCategory::Components, "Party"),
        (FixCategory::Groups, "Parties"),
        (FixCategory::Components, "NewOrderSingle"),
    ] {
        for (replace, description) in [(false, "Updated metadata"), (true, "Replaced metadata")] {
            let mut field = registry.definition(category, name).unwrap().clone();
            field.set_name(name.to_ascii_lowercase());
            field.as_fix_mut().set_description(description).unwrap();
            if replace {
                registry.insert_definition(category, field).unwrap();
            } else {
                registry.update_definition(category, field).unwrap();
            }
            let canonical = registry.definition(category, name).unwrap();
            assert_eq!(canonical.name(), name);
            assert_eq!(canonical.as_fix().description(), Some(description));
        }
    }
    let partyid = registry
        .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
        .unwrap();
    assert_eq!(partyid.name(), "PartyID");
    assert_eq!(partyid.as_fix().description(), Some("Replaced metadata"));
    let group = registry
        .field_by_path(&fpath("NewOrderSingle.Parties"))
        .unwrap();
    assert_eq!(group.name(), "Parties");
    assert_eq!(group.as_fix().description(), Some("Replaced metadata"));
    let DataType::List(item) = group.dtype() else {
        panic!("a group list")
    };
    assert_eq!(item.name(), "Party");
    assert_eq!(item.as_fix().description(), Some("Replaced metadata"));
    registry.write_into(&mut folder).unwrap();
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();

    let before = registry.clone();
    let mut component = registry
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    component.set_name("Pa_rty");
    assert!(
        registry
            .update_definition(FixCategory::Components, component)
            .is_err()
    );
    assert_eq!(registry, before);
}

#[test]
fn folded_field_updates_keep_canonical_names_and_refresh_references() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let mut registry = FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap();
    let mut incoming = tagged("Symbol", 55, DataType::utf8());
    incoming.as_fix_mut().set_tags(&[9001]).unwrap();
    incoming.as_fix_mut().set_aliases(["Sym"]).unwrap();
    registry.update(incoming.clone()).unwrap();
    let canonical = registry.field(55).unwrap();
    assert_eq!(canonical.name(), "symbol");
    assert_eq!(canonical.as_fix().tags().unwrap(), [9001]);
    assert!(std::ptr::eq(registry.field(9001).unwrap(), canonical));
    for path in ["Instrument.Symbol", "NewOrderSingle.Instrument.Symbol"] {
        let occurrence = registry.field_by_path(&fpath(path)).unwrap();
        assert_eq!(occurrence.name(), "symbol");
        assert_eq!(occurrence.as_fix().field_ref(), Some("symbol"));
        assert_eq!(occurrence.as_fix().tags().unwrap(), [9001]);
        let mut detached = occurrence.clone();
        detached.as_fix_mut().remove_field_ref();
        assert_eq!(detached.as_metadata(), canonical.as_metadata());
    }
    let before = registry.clone();
    registry.update(incoming.clone()).unwrap();
    assert_eq!(
        registry, before,
        "repeating the metadata merge is idempotent"
    );
    incoming.set_dtype(DataType::Int32).unwrap();
    assert!(registry.update(incoming).is_err());
    assert_eq!(registry, before, "datatype refusal leaves the graph intact");
}

#[test]
fn duplicate_persisted_field_declarations_are_refused() {
    let root = scratch("duplicates");
    let folder = Folder::new(&root).unwrap();
    let field = tagged("Symbol", 55, DataType::utf8());
    let bytes = yggdryl::text::json::into_bytes(&Scalar::from_sequence([
        field.clone().into_value(),
        field.into_value(),
    ]))
    .unwrap();
    folder
        .child_by_path("fields/0.json")
        .unwrap()
        .write_all_bytes(&bytes)
        .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("0.json"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mixed_inline_and_reference_depth_has_one_bound() {
    let root = scratch("mixed-depth");
    let folder = Folder::new(&root).unwrap();
    for index in (0..34).rev() {
        let mut child = DataType::Null.nullable_field("child");
        if index < 33 {
            child
                .as_fix_mut()
                .set_component(&format!("Chain{:02}", index + 1))
                .unwrap();
        }
        let inline = DataType::from_fields([child])
            .unwrap()
            .required_field("inline");
        let field = DataType::from_fields([inline])
            .unwrap()
            .required_field(format!("Chain{index:02}"));
        folder
            .child_by_path(&format!("components/Chain{index:02}.json"))
            .unwrap()
            .write_all_bytes(&field.into_json_bytes().unwrap())
            .unwrap();
    }
    let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(
        error.contains("64") && error.contains("Chain00.json"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn message_types_borrow_the_catalog_schema_and_keep_wire_codes_case_sensitive() {
    let registry = catalog();
    let by_code = registry.msgtype("D").unwrap();
    let by_name = registry.msgtype("new_order_single").unwrap();
    assert!(std::ptr::eq(by_code, by_name));
    assert!(std::ptr::eq(
        by_code.as_field(),
        registry
            .definition(FixCategory::Components, "NewOrderSingle")
            .unwrap()
    ));
    assert_eq!(by_code.name(), "NewOrderSingle");
    assert_eq!(by_code.as_str(), "D");
    assert!(registry.get_msgtype("d").is_none());
    assert_eq!(registry.msgtypes().count(), 1 + super::crated_messages());
    assert!(registry.msgtype("unknown").is_err());
    assert!(std::ptr::eq(by_code, registry.msgtype_at(0).unwrap()));
    assert!(registry.msgtype_at(1 + super::crated_messages()).is_none());
    assert!(registry.msgtype_at(usize::MAX).is_none());
}

#[test]
fn message_position_keeps_identity_when_a_name_is_another_messages_wire_code() {
    let mut registry = catalog();
    let mut field = DataType::from_fields([]).unwrap().required_field("D");
    field.as_fix_mut().set_msgtype("X").unwrap();
    registry
        .insert_definition(FixCategory::Components, field)
        .unwrap();
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(registry.msgtype_at(0).unwrap().name(), "D");
    assert_eq!(registry.msgtype_at(0).unwrap().as_str(), "X");
    assert_eq!(registry.msgtype_at(1).unwrap().name(), "NewOrderSingle");
    assert!(registry.msgtype_at(2 + super::crated_messages()).is_none());
}

#[test]
fn one_message_code_namespace_answers_the_bare_code_to_its_first_holder() {
    let mut registry = catalog();
    // A code re-declared under another name is a second message: the bare
    // code keeps answering the first holder, the newcomer is reached by name.
    let mut other = DataType::from_fields([])
        .unwrap()
        .required_field("OtherOrder");
    other.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .insert_definition(FixCategory::Components, other)
        .unwrap();
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(registry.msgtype("OtherOrder").unwrap().as_str(), "D");
    assert_eq!(registry.msgtype("other_order").unwrap().as_str(), "D");
    assert_eq!(registry.msgtypes().count(), 2 + super::crated_messages());
    // Once the first holder goes, the code answers the one left.
    registry
        .remove_definition(FixCategory::Components, "NewOrderSingle")
        .unwrap()
        .unwrap();
    assert_eq!(registry.msgtype("D").unwrap().name(), "OtherOrder");
    registry
        .remove_definition(FixCategory::Components, "OtherOrder")
        .unwrap()
        .unwrap();
    assert!(registry.get_msgtype("D").is_none());

    // A dictionary's message on a standard code is the same one namespace:
    // re-declared under the folded name it folds into the stored message,
    // under its own name it stands beside it, carrying its membership.
    let mut registry = catalog();
    let mut restated = DataType::from_fields([])
        .unwrap()
        .required_field("new_order_single");
    restated.as_fix_mut().set_msgtype("D").unwrap();
    restated.as_fix_mut().set_branches(["venue"]).unwrap();
    assert!(
        !registry
            .add_definition(FixCategory::Components, restated)
            .unwrap()
    );
    assert_eq!(registry.msgtypes().count(), 1 + super::crated_messages());
    let folded = registry.msgtype("D").unwrap();
    assert_eq!(folded.name(), "NewOrderSingle");
    assert!(folded.as_field().as_fix().has_branch("venue"));
    assert_eq!(folded.get_group_by_tag(453).unwrap().name(), "Parties");
    let mut message = DataType::from_fields([])
        .unwrap()
        .required_field("VenueOrder");
    message.as_fix_mut().set_msgtype("D").unwrap();
    message.as_fix_mut().set_branches(["venue"]).unwrap();
    registry
        .insert_definition(FixCategory::Components, message)
        .unwrap();
    assert_eq!(registry.msgtypes().count(), 2 + super::crated_messages());
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    let venue = registry.msgtype("VenueOrder").unwrap();
    assert_eq!(venue.as_str(), "D");
    assert!(venue.as_field().as_fix().has_branch("venue"));
    assert_eq!(registry.dialects(), ["venue"]);
    let loaded = FixRegistry::from_json(&registry.into_json().unwrap()).unwrap();
    assert_eq!(loaded, registry);
    assert_eq!(loaded.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(loaded.msgtype("VenueOrder").unwrap().as_str(), "D");
}

#[test]
fn message_code_aliases_reindex_after_field_enum_mutation() {
    let mut registry = catalog();
    let mut field = tagged("MsgType", 35, DataType::utf8());
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Order", "D").with_aliases(["NOS"])])
        .unwrap();
    registry.insert(field).unwrap();
    assert_eq!(registry.msgtype("nos").unwrap().as_str(), "D");
    let mut field = registry.field(35).unwrap().clone();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Order", "D").with_aliases(["NewOrder"])])
        .unwrap();
    registry.insert(field).unwrap();
    assert!(registry.get_msgtype("NOS").is_none());
    assert_eq!(registry.msgtype("new_order").unwrap().as_str(), "D");
}

#[test]
fn field_enum_updates_refresh_component_and_message_references_atomically() {
    let mut registry = catalog();
    registry
        .insert(tagged("MsgType", 35, DataType::utf8()))
        .unwrap();
    let mut member = registry.field(35).unwrap().clone();
    member.as_fix_mut().set_field_ref("MsgType").unwrap();
    let mut header = DataType::from_fields([member])
        .unwrap()
        .required_field("Header");
    registry
        .insert_definition(FixCategory::Components, header.clone())
        .unwrap();
    header.as_fix_mut().set_component("Header").unwrap();
    let mut message = DataType::from_fields([header])
        .unwrap()
        .required_field("EnumReport");
    message.as_fix_mut().set_msgtype("R").unwrap();
    registry
        .insert_definition(FixCategory::Components, message)
        .unwrap();

    let mut coded = registry.field(35).unwrap().clone();
    coded
        .as_fix_mut()
        .set_codes(&[FixCode::new("Order", "D").with_aliases(["NOS"])])
        .unwrap();
    registry.update(coded).unwrap();
    assert_eq!(registry.msgtype("NOS").unwrap().as_str(), "D");
    assert_eq!(
        registry
            .field_by_path(&fpath("EnumReport.Header.MsgType"))
            .unwrap()
            .as_fix()
            .code_value("NOS"),
        Some("D")
    );

    let before = registry.clone();
    let mut malformed = registry.field(35).unwrap().clone();
    malformed.update_metadata([("fix:codes", "[")]).unwrap();
    assert!(registry.insert(malformed).is_err());
    assert_eq!(registry, before);
    let mut changed = registry.field(35).unwrap().clone();
    changed.as_fix_mut().set_codes(&[]).unwrap();
    changed.as_fix_mut().set_description("Changed").unwrap();
    registry.insert(changed).unwrap();
    assert_eq!(
        registry
            .field_by_path(&fpath("EnumReport.Header.MsgType"))
            .unwrap()
            .as_fix()
            .description(),
        Some("Changed")
    );

    let mut uncoded = registry.field(35).unwrap().clone();
    uncoded.as_fix_mut().set_codes(&[]).unwrap();
    registry.insert(uncoded).unwrap();
    assert!(registry.get_msgtype("NOS").is_none());
    assert!(
        registry
            .field_by_path(&fpath("EnumReport.Header.MsgType"))
            .unwrap()
            .as_fix()
            .codes()
            .next()
            .is_none()
    );
}

#[test]
fn message_context_resolves_a_group_whose_global_counter_is_ambiguous() {
    let mut registry = catalog();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    group.set_name("TradeParties");
    registry
        .insert_definition(FixCategory::Groups, group.clone())
        .unwrap();
    assert!(registry.get_group_by_tag(453).is_none());
    assert_eq!(
        registry
            .msgtype("D")
            .unwrap()
            .get_group_by_tag(453)
            .unwrap()
            .name(),
        "Parties"
    );
    group.as_fix_mut().set_group("TradeParties").unwrap();
    let outer = DataType::from_fields([group])
        .unwrap()
        .required_field("Outer");
    let mut message = DataType::from_fields([outer])
        .unwrap()
        .required_field("Trade");
    message.as_fix_mut().set_msgtype("T").unwrap();
    registry
        .insert_definition(FixCategory::Components, message)
        .unwrap();
    assert_eq!(
        registry
            .msgtype("T")
            .unwrap()
            .get_group_by_tag(453)
            .unwrap()
            .name(),
        "TradeParties"
    );
}

#[test]
fn message_group_paths_cross_list_items_and_refuse_repeated_contexts() {
    let mut registry = catalog();
    registry
        .insert(tagged("NoHops", 627, DataType::Int32))
        .unwrap();
    let mut parties = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    parties.as_fix_mut().set_group("Parties").unwrap();
    let hop = DataType::from_fields([parties.clone()])
        .unwrap()
        .required_field("Hop");
    registry
        .insert_definition(FixCategory::Components, hop.clone())
        .unwrap();
    let mut hops = DataType::large_list(hop).nullable_field("Hops");
    hops.as_fix_mut().set_counter(627).unwrap();
    hops.as_fix_mut().set_component("Hop").unwrap();
    registry
        .insert_definition(FixCategory::Groups, hops)
        .unwrap();
    let mut hops = registry
        .definition(FixCategory::Groups, "Hops")
        .unwrap()
        .clone();
    hops.as_fix_mut().set_group("Hops").unwrap();
    let mut message = DataType::from_fields([hops])
        .unwrap()
        .required_field("HopReport");
    message.as_fix_mut().set_msgtype("H").unwrap();
    registry
        .insert_definition(FixCategory::Components, message)
        .unwrap();
    let message = registry.msgtype("H").unwrap();
    assert_eq!(message.get_group_by_tag(627).unwrap().name(), "Hops");
    assert_eq!(message.get_group_by_tag(453).unwrap().name(), "Parties");
    let mut duplicate = DataType::from_fields([
        DataType::from_fields([parties.clone()])
            .unwrap()
            .required_field("Left"),
        DataType::from_fields([parties])
            .unwrap()
            .required_field("Right"),
    ])
    .unwrap()
    .required_field("DuplicateContexts");
    duplicate.as_fix_mut().set_msgtype("R").unwrap();
    registry
        .insert_definition(FixCategory::Components, duplicate)
        .unwrap();
    assert!(
        registry
            .msgtype("R")
            .unwrap()
            .get_group_by_tag(453)
            .is_none()
    );
}

#[test]
fn message_types_require_non_null_structs_and_complete_non_control_codes() {
    let mut registry = FixRegistry::new();
    // A Struct stating no message type is a plain component (decision 13):
    // it is accepted, and no code reaches it.
    let missing = DataType::from_fields([]).unwrap().required_field("Missing");
    registry
        .insert_definition(FixCategory::Components, missing)
        .unwrap();
    assert!(registry.get_msgtype("Missing").is_none());
    assert_eq!(registry.msgtypes().count(), super::crated_messages());
    let mut nullable = DataType::from_fields([])
        .unwrap()
        .nullable_field("Nullable");
    nullable.as_fix_mut().set_msgtype("X").unwrap();
    assert!(
        registry
            .insert_definition(FixCategory::Components, nullable)
            .is_err()
    );
    let mut composite = DataType::from_fields([]).unwrap().required_field("Report");
    composite
        .as_fix_mut()
        .set_msgtype("P Report Acknowledgement")
        .unwrap();
    registry
        .insert_definition(FixCategory::Components, composite)
        .unwrap();
    assert_eq!(
        registry
            .msgtype("P Report Acknowledgement")
            .unwrap()
            .as_str(),
        "P Report Acknowledgement"
    );
    let mut refused = DataType::from_fields([]).unwrap().required_field("Refused");
    assert!(refused.as_fix_mut().set_msgtype("A\nB").is_err());
}

/// A message is a component carrying `fix:msgtype` (decision 13): the marker
/// is a property the component gains or loses through an ordinary update,
/// and the message index follows it rather than a category move.
#[test]
fn the_msgtype_marker_makes_a_component_a_message_and_its_removal_unmakes_it() {
    let mut registry = catalog();
    assert_eq!(registry.msgtypes().count(), 1 + super::crated_messages());
    // A plain component given the marker is a message afterwards.
    let mut party = registry
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    assert!(registry.get_msgtype("Party").is_none());
    party.set_nullable(false);
    party.as_fix_mut().set_msgtype("UPTY").unwrap();
    registry
        .update_definition(FixCategory::Components, party)
        .unwrap();
    assert_eq!(registry.msgtype("UPTY").unwrap().name(), "Party");
    assert_eq!(registry.msgtype("party").unwrap().as_str(), "UPTY");
    assert_eq!(registry.msgtypes().count(), 2 + super::crated_messages());
    assert_eq!(
        registry
            .msgtypes()
            .map(|held| held.name())
            // The two this test named, without the one every registry has.
            .filter(|name| *name != yggdryl::PLUGINCONFIG_CODE_NAME.1)
            .collect::<Vec<_>>(),
        ["NewOrderSingle", "Party"]
    );
    assert_eq!(registry.msgtype_at(1).unwrap().name(), "Party");
    assert!(registry.msgtype_at(2 + super::crated_messages()).is_none());
    // A message whose marker is removed is a component and answers no code.
    let mut order = registry
        .definition(FixCategory::Components, "NewOrderSingle")
        .unwrap()
        .clone();
    order.as_fix_mut().remove_msgtype();
    registry
        .update_definition(FixCategory::Components, order)
        .unwrap();
    assert!(registry.get_msgtype("D").is_none());
    assert!(registry.get_msgtype("NewOrderSingle").is_none());
    assert!(
        registry
            .get_definition(FixCategory::Components, "NewOrderSingle")
            .is_some()
    );
    assert_eq!(registry.msgtypes().count(), 1 + super::crated_messages());
    assert_eq!(registry.msgtype_at(0).unwrap().name(), "Party");
    // The category refuses the fourth name everywhere it is spelled.
    assert!(FixCategory::from_str("messages").is_err());
    assert_eq!(FixCategory::ALL.len(), 3);
}

/// `msgtype_at` indexes exactly what `msgtypes` iterates, over a catalog
/// where messages and components interleave by name.
#[test]
fn msgtype_at_indexes_the_marked_components_of_the_committed_dictionary() {
    let registry = super::committed_registry();
    let iterated: Vec<&str> = registry.msgtypes().map(|held| held.name()).collect();
    assert_eq!(iterated.len(), 181 + super::crated_messages());
    let indexed: Vec<&str> = (0..)
        .map_while(|index| registry.msgtype_at(index))
        .map(|held| held.name())
        .collect();
    assert_eq!(indexed, iterated);
    // Name order, among the components rather than in a folder of their
    // own: the first message sorts after the first component.
    let mut sorted = iterated.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, iterated);
    assert_eq!(iterated[0], "accountsummaryreport");
    assert_eq!(
        registry
            .definitions(FixCategory::Components)
            .next()
            .unwrap()
            .name(),
        "accountsummaryreport"
    );
    assert_eq!(registry.msgtype("D").unwrap().name(), "newordersingle");
    assert_eq!(
        registry
            .definition(FixCategory::Components, "NewOrderSingle")
            .unwrap()
            .as_fix()
            .msgtype(),
        Some("D")
    );
}

#[test]
fn a_document_property_is_stored_as_the_json_it_is_and_read_back_as_its_text() {
    let mut side = tagged("Side", 54, DataType::utf8());
    side.as_fix_mut()
        .set_codes(&[
            FixCode::new("Buy", "1").with_description("Buy side"),
            FixCode::new("Sell", "2"),
        ])
        .unwrap();
    let canonical = side.get_metadata("fix:codes").unwrap().to_owned();
    assert!(canonical.starts_with("[{"), "{canonical}");

    // The store writes the document rather than one escaped line, so the
    // file an operator opens renders a code set as a code set.
    let document = yggdryl::into_fix_document(side.clone()).unwrap();
    let codes = document
        .get_key_str("metadata")
        .and_then(|metadata| metadata.get_key_str("fix:codes"))
        .expect("the code set");
    assert_eq!(codes.len(), 2);
    assert_eq!(
        codes
            .get(0)
            .and_then(|code| code.get_key_str("value"))
            .and_then(Scalar::as_str),
        Some("1"),
    );
    // And the keys stay in the order the reader walks them, so the file
    // reads the way the document is written.
    assert_eq!(
        codes.get(0).map(Scalar::keys),
        Some(vec!["value", "name", "doc"]),
    );

    // Reading one back restates the canonical text, whatever order the file
    // spelled an entry's keys in.
    assert_eq!(yggdryl::from_fix_document(document).unwrap(), side);
    let reordered = yggdryl::from_json_scalar(
        r#"{"name":"Side","dtype":{"type":"string"},"nullable":true,"metadata":{
            "fix:tag":"54",
            "fix:codes":[{"doc":"Buy side","name":"Buy","value":"1"},{"name":"Sell","value":"2"}]
        }}"#,
    )
    .unwrap();
    assert_eq!(
        yggdryl::from_fix_document(reordered)
            .unwrap()
            .get_metadata("fix:codes"),
        Some(canonical.as_str()),
    );
}

#[test]
fn a_document_property_the_store_cannot_read_is_refused_by_name() {
    // One shape: a property the file spells as text is not read as canonical
    // text, because the store writes the document itself.
    let text = yggdryl::from_json_scalar(
        r#"{"name":"Side","dtype":{"type":"string"},"nullable":true,
            "metadata":{"fix:tag":"54","fix:codes":"[{\"value\":\"1\",\"name\":\"Buy\"}]"}}"#,
    )
    .unwrap();
    let error = yggdryl::from_fix_document(text).expect_err("the escaped shape is not the shape");
    assert!(error.to_string().contains("fix:codes"), "{error}");

    // An entry stating a key the document does not declare is refused the
    // same way rather than dropped.
    let unknown = yggdryl::from_json_scalar(
        r#"{"name":"Side","dtype":{"type":"string"},"nullable":true,
            "metadata":{"fix:tag":"54","fix:codes":[{"value":"1","name":"Buy","note":"x"}]}}"#,
    )
    .unwrap();
    let error = yggdryl::from_fix_document(unknown).expect_err("an undeclared key");
    assert!(error.to_string().contains("note"), "{error}");

    // And a field holding text no reader can parse is named where it is
    // written rather than copied out for a reader to refuse later.
    let mut broken = tagged("Side", 54, DataType::utf8());
    broken
        .set_metadata([("fix:codes", "not a document")])
        .unwrap();
    let error = yggdryl::into_fix_document(broken).expect_err("a malformed document");
    assert!(error.to_string().contains("fix:codes"), "{error}");
}

#[test]
fn every_committed_field_document_round_trips_through_the_store_shape() {
    // The whole shipped dictionary, both directions: what the store writes
    // reads back as the same field, byte for byte in its metadata.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap();
    let mut carried = 0_usize;
    for field in &registry {
        let document = yggdryl::into_fix_document(field.clone()).unwrap();
        if document
            .get_key_str("metadata")
            .is_some_and(|metadata| metadata.get_key_str("fix:codes").is_some())
        {
            carried += 1;
        }
        assert_eq!(&yggdryl::from_fix_document(document).unwrap(), field);
    }
    assert!(carried > 400, "only {carried} fields carry a code set");
    for category in [FixCategory::Components, FixCategory::Groups] {
        for field in registry.definitions(category) {
            let document = yggdryl::into_fix_document(field.clone()).unwrap();
            assert_eq!(&yggdryl::from_fix_document(document).unwrap(), field);
        }
    }
}

#[test]
fn a_json_snapshot_file_folds_in_the_way_a_cblock_does() {
    let root = scratch("add-json");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("venue.json");

    // What one dictionary wrote is what another reads: a snapshot states its
    // own memberships, so no dialect is named at this door.
    let mut venue = tagged("VenueRef", 9001, DataType::utf8());
    venue.as_fix_mut().set_branches(["venue"]).unwrap();
    let source = FixRegistry::from_fields([venue]).unwrap();
    std::fs::write(&path, source.into_json().unwrap()).unwrap();

    let mut registry = FixRegistry::new();
    let file = yggdryl::holder::local::File::new(&path).unwrap();
    let (added, merged) = registry.add_json_file(&file).unwrap();
    assert_eq!((added, merged), (1, 2), "one field, the two clock seeds");
    assert_eq!(registry.field_by_tag(9001).unwrap().name(), "VenueRef");
    assert!(
        registry
            .field_by_tag(9001)
            .unwrap()
            .as_fix()
            .has_branch("venue")
    );

    // One mutation: a document that does not parse leaves it as it was.
    let before = registry.stable_hash();
    std::fs::write(&path, br#"{"fields":[],"components":[],"groups":"no"}"#).unwrap();
    let error = registry
        .add_json_file(&yggdryl::holder::local::File::new(&path).unwrap())
        .expect_err("a category that is not an array");
    assert!(error.to_string().contains("venue.json"), "{error}");
    assert_eq!(registry.stable_hash(), before);

    std::fs::remove_dir_all(&root).unwrap();
}
