//! FIX category storage and atomic catalog mutations.

use std::path::PathBuf;
use yggdryl::holder::local::Folder;
use yggdryl::{
    DataType, Field, FixBranch, FixCategory, FixCode, FixId, FixRegistry, IOBase, Scalar,
};

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
    let mut partyid = tagged("PartyID", 448, DataType::Utf8);
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
        .insert_definition(FixCategory::Components, component.clone())
        .unwrap();
    let mut group = DataType::list(component).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component("Party").unwrap();
    registry
        .insert_definition(FixCategory::Groups, group)
        .unwrap();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties", None)
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
        .insert_definition(FixCategory::Messages, message)
        .unwrap();
    registry
}

#[test]
fn registry_json_snapshots_preserve_the_graph_and_all_branch_declarations() {
    let mut registry = catalog();
    let branch = FixBranch::from_parts("cme", "3.4.5".parse().unwrap())
        .unwrap()
        .with_aliases(["merc"])
        .unwrap();
    let mut venue = tagged("VenueTrade", 5001, DataType::Utf8);
    venue.as_fix_mut().set_branch(&branch).unwrap();
    registry
        .create_definition(FixCategory::Fields, venue)
        .unwrap();
    registry.set_branch(branch.clone()).unwrap();
    let empty = FixBranch::from_parts("pending", "7.1".parse().unwrap()).unwrap();
    registry.set_branch(empty.clone()).unwrap();

    let json = registry.into_json().unwrap();
    let document = yggdryl::from_json_scalar(&json).unwrap();
    let record = document.as_record().unwrap();
    assert_eq!(record.len(), 5);
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
    assert_eq!(loaded.branch_named("merc"), Some(&branch));
    assert_eq!(loaded.branch_named("pending"), Some(&empty));
    assert_eq!(loaded.field(453).unwrap().dtype(), &DataType::Int32);
    assert_eq!(
        loaded.field(448).unwrap().as_fix().code_name("B"),
        Some("Broker")
    );
    let message = loaded.msgtype("D", None).unwrap();
    assert_eq!(message.name(), "NewOrderSingle");
    assert_eq!(
        message
            .get_group_by_counter(FixId::standard(453))
            .unwrap()
            .name(),
        "Parties"
    );

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
        assert_eq!(registry.len(), 2);
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
            tagged("other", 1865, DataType::Int32),
            tagged("quoteackstatus", 1866, DataType::Int32),
        ] {
            assert!(
                registry
                    .create_definition(FixCategory::Fields, duplicate)
                    .is_err()
            );
            assert_eq!(registry, before);
        }
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
    assert_eq!(loaded.len(), 6203);
    assert_eq!(loaded.definitions(FixCategory::Messages).count(), 181);
    assert_eq!(loaded.definitions(FixCategory::Components).count(), 747);
    assert_eq!(loaded.definitions(FixCategory::Groups).count(), 580);
}

#[test]
fn registry_hashes_include_named_definitions_and_branch_metadata() {
    let original = catalog();
    let mut changed = original.clone();
    let old_message = original.msgtype("D", None).unwrap();
    let mut field = old_message.as_field().clone();
    field.as_fix_mut().set_msgtype("D2").unwrap();
    changed
        .update_definition(FixCategory::Messages, field)
        .unwrap();
    assert_ne!(changed, original);
    assert_ne!(changed.stable_hash(), original.stable_hash());
    assert_ne!(
        changed.msgtype("D2", None).unwrap().stable_hash(),
        old_message.stable_hash()
    );
    assert_eq!(old_message.stable_hash(), old_message.clone().stable_hash());

    let mut declared = original.clone();
    declared
        .set_branch(FixBranch::from_parts("pending", "1.0".parse().unwrap()).unwrap())
        .unwrap();
    let before = declared.stable_hash();
    declared
        .set_branch(FixBranch::from_parts("pending", "1.1".parse().unwrap()).unwrap())
        .unwrap();
    assert_ne!(declared.stable_hash(), before);
    assert_ne!(declared.stable_hash(), original.stable_hash());
}

#[test]
fn registry_snapshots_reject_missing_categories_and_unresolved_references() {
    for json in [
        "[]",
        r#"{"fields":[],"messages":[],"components":[],"groups":[]}"#,
        r#"{"fields":[],"messages":[],"components":[],"groups":[],"branches":[],"codesets":[]}"#,
    ] {
        assert!(FixRegistry::from_json(json).is_err(), "{json}");
    }
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
        "messages/NewOrderSingle.json",
    ] {
        assert!(root.join(file).is_file(), "{file}");
    }
    let document =
        Field::from_json_bytes(&std::fs::read(root.join("groups/Parties.json")).unwrap()).unwrap();
    let DataType::List(item) = document.dtype() else {
        panic!("a group list")
    };
    assert_eq!(item.dtype(), &DataType::Null);
    assert_eq!(item.as_fix().component(), Some("party"));
    assert_eq!(document.as_fix().tag().unwrap(), None);
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded, registry);
    assert_eq!(loaded.field(453).unwrap().dtype(), &DataType::Int32);
    assert_eq!(
        loaded
            .group_by_counter(FixId::standard(453))
            .unwrap()
            .name(),
        "Parties"
    );
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
            registry.remove_definition(category, name, None).is_err(),
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
    assert!(
        registry
            .create_definition(
                FixCategory::Fields,
                tagged("OtherName", 448, DataType::Utf8)
            )
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
        .remove_definition(FixCategory::Messages, "NewOrderSingle", None)
        .unwrap()
        .unwrap();
    registry
        .remove_definition(FixCategory::Groups, "Parties", None)
        .unwrap()
        .unwrap();
    registry
        .remove_definition(FixCategory::Components, "Party", None)
        .unwrap()
        .unwrap();
    registry
        .remove_definition(FixCategory::Fields, "PartyID", None)
        .unwrap()
        .unwrap();
}

#[test]
fn enum_codes_belong_to_each_field_and_branch() {
    let mut registry = catalog();
    let venue = FixBranch::from_str("venue").unwrap();
    let mut field = DataType::Utf8.nullable_field("PartyID");
    field.as_fix_mut().set_id(&venue, 5001).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("VenueBroker", "V")])
        .unwrap();
    registry.insert(field).unwrap();
    assert_eq!(
        registry
            .field(FixId::from_parts(&venue, 5001).unwrap())
            .unwrap()
            .as_fix()
            .code_value("VenueBroker"),
        Some("V")
    );
    assert_eq!(
        registry.field(448).unwrap().as_fix().code_value("Broker"),
        Some("B")
    );
    assert_eq!(FixCategory::ALL.len(), 4);
    assert!(FixCategory::from_str("codesets").is_err());
}

#[test]
fn contexts_sharing_a_counter_are_explicitly_ambiguous() {
    let mut registry = catalog();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties", None)
        .unwrap()
        .clone();
    group.set_name("TradeParties");
    registry
        .insert_definition(FixCategory::Groups, group)
        .unwrap();
    assert!(
        registry
            .get_group_by_counter(FixId::standard(453))
            .is_none()
    );
    assert!(registry.group_by_counter(FixId::standard(453)).is_err());
    assert_eq!(registry.definitions(FixCategory::Groups).count(), 2);
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
}

#[test]
fn store_removes_empty_shards_and_named_documents() {
    let root = scratch("cleanup");
    let mut folder = Folder::new(&root).unwrap();
    let mut registry = catalog();
    registry
        .insert(tagged("Distant", 10000, DataType::Utf8))
        .unwrap();
    registry.write_into(&mut folder).unwrap();
    registry.remove(10000).unwrap();
    registry
        .remove_definition(FixCategory::Messages, "NewOrderSingle", None)
        .unwrap();
    registry.write_into(&mut folder).unwrap();
    assert!(!root.join("fields/100.json").exists());
    assert!(!root.join("messages").exists());
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
fn malformed_shards_and_folder_disagreements_are_located() {
    let root = scratch("malformed");
    let folder = Folder::new(&root).unwrap();
    for bytes in [
        b"not json".to_vec(),
        b"{}".to_vec(),
        yggdryl::text::json::into_bytes(&Scalar::from_sequence([tagged(
            "Misplaced",
            150,
            DataType::Utf8,
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
    let field = tagged("WrongBranch", 5001, DataType::Utf8);
    folder
        .child_by_path("fields/venue/50.json")
        .unwrap()
        .write_all_bytes(
            &yggdryl::text::json::into_bytes(&Scalar::from_sequence([field.into_value()])).unwrap(),
        )
        .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(
        error.contains("venue") && error.contains("50.json"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tracked_seed_resolves_every_category_and_native_reference_graph() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&Folder::new(root).unwrap()).unwrap();
    assert_eq!(registry.len(), 6203);
    for (category, count) in [
        (FixCategory::Components, 747),
        (FixCategory::Groups, 580),
        (FixCategory::Messages, 181),
    ] {
        assert_eq!(registry.definitions(category).count(), count, "{category}");
    }
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
    let group = registry
        .definition(FixCategory::Groups, "Parties", None)
        .unwrap();
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
    assert!(left.definition_at(FixCategory::Components, 2).is_none());
    assert!(
        left.definition_at(FixCategory::Components, usize::MAX)
            .is_none()
    );
}

#[test]
fn merging_a_complete_catalog_commits_references_together() {
    let source = catalog();
    let mut target = FixRegistry::new();
    assert_eq!(target.merge_with(&source).unwrap(), (2, 0));
    assert_eq!(target, source);
    assert_eq!(target.merge_with(&source).unwrap(), (0, 2));
    assert_eq!(target, source);
}

#[test]
fn merging_case_only_named_definitions_preserves_canonical_names_and_references() {
    let original = catalog();
    let categories = [
        FixCategory::Components,
        FixCategory::Groups,
        FixCategory::Messages,
    ];
    let source = |collision: Option<FixCategory>| {
        let mut source = FixRegistry::from_fields(original.iter().cloned()).unwrap();
        for category in categories {
            for field in original.definitions(category) {
                let mut field = field.clone();
                let name = if collision == Some(category) {
                    format!("_{}", field.name())
                } else {
                    field.name().to_ascii_uppercase()
                };
                field.set_name(name);
                source.create_definition(category, field).unwrap();
            }
        }
        source
    };
    let mut target = original.clone();
    let incoming = source(None);
    assert_eq!(target.merge_with(&incoming).unwrap(), (0, 2));
    assert_eq!(target, original);
    for (category, name) in [
        (FixCategory::Components, "Party"),
        (FixCategory::Groups, "Parties"),
        (FixCategory::Messages, "NewOrderSingle"),
    ] {
        assert_eq!(
            target.definition(category, name, None).unwrap().name(),
            name
        );
    }
    let group = target
        .msgtype("D", None)
        .unwrap()
        .get_group_by_counter(FixId::standard(453))
        .unwrap();
    assert_eq!(group.name(), "Parties");
    assert_eq!(
        target
            .field_by_path("NewOrderSingle.Parties.PartyID", None)
            .unwrap()
            .as_fix()
            .code_name("B"),
        Some("Broker")
    );
    assert_eq!(
        FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
        target
    );
    for category in categories {
        let incoming = source(Some(category));
        let error = target.merge_with(&incoming).unwrap_err().to_string();
        assert!(error.contains("canonical"), "{error}");
        assert_eq!(target, original, "{category}");
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
        .definition(FixCategory::Messages, "NewOrderSingle", None)
        .unwrap()
        .clone();
    message.set_name("IncomingOrder");
    message.as_fix_mut().set_msgtype("I").unwrap();
    source
        .create_definition(FixCategory::Messages, message)
        .unwrap();
    let before_source = source.clone();

    assert_eq!(target.merge_with(&source).unwrap(), (0, 2));
    for path in [
        "PartyID",
        "Party.PartyID",
        "Parties.PartyID",
        "NewOrderSingle.Parties.PartyID",
        "IncomingOrder.Parties.PartyID",
    ] {
        let field = target.field_by_path(path, None).unwrap();
        assert_eq!(field.as_fix().code_name("B"), Some("Broker"), "{path}");
        assert_eq!(field.as_fix().code_name("C"), Some("Client"), "{path}");
    }
    let group = target
        .msgtype("I", None)
        .unwrap()
        .get_group_by_counter(FixId::standard(453))
        .unwrap();
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
fn merging_catalogs_still_refuses_referenced_structural_changes_atomically() {
    let mut target = catalog();
    let before = target.clone();
    let mut coded = target.field(448).unwrap().clone();
    coded
        .as_fix_mut()
        .set_codes(&[FixCode::new("Client", "C")])
        .unwrap();
    let mut source = FixRegistry::from_fields([coded]).unwrap();
    source
        .set_branch(FixBranch::from_str("incoming").unwrap())
        .unwrap();
    let mut member = source.field(448).unwrap().clone();
    member.as_fix_mut().set_field_ref("PartyID").unwrap();
    let changed = DataType::from_fields([member, DataType::Int32.nullable_field("Extra")])
        .unwrap()
        .required_field("Party");
    source
        .create_definition(FixCategory::Components, changed)
        .unwrap();
    let before_source = source.clone();

    let error = target.merge_with(&source).unwrap_err().to_string();
    assert!(error.contains("datatype"), "{error}");
    assert_eq!(target, before);
    assert_eq!(target.stable_hash(), before.stable_hash());
    assert_eq!(source, before_source);
}

#[test]
fn referenced_metadata_updates_cascade_and_occurrence_overrides_fail_without_loss() {
    let mut registry = catalog();
    let mut component = registry
        .definition(FixCategory::Components, "Party", None)
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
        .field_by_path("NewOrderSingle.Parties", None)
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
            .field_by_path("NewOrderSingle.Parties.PartyID", None)
            .unwrap()
            .as_fix()
            .description(),
        Some("A party identifier")
    );
    let mut group = registry
        .definition(FixCategory::Groups, "Parties", None)
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
            .field_by_path("NewOrderSingle.Parties", None)
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
        (FixCategory::Messages, "NewOrderSingle"),
    ] {
        for (replace, description) in [(false, "Updated metadata"), (true, "Replaced metadata")] {
            let mut field = registry.definition(category, name, None).unwrap().clone();
            field.set_name(name.to_ascii_lowercase());
            field.as_fix_mut().set_description(description).unwrap();
            if replace {
                registry.insert_definition(category, field).unwrap();
            } else {
                registry.update_definition(category, field).unwrap();
            }
            let canonical = registry.definition(category, name, None).unwrap();
            assert_eq!(canonical.name(), name);
            assert_eq!(canonical.as_fix().description(), Some(description));
        }
    }
    let partyid = registry
        .field_by_path("NewOrderSingle.Parties.PartyID", None)
        .unwrap();
    assert_eq!(partyid.name(), "PartyID");
    assert_eq!(partyid.as_fix().description(), Some("Replaced metadata"));
    let group = registry
        .field_by_path("NewOrderSingle.Parties", None)
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
        .definition(FixCategory::Components, "Party", None)
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
    let mut incoming = tagged("Symbol", 55, DataType::Utf8);
    incoming.as_fix_mut().set_tags(&[9001]).unwrap();
    incoming.as_fix_mut().set_aliases(["Sym"]).unwrap();
    registry.update(incoming.clone()).unwrap();
    let canonical = registry.field(55).unwrap();
    assert_eq!(canonical.name(), "symbol");
    assert_eq!(canonical.as_fix().tags().unwrap(), [9001]);
    assert!(std::ptr::eq(registry.field(9001).unwrap(), canonical));
    for path in ["Instrument.Symbol", "NewOrderSingle.Instrument.Symbol"] {
        let occurrence = registry.field_by_path(path, None).unwrap();
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
    let field = tagged("Symbol", 55, DataType::Utf8);
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
    let by_code = registry.msgtype("D", None).unwrap();
    let by_name = registry.msgtype("new_order_single", None).unwrap();
    assert!(std::ptr::eq(by_code, by_name));
    assert!(std::ptr::eq(
        by_code.as_field(),
        registry
            .definition(FixCategory::Messages, "NewOrderSingle", None)
            .unwrap()
    ));
    assert_eq!(by_code.name(), "NewOrderSingle");
    assert_eq!(by_code.as_str(), "D");
    assert!(registry.get_msgtype("d", None).is_none());
    assert_eq!(registry.msgtypes().count(), 1);
    assert!(registry.msgtype("unknown", None).is_err());
    assert!(std::ptr::eq(by_code, registry.msgtype_at(0).unwrap()));
    assert!(registry.msgtype_at(1).is_none());
    assert!(registry.msgtype_at(usize::MAX).is_none());
}

#[test]
fn message_position_keeps_identity_when_a_name_is_another_messages_wire_code() {
    let mut registry = catalog();
    let mut field = DataType::from_fields([]).unwrap().required_field("D");
    field.as_fix_mut().set_msgtype("X").unwrap();
    registry
        .insert_definition(FixCategory::Messages, field)
        .unwrap();
    assert_eq!(
        registry.msgtype("D", None).unwrap().name(),
        "NewOrderSingle"
    );
    assert_eq!(registry.msgtype_at(0).unwrap().name(), "D");
    assert_eq!(registry.msgtype_at(0).unwrap().as_str(), "X");
    assert_eq!(registry.msgtype_at(1).unwrap().name(), "NewOrderSingle");
    assert!(registry.msgtype_at(2).is_none());
}

#[test]
fn message_code_ambiguity_and_branch_namespaces_are_explicit() {
    let mut registry = catalog();
    let mut other = DataType::from_fields([])
        .unwrap()
        .required_field("OtherOrder");
    other.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .insert_definition(FixCategory::Messages, other)
        .unwrap();
    assert!(registry.get_msgtype("D", None).is_none());
    assert_eq!(registry.msgtype("OtherOrder", None).unwrap().as_str(), "D");
    registry
        .remove_definition(FixCategory::Messages, "OtherOrder", None)
        .unwrap();
    assert_eq!(
        registry.msgtype("D", None).unwrap().name(),
        "NewOrderSingle"
    );

    let venue = FixBranch::from_str("venue").unwrap();
    let mut message = DataType::from_fields([])
        .unwrap()
        .required_field("VenueOrder");
    message.as_fix_mut().set_msgtype("D").unwrap();
    message.as_fix_mut().set_branch(&venue).unwrap();
    registry
        .insert_definition(FixCategory::Messages, message)
        .unwrap();
    assert_eq!(
        registry.msgtype("D", Some(&venue)).unwrap().name(),
        "VenueOrder"
    );
    assert_eq!(
        registry.msgtype("D", None).unwrap().name(),
        "NewOrderSingle"
    );
}

#[test]
fn message_code_aliases_reindex_after_field_enum_mutation() {
    let mut registry = catalog();
    let mut field = tagged("MsgType", 35, DataType::Utf8);
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Order", "D").with_aliases(["NOS"])])
        .unwrap();
    registry.insert(field).unwrap();
    assert_eq!(registry.msgtype("nos", None).unwrap().as_str(), "D");
    let mut field = registry.field(35).unwrap().clone();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Order", "D").with_aliases(["NewOrder"])])
        .unwrap();
    registry.insert(field).unwrap();
    assert!(registry.get_msgtype("NOS", None).is_none());
    assert_eq!(registry.msgtype("new_order", None).unwrap().as_str(), "D");
}

#[test]
fn field_enum_updates_refresh_component_and_message_references_atomically() {
    let mut registry = catalog();
    registry
        .insert(tagged("MsgType", 35, DataType::Utf8))
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
        .insert_definition(FixCategory::Messages, message)
        .unwrap();

    let mut coded = registry.field(35).unwrap().clone();
    coded
        .as_fix_mut()
        .set_codes(&[FixCode::new("Order", "D").with_aliases(["NOS"])])
        .unwrap();
    registry.update(coded).unwrap();
    assert_eq!(registry.msgtype("NOS", None).unwrap().as_str(), "D");
    assert_eq!(
        registry
            .field_by_path("EnumReport.Header.MsgType", None)
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
            .field_by_path("EnumReport.Header.MsgType", None)
            .unwrap()
            .as_fix()
            .description(),
        Some("Changed")
    );

    let mut uncoded = registry.field(35).unwrap().clone();
    uncoded.as_fix_mut().set_codes(&[]).unwrap();
    registry.insert(uncoded).unwrap();
    assert!(registry.get_msgtype("NOS", None).is_none());
    assert!(
        registry
            .field_by_path("EnumReport.Header.MsgType", None)
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
        .definition(FixCategory::Groups, "Parties", None)
        .unwrap()
        .clone();
    group.set_name("TradeParties");
    registry
        .insert_definition(FixCategory::Groups, group.clone())
        .unwrap();
    assert!(
        registry
            .get_group_by_counter(FixId::standard(453))
            .is_none()
    );
    assert_eq!(
        registry
            .msgtype("D", None)
            .unwrap()
            .get_group_by_counter(FixId::standard(453))
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
        .insert_definition(FixCategory::Messages, message)
        .unwrap();
    assert_eq!(
        registry
            .msgtype("T", None)
            .unwrap()
            .get_group_by_counter(FixId::standard(453))
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
        .definition(FixCategory::Groups, "Parties", None)
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
        .definition(FixCategory::Groups, "Hops", None)
        .unwrap()
        .clone();
    hops.as_fix_mut().set_group("Hops").unwrap();
    let mut message = DataType::from_fields([hops])
        .unwrap()
        .required_field("HopReport");
    message.as_fix_mut().set_msgtype("H").unwrap();
    registry
        .insert_definition(FixCategory::Messages, message)
        .unwrap();
    let message = registry.msgtype("H", None).unwrap();
    assert_eq!(
        message
            .get_group_by_counter(FixId::standard(627))
            .unwrap()
            .name(),
        "Hops"
    );
    assert_eq!(
        message
            .get_group_by_counter(FixId::standard(453))
            .unwrap()
            .name(),
        "Parties"
    );
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
        .insert_definition(FixCategory::Messages, duplicate)
        .unwrap();
    assert!(
        registry
            .msgtype("R", None)
            .unwrap()
            .get_group_by_counter(FixId::standard(453))
            .is_none()
    );
}

#[test]
fn message_types_require_non_null_structs_and_complete_non_control_codes() {
    let mut registry = FixRegistry::new();
    let missing = DataType::from_fields([]).unwrap().required_field("Missing");
    assert!(
        registry
            .insert_definition(FixCategory::Messages, missing)
            .is_err()
    );
    let mut nullable = DataType::from_fields([])
        .unwrap()
        .nullable_field("Nullable");
    nullable.as_fix_mut().set_msgtype("X").unwrap();
    assert!(
        registry
            .insert_definition(FixCategory::Messages, nullable)
            .is_err()
    );
    let mut composite = DataType::from_fields([]).unwrap().required_field("Report");
    composite
        .as_fix_mut()
        .set_msgtype("P Report Acknowledgement")
        .unwrap();
    registry
        .insert_definition(FixCategory::Messages, composite)
        .unwrap();
    assert_eq!(
        registry
            .msgtype("P Report Acknowledgement", None)
            .unwrap()
            .as_str(),
        "P Report Acknowledgement"
    );
    let mut refused = DataType::from_fields([]).unwrap().required_field("Refused");
    assert!(refused.as_fix_mut().set_msgtype("A\nB").is_err());
}
