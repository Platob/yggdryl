//! `rust/src/fix/store.rs`: FIX category storage and atomic catalog mutations.

use super::committed_registry;
use super::crated_components;

use super::path as fpath;

use std::path::PathBuf;
use yggdryl::fix::FixReplacement;
use yggdryl::local::LocalFolder;
use yggdryl::{
    DataType, Field, FixCategory, FixCode, FixId, FixRegistry, IOBase, Scalar, StructType,
};

fn scratch(label: &str) -> PathBuf {
    let path = LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!(
            "yggdryl-fix-catalog-{label}-{}",
            std::process::id()
        ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// Where a snapshot states the definition named `name`.
///
/// A dump states every definition the registry holds, the crate's own among
/// them, so a test reads back the one it wrote by name rather than by the
/// position it happened to land in.
fn definition_at(document: &Scalar, category: &str, name: &str) -> usize {
    document.as_struct().unwrap()[category]
        .as_sequence()
        .unwrap()
        .iter()
        .position(|value| value.as_struct().unwrap()["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("{category} states {name}"))
}

/// The environment variable that rewrites the crate's own documents under
/// `config/fix` from the running crate instead of checking them.
const DUMP_WRITE: &str = "YGGDRYL_FIX_DUMP_WRITE";

/// The documents a store dump states that the generator does not: the
/// crate's field shard and the fixed row.
const CRATE_DOCUMENTS: [&str; 5] = [
    "fields/000000650.json",
    "components/fixmsg.json",
    "groups/identifiers.json",
    "groups/metadata.json",
    "codesets/msgcatcodeset.json",
];

#[test]
fn the_committed_store_carries_the_crate_dump() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = super::committed_registry();
    let scratch = scratch("crate-dump");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut folder = LocalFolder::new(scratch.clone()).unwrap();
    registry.commit(&mut folder).unwrap();
    for name in CRATE_DOCUMENTS {
        let written = std::fs::read(scratch.join(name)).unwrap();
        let committed = root.join(name);
        if std::env::var(DUMP_WRITE).as_deref() == Ok("1") {
            std::fs::write(&committed, &written).unwrap();
            continue;
        }
        let held = std::fs::read(&committed).unwrap_or_else(|absent| {
            panic!(
                "{}: {absent}. Write it with {DUMP_WRITE}=1 cargo test --locked -p yggdryl --test fix the_committed_store_carries_the_crate_dump",
                committed.display()
            )
        });
        assert!(
            held == written,
            "{name} is stale: write it with {DUMP_WRITE}=1 cargo test --locked -p yggdryl --test fix the_committed_store_carries_the_crate_dump"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

/// The set `PartyID` reads by, named the way a field that states none is
/// named: the folded field name and `codeset`.
const PARTY_CODESET: &str = "partyidcodeset";

fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn catalog() -> FixRegistry {
    let counter = tagged("NoPartyIDs", 453, DataType::Int32);
    let mut partyid = tagged("PartyID", 448, DataType::utf8());
    partyid.as_fix_mut().set_codeset(PARTY_CODESET).unwrap();
    // The vocabulary first: a field names the set it reads by, and a
    // registry refuses a field naming one it does not hold.
    let mut registry = FixRegistry::new();
    registry
        .set_codeset(PARTY_CODESET, &[FixCode::new("Broker", "B")])
        .unwrap();
    for field in [counter, partyid] {
        registry.insert(field).unwrap();
    }
    let mut member = registry.field(448).unwrap().clone();
    member.as_fix_mut().set_field_ref("PartyID").unwrap();
    let component = StructType::from_fields([member])
        .map(DataType::from)
        .unwrap()
        .required_field("Party");
    registry.insert(component).unwrap();
    // The occurrence is the stored definition, derived tag and all, the way
    // the message below takes the stored group rather than the field it was
    // built from.
    let component = registry.field_by_name("Party").unwrap().clone();
    let mut group = DataType::list(component).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component("Party").unwrap();
    registry.insert(group).unwrap();
    let mut group = registry.field_by_name("Parties").unwrap().clone();
    group.as_fix_mut().set_group("Parties").unwrap();
    let mut counter = registry.field(453).unwrap().clone();
    counter.as_fix_mut().set_field_ref("NoPartyIDs").unwrap();
    let mut message = StructType::from_fields([counter, group])
        .map(DataType::from)
        .unwrap()
        .required_field("NewOrderSingle");
    message.as_fix_mut().set_msgtype("D").unwrap();
    registry.insert(message).unwrap();
    registry
}

#[test]
fn crate_map_groups_are_written_and_still_win_over_a_stored_override() {
    let registry = FixRegistry::new();
    let map = registry.get_field_by_counter(65_020).unwrap();
    let mut stated = map.clone();
    stated.set_comment("not the crate's declaration").unwrap();
    let snapshot = registry.into_json().unwrap();
    // The dump states it - a snapshot is the whole dictionary - and reading
    // one back takes the held declaration over the document's.
    assert!(snapshot.contains("identifiers"));
    assert_eq!(FixRegistry::from_json(&snapshot).unwrap(), registry);

    let document = Scalar::from_struct([
        ("fields", Scalar::from_sequence([])),
        ("components", Scalar::from_sequence([])),
        (
            "groups",
            Scalar::from_sequence([stated.clone().into_value()]),
        ),
    ])
    .unwrap();
    let loaded = FixRegistry::from_json(&yggdryl::into_json_scalar(&document).unwrap()).unwrap();
    assert_eq!(loaded.get_field_by_counter(65_020), Some(map));

    let root = scratch("crate-map");
    let mut folder = LocalFolder::new(&root).unwrap();
    registry.commit(&mut folder).unwrap();
    assert!(root.join("groups/identifiers.json").exists());
    folder
        .child_by_path("groups/identifiers.json")
        .unwrap()
        .write_all_bytes(&stated.into_json_bytes().unwrap())
        .unwrap();
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded.get_field_by_counter(65_020), Some(map));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn builtin_map_group_references_resolve_after_snapshot_and_directory_roundtrips() {
    let mut registry = FixRegistry::new();
    let mut map = registry.get_field_by_counter(65_020).unwrap().clone();
    map.as_fix_mut().set_group("identifiers").unwrap();
    let component = StructType::from_fields([map])
        .map(DataType::from)
        .unwrap()
        .required_field("identified");
    registry.insert(component).unwrap();
    let snapshot = registry.into_json().unwrap();
    let loaded = FixRegistry::from_json(&snapshot).unwrap();
    assert_eq!(loaded, registry);

    let root = scratch("crate-map-reference");
    let mut folder = LocalFolder::new(&root).unwrap();
    registry.commit(&mut folder).unwrap();
    assert!(root.join("groups/identifiers.json").exists());
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
        let entries = StructType::from_fields([key, value])
            .map(DataType::from)
            .unwrap()
            .required_field("entries");
        let mapping = DataType::map(entries, sorted)
            .unwrap()
            .nullable_field("pairs");
        let component = StructType::from_fields([mapping])
            .map(DataType::from)
            .unwrap()
            .required_field("lookup");
        registry.insert(component).unwrap();

        let json = registry.into_json().unwrap();
        let snapshot = yggdryl::from_json_scalar(&json).unwrap();
        let at = definition_at(&snapshot, "components", "lookup");
        let component = Field::from_value(
            snapshot.as_struct().unwrap()["components"]
                .get(at)
                .unwrap()
                .into_owned(),
        )
        .unwrap();
        let Some(map) = (component.fields()[0].dtype()).as_mapping() else {
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
            registry.update(changed).unwrap();
        }
        let component = registry.field_by_name("lookup").unwrap();
        let Some(map) = (component.fields()[0].dtype()).as_mapping() else {
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
        let mut folder = LocalFolder::new(&root).unwrap();
        registry.commit(&mut folder).unwrap();
        assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn map_entries_component_references_refresh_without_losing_the_storage_contract() {
    for sorted in [false, true] {
        let mut component = StructType::from_fields([
            DataType::utf8().required_field("key"),
            DataType::utf8().nullable_field("value"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("lookupentry");
        component.set_comment("Original entries").unwrap();
        let mut registry = FixRegistry::new();
        registry.insert(component).unwrap();
        let mut entries = registry.field_by_name("lookupentry").unwrap().clone();
        entries.as_fix_mut().set_component("lookupentry").unwrap();
        let mapping = DataType::map(entries, sorted)
            .unwrap()
            .nullable_field("nativepairs");
        let containing = StructType::from_fields([mapping])
            .map(DataType::from)
            .unwrap()
            .required_field("mappedlookup");
        registry.insert(containing).unwrap();

        let json = registry.into_json().unwrap();
        let document = yggdryl::from_json_scalar(&json).unwrap();
        let at = definition_at(&document, "components", "mappedlookup");
        let mut stored = Field::from_value(
            document.as_struct().unwrap()["components"]
                .get(at)
                .unwrap()
                .into_owned(),
        )
        .unwrap();
        assert_eq!(stored.name(), "mappedlookup");
        let Some(map) = (stored.fields()[0].dtype()).as_mapping() else {
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
            .set_dtype(DataType::from(StructType::from_fields([mapping]).unwrap()))
            .unwrap();
        let refused =
            Scalar::from_struct(document.as_struct().unwrap().iter().map(|(key, value)| {
                (
                    key.clone(),
                    if key == "components" {
                        Scalar::from_sequence(
                            value
                                .as_sequence()
                                .unwrap()
                                .iter()
                                .enumerate()
                                .map(|(index, held)| {
                                    if index == at {
                                        stored.clone().into_value()
                                    } else {
                                        held.clone()
                                    }
                                })
                                .collect::<Vec<_>>(),
                        )
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

        let mut changed = registry.field_by_name("lookupentry").unwrap().clone();
        changed.set_comment("Reviewed entries").unwrap();
        registry.update(changed).unwrap();
        let containing = registry.field_by_name("mappedlookup").unwrap();
        let mapping = &containing.fields()[0];
        let Some(map) = (mapping.dtype()).as_mapping() else {
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
        let mut extended = registry.field_by_name("lookupentry").unwrap().clone();
        extended
            .set_dtype(
                StructType::from_fields(
                    extended
                        .fields()
                        .iter()
                        .cloned()
                        .chain([DataType::utf8().nullable_field("extra")]),
                )
                .map(DataType::from)
                .unwrap(),
            )
            .unwrap();
        assert!(registry.add_field(extended).is_err());
        assert_eq!(registry, before);
        assert_eq!(registry.stable_hash(), before.stable_hash());

        let root = scratch(if sorted {
            "sorted-map-component"
        } else {
            "map-component"
        });
        let mut folder = LocalFolder::new(&root).unwrap();
        registry.commit(&mut folder).unwrap();
        assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn ordinary_stored_component_references_still_require_null_placeholders() {
    let component = StructType::from_fields([DataType::utf8().nullable_field("value")])
        .map(DataType::from)
        .unwrap()
        .required_field("ordinary");
    let mut occurrence = component.clone();
    occurrence.as_fix_mut().set_component("ordinary").unwrap();
    let containing = StructType::from_fields([occurrence])
        .map(DataType::from)
        .unwrap()
        .required_field("containing");
    let document = Scalar::from_struct([
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
    let map = registry.get_field_by_counter(65_020).unwrap();
    let mut substituted = map.clone();
    substituted.as_fix_mut().set_tag(9001).unwrap();
    substituted.as_fix_mut().set_counter(9001).unwrap();
    let document = Scalar::from_struct([
        ("fields", Scalar::from_sequence([])),
        ("components", Scalar::from_sequence([])),
        (
            "groups",
            Scalar::from_sequence([substituted.clone().into_value()]),
        ),
    ])
    .unwrap();
    let loaded = FixRegistry::from_json(&yggdryl::into_json_scalar(&document).unwrap()).unwrap();
    assert_eq!(loaded.get_field_by_counter(65_020), Some(map));
    assert!(loaded.get_field_by_counter(9001).is_none());

    let root = scratch("crate-map-substitution");
    let folder = LocalFolder::new(&root).unwrap();
    folder
        .child_by_path("groups/identifiers.json")
        .unwrap()
        .write_all_bytes(&substituted.into_json_bytes().unwrap())
        .unwrap();
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded.get_field_by_counter(65_020), Some(map));
    assert!(loaded.get_field_by_counter(9001).is_none());
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
    registry.insert(venue).unwrap();
    let mut pending = registry.field_by_name("NewOrderSingle").unwrap().clone();
    pending.as_fix_mut().set_branches(["pending"]).unwrap();
    registry.update(pending).unwrap();

    let json = registry.into_json().unwrap();
    let document = yggdryl::from_json_scalar(&json).unwrap();
    let record = document.as_struct().unwrap();
    // The three categories and the `codesets` the fields read by, which is
    // a folder of vocabularies rather than a fourth category.
    assert_eq!(record.len(), FixCategory::ALL.len() + 1);
    assert!(record["codesets"].as_sequence().is_some());
    assert!(record.get("branches").is_none());
    for category in FixCategory::ALL {
        assert!(record[category.as_str()].as_sequence().is_some());
    }
    let group = Field::from_value(record["groups"].get(0).unwrap().into_owned()).unwrap();
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
        loaded
            .codeset_of(loaded.field(448).unwrap())
            .unwrap()
            .code_name("B"),
        Some("Broker")
    );
    let message = loaded.msgtype("D").unwrap();
    assert_eq!(message.name(), "NewOrderSingle");
    assert_eq!(message.get_group_by_tag(453).unwrap().name(), "Parties");

    let reversed = Scalar::from_struct(record.iter().map(|(key, value)| {
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
    old.as_fix_mut().set_names(["quoteackstatus"]).unwrap();
    old.as_fix_mut().set_tags(&[1865]).unwrap();
    let current = tagged("quoteackstatus", 1865, DataType::Int32);
    for fields in [
        [old.clone(), current.clone()],
        [current.clone(), old.clone()],
    ] {
        let mut registry = FixRegistry::new();
        for field in fields {
            registry.insert(field).unwrap();
        }
        assert_eq!(registry.field("quoteackstatus").unwrap(), &current);
        assert_eq!(registry.field(1865).unwrap(), &current);
        assert_eq!(super::scalars(&registry), 2 + super::seeded_fields());
        let loaded = FixRegistry::from_json(&registry.into_json().unwrap()).unwrap();
        assert_eq!(loaded, registry);
        let document = yggdryl::from_json_scalar(registry.into_json().unwrap()).unwrap();
        let reversed =
            Scalar::from_struct(document.as_struct().unwrap().iter().map(|(key, value)| {
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
        // The canonical field is what the tag and the fold both answer, so
        // restating it replaces it by itself and a second tag under the
        // held name is refused.
        assert!(registry.insert(current.clone()).unwrap().is_some());
        assert_eq!(registry, before);
        assert!(
            registry
                .insert(tagged("quoteackstatus", 1866, DataType::Int32))
                .is_err()
        );
        assert_eq!(registry, before);
        // Another name on the held tag is a field of its own beside the
        // holder, which lends nothing but learns the name; the bare tag
        // keeps answering the holder.
        registry
            .insert(tagged("other", 1865, DataType::Int32))
            .unwrap();
        assert_eq!(super::scalars(&registry), 3 + super::seeded_fields());
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
                .names()
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
    // Every field the registry holds, the definitions included: the
    // dictionary's own scalars and the crate's, then its components and
    // groups.
    assert_eq!(
        loaded.len(),
        super::scalars(&loaded)
            + super::definitions(&loaded, FixCategory::Components).count()
            + super::definitions(&loaded, FixCategory::Groups).count()
    );
    assert_eq!(super::scalars(&loaded), 6241 + super::crated_fields());
    assert_eq!(
        super::definitions(&loaded, FixCategory::Components).count(),
        928 + super::crated_components()
    );
    assert_eq!(super::msgtypes(&loaded).count(), 181);
    // The generated groups, and the crate's two Map groups beside them.
    assert_eq!(
        super::definitions(&loaded, FixCategory::Groups).count(),
        580 + 2
    );
}

#[test]
fn registry_hashes_include_named_definitions_and_membership() {
    let original = catalog();
    let mut changed = original.clone();
    let old_message = original.msgtype("D").unwrap();
    let mut field = old_message.as_field().clone();
    field.as_fix_mut().set_msgtype("D2").unwrap();
    changed.update(field).unwrap();
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
        // A message is a component: a snapshot written with a
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
    // The three categories are the whole of a snapshot's definitions - only
    // the `codesets` the fields read by stand beside them, and a snapshot
    // stating none is a registry of the crate's own fields and nothing more.
    let empty = FixRegistry::from_json(r#"{"fields":[],"components":[],"groups":[]}"#).unwrap();
    assert_eq!(empty, FixRegistry::new());
    assert_eq!(super::scalars(&empty), super::seeded_fields());
    assert!(empty.dialects().is_empty());
    let document = yggdryl::from_json_scalar(catalog().into_json().unwrap()).unwrap();
    let missing = Scalar::from_struct(document.as_struct().unwrap().iter().map(|(key, value)| {
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
    let mut folder = LocalFolder::new(&root).unwrap();
    let registry = catalog();
    registry.commit(&mut folder).unwrap();
    for file in [
        "fields/000000004.json",
        "components/Party.json",
        "groups/Parties.json",
        "components/NewOrderSingle.json",
        "codesets/partyidcodeset.json",
    ] {
        assert!(root.join(file).is_file(), "{file}");
    }
    // A message is a component with a marker, so no folder is written for
    // one: what stands beside the three categories is `codesets`, which
    // holds vocabularies rather than Field documents.
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
            .field_by_name("Parties")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(derived),
        "the derived tag survives the round trip"
    );
    assert_eq!(loaded.field(453).unwrap().dtype(), &DataType::Int32);
    assert_eq!(loaded.field_by_counter(453).unwrap().name(), "Parties");
    let set = loaded.codeset_of(loaded.field(448).unwrap()).unwrap();
    assert_eq!(set.codes().count(), 1);
    assert_eq!(set.code_name("B"), Some("Broker"));
    assert_eq!(set.name(), PARTY_CODESET);
    let party = loaded.field(448).unwrap();
    assert_eq!(party.as_fix().codeset(), Some(PARTY_CODESET));
    assert_eq!(party.get_metadata("FIX:codeset"), Some(PARTY_CODESET));
    assert!(
        party.as_fix().get("codes").is_none(),
        "members live in the registry set"
    );
    // A definition is a field of the registry: the one namespace answers it
    // under the name it is stored by.
    assert!(loaded.get_field("Parties").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn code_sets_round_trip_through_their_own_folder_and_are_pruned_when_they_go() {
    let root = scratch("codesets");
    let mut folder = LocalFolder::new(&root).unwrap();
    let mut registry = catalog();
    // A second vocabulary no field reads by: a set is the dictionary's, so
    // what takes one away is the dictionary rather than a field.
    registry
        .set_codeset("venuecodeset", &[FixCode::new("Venue", "V")])
        .unwrap();
    registry.commit(&mut folder).unwrap();
    // One document per set, each stating the name it is filed under so the
    // file says what it is without its own path.
    let stored = std::fs::read(root.join("codesets/partyidcodeset.json")).unwrap();
    let document = yggdryl::from_json_scalar(stored).unwrap();
    assert_eq!(
        document.get_key_str("name").and_then(Scalar::as_str),
        Some(PARTY_CODESET)
    );
    assert_eq!(
        document
            .get_key_str("codes")
            .and_then(|codes| codes.get(0))
            .as_deref()
            .and_then(|code| code.get_key_str("value"))
            .and_then(Scalar::as_str),
        Some("B")
    );
    assert!(root.join("codesets/venuecodeset.json").is_file());
    let loaded = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(loaded, registry);
    // The persisted party and venue sets sit beside the built-in MsgCat set.
    assert_eq!(loaded.codesets().len(), 3);
    assert_eq!(
        loaded.codeset("venuecodeset").unwrap().code_name("V"),
        Some("Venue")
    );

    // A set the dictionary no longer holds takes its document with it, the
    // way a definition no longer held takes its own.
    assert!(registry.remove_codeset("venuecodeset").unwrap().is_some());
    registry.commit(&mut folder).unwrap();
    assert!(!root.join("codesets/venuecodeset.json").exists());
    assert!(root.join("codesets/partyidcodeset.json").is_file());
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_folded_code_set_name_collision_refuses_atomically() {
    let mut registry = catalog();
    let before = registry.clone();
    // The alternate spelling resolves to the held set. A rendered vocabulary
    // may not make two folded names answer, and the refusal must not replace
    // the held document under that canonical key.
    let error = registry
        .set_codeset(
            "Party_ID_CodeSet",
            &[
                FixCode::new("GoodTillDate", "6"),
                FixCode::new("good_till_date", "7"),
            ],
        )
        .unwrap_err();
    assert!(matches!(error, yggdryl::Error::Parse { .. }), "{error}");
    assert_eq!(registry, before);
    assert_eq!(
        registry
            .codeset_of(registry.field(448).unwrap())
            .unwrap()
            .code_value("Broker"),
        Some("B")
    );
}

#[test]
fn replacing_a_code_set_forgets_warm_typed_parse_memos() {
    let mut registry = std::sync::Arc::new(catalog());
    {
        let codec = super::fixed_codec(std::sync::Arc::clone(&registry));
        let message = codec
            .parse_line(b"8=FIX.4.4|35=D|448=Broker|10=0|")
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(message.by_tag(448).unwrap().as_str(), Some("B"));
    }
    assert_eq!(std::sync::Arc::strong_count(&registry), 1);
    // Cached plans retain Weak references. `make_mut` dissociates those
    // without cloning while this remains the sole strong owner.
    std::sync::Arc::make_mut(&mut registry)
        .set_codeset(PARTY_CODESET, &[FixCode::new("Broker", "D")])
        .unwrap();
    let codec = super::fixed_codec(std::sync::Arc::clone(&registry));
    let message = codec
        .parse_line(b"8=FIX.4.4|35=D|448=Broker|10=0|")
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(message.by_tag(448).unwrap().as_str(), Some("D"));
}

#[test]
fn a_stored_code_set_naming_another_stem_than_its_own_is_refused() {
    let root = scratch("codeset-stem");
    let mut folder = LocalFolder::new(&root).unwrap();
    catalog().commit(&mut folder).unwrap();
    // The stem is how a set is addressed, so a file stating another name is
    // refused the way a definition's document is.
    folder
        .child_by_path("codesets/othercodeset.json")
        .unwrap()
        .write_all_bytes(br#"{"name":"partyidcodeset","codes":[{"value":"B","name":"Broker"}]}"#)
        .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(
        error.contains("expected the code set name to equal its filename stem"),
        "{error}"
    );
    assert!(error.contains("othercodeset.json"), "{error}");
    std::fs::remove_file(root.join("codesets/othercodeset.json")).unwrap();
    assert!(FixRegistry::from_handle(&folder).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_field_naming_a_code_set_the_store_does_not_hold_is_refused() {
    let root = scratch("codeset-absent");
    let mut folder = LocalFolder::new(&root).unwrap();
    catalog().commit(&mut folder).unwrap();
    // A field may not be left reading by a vocabulary nothing states, so
    // the field's own document is where the absence is named.
    std::fs::remove_file(root.join("codesets/partyidcodeset.json")).unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err();
    assert!(matches!(error, yggdryl::Error::Absent { .. }), "{error}");
    let error = error.to_string();
    assert!(error.contains("codesets"), "{error}");
    assert!(error.contains(PARTY_CODESET), "{error}");
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
        assert!(registry.remove(name).is_none(), "{category}/{name}");
        assert_eq!(registry, before);
    }
    let changed = tagged("PartyID", 448, DataType::Int32);
    assert!(registry.update(changed).is_err());
    assert_eq!(registry, before);
    // The held name on another tag is the same field spelled with another
    // number, which a creation refuses: only a merge folds it in.
    assert!(
        registry
            .insert(tagged("PartyID", 5001, DataType::utf8()))
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
            .update(DataType::from(StructType::from_fields([]).unwrap()).required_field("Missing"))
            .is_err()
    );
    assert_eq!(registry, before);
    registry.remove("NewOrderSingle").unwrap();
    registry.remove("Parties").unwrap();
    registry.remove("Party").unwrap();
    registry.remove("PartyID").unwrap();
}

#[test]
fn enum_codes_belong_to_each_field() {
    let mut registry = catalog();
    // A venue's field on the standard tag, under its own name: a second
    // field beside the holder, each reading by the set it named.
    let mut field = tagged("VenuePartyID", 448, DataType::utf8());
    field.as_fix_mut().set_branches(["venue"]).unwrap();
    registry
        .set_codeset("venuepartyidcodeset", &[FixCode::new("VenueBroker", "V")])
        .unwrap();
    field
        .as_fix_mut()
        .set_codeset("venuepartyidcodeset")
        .unwrap();
    registry.insert(field).unwrap();
    let held = registry
        .field(FixId::of(448, "VenuePartyID").unwrap())
        .unwrap();
    let venue = registry.codeset_of(held).unwrap();
    assert_eq!(venue.code_value("VenueBroker"), Some("V"));
    assert_eq!(venue.code_value("Broker"), None);
    assert!(held.as_fix().has_branch("venue"));
    assert_eq!(registry.field(448).unwrap().name(), "PartyID");
    let holder = registry
        .codeset_of(registry.field(448).unwrap())
        .expect("the holder's own set");
    assert_eq!(holder.code_value("VenueBroker"), None);
    assert_eq!(holder.code_value("Broker"), Some("B"));
    // The two test vocabularies and built-in MsgCat are held once each under
    // their names. Categories hold Field documents and resolve references,
    // which is why `codesets` is written beside the three folders rather
    // than as a fourth.
    assert_eq!(registry.codesets().len(), 3);
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
        assert_eq!(super::scalars(registry), 3 + super::seeded_fields());
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
                .names()
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
        let party = registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
            .unwrap();
        assert_eq!(
            registry.codeset_of(party).unwrap().code_name("B"),
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
    let reversed = Scalar::from_struct(document.as_struct().unwrap().iter().map(|(key, value)| {
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
            .names()
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
    let mut folder = LocalFolder::new(&root).unwrap();
    registry.commit(&mut folder).unwrap();
    assert!(root.join("fields/000000004.json").is_file());
    let shard =
        yggdryl::from_json_scalar(std::fs::read(root.join("fields/000000004.json")).unwrap())
            .unwrap();
    assert_eq!(shard.as_sequence().unwrap().len(), 3);
    let stored = FixRegistry::from_handle(&folder).unwrap();
    assert_eq!(stored, registry);
    check(&stored);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn contexts_sharing_a_counter_are_explicitly_ambiguous() {
    let mut registry = catalog();
    let mut group = registry.field_by_name("Parties").unwrap().clone();
    group.set_name("TradeParties");
    registry.insert(group).unwrap();
    assert!(registry.get_field_by_counter(453).is_none());
    assert!(registry.field_by_counter(453).is_err());
    // The catalog's two groups, the renamed one beside them, and the
    // crate's own `metadata` Map.
    assert_eq!(
        super::definitions(&registry, FixCategory::Groups).count(),
        4
    );
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
}

#[test]
fn store_removes_empty_shards_and_named_documents() {
    let root = scratch("cleanup");
    let mut folder = LocalFolder::new(&root).unwrap();
    let mut registry = catalog();
    registry
        .insert(tagged("Distant", 10000, DataType::utf8()))
        .unwrap();
    registry.commit(&mut folder).unwrap();
    registry.remove(10000).unwrap();
    registry.remove("NewOrderSingle").unwrap();
    registry.commit(&mut folder).unwrap();
    assert!(!root.join("fields/000000100.json").exists());
    assert!(!root.join("components/NewOrderSingle.json").exists());
    assert!(root.join("components/Party.json").exists());
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unresolved_and_cyclic_compact_references_name_the_failure() {
    for (label, reference) in [("missing", "Missing"), ("cycle", "Cycle")] {
        let root = scratch(label);
        let folder = LocalFolder::new(&root).unwrap();
        let mut child = DataType::Null.nullable_field("child");
        child.as_fix_mut().set_component(reference).unwrap();
        let field = StructType::from_fields([child])
            .map(DataType::from)
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
    let folder = LocalFolder::new(&root).unwrap();
    for bytes in [
        b"not json".to_vec(),
        b"{}".to_vec(),
        yggdryl::json::into_bytes(&Scalar::from_sequence([tagged(
            "Misplaced",
            150,
            DataType::utf8(),
        )
        .into_value()]))
        .unwrap(),
    ] {
        folder
            .child_by_path("fields/000000000.json")
            .unwrap()
            .write_all_bytes(&bytes)
            .unwrap();
        let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
        assert!(error.contains("0.json"), "{error}");
    }
    folder
        .child_by_path("fields/000000000.json")
        .unwrap()
        .remove(false)
        .unwrap();
    // A shard whose number disagrees with the tags inside is located by
    // its own name.
    let field = tagged("Misplaced", 5001, DataType::utf8());
    let bytes =
        yggdryl::json::into_bytes(&Scalar::from_sequence([field.clone().into_value()])).unwrap();
    folder
        .child_by_path("fields/000000049.json")
        .unwrap()
        .write_all_bytes(&bytes)
        .unwrap();
    let error = FixRegistry::from_handle(&folder).unwrap_err().to_string();
    assert!(error.contains("49.json"), "{error}");
    folder
        .child_by_path("fields/000000049.json")
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
    assert_eq!(super::scalars(&loaded), super::seeded_fields());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tracked_seed_resolves_every_category_and_native_reference_graph() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root).unwrap()).unwrap();
    assert_eq!(
        registry.len(),
        super::scalars(&registry)
            + super::definitions(&registry, FixCategory::Components).count()
            + super::definitions(&registry, FixCategory::Groups).count()
    );
    assert_eq!(super::scalars(&registry), 6241 + super::crated_fields());
    // The census the one namespace rests on: 747 components and 181 messages fold
    // to 928 distinct names, so no message and component share one.
    for (category, count) in [
        (FixCategory::Components, 928 + super::crated_components()),
        (FixCategory::Groups, 580 + 2),
    ] {
        assert_eq!(
            super::definitions(&registry, category).count(),
            count,
            "{category}"
        );
    }
    assert_eq!(super::msgtypes(&registry).count(), 181);
    assert_eq!(
        super::definitions(&registry, FixCategory::Components)
            .filter(|field| field.as_fix().msgtype().is_some())
            .count(),
        181
    );
    assert_eq!(registry.field(453).unwrap().dtype(), &DataType::Int32);
    let group = registry.field_by_name("Parties").unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("parties list")
    };
    assert_eq!(item.name(), "party");
    assert_eq!(
        item.get_field("partyid").unwrap().as_fix().tag().unwrap(),
        Some(448)
    );
    assert_eq!(
        registry
            .codeset_of(registry.field(54).unwrap())
            .unwrap()
            .code_name("1"),
        Some("Buy")
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
    // over the group - a message is a component, so name order
    // alone would put `NewOrderSingle` before the `Parties` it references.
    let source = |respell: fn(&str) -> String| {
        // The scalars alone: `iter` answers the definitions beside them now,
        // and a definition handed to `from_fields` ahead of what it
        // references has nothing to resolve against.
        //
        // The vocabularies lead them, because a field names the set it reads
        // by and a dictionary refuses one naming a set it does not hold: the
        // sets are what the scalars are folded into, never a copy each field
        // carries.
        let mut source = FixRegistry::new();
        for set in original.codesets() {
            let codes: Vec<FixCode> = set
                .codes()
                .map(|code| FixCode::from(code.unwrap()))
                .collect();
            source.set_codeset(set.name(), &codes).unwrap();
        }
        source
            .add_fields(
                original
                    .iter()
                    .filter(|field| super::category_of(field) == FixCategory::Fields)
                    .cloned(),
            )
            .unwrap();
        let plain = super::definitions(&original, FixCategory::Components)
            .filter(|field| field.as_fix().msgtype().is_none())
            // The crate's own components - `instids` - are already in the
            // source, which starts from a registry of its own, so respelling
            // one would restate it.
            .filter(|field| {
                !field
                    .as_fix()
                    .tag()
                    .unwrap()
                    .is_some_and(yggdryl::is_crate_tag)
            })
            .map(|field| (FixCategory::Components, field));
        let groups = super::definitions(&original, FixCategory::Groups)
            .filter(|field| {
                !field
                    .as_fix()
                    .tag()
                    .unwrap()
                    .is_some_and(yggdryl::is_crate_tag)
            })
            .map(|field| (FixCategory::Groups, field));
        let messages = original
            .iter()
            .filter(|field| field.as_fix().msgtype().is_some())
            .map(|held| (FixCategory::Components, held));
        for (_, field) in plain.chain(groups).chain(messages) {
            let mut field = field.clone();
            field.set_name(respell(field.name()));
            source.insert(field).unwrap();
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
        for (_, name) in [
            (FixCategory::Components, "Party"),
            (FixCategory::Groups, "Parties"),
            (FixCategory::Components, "NewOrderSingle"),
        ] {
            assert_eq!(target.field_by_name(name).unwrap().name(), name);
        }
        let group = target.msgtype("D").unwrap().get_group_by_tag(453).unwrap();
        assert_eq!(group.name(), "Parties");
        let party = target
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
            .unwrap();
        assert_eq!(
            target.codeset_of(party).unwrap().code_name("B"),
            Some("Broker")
        );
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );
    }
}

#[test]
fn merging_catalogs_resolves_imported_references_against_the_code_set_union() {
    let mut target = catalog();
    let mut source = catalog();
    // The source knows one more member of the set both dictionaries hold,
    // which is what the merge has to union before it folds the fields.
    source
        .merge_codeset(PARTY_CODESET, &[FixCode::new("Client", "C")])
        .unwrap();
    let mut message = source.field_by_name("NewOrderSingle").unwrap().clone();
    message.set_name("IncomingOrder");
    message.as_fix_mut().set_msgtype("I").unwrap();
    source.insert(message).unwrap();
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
        let set = target.codeset_of(field).unwrap_or_else(|| panic!("{path}"));
        assert_eq!(set.code_name("B"), Some("Broker"), "{path}");
        assert_eq!(set.code_name("C"), Some("Client"), "{path}");
    }
    let group = target.msgtype("I").unwrap().get_group_by_tag(453).unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("the resolved group list")
    };
    assert_eq!(
        target
            .codeset_of(item.get_field("PartyID").unwrap())
            .unwrap()
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
    coded.as_fix_mut().set_branches(["incoming"]).unwrap();
    // The source states the set its field reads by before the field
    // arrives, and states one member of it: the fold unions the two.
    let mut source = FixRegistry::new();
    source
        .set_codeset(PARTY_CODESET, &[FixCode::new("Client", "C")])
        .unwrap();
    source.insert(coded).unwrap();
    let mut member = source.field(448).unwrap().clone();
    member.as_fix_mut().set_field_ref("PartyID").unwrap();
    let extended = StructType::from_fields([member, DataType::Int32.nullable_field("Extra")])
        .map(DataType::from)
        .unwrap()
        .required_field("Party");
    source.insert(extended).unwrap();
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
    let party = target
        .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
        .unwrap();
    assert_eq!(
        target.codeset_of(party).unwrap().code_name("C"),
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
    let changed = StructType::from_fields([member, DataType::Int64.nullable_field("Extra")])
        .map(DataType::from)
        .unwrap()
        .required_field("Party");
    source.insert(changed).unwrap();
    let error = target.merge_with(&source).unwrap_err().to_string();
    assert!(error.contains("Party.Extra"), "{error}");
    assert_eq!(target, before);
    assert_eq!(target.stable_hash(), before.stable_hash());
}

#[test]
fn referenced_metadata_updates_cascade_and_occurrence_overrides_fail_without_loss() {
    let mut registry = catalog();
    let mut component = registry.field_by_name("Party").unwrap().clone();
    component
        .as_fix_mut()
        .set_description("A changed description")
        .unwrap();
    registry.update(component).unwrap();
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
    registry.update(scalar).unwrap();
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyID"))
            .unwrap()
            .as_fix()
            .description(),
        Some("A party identifier")
    );
    let mut group = registry.field_by_name("Parties").unwrap().clone();
    group
        .as_fix_mut()
        .set_description("Reviewed parties")
        .unwrap();
    registry.update(group).unwrap();
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties"))
            .unwrap()
            .as_fix()
            .description(),
        Some("Reviewed parties")
    );
    let root = scratch("metadata-refresh");
    let mut folder = LocalFolder::new(&root).unwrap();
    registry.commit(&mut folder).unwrap();
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();
    let before = registry.clone();
    let mut child = registry.field(448).unwrap().clone();
    child.as_fix_mut().set_field_ref("PartyID").unwrap();
    child
        .as_fix_mut()
        .set_description("An occurrence override")
        .unwrap();
    let component = StructType::from_fields([child])
        .map(DataType::from)
        .unwrap()
        .required_field("OverriddenParty");
    assert!(registry.insert(component).is_err());
    assert_eq!(registry, before);
}

#[test]
fn case_only_replacements_keep_canonical_spelling_and_refresh_every_category() {
    let mut registry = catalog();
    let root = scratch("canonical-case");
    let mut folder = LocalFolder::new(&root).unwrap();
    registry.commit(&mut folder).unwrap();
    for (_, name) in [
        (FixCategory::Fields, "PartyID"),
        (FixCategory::Components, "Party"),
        (FixCategory::Groups, "Parties"),
        (FixCategory::Components, "NewOrderSingle"),
    ] {
        for (replace, description) in [(false, "Updated metadata"), (true, "Replaced metadata")] {
            let mut field = registry.field_by_name(name).unwrap().clone();
            field.set_name(name.to_ascii_lowercase());
            field.as_fix_mut().set_description(description).unwrap();
            if replace {
                registry.insert(field).unwrap();
            } else {
                registry.update(field).unwrap();
            }
            let canonical = registry.field_by_name(name).unwrap();
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
    registry.commit(&mut folder).unwrap();
    assert_eq!(FixRegistry::from_handle(&folder).unwrap(), registry);
    std::fs::remove_dir_all(root).unwrap();

    let before = registry.clone();
    let mut component = registry.field_by_name("Party").unwrap().clone();
    component.set_name("Pa_rty");
    assert!(registry.update(component).is_err());
    assert_eq!(registry, before);
}

#[test]
fn folded_field_updates_keep_canonical_names_and_refresh_references() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let mut registry = FixRegistry::from_handle(&LocalFolder::new(root).unwrap()).unwrap();
    let mut incoming = tagged("Symbol", 55, DataType::utf8());
    incoming.as_fix_mut().set_tags(&[9001]).unwrap();
    incoming.as_fix_mut().set_names(["Sym"]).unwrap();
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
    let folder = LocalFolder::new(&root).unwrap();
    let field = tagged("Symbol", 55, DataType::utf8());
    let bytes = yggdryl::json::into_bytes(&Scalar::from_sequence([
        field.clone().into_value(),
        field.into_value(),
    ]))
    .unwrap();
    folder
        .child_by_path("fields/000000000.json")
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
    let folder = LocalFolder::new(&root).unwrap();
    for index in (0..34).rev() {
        let mut child = DataType::Null.nullable_field("child");
        if index < 33 {
            child
                .as_fix_mut()
                .set_component(&format!("Chain{:02}", index + 1))
                .unwrap();
        }
        let inline = StructType::from_fields([child])
            .map(DataType::from)
            .unwrap()
            .required_field("inline");
        let field = StructType::from_fields([inline])
            .map(DataType::from)
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
fn one_message_code_namespace_answers_the_bare_code_to_its_first_holder() {
    let mut registry = catalog();
    // A code re-declared under another name is a second message: the bare
    // code keeps answering the first holder, the newcomer is reached by name.
    let mut other = StructType::from_fields([])
        .map(DataType::from)
        .unwrap()
        .required_field("OtherOrder");
    other.as_fix_mut().set_msgtype("D").unwrap();
    registry.insert(other).unwrap();
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(registry.msgtype("OtherOrder").unwrap().as_str(), "D");
    assert_eq!(registry.msgtype("other_order").unwrap().as_str(), "D");
    assert_eq!(super::msgtypes(&registry).count(), 2);
    // Once the first holder goes, the code answers the one left.
    registry.remove("NewOrderSingle").unwrap();
    assert_eq!(registry.msgtype("D").unwrap().name(), "OtherOrder");
    registry.remove("OtherOrder").unwrap();
    assert!(registry.get_msgtype("D").is_none());

    // A dictionary's message on a standard code is the same one namespace:
    // re-declared under the folded name it folds into the stored message,
    // under its own name it stands beside it, carrying its membership.
    let mut registry = catalog();
    let mut restated = StructType::from_fields([])
        .map(DataType::from)
        .unwrap()
        .required_field("new_order_single");
    restated.as_fix_mut().set_msgtype("D").unwrap();
    restated.as_fix_mut().set_branches(["venue"]).unwrap();
    assert!(!registry.add_field(restated).unwrap());
    assert_eq!(super::msgtypes(&registry).count(), 1);
    let folded = registry.msgtype("D").unwrap();
    assert_eq!(folded.name(), "NewOrderSingle");
    assert!(folded.as_field().as_fix().has_branch("venue"));
    assert_eq!(folded.get_group_by_tag(453).unwrap().name(), "Parties");
    let mut message = StructType::from_fields([])
        .map(DataType::from)
        .unwrap()
        .required_field("VenueOrder");
    message.as_fix_mut().set_msgtype("D").unwrap();
    message.as_fix_mut().set_branches(["venue"]).unwrap();
    registry.insert(message).unwrap();
    assert_eq!(super::msgtypes(&registry).count(), 2);
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
fn message_code_aliases_reindex_after_a_code_set_mutation() {
    let mut registry = catalog();
    registry
        .set_codeset(
            "msgtypecodeset",
            &[FixCode::new("Order", "D").with_aliases(["NOS"])],
        )
        .unwrap();
    let mut field = tagged("MsgType", 35, DataType::utf8());
    field.as_fix_mut().set_codeset("msgtypecodeset").unwrap();
    registry.insert(field).unwrap();
    assert_eq!(registry.msgtype("nos").unwrap().as_str(), "D");
    // The set tag 35 reads by is restated whole, the field untouched: the
    // alias it no longer lists stops answering and the one it now lists
    // answers in its place.
    registry
        .set_codeset(
            "msgtypecodeset",
            &[FixCode::new("Order", "D").with_aliases(["NewOrder"])],
        )
        .unwrap();
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
    let mut header = StructType::from_fields([member])
        .map(DataType::from)
        .unwrap()
        .required_field("Header");
    registry.insert(header.clone()).unwrap();
    header.as_fix_mut().set_component("Header").unwrap();
    let mut message = StructType::from_fields([header])
        .map(DataType::from)
        .unwrap()
        .required_field("EnumReport");
    message.as_fix_mut().set_msgtype("R").unwrap();
    registry.insert(message).unwrap();

    registry
        .set_codeset(
            "msgtypecodeset",
            &[FixCode::new("Order", "D").with_aliases(["NOS"])],
        )
        .unwrap();
    let mut coded = registry.field(35).unwrap().clone();
    coded.as_fix_mut().set_codeset("msgtypecodeset").unwrap();
    registry.update(coded).unwrap();
    assert_eq!(registry.msgtype("NOS").unwrap().as_str(), "D");
    let stated = registry
        .field_by_path(&fpath("EnumReport.Header.MsgType"))
        .unwrap();
    assert_eq!(
        registry.codeset_of(stated).unwrap().code_value("NOS"),
        Some("D")
    );

    // A set name nothing can file is refused where it is stated, and the
    // occurrences it reaches are left as they were.
    let before = registry.clone();
    let mut malformed = registry.field(35).unwrap().clone();
    malformed.update_metadata([("FIX:codeset", "[")]).unwrap();
    assert!(registry.insert(malformed).is_err());
    assert_eq!(registry, before);
    let mut changed = registry.field(35).unwrap().clone();
    changed.as_fix_mut().remove_codeset();
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
    uncoded.as_fix_mut().remove_codeset();
    registry.insert(uncoded).unwrap();
    assert!(registry.get_msgtype("NOS").is_none());
    let stated = registry
        .field_by_path(&fpath("EnumReport.Header.MsgType"))
        .unwrap();
    assert!(registry.codeset_of(stated).is_none());
}

#[test]
fn message_context_resolves_a_group_whose_global_counter_is_ambiguous() {
    let mut registry = catalog();
    let mut group = registry.field_by_name("Parties").unwrap().clone();
    group.set_name("TradeParties");
    registry.insert(group.clone()).unwrap();
    assert!(registry.get_field_by_counter(453).is_none());
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
    let outer = StructType::from_fields([group])
        .map(DataType::from)
        .unwrap()
        .required_field("Outer");
    let mut message = StructType::from_fields([outer])
        .map(DataType::from)
        .unwrap()
        .required_field("Trade");
    message.as_fix_mut().set_msgtype("T").unwrap();
    registry.insert(message).unwrap();
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
    let mut parties = registry.field_by_name("Parties").unwrap().clone();
    parties.as_fix_mut().set_group("Parties").unwrap();
    let hop = StructType::from_fields([parties.clone()])
        .map(DataType::from)
        .unwrap()
        .required_field("Hop");
    registry.insert(hop.clone()).unwrap();
    let mut hops = DataType::large_list(hop).nullable_field("Hops");
    hops.as_fix_mut().set_counter(627).unwrap();
    hops.as_fix_mut().set_component("Hop").unwrap();
    registry.insert(hops).unwrap();
    let mut hops = registry.field_by_name("Hops").unwrap().clone();
    hops.as_fix_mut().set_group("Hops").unwrap();
    let mut message = StructType::from_fields([hops])
        .map(DataType::from)
        .unwrap()
        .required_field("HopReport");
    message.as_fix_mut().set_msgtype("H").unwrap();
    registry.insert(message).unwrap();
    let message = registry.msgtype("H").unwrap();
    assert_eq!(message.get_group_by_tag(627).unwrap().name(), "Hops");
    assert_eq!(message.get_group_by_tag(453).unwrap().name(), "Parties");
    let mut duplicate = StructType::from_fields([
        StructType::from_fields([parties.clone()])
            .map(DataType::from)
            .unwrap()
            .required_field("Left"),
        StructType::from_fields([parties])
            .map(DataType::from)
            .unwrap()
            .required_field("Right"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("DuplicateContexts");
    duplicate.as_fix_mut().set_msgtype("R").unwrap();
    registry.insert(duplicate).unwrap();
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
    // A Struct stating no message type is a plain component:
    // it is accepted, and no code reaches it.
    let missing = DataType::from(StructType::from_fields([]).unwrap()).required_field("Missing");
    registry.insert(missing).unwrap();
    assert!(registry.get_msgtype("Missing").is_none());
    assert_eq!(super::msgtypes(&registry).count(), 0);
    let mut nullable = StructType::from_fields([])
        .map(DataType::from)
        .unwrap()
        .nullable_field("Nullable");
    nullable.as_fix_mut().set_msgtype("X").unwrap();
    assert!(registry.insert(nullable).is_err());
    let mut composite =
        DataType::from(StructType::from_fields([]).unwrap()).required_field("Report");
    composite
        .as_fix_mut()
        .set_msgtype("P Report Acknowledgement")
        .unwrap();
    registry.insert(composite).unwrap();
    assert_eq!(
        registry
            .msgtype("P Report Acknowledgement")
            .unwrap()
            .as_str(),
        "P Report Acknowledgement"
    );
    let mut refused =
        DataType::from(StructType::from_fields([]).unwrap()).required_field("Refused");
    assert!(refused.as_fix_mut().set_msgtype("A\nB").is_err());
}

#[test]
fn a_document_property_is_stored_as_the_json_it_is_and_read_back_as_its_text() {
    // A field's `FIX:codeset` names the set it reads by and is one text, so
    // the document property this is about is one of the four that still
    // hold an array: the rules restating a retired field.
    let mut floor = tagged("MaxFloor", 111, DataType::Float64);
    floor
        .as_fix_mut()
        .set_replacements(&[
            FixReplacement::new("select maxfloor as displayqty".parse().unwrap())
                .with_doc("MaxFloor became DisplayQty"),
            FixReplacement::new("select maxfloor as maxshow".parse().unwrap()),
        ])
        .unwrap();
    let canonical = floor.get_metadata("FIX:replacements").unwrap().to_owned();
    assert!(canonical.starts_with("[{"), "{canonical}");

    // The store writes the document rather than one escaped line, so the
    // file an operator opens renders the rules as rules.
    let document = yggdryl::into_fix_document(floor.clone()).unwrap();
    let rules = document
        .get_key_str("metadata")
        .and_then(|metadata| metadata.get_key_str("FIX:replacements"))
        .expect("the replacement rules");
    assert_eq!(rules.len(), 2);
    assert_eq!(
        rules
            .get(0)
            .as_deref()
            .and_then(|rule| rule.get_key_str("plan"))
            .and_then(Scalar::as_str),
        Some("select maxfloor as displayqty"),
    );
    // And the keys stay in the order the reader walks them, so the file
    // reads the way the document is written.
    assert_eq!(
        rules.get(0).as_deref().map(Scalar::keys),
        Some(vec!["plan", "doc"])
    );

    // Reading one back restates the canonical text, whatever order the file
    // spelled an entry's keys in.
    assert_eq!(yggdryl::from_fix_document(document).unwrap(), floor);
    let reordered = yggdryl::from_json_scalar(
        r#"{"name":"MaxFloor","dtype":{"type":"float64"},"nullable":true,"metadata":{
            "FIX:tag":"111",
            "FIX:replacements":[
                {"doc":"MaxFloor became DisplayQty","plan":"select maxfloor as displayqty"},
                {"plan":"select maxfloor as maxshow"}
            ]
        }}"#,
    )
    .unwrap();
    assert_eq!(
        yggdryl::from_fix_document(reordered)
            .unwrap()
            .get_metadata("FIX:replacements"),
        Some(canonical.as_str()),
    );
}

#[test]
fn a_document_property_the_store_cannot_read_is_refused_by_name() {
    // One shape: a property the file spells as text is not read as canonical
    // text, because the store writes the document itself.
    let text = yggdryl::from_json_scalar(
        r#"{"name":"MaxFloor","dtype":{"type":"float64"},"nullable":true,
            "metadata":{"FIX:tag":"111",
            "FIX:replacements":"[{\"plan\":\"select maxfloor as displayqty\"}]"}}"#,
    )
    .unwrap();
    let error = yggdryl::from_fix_document(text).expect_err("the escaped shape is not the shape");
    assert!(error.to_string().contains("FIX:replacements"), "{error}");

    // An entry stating a key the document does not declare is refused the
    // same way rather than dropped.
    let unknown = yggdryl::from_json_scalar(
        r#"{"name":"MaxFloor","dtype":{"type":"float64"},"nullable":true,
            "metadata":{"FIX:tag":"111",
            "FIX:replacements":[{"plan":"select maxfloor as displayqty","note":"x"}]}}"#,
    )
    .unwrap();
    let error = yggdryl::from_fix_document(unknown).expect_err("an undeclared key");
    assert!(error.to_string().contains("note"), "{error}");

    // And a field holding text no reader can parse is named where it is
    // written rather than copied out for a reader to refuse later.
    let mut broken = tagged("MaxFloor", 111, DataType::Float64);
    broken
        .set_metadata([("FIX:replacements", "not a document")])
        .unwrap();
    let error = yggdryl::into_fix_document(broken).expect_err("a malformed document");
    assert!(error.to_string().contains("FIX:replacements"), "{error}");
}

#[test]
fn the_names_and_tags_cross_a_store_as_the_arrays_they_are() {
    // The two list properties are dumped as JSON arrays, like the three
    // documents of entries, and read back as the compact text the setters
    // write - whatever spacing or integer width the file used.
    let mut qty = tagged("OrderQty", 38, DataType::Float64);
    qty.as_fix_mut().set_names(["Qty", "Quantity"]).unwrap();
    qty.as_fix_mut().set_tags(&[1088, 152]).unwrap();
    assert_eq!(qty.get_metadata("FIX:names"), Some(r#"["Qty","Quantity"]"#));
    assert_eq!(qty.get_metadata("FIX:tags"), Some("[1088,152]"));

    let document = yggdryl::into_fix_document(qty.clone()).unwrap();
    let metadata = document.get_key_str("metadata").expect("the metadata");
    let names = metadata.get_key_str("FIX:names").expect("the names");
    assert_eq!(
        names
            .as_sequence()
            .map(|held| held.iter().filter_map(Scalar::as_str).collect::<Vec<_>>()),
        Some(vec!["Qty", "Quantity"]),
    );
    let tags = metadata.get_key_str("FIX:tags").expect("the tags");
    assert_eq!(
        tags.as_sequence()
            .map(|held| held.iter().filter_map(Scalar::as_i64).collect::<Vec<_>>()),
        Some(vec![1088, 152]),
    );
    assert_eq!(yggdryl::from_fix_document(document).unwrap(), qty);

    // A file may space the arrays however it likes; the field holds one text.
    let spaced = yggdryl::from_json_scalar(
        r#"{"name":"OrderQty","dtype":{"type":"float64"},"nullable":true,"metadata":{
            "FIX:tag":"38",
            "FIX:names":[ "Qty" ,  "Quantity" ],
            "FIX:tags":[ 1088 , 152 ]
        }}"#,
    )
    .unwrap();
    assert_eq!(yggdryl::from_fix_document(spaced).unwrap(), qty);

    // One shape: text under either key is refused by name, and so is an
    // element that is not what the list holds - an empty or escaped name, a
    // tag that is not a positive integer.
    for (key, spelled) in [
        ("FIX:names", r#""Qty,Quantity""#),
        ("FIX:names", r#"["Qty",""]"#),
        ("FIX:names", r#"["Qty","Qu\"antity"]"#),
        ("FIX:names", r#"["Qty",152]"#),
        ("FIX:names", r#"["Qty","qty"]"#),
        ("FIX:names", r#"{"Qty":true}"#),
        ("FIX:tags", r#""1088,152""#),
        ("FIX:tags", r#"[1088,1088]"#),
        ("FIX:tags", r#"[1088,0]"#),
        ("FIX:tags", r#"[1088,-152]"#),
        ("FIX:tags", r#"[1088,2147483648]"#),
        ("FIX:tags", r#"[1088,"152"]"#),
        ("FIX:tags", r#"[1088,1.5]"#),
    ] {
        let document = yggdryl::from_json_scalar(format!(
            r#"{{"name":"OrderQty","dtype":{{"type":"float64"}},"nullable":true,
                "metadata":{{"FIX:tag":"38","{key}":{spelled}}}}}"#
        ))
        .unwrap();
        let error = yggdryl::from_fix_document(document).expect_err(&format!("{key}: {spelled}"));
        assert!(error.to_string().contains(key), "{key}: {spelled}: {error}");
    }

    // And a field holding text no reader can parse under either key is named
    // where it is written.
    for (key, stored) in [("FIX:names", "Qty,Quantity"), ("FIX:tags", "1088,152")] {
        let mut broken = tagged("OrderQty", 38, DataType::Float64);
        broken.set_metadata([(key, stored)]).unwrap();
        let error = yggdryl::into_fix_document(broken).expect_err("the comma text");
        assert!(error.to_string().contains(key), "{key}: {error}");
    }
}

#[test]
fn every_committed_field_document_round_trips_through_the_store_shape() {
    // The whole shipped dictionary, both directions: what the store writes
    // reads back as the same field, byte for byte in its metadata.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = FixRegistry::from_handle(&LocalFolder::new(root).unwrap()).unwrap();
    let mut carried = 0_usize;
    for field in &registry {
        let document = yggdryl::into_fix_document(field.clone()).unwrap();
        if document
            .get_key_str("metadata")
            .is_some_and(|metadata| metadata.get_key_str("FIX:codeset").is_some())
        {
            carried += 1;
        }
        assert_eq!(&yggdryl::from_fix_document(document).unwrap(), field);
    }
    assert!(carried > 400, "only {carried} fields carry a code set");
    for category in [FixCategory::Components, FixCategory::Groups] {
        for field in super::definitions(&registry, category) {
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
    let file = yggdryl::local::LocalFile::new(&path).unwrap();
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
    // Windows will not replace a file while this local backend still owns
    // its mapped view. The registry has consumed the document by this point.
    drop(file);

    // One mutation: a document that does not parse leaves it as it was.
    let before = registry.stable_hash();
    std::fs::write(&path, br#"{"fields":[],"components":[],"groups":"no"}"#).unwrap();
    let error = registry
        .add_json_file(&yggdryl::local::LocalFile::new(&path).unwrap())
        .expect_err("a category that is not an array");
    assert!(error.to_string().contains("venue.json"), "{error}");
    assert_eq!(registry.stable_hash(), before);

    std::fs::remove_dir_all(&root).unwrap();
}

// ---------------------------------------------------------------------------
// Moved out of `rust/src/fix/store.rs`, which is the file this one mirrors:
// the seeded clocks a stored dictionary replaces. `CLOCK_DATATYPE` is the one
// thing here a caller cannot name, so it is reached through
// `yggdryl::internals`.
// ---------------------------------------------------------------------------

#[cfg(feature = "internals")]
mod clock_seed_tests {
    use yggdryl::internals::fix_schema::clock_datatype;
    use yggdryl::{Error, FixRegistry};

    #[test]
    fn stored_clocks_replace_seeds_but_duplicate_documents_refuse() {
        let registry = FixRegistry::new();
        let mut sending = registry.field_by_tag(52).unwrap().clone();
        sending
            .set_description("store owns this declaration")
            .unwrap();
        let field = sending.clone().into_json().unwrap();
        let document = format!(r#"{{"fields":[{field}],"components":[],"groups":[]}}"#);
        let loaded = FixRegistry::from_json(&document).unwrap();
        assert_eq!(loaded.field_by_tag(52).unwrap(), &sending);
        assert_eq!(loaded.field_by_tag(60).unwrap().dtype(), &clock_datatype());
        assert_eq!(
            FixRegistry::from_json(&loaded.into_json().unwrap()).unwrap(),
            loaded
        );
        let duplicate = format!(r#"{{"fields":[{field},{field}],"components":[],"groups":[]}}"#);
        assert!(matches!(
            FixRegistry::from_json(&duplicate),
            Err(Error::Conflict { .. })
        ));
    }

    #[test]
    fn renamed_loaded_clock_keeps_its_canonical_identity() {
        let registry = FixRegistry::new();
        let sending = registry
            .field_by_tag(52)
            .unwrap()
            .clone()
            .with_name("VenueSendingClock");
        let field = sending.into_json().unwrap();
        let document = format!(r#"{{"fields":[{field}],"components":[],"groups":[]}}"#);
        let loaded = FixRegistry::from_json(&document).unwrap();
        assert_eq!(loaded.field_by_tag(52).unwrap().name(), "VenueSendingClock");
        assert!(loaded.get_field_by_name("SendingTime").is_none());
        assert_eq!(loaded.field_by_tag(60).unwrap().name(), "transacttime");
    }
}

mod committed {
    use std::collections::BTreeSet;

    use yggdryl::local::LocalFolder;
    use yggdryl::{
        DataType, Field, FixCategory, FixRegistry, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS,
        Scalar, TimeUnit, Timezone,
    };

    fn seed() -> FixRegistry {
        super::committed_registry().as_ref().clone()
    }

    /// The registry's fields of one category: a definition is filed by the
    /// shape it has - a Struct is a component, a List or a Map a group - and
    /// everything else is a wire field.
    fn definitions(registry: &FixRegistry, category: FixCategory) -> impl Iterator<Item = &Field> {
        registry
            .iter()
            .filter(move |field| category_of(field) == category)
    }

    fn category_of(field: &Field) -> FixCategory {
        match field.dtype() {
            DataType::Struct(_) => FixCategory::Components,
            dtype if dtype.is_nested() => FixCategory::Groups,
            _ => FixCategory::Fields,
        }
    }

    #[test]
    fn the_committed_dictionary_answers_the_worked_case_end_to_end() {
        let registry = seed();
        assert!(
            registry.len() > 5_000,
            "a whole registry, got {}",
            registry.len()
        );

        // Tag 32: `LastShares` typed `int` in 4.0, `LastShares` typed `Qty` from
        // 4.2, and `LastQty` from 4.3 on.
        let last_qty = registry.field_by_tag(32).expect("tag 32");
        assert_eq!(last_qty.name(), "lastqty");
        assert_eq!(last_qty.dtype(), &DataType::DECIMAL);
        assert_eq!(last_qty.as_metadata().get("display"), Some("LastQty"));

        // The dictionary holds one reading of the tag, under one name and one
        // datatype; the spellings earlier versions used reach it as aliases.
        let view = last_qty.as_fix();
        assert_eq!(view.names().collect::<Vec<_>>(), ["lastshares"]);

        // A query by either spelling answers the same field, and the
        // specification's own casing still resolves.
        for spelling in ["lastqty", "LastQty", "LastShares", "lastshares"] {
            assert_eq!(
                registry.field(spelling).expect(spelling).name(),
                "lastqty",
                "{spelling}"
            );
        }
    }

    #[test]
    fn the_committed_dictionary_is_no_dialects_member_and_a_field_is_its_tag_and_its_name() {
        let registry = seed();
        // The shipped dictionary never declared a branch, so nothing it holds
        // carries a membership and it lists no dialect.
        assert!(registry.dialects().is_empty(), "{:?}", registry.dialects());
        for category in FixCategory::ALL {
            for field in definitions(&registry, category) {
                assert_eq!(
                    field.as_fix().branches().count(),
                    0,
                    "{category}/{} carries a membership",
                    field.name()
                );
            }
        }
        // A field's identity is derived from its tag and its name, never stored:
        // the id every read answers is the one the pair hashes to, under the fold
        // every name lookup already applies.
        let msgtype = registry.field_by_tag(35).expect("tag 35");
        let id = msgtype.as_fix().id().unwrap().expect("an identity");
        assert_eq!(id, yggdryl::FixId::of(35, "MsgType").unwrap());
        assert_eq!(id, yggdryl::FixId::of(35, "Msg_Type").unwrap());
        assert_eq!(id, yggdryl::FixId::of(35, "msgtype").unwrap());
        assert_ne!(id, yggdryl::FixId::of(36, "MsgType").unwrap());
        assert_ne!(id, yggdryl::FixId::of(35, "MsgSeqNum").unwrap());
        assert_eq!(registry.field_by_id(id).unwrap().name(), "msgtype");
        assert!(
            msgtype.as_metadata().get("FIX:id").is_none(),
            "an id is derived, never stored"
        );
    }

    #[test]
    fn every_generated_name_is_folded_and_no_two_collide() {
        let registry = seed();
        let scalar_names: BTreeSet<_> = definitions(&registry, FixCategory::Fields)
            .map(Field::name)
            .collect();
        // Across every category, not within one: a derived tag is the definition's
        // identity in the whole catalog.
        let mut derived_tags = BTreeSet::new();
        for category in FixCategory::ALL {
            let mut seen = BTreeSet::new();
            for field in definitions(&registry, category) {
                let name = field.name();
                assert!(
                    !name.bytes().any(|byte| byte.is_ascii_uppercase()),
                    "{name} holds an uppercase byte"
                );
                assert!(!name.contains('_'), "{name} holds an underscore");
                assert!(seen.insert(name), "duplicate {category}/{name}");
                if category != FixCategory::Fields {
                    assert!(
                        !scalar_names.contains(name),
                        "{category}/{name} collides with a wire field"
                    );
                    // Every named definition carries a tag of its own, derived
                    // into the block nothing published claims, and no two share
                    // one - the `seen` set below proves the names, this the tags.
                    let derived = field
                        .as_fix()
                        .tag()
                        .expect("valid tag")
                        .expect("a derived definition tag");
                    assert!(
                        yggdryl::FixId::is_definition_tag(derived)
                            || yggdryl::is_crate_tag(derived),
                        "{category}/{name} tag {derived}"
                    );
                    // A crate tag on a named definition means the definition is
                    // one of this crate's own columns, reached by the tag the
                    // fixed row files it under: `identifiers` and `metadata` are
                    // the Maps whose counter is that tag.
                    if yggdryl::is_crate_tag(derived) {
                        assert!(
                            yggdryl::fix_crate_fields()
                                .expect("the crate's own fields")
                                .iter()
                                .any(|own| own.name() == name
                                    && own.as_fix().tag().unwrap() == Some(derived)),
                            "{category}/{name} holds crate tag {derived} without being one"
                        );
                    }
                    assert!(
                        derived_tags.insert(derived),
                        "{category}/{name} repeats derived tag {derived}"
                    );
                } else {
                    assert!(!field.dtype().is_nested(), "wire field {name} is nested");
                }
            }
            assert!(!seen.is_empty(), "the {category} category is missing");
        }
    }

    #[test]
    fn the_standard_declares_its_code_sets_and_the_generator_honours_them() {
        let registry = seed();

        // Each field names the set its values are drawn from, over the scalar
        // datatype the generator gave it. Message codes remain unrestricted
        // text; Side keeps its generic ASCII datatype.
        let msgtype = registry.field_by_tag(35).expect("tag 35");
        assert_eq!(msgtype.dtype(), &DataType::utf8());
        let codes = registry.codeset_of(msgtype).expect("the MsgType code set");
        assert_eq!(codes.name(), "msgtypecodeset");
        assert_eq!(codes.code_name("D"), Some("NewOrderSingle"));
        assert_eq!(codes.code_value("NewOrderSingle"), Some("D"));

        let side = registry.field_by_tag(54).expect("tag 54");
        assert_eq!(side.dtype(), &DataType::Side);
        let codes = registry.codeset_of(side).expect("the Side code set");
        assert_eq!(codes.name(), "sidecodeset");
        assert_eq!(codes.code_name("1"), Some("Buy"));
        assert_eq!(codes.code_value("buy"), Some("1"));
        // The order's state is declared twice, as `OrdStatus` and as `ExecType`,
        // each naming a code set of its own; only the crate's `state` column
        // is typed as a state, and it reads either.
        for tag in [39, 150] {
            let state = registry.field_by_tag(tag).expect("a state tag");
            assert_eq!(state.dtype(), &DataType::utf8(), "tag {tag}");
            let codes = registry
                .codeset_of(state)
                .unwrap_or_else(|| panic!("tag {tag}"));
            assert!(codes.codes().count() > 5, "tag {tag}");
        }
        assert_eq!(
            registry
                .codeset_of(registry.field_by_tag(39).unwrap())
                .unwrap()
                .code_name("1"),
            Some("PartiallyFilled")
        );
        // Every other code set keeps its base type, and one code set two fields
        // declare is held once under the one name both of them name.
        let ord_type = registry.field_by_tag(40).expect("tag 40");
        assert_eq!(ord_type.dtype(), &DataType::utf8());
        assert!(registry.codeset_of(ord_type).unwrap().codes().count() > 5);
        let source = registry.field_by_tag(22).expect("SecurityIDSource");
        let alternative = registry.field_by_tag(456).expect("SecurityAltIDSource");
        assert_eq!(
            source.as_metadata().get("FIX:codeset"),
            alternative.as_metadata().get("FIX:codeset")
        );
        assert_eq!(
            source.as_metadata().get("FIX:codeset"),
            Some("securityidsourcecodeset")
        );
        assert_eq!(
            registry.codeset_of(source),
            registry.codeset_of(alternative)
        );
        assert!(source.as_metadata().get("FIX:codes").is_none());

        // A price, a quantity, a price offset and an amount are exact numbers,
        // at the one decimal width this crate keeps them at; a percentage and
        // FIX's own `float` stay the floating count the specification names.
        for tag in [31, 38, 44, 6] {
            let field = registry.field_by_tag(tag).expect("an exact-number tag");
            assert_eq!(field.dtype(), &DataType::DECIMAL, "tag {tag}");
        }
        for tag in [155, 231, 211] {
            let field = registry.field_by_tag(tag).expect("a float tag");
            assert_eq!(field.dtype(), &DataType::Float64, "tag {tag}");
        }
        // And the ones it types otherwise keep those types.
        assert_eq!(registry.field_by_tag(34).unwrap().dtype(), &DataType::Int64);
        assert_eq!(
            registry.field_by_tag(10).unwrap().dtype(),
            &DataType::utf8()
        );
        assert_eq!(registry.field_by_tag(9).unwrap().dtype(), &DataType::Int32);
    }

    #[test]
    fn a_repeating_group_has_a_scalar_counter_and_a_separately_named_component() {
        let registry = seed();
        let counter = registry.field_by_tag(453).expect("NoPartyIDs");
        assert_eq!(counter.name(), "nopartyids");
        assert_eq!(counter.dtype(), &DataType::Int32);
        let parties = registry.field_by_name("Parties").expect("Parties group");
        assert_eq!(parties.name(), "parties");
        assert_eq!(parties.display(), Some("Parties"));
        assert_eq!(parties.as_fix().counter().unwrap(), Some(453));
        // The counter it heads is the published 453; its own identity is derived,
        // and the two are never the same number.
        let derived = parties.as_fix().tag().unwrap().expect("a derived tag");
        assert!(yggdryl::FixId::is_definition_tag(derived), "{derived}");
        assert_ne!(derived, 453);
        let DataType::List(item) = parties.dtype() else {
            panic!("a list, got {}", parties.dtype());
        };
        assert_eq!(item.name(), "party");
        assert_eq!(item.as_fix().component(), Some("party"));
        assert!(!item.is_nullable());
        let component = registry.field_by_name("Party").expect("Party component");
        assert_eq!(component.dtype(), item.dtype());
        let members: Vec<&str> = item
            .dtype()
            .as_fields()
            .expect("a struct item")
            .iter()
            .map(yggdryl::Field::name)
            .collect();
        assert!(members.contains(&"partyid"), "{members:?}");
        assert!(members.contains(&"partyrole"), "{members:?}");
        assert!(!members.contains(&"nopartyids"), "{members:?}");
        assert_eq!(
            parties.get_field_by_path("party.partyid").map(Field::name),
            Some("partyid")
        );
    }

    #[test]
    fn every_header_and_trailer_tag_resolves_in_the_generated_dictionary() {
        let registry = seed();
        for tag in STANDARD_HEADER_TAGS {
            assert!(registry.contains(tag), "header tag {tag} does not resolve");
        }
        for tag in STANDARD_TRAILER_TAGS {
            assert!(registry.contains(tag), "trailer tag {tag} does not resolve");
        }
        // The union across versions, not FIX Latest alone: Latest no longer lists
        // `SecureDataLen(90)` in the header nor `SignatureLength(93)` in the
        // trailer, and a 4.2 message carries both.
        assert!(
            STANDARD_HEADER_TAGS.contains(&90),
            "{STANDARD_HEADER_TAGS:?}"
        );
        assert!(
            STANDARD_TRAILER_TAGS.contains(&93),
            "{STANDARD_TRAILER_TAGS:?}"
        );
        assert_eq!(STANDARD_HEADER_TAGS[..3], [8, 9, 35]);
        assert_eq!(STANDARD_TRAILER_TAGS.last(), Some(&10));
    }

    #[test]
    fn the_dictionary_writes_back_byte_identically() {
        let registry = seed();
        let scratch = std::env::temp_dir().join("yggdryl-fix-roundtrip");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("a scratch folder");
        let mut folder = LocalFolder::new(scratch.clone()).expect("a local folder");
        registry.commit(&mut folder).expect("the dictionary writes");

        let written = FixRegistry::from_handle(&LocalFolder::new(scratch.clone()).unwrap())
            .expect("what was written loads");
        assert_eq!(written.len(), registry.len());
        assert_eq!(written, registry);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn every_stored_document_walks_to_its_end() {
        let registry = seed();
        let mut with_codes = 0_usize;
        let mut codes = 0_usize;
        for field in registry.iter() {
            // A refusal ends a borrowed walk, and the walk is what resolution
            // reads - so one malformed record does not fail loudly, it silently
            // removes every record after it. Orchestra's `addedEP="-1"` did
            // exactly that to 169 code sets: `ClearingFirm` stopped resolving
            // because a code 60 records earlier would not parse. Nothing here can
            // catch that but walking every document to its end.
            let mut seen = 0_usize;
            if let Some(set) = registry.codeset_of(field) {
                for code in set.codes() {
                    code.unwrap_or_else(|error| panic!("{}: {error}", field.name()));
                    seen += 1;
                }
            }
            if seen > 0 {
                with_codes += 1;
            }
            codes += seen;
        }
        // And every set the store holds, whether or not a field reads by it:
        // the documents live in `codesets/` now, one file per name.
        for set in registry.codesets() {
            for code in set.codes() {
                code.unwrap_or_else(|error| panic!("{}: {error}", set.name()));
            }
        }
        for field in registry.iter() {
            assert!(field.as_metadata().get("FIX:codes").is_none());
            assert!(field.as_metadata().get("FIX:lineage").is_none());
        }
        // A dictionary this size is the point: a truncation that hides one code
        // in twenty thousand is exactly what nobody notices by reading.
        assert!(with_codes > 900, "{with_codes} fields name a code set");
        assert!(
            codes > 10_000,
            "{codes} enum records reached through the fields that name them"
        );
        // The spelling the truncation hid, end to end.
        let role = registry.field_by_tag(452).expect("PartyRole");
        let set = registry.codeset_of(role).expect("the PartyRole code set");
        assert_eq!(set.code_value("ClearingFirm"), Some("4"));
        assert_eq!(set.code_name("4"), Some("ClearingFirm"));
    }

    /// Every field the dictionary holds, occurrences and group members included.
    fn every_field(registry: &FixRegistry) -> Vec<Field> {
        fn walk(field: &Field, out: &mut Vec<Field>) {
            out.push(field.clone());
            match field.dtype() {
                DataType::List(item) | DataType::LargeList(item) => walk(item, out),
                DataType::Struct(fields) => {
                    for held in fields.iter() {
                        walk(held, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for field in registry.iter() {
            walk(field, &mut out);
        }
        out
    }

    #[test]
    fn every_date_is_an_instant_and_every_zone_is_the_one_its_name_states() {
        let registry = seed();
        // Nothing anywhere is a day or a second, occurrences and group members
        // included: a FIX date is that day's midnight, so a capture joining a
        // settlement date to a transact time compares them without a cast, and
        // `LocalMktTime` and `UTCTimeOnly` are one type for the same reason.
        for field in every_field(&registry) {
            match field.dtype() {
                DataType::Date32 | DataType::Date64 => {
                    panic!("{} is still a day rather than an instant", field.name())
                }
                DataType::Time32(_) => {
                    panic!("{} is still typed to a second", field.name())
                }
                DataType::Time64(unit) => {
                    assert_eq!(*unit, TimeUnit::Nanosecond, "{}", field.name())
                }
                DataType::DateTime64 { unit, timezone } => {
                    assert_eq!(*unit, TimeUnit::Nanosecond, "{}", field.name());
                    // Two zones, and only two: what the datatype's own name says.
                    // A `UTCTimestamp` is UTC and a `LocalMktDate` states no zone,
                    // so neither reads as the other.
                    assert!(
                        *timezone == Timezone::UTC || timezone.is_naive(),
                        "{} states {timezone}",
                        field.name()
                    );
                }
                _ => {}
            }
        }

        // The registry's own entries, where each field is counted once.
        let clock = DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        };
        let mut times = 0_usize;
        let mut naive = 0_usize;
        let mut utc = 0_usize;
        for field in registry.iter() {
            match field.dtype() {
                DataType::Time64(_) => times += 1,
                DataType::DateTime64 { timezone, .. } if timezone.is_naive() => naive += 1,
                DataType::DateTime64 { .. } => utc += 1,
                _ => {}
            }
        }
        assert_eq!(times, 57, "zone-less times of day");
        assert_eq!(naive, 369, "local values, stating no zone");
        // Sixty-eight shipped fields, plus the crate's eight clocks: `currunix`,
        // `creaunix`, `prevunix`, `snapunix`, `execunix`, `recdunix`,
        // `refrecdunix` and `exprtime`.
        let crated = registry
            .iter()
            .filter(|field| {
                field.dtype() == &clock
                    && field
                        .as_fix()
                        .tag()
                        .ok()
                        .flatten()
                        .is_some_and(yggdryl::is_crate_tag)
            })
            .count();
        assert_eq!(crated, 8, "the crate's own clocks");
        assert_eq!(utc, 68 + crated, "instants stated in UTC");
    }

    /// A field FIX Latest removed is still the dictionary's, marked so: a
    /// message stating it is read, and its value restated to the field that
    /// replaced it, so a capture of every version lands in one row.
    #[test]
    fn a_removed_field_is_kept_and_marked_deprecated() {
        let registry = seed();
        // `MaxFloor(111)` went in 5.0. Its canonical metadata records the one
        // specification replacement, so every version restates it as DisplayQty.
        let floor = registry.field_by_tag(111).expect("MaxFloor");
        assert_eq!(floor.as_fix().deprecated(), Some("5.0"), "{floor:?}");
        let mut replacements = floor.as_fix().replacements();
        assert_eq!(
            replacements
                .next()
                .expect("one rule")
                .expect("a valid rule")
                .plan(),
            "select maxfloor as displayqty"
        );
        assert!(replacements.next().is_none(), "one canonical replacement");
        // `Signature(89)` went with it and nothing replaces it: the mark stands
        // on its own.
        let signature = registry.field_by_tag(89).expect("Signature");
        assert_eq!(signature.as_fix().deprecated(), Some("5.0"));
        assert!(signature.as_fix().replacements().next().is_none());
        assert!(
            registry
                .field_by_tag(11)
                .unwrap()
                .as_fix()
                .deprecated()
                .is_none(),
            "ClOrdID stands"
        );
        let deprecated = definitions(&registry, FixCategory::Fields)
            .filter(|field| field.as_fix().deprecated().is_some())
            .count();
        assert_eq!(deprecated, 56);
    }

    /// A component's member is a reference to the field or group it holds,
    /// and it names the tag beside the name, so a reader resolves the member
    /// by identity without the shard in hand.
    #[test]
    fn a_member_reference_carries_the_field_and_its_tag() {
        let registry = seed();
        let party = registry.field_by_name("Party").expect("Party component");
        let partyid = party
            .get_field_by_path("partyid")
            .expect("the PartyID member");
        assert_eq!(partyid.as_fix().field_ref(), Some("partyid"));
        assert_eq!(partyid.as_fix().tag().unwrap(), Some(448));
        let subgroup = party
            .get_field_by_path("partysubids")
            .expect("the sub-party group member");
        assert_eq!(subgroup.as_fix().group(), Some("partysubids"));
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
        let document = std::fs::read_to_string(root.join("components/party.json")).unwrap();
        let stored = yggdryl::from_json_scalar(&document).unwrap();
        let member = stored
            .get_key_str("dtype")
            .and_then(|dtype| dtype.get_key_str("fields"))
            .and_then(Scalar::as_sequence)
            .and_then(|members| members.first())
            .and_then(|member| member.get_key_str("metadata"))
            .expect("the first member's metadata");
        assert_eq!(
            member.get_key_str("FIX:field").and_then(Scalar::as_str),
            Some("partyid")
        );
        assert_eq!(
            member.get_key_str("FIX:tag").and_then(Scalar::as_str),
            Some("448")
        );
        // A field's shard is its tag over a hundred, named nine digits wide:
        // tag 55 sits in the first, tag 50000 in the five-hundredth.
        assert!(root.join("fields/000000000.json").exists());
        assert!(root.join("fields/000000500.json").exists());
        assert!(!root.join("fields/0.json").exists(), "the old shard name");
    }

    /// The committed dictionary's hash, pinned as a literal.
    ///
    /// The registry hash walks scalar fields, then the components and the groups
    /// in name order, so a change to that walk or to any shipped document moves
    /// this number on purpose, in the commit that says why. It last moved when
    /// every protocol key took its scheme upper case - `FIX:tag`, `FIX:codeset`
    /// and `FIX:branches` beside `ARROW:extension:name` and `PARQUET:field_id` -
    /// so every stored key the dictionary hashes changed its spelling. It last
    /// moved when
    /// the capture's clock stopped being a crate field: `recordedat` (65028) is
    /// retired, a message's instant being what it states - `SendingTime(52)` and
    /// the settled `currunix` - and the plugin that logged a line is
    /// `msgpluginid` (`MsgPluginId`, tag 65009 unchanged), spelled like
    /// `msgctxid` and `msgsessionid`, and the instant a message was created is
    /// `creaunix` (`CreaUnix`, tag 65023 unchanged), spelled like `currunix`,
    /// `prevunix` and `snapunix`. It last moved when
    /// the message became a typed market event: the crate's columns are the
    /// event's facts - `currunix`, `creaunix`, `currhashcode`, `crosshashcode`,
    /// `crosscode`, the identities as UUIDs, the lanes, the two Map groups
    /// `identifiers` and `metadata` - the shards are named nine digits wide,
    /// every member reference carries its `FIX:tag`, and a field FIX Latest
    /// removed is marked `FIX:deprecated`. It last moved when the market numbers
    /// merged onto the event: `Price(44)`, `OrderQty(38)` and `Quantity(53)` stop
    /// being columns of their own and the crate's `px` and `qty` answer for them,
    /// and `prevpx`, `prevqty`, `tradable` and `symbolticker` join the block. It
    /// last moved when the capture's own columns stopped being facts of a
    /// message: `recordedat` states no `FIX:derivation`, because when a capture
    /// wrote a line down is whoever read it to say and never the message's own
    /// `SendingTime`. It last moved when the event's instant and its own digest
    /// took the names their columns carry: `unix` became `currunix` and
    /// `hashcode` became `currhashcode`, so the crate's tags 65003 and 65017
    /// read as the current instant and the current code next to `crosshashcode`.
    /// It last moved when every FIX quantity, price, price offset and amount
    /// became an exact number: 478 fields are `decimal128(38, 18)` where they
    /// were `float64`, a percentage and FIX's own `float` stay floating, and
    /// the seven derivations that multiply or add across the two state each
    /// operand's exact scale. It last moved when the crate stopped owning a
    /// market column: eighteen of its thirty-eight definitions are gone - the
    /// price, the quantity, the unit, the instrument's codes, the market, the
    /// state, the lanes' currencies and units, what a price moved from, whether
    /// it could trade and the ticker - because every one of them restated a FIX
    /// field the traits now read, and the two rules that read `isincode` read
    /// `SecurityID(48)` under its source and the `SecurityAltID` group instead.
    /// It last moved when the nested datatypes became families: `DataType` derives
    /// its hash, so a family variant contributes its leaf's discriminant too, and
    /// every sequence, struct and mapping in the dictionary hashes one level
    /// deeper than it did. `identifiers` and `metadata` also declare sorted keys, so they are
    /// `sorted_map` rather than `map` beside a flag, and their entries the struct
    /// of two children every mapping's are. It
    /// last moved when a field became an enum over its leaves and the dictionary
    /// sidecar moved onto the one leaf that has it: a field hashes the datatype
    /// its leaf holds rather than the `DataType` it widens to, and it hashes one
    /// sidecar - nothing at all for a field that is not dictionary-encoded -
    /// rather than an identifier and a flag every field carried. It last moved
    /// when the four decimal widths became one `Decimal` family variant: every
    /// datatype declared after them in the enum shifted by three discriminants,
    /// and a decimal hashes its leaf one level deeper. It last moved when uuid
    /// became a family: a uuid column hashes the leaf that says which RFC 9562
    /// versions it admits, where it used to hash a variant with nothing in it. It
    /// last moved when the byte family became six leaves: a byte column hashes
    /// one leaf that already carries its count, where it used to hash a layout
    /// and an optional bound beside it. It last moved when the two sets of moves
    /// above met: the families hash the dictionary main settled on, so neither
    /// side's pinned number survives the merge and this one is what the merged
    /// tree answers. It last moved when the string family became eighteen
    /// leaves: a string column hashes one leaf that already carries its charset
    /// and its count, where it used to hash a layout, a charset and an optional
    /// bound beside them. It last moved when the eight temporal variants became
    /// five families: a temporal column hashes its family's leaf one level
    /// deeper, where it used to hash a variant of its own. It last moved when
    /// uuid became one parameter-free datatype again: a uuid column hashes a
    /// variant with nothing in it, where it used to hash the leaf that said which
    /// RFC 9562 versions it admitted. It last moved when the crate's own block
    /// gained `srcuuids`: a twentieth definition, the list of the elements a
    /// message was read from, hashes beside the nineteen and as a member of the
    /// fixed row. It last moved when the datatype identifiers were laid out by
    /// family and a mapping's entries became the plain struct of two children:
    /// every datatype tag in the hash is the family-laid byte, and `identifiers`
    /// and `metadata` hash their entries as a struct rather than a leaf of their
    /// own. It last moved when the crate's own block gained `state` and
    /// `exprtime`: the two lifecycle facts a walk folds forward hash as a
    /// twenty-first and a twenty-second definition, each at its event column's
    /// datatype - a ranked state and a nanosecond clock - and as members of the
    /// fixed row, the state ahead of `OrdStatus` and the expiry beside the clocks.
    /// It last moved when what the specification retired became the crate's own
    /// table: the thirty-seven fields that carried a `FIX:replacements` document
    /// hash without it, one metadata entry fewer each, and the dictionary states
    /// no rule of its own.
    /// It last moved when `sourceurl` left the fixed row: the object a line was
    /// read from is the reader's word about the line and not the message's about
    /// itself, so it travels as one of the capture's own columns beside the row,
    /// the definition still hashes as a crate field of its own, and the fixed
    /// row hashes one member fewer. It last moved when those two settled changes
    /// met in this merge: the thirty-seven retired `FIX:replacements` entries are
    /// absent and `sourceurl` remains a crate field while leaving the fixed row,
    /// so their combined dictionary is the value pinned here. It moved again when
    /// a field's vocabulary became a named code set: `FIX:codeset` now names the
    /// set, and the dictionary hashes every named set once beside its fields.
    /// It also moved when expiry became `exprtime`, the 181 message components
    /// gained `FIX:msgcat`, and MsgCat plus five normalized identifier definitions
    /// joined the fixed row. CFI keeps standard tag 461; the expiry description
    /// now states that a newer explicit deadline replaces the preceding one.
    /// It last moved when the merged store centralized 735 published vocabularies
    /// and MsgCat's crate vocabulary under `FIX:codeset`, including the shared
    /// PartyIDSource `proprietary/customcode` alias. FIGICode adds one crate
    /// definition and one fixed-row member; `nofixentries` now describes the
    /// residual count rather than the complete in-memory content.
    /// It last moved when the identifiers group described the synthesized
    /// complete `msgsesseventid` delivery key beside the ordinary identifiers.
    /// It moved when FIX's `SecAltIDGrp` took the canonical group name
    /// `secaltids` and the crate added the `execunix` and `recdunix` fields; those
    /// definitions, references, derivations and generated tags are now part of the
    /// committed registry hash.
    /// It moved again when the proprietary `CreationTime` spelling became the
    /// indexed alias of the crate's `creaunix` field.
    /// It moved when `refrecdunix` persisted the latest recording clock used to
    /// select a reference across repeated generic merges.
    /// It moved when `RegulatoryTradeIDGrp` took the canonical collection name
    /// `regulatorytradeids` and joined the fixed row under counter 1907.
    /// It moved when 421 unambiguous standard groups took semantic plural names
    /// and displays, while the other 103 retained an explicit `Grp` suffix.
    /// It moved when the crate identifier document replaced its partial two-part
    /// capture key with the complete length-prefixed `msgsesseventid` key.
    /// It moved when `curruuid` said its instant is a microsecond one: the
    /// UUIDv7 a message derives carries the microsecond within the
    /// millisecond in `rand_a`, so that one definition's description hashes
    /// differently and no other document in the store does.
    /// It last moved when that identity stopped naming a sequence and a cross
    /// seed: the code is stored whole in the identifier rather than rehashed
    /// with them, and since the code already holds both, the same one
    /// description is shorter by what it no longer has to say.
    /// It last moved when the crate's event columns stopped restating what
    /// `EventColumn` already owns: ten of the nineteen add nothing FIX's own
    /// and now take the column's display and wording, so a text line's batch
    /// and a FIX row describe one column with one sentence - and `curruuid`'s
    /// microsecond wording above reaches the crate field through that column
    /// rather than through a second copy of the sentence. Eleven descriptions
    /// moved, those ten and `recdunix`'s own wording beside them - no
    /// definition, reference, tag or count did, which is why the census below
    /// stands unchanged.
    /// It moved when MsgCat became the intrinsic int32 market-operation ID
    /// vocabulary and parent UUIDs became a sorted unique set: the one code
    /// set now hashes numeric values, and those two crate-field definitions
    /// hash their current datatypes and descriptions.
    /// It last moved when `parentuuids` left the crate: a message names its
    /// predecessor by `prevuuid` alone, so the crate field at 65041 and its
    /// member of the fixed row are gone and nothing else moved.
    #[test]
    fn the_committed_dictionary_hashes_to_one_pinned_value() {
        let registry = seed();
        assert_eq!(registry.stable_hash(), 12_241_597_752_383_919_107);
        let messages = definitions(&registry, FixCategory::Components)
            .filter(|component| component.as_fix().msgtype().is_some())
            .count();
        assert_eq!(messages, 181);
        assert_eq!(
            definitions(&registry, FixCategory::Components).count(),
            928 + super::crated_components()
        );
        // Every group the store ships resolves, the crate's two Map groups
        // among them: the census is the directory rather than a literal, so a
        // generated group added or dropped moves nothing here.
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix/groups");
        for entry in std::fs::read_dir(root).expect("the generated group directory") {
            let path = entry.expect("a group document").path();
            let name = path
                .file_stem()
                .expect("a group name")
                .to_str()
                .expect("ASCII");
            assert!(
                definitions(&registry, FixCategory::Groups).any(|group| group.name() == name),
                "{name} resolves as a group"
            );
        }
        assert_eq!(
            registry.len(),
            definitions(&registry, FixCategory::Fields).count()
                + definitions(&registry, FixCategory::Components).count()
                + definitions(&registry, FixCategory::Groups).count()
        );
    }
}

/// A commit states every document the first time and none the second.
///
/// This is what makes a dump replayable: the bytes settle in one pass, and a
/// commit run again over the same registry is a read of each document and a
/// write of nothing. A store that rewrote every document each time would churn
/// two thousand files to change one, which is the cost the comparison buys off.
#[test]
fn committing_twice_writes_the_store_once() {
    let registry = committed_registry();
    let scratch = scratch("commit-twice");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut folder = LocalFolder::new(scratch.clone()).unwrap();

    let first = registry.commit(&mut folder).unwrap();
    assert!(
        !first.written.is_empty(),
        "an empty root takes every document"
    );
    assert_eq!(first.skipped, 0, "nothing is there to skip yet");
    assert!(
        first.removed.is_empty(),
        "an empty root holds nothing to remove"
    );
    assert!(
        !first.is_clean(),
        "writing the whole store is not a clean commit"
    );

    let second = registry.commit(&mut folder).unwrap();
    assert!(
        second.is_clean(),
        "a second commit moved {:?} and removed {:?}",
        second.written,
        second.removed,
    );
    assert_eq!(
        second.skipped,
        first.written.len(),
        "every document the first commit wrote is skipped by the second",
    );
    assert_eq!(first.len(), second.len(), "both commits consider one store");
    std::fs::remove_dir_all(&scratch).ok();
}

/// One changed field is one written document, and the rest are left alone.
///
/// The point of comparing before writing: a caller loads a dictionary, merges
/// a dialect into it and commits, and the report names the documents that
/// actually moved rather than the whole store.
#[test]
fn a_commit_writes_the_documents_a_change_reaches_and_no_others() {
    let mut registry = (*committed_registry()).clone();
    let scratch = scratch("commit-delta");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut folder = LocalFolder::new(scratch.clone()).unwrap();
    let settled = registry.commit(&mut folder).unwrap();
    assert!(registry.commit(&mut folder).unwrap().is_clean());

    // One definition the store did not hold, filed on the shard its tag picks:
    // one scalar moves, so one shard document moves and nothing else does.
    let mut field = DataType::Int32.nullable_field("commitprobe");
    field.as_fix_mut().set_tag(9_999).unwrap();
    field
        .as_fix_mut()
        .set_description("What a commit reaches, and nothing beside it.")
        .unwrap();
    assert!(registry.add_field(field).unwrap(), "the probe is new");

    let moved = registry.commit(&mut folder).unwrap();
    assert!(moved.removed.is_empty(), "a restated field removes nothing");
    assert_eq!(
        moved.written.len(),
        1,
        "one changed field is one document, not {:?}",
        moved.written,
    );
    assert!(
        moved.written[0].starts_with("fields/"),
        "a scalar lives on a field shard, not at {}",
        moved.written[0],
    );
    assert_eq!(
        moved.skipped + moved.written.len(),
        moved.len(),
        "a commit considers every document it states",
    );
    assert!(
        moved.skipped >= settled.written.len() - 1,
        "every document the probe did not reach is left where it lies",
    );
    assert!(registry.commit(&mut folder).unwrap().is_clean());
    std::fs::remove_dir_all(&scratch).ok();
}

/// A store read back and committed again writes nothing.
///
/// The round trip is the replay: load the documents a commit wrote, and the
/// registry they rebuild states the same bytes. A dump whose ordering or
/// rendering depended on how the registry was built would write here.
#[test]
fn a_store_read_back_commits_clean() {
    let registry = committed_registry();
    let scratch = scratch("commit-replay");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut folder = LocalFolder::new(scratch.clone()).unwrap();
    registry.commit(&mut folder).unwrap();

    let loaded = FixRegistry::from_handle(&folder).unwrap();
    let replay = loaded.commit(&mut folder).unwrap();
    assert!(
        replay.is_clean(),
        "a loaded store restates itself; it moved {:?} and removed {:?}",
        replay.written,
        replay.removed,
    );
    std::fs::remove_dir_all(&scratch).ok();
}

/// The two doors do one thing, so neither can drift from the other.
///
/// `write_into` is `commit` with its report discarded, kept for a caller that
/// reads no report. It is one implementation: what the store holds after each
/// is the same store, and a commit after a write_into is clean.
#[test]
fn write_into_is_the_commit_a_caller_reads_nothing_from() {
    let registry = committed_registry();
    let scratch = scratch("write-into-door");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut folder = LocalFolder::new(scratch.clone()).unwrap();

    registry.write_into(&mut folder).unwrap();
    let after = registry.commit(&mut folder).unwrap();
    assert!(
        after.is_clean(),
        "write_into settled the store; a commit after it moved {:?}",
        after.written,
    );
    assert!(after.skipped > 0, "the store is there to skip");
    std::fs::remove_dir_all(&scratch).ok();
}
