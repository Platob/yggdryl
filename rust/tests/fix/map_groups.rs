//! Native mapping groups keep their own counters and the Arrow map contract.

use std::sync::Arc;

use yggdryl::graph::Element;
use yggdryl::{DataType, Field, FixCategory, FixMsg, FixRegistry, Scalar, fix_schema};

fn mapping_group(name: &str, tag: i32, sorted: bool) -> Field {
    let mut field = DataType::map_of(DataType::utf8(), DataType::utf8(), sorted)
        .unwrap()
        .nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field.as_fix_mut().set_counter(tag).unwrap();
    field
}

fn fresh(registry: Arc<FixRegistry>, schema: &Field, mut values: Vec<Scalar>) -> FixMsg {
    let mut fields = schema.fields().to_vec();
    fields.push(registry.field_by_tag(52).unwrap().clone());
    values.push(
        super::fixed_codec(Arc::clone(&registry))
            .default_sending_time()
            .unwrap()
            .clone(),
    );
    let schema = DataType::from_fields(fields)
        .unwrap()
        .required_field(schema.name());
    FixMsg::with_registry(registry, schema, Scalar::from_sequence(values)).unwrap()
}

#[test]
fn maps_are_groups_with_one_reserved_counter_and_never_scalar_fields() {
    let mut registry = FixRegistry::new();
    let group = mapping_group("nativeids", 65_090, true);
    // A definition is filed by the shape it has: a Map is a group, so the
    // one insert door files it as one and no scalar stands beside it.
    registry.insert(group.clone()).unwrap();
    assert_eq!(registry.get_field_by_counter(65_090), Some(&group));
    assert!(registry.get_field_by_tag(65_090).is_none());
    assert!(
        super::definitions(&registry, FixCategory::Fields).all(|field| !field.dtype().is_nested())
    );

    let before = registry.clone();
    let mut conflicting = mapping_group("anotherids", 65_090, true);
    assert!(registry.insert(conflicting.clone()).is_err());
    conflicting.as_fix_mut().set_counter(65_091).unwrap();
    assert!(registry.insert(conflicting).is_err());
    assert!(
        registry
            .insert(mapping_group("foreignids", 55, true))
            .is_err()
    );
    assert_eq!(registry, before);
}

#[test]
fn map_counters_refuse_scalar_collisions_in_either_insertion_order() {
    let group = mapping_group("nativeids", 65_090, true);
    for alternate in [false, true] {
        let mut scalar = DataType::Int32.nullable_field("count");
        scalar
            .as_fix_mut()
            .set_tag(if alternate { 9001 } else { 65_090 })
            .unwrap();
        if alternate {
            scalar.as_fix_mut().set_tags(&[65_090]).unwrap();
        }
        for scalar_first in [false, true] {
            let mut registry = FixRegistry::new();
            if scalar_first {
                registry.insert(scalar.clone()).unwrap();
            } else {
                registry.insert(group.clone()).unwrap();
            }
            let before = registry.clone();
            let refused = if scalar_first {
                registry.insert(group.clone())
            } else {
                registry.insert(scalar.clone())
            };
            assert!(
                refused.is_err(),
                "alternate={alternate}, scalar_first={scalar_first}"
            );
            assert_eq!(registry, before);
            assert_eq!(
                registry.get_field_by_tag(65_090),
                before.get_field_by_tag(65_090)
            );
            assert_eq!(
                registry.get_field_by_counter(65_090),
                before.get_field_by_counter(65_090)
            );
        }
    }
}

#[test]
fn ordinary_list_groups_still_require_a_separate_int32_counter() {
    let mut group = DataType::list(
        DataType::from_fields([DataType::utf8().nullable_field("id")])
            .unwrap()
            .required_field("occurrence"),
    )
    .nullable_field("ordinary");
    group.as_fix_mut().set_counter(9001).unwrap();
    let mut registry = FixRegistry::new();
    assert!(registry.insert(group.clone()).is_err());
    let mut wrong = DataType::Int64.nullable_field("count");
    wrong.as_fix_mut().set_tag(9001).unwrap();
    registry.insert(wrong).unwrap();
    assert!(registry.insert(group).is_err());
}

#[test]
fn merging_map_groups_preserves_layout_sortedness_and_key_nullability() {
    let mut registry = FixRegistry::new();
    let group = mapping_group("nativeids", 65_090, true);
    registry.insert(group).unwrap();
    let mut incoming = mapping_group("nativeids", 65_090, false);
    incoming.set_comment("merged").unwrap();
    assert!(!registry.add_field(incoming).unwrap());
    let stored = registry.get_field_by_counter(65_090).unwrap();
    let DataType::Map(map) = stored.dtype() else {
        panic!("merging preserves Map")
    };
    assert_eq!(stored.comment(), Some("merged"));
    assert!(map.keys_sorted());
    assert!(!map.entries().is_nullable());
    assert!(!map.entries().fields()[0].is_nullable());
}

#[test]
fn altids_has_exactly_one_nullable_sorted_column_without_a_scalar_counter() {
    let registry = FixRegistry::new();
    let schema = fix_schema(&registry, "fix").unwrap();
    let columns: Vec<_> = schema
        .fields()
        .iter()
        .filter(|field| field.name() == "altids")
        .collect();
    assert_eq!(columns.len(), 1);
    let column = columns[0];
    assert!(column.is_nullable());
    assert_eq!(column.as_fix().tag().unwrap(), Some(65_020));
    assert_eq!(column.as_fix().counter().unwrap(), Some(65_020));
    let DataType::Map(map) = column.dtype() else {
        panic!("altids is a Map")
    };
    assert!(map.keys_sorted());
    assert!(registry.get_field_by_tag(65_020).is_none());
    assert_eq!(yggdryl::fix::GROUP_TAGS, [453, 454, 768]);
}

#[test]
fn native_mapping_survives_message_rows_and_arrow_in_both_directions() {
    let registry = Arc::new(FixRegistry::new());
    let group = registry.get_field_by_counter(65_020).unwrap().clone();
    let schema = DataType::from_fields([group])
        .unwrap()
        .required_field("fix");
    for pairs in [
        vec![],
        vec![
            (Scalar::from("clordid"), Scalar::from("O-1")),
            (Scalar::from("execid"), Scalar::from("E-1")),
        ],
    ] {
        let mapping = Scalar::from_mapping(pairs).unwrap();
        let source = fresh(Arc::clone(&registry), &schema, vec![mapping.clone()]);
        assert_eq!(source.by_tag(65_020).unwrap(), mapping);
        let schema = source.as_field();
        let row = source.as_value();
        let msg = FixMsg::from_row(Arc::clone(&registry), schema, row).unwrap();
        assert_eq!(&msg.into_row(schema).unwrap(), row);
        let array = yggdryl::arrow::scalar_array(schema, row).unwrap();
        let restored = yggdryl::arrow::scalar_value(schema, array.as_ref()).unwrap();
        assert_eq!(&restored, row);
        let restored_msg = FixMsg::from_row(Arc::clone(&registry), schema, &restored).unwrap();
        assert_eq!(&restored_msg.into_row(schema).unwrap(), row);
    }
}

#[test]
fn map_paths_distinguish_present_null_missing_and_absent_maps() {
    let registry = Arc::new(FixRegistry::new());
    let schema = DataType::from_fields([registry.get_field_by_counter(65_020).unwrap().clone()])
        .unwrap()
        .required_field("fix");
    let key = yggdryl::FieldPath::from_str("altids['clordid']").unwrap();
    let missing = yggdryl::FieldPath::from_str("altids['missing']").unwrap();
    let named_child = yggdryl::FieldPath::from_str("altids.clordid").unwrap();
    let indexed_child = yggdryl::FieldPath::from_str("altids[0]").unwrap();
    let value_field = registry.field_by_path(&key).unwrap();
    assert_eq!(value_field.dtype(), &DataType::utf8());
    assert!(value_field.is_nullable());
    assert_eq!(registry.field_by_path(&missing).unwrap(), value_field);
    for invalid in [&named_child, &indexed_child] {
        assert!(registry.get_field_by_path(invalid).is_none());
    }
    for value in [Scalar::from("O-1"), Scalar::Null] {
        let mapping = Scalar::from_mapping([(Scalar::from("clordid"), value.clone())]).unwrap();
        let message = fresh(Arc::clone(&registry), &schema, vec![mapping]);
        assert_eq!(message.get_by_path(&key), Some(value.clone()));
        assert_eq!(message.get("altids['clordid']"), Some(value));
        assert_eq!(message.get_by_path(&missing), None);
        for invalid in [&named_child, &indexed_child] {
            assert_eq!(message.get_by_path(invalid), None);
        }
    }
    for mapping in [Scalar::Null, Scalar::from_mapping([]).unwrap()] {
        let message = fresh(Arc::clone(&registry), &schema, vec![mapping]);
        assert_eq!(message.get_by_path(&key), None);
    }
}

#[test]
fn canonical_map_names_win_over_scalar_aliases_for_reads_writes_and_paths() {
    let mut registry = FixRegistry::new();
    let mut label = DataType::utf8().nullable_field("label");
    label.as_fix_mut().set_tag(9001).unwrap();
    label.as_fix_mut().set_names(["AltIds"]).unwrap();
    registry.insert(label.clone()).unwrap();
    let map = registry.get_field_by_counter(65_020).unwrap().clone();
    let schema = DataType::from_fields([label, map])
        .unwrap()
        .required_field("fix");
    let mapping = |value: &str| {
        Scalar::from_mapping([(Scalar::from("clordid"), Scalar::from(value))]).unwrap()
    };
    let registry = Arc::new(registry);
    let mut message = fresh(
        Arc::clone(&registry),
        &schema,
        vec![Scalar::from("stated-label"), mapping("O-1")],
    );
    let root = yggdryl::FieldPath::from_str("AltIds").unwrap();
    let path = yggdryl::FieldPath::from_str("altids['clordid']").unwrap();
    assert_eq!(registry.field_by_name("AltIds").unwrap().name(), "label");
    let declared = registry.field_by_path(&root).unwrap();
    assert_eq!(declared, registry.get_field_by_counter(65_020).unwrap());
    let DataType::Map(map) = declared.dtype() else {
        panic!("the canonical Map owns the resolved path")
    };
    assert_eq!(
        registry.field_by_path(&path).unwrap(),
        &map.entries().fields()[1]
    );
    assert_eq!(message.get_by_name("AltIds"), Some(mapping("O-1")));
    assert_eq!(message.get_by_path(&root), Some(mapping("O-1")));
    assert_eq!(message.get_by_path(&path), Some(Scalar::from("O-1")));
    message.set("ALTIDS", mapping("O-2")).unwrap();
    assert_eq!(message.get_by_path(&root), Some(mapping("O-2")));
    assert_eq!(message.get_by_path(&path), Some(Scalar::from("O-2")));
    assert_eq!(
        message.get_by_name("label"),
        Some(Scalar::from("stated-label"))
    );
    assert_eq!(
        message.as_field().fields().len(),
        9,
        "two business fields and the replay bundle"
    );
}

#[test]
fn a_tagless_canonical_map_outranks_ordinary_and_mandatory_scalar_aliases() {
    for alias_tag in [9001, yggdryl::HASHCODE_TAG_NAME.0, 52] {
        let mut registry = FixRegistry::new();
        let mut alias = if let Some(field) = registry.get_field_by_tag(alias_tag) {
            field.clone()
        } else {
            let mut field = DataType::utf8().nullable_field("label");
            field.as_fix_mut().set_tag(alias_tag).unwrap();
            field
        };
        alias.as_fix_mut().set_names(["AltIds"]).unwrap();
        registry.insert(alias).unwrap();
        assert_eq!(
            registry
                .field_by_name("AltIds")
                .unwrap()
                .as_fix()
                .tag()
                .unwrap(),
            Some(alias_tag)
        );
        let mut map = registry.get_field_by_counter(65_020).unwrap().clone();
        map.remove_metadata("fix:tag");
        assert_eq!(map.as_fix().counter().unwrap(), Some(65_020));
        let schema = DataType::from_fields([map]).unwrap().required_field("fix");
        let registry = Arc::new(registry);
        let mapping =
            Scalar::from_mapping([(Scalar::from("clordid"), Scalar::from("O-1"))]).unwrap();
        let source = fresh(Arc::clone(&registry), &schema, vec![mapping.clone()]);
        assert_eq!(source.by_tag(65_020).unwrap(), mapping);
        assert_ne!(source.get_hashcode(), 0);
        let row = source.into_row(source.as_field()).unwrap();
        let restored = FixMsg::from_row(Arc::clone(&registry), source.as_field(), &row).unwrap();
        assert_eq!(restored.by_tag(65_020).unwrap(), mapping);
        assert_eq!(restored.get_by_name("AltIds"), Some(mapping.clone()));
        assert_eq!(restored.into_row(source.as_field()).unwrap(), row);

        let empty = DataType::from_fields([]).unwrap().required_field("fix");
        let mut absent = fresh(Arc::clone(&registry), &empty, Vec::new());
        let root = yggdryl::FieldPath::from_str("AltIds").unwrap();
        let key = yggdryl::FieldPath::from_str("altids['clordid']").unwrap();
        let sending = absent.by_tag(52).unwrap().clone();
        assert_eq!(absent.get_by_name("AltIds"), None);
        assert_eq!(absent.get_by_path(&root), None);
        assert_eq!(absent.get_by_path(&key), None);
        assert_eq!(absent.get_by_tag(65_020), None);
        assert_eq!(absent.remove("AltIds").unwrap(), None);
        let before = absent.clone();
        assert!(absent.set("AltIds", sending.clone()).is_err());
        assert_eq!(absent, before, "a Map name never writes the aliased clock");
        absent.set("AltIds", mapping.clone()).unwrap();
        assert_eq!(absent.get_by_name("AltIds"), Some(mapping.clone()));
        assert_eq!(absent.get_by_path(&root), Some(mapping.clone()));
        assert_eq!(absent.get_by_path(&key), Some(Scalar::from("O-1")));
        assert_eq!(absent.get_by_tag(65_020), Some(mapping.clone()));
        assert_eq!(absent.by_tag(52).unwrap(), sending);
    }
}

#[test]
fn map_and_component_roots_remain_ambiguous_despite_scalar_aliases() {
    for scalar_alias in [false, true] {
        let mut registry = FixRegistry::new();
        if scalar_alias {
            let mut label = DataType::utf8().nullable_field("label");
            label.as_fix_mut().set_tag(9001).unwrap();
            label.as_fix_mut().set_names(["AltIds"]).unwrap();
            registry.insert(label).unwrap();
        }
        let component = DataType::from_fields([DataType::utf8().nullable_field("note")])
            .unwrap()
            .required_field("AltIds");
        registry.insert(component).unwrap();
        for category in [FixCategory::Components, FixCategory::Groups] {
            assert!(registry.get_field_by_name("altids").is_some());
        }
        for spelling in ["altids", "AltIds", "altids['clordid']", "AltIds.note"] {
            let path = yggdryl::FieldPath::from_str(spelling).unwrap();
            assert!(
                registry.get_field_by_path(&path).is_none(),
                "{spelling}, scalar_alias={scalar_alias}"
            );
            assert!(registry.field_by_path(&path).is_err());
        }
    }
}

#[test]
fn canonical_scalar_and_map_names_conflict_atomically_in_either_order() {
    let group = mapping_group("nativeids", 65_090, true);
    let mut scalar = DataType::utf8().nullable_field("NATIVEIDS");
    scalar.as_fix_mut().set_tag(9001).unwrap();
    for scalar_first in [false, true] {
        let mut registry = FixRegistry::new();
        if scalar_first {
            registry.insert(scalar.clone()).unwrap();
        } else {
            registry.insert(group.clone()).unwrap();
        }
        let before = registry.clone();
        let refused = if scalar_first {
            registry.insert(group.clone())
        } else {
            registry.insert(scalar.clone())
        };
        assert!(refused.is_err(), "scalar_first={scalar_first}");
        assert_eq!(registry, before);
        assert_eq!(
            registry.get_field_by_name("nativeids"),
            before.get_field_by_name("nativeids")
        );
        assert_eq!(
            registry.get_field_by_counter(65_090),
            before.get_field_by_counter(65_090)
        );
    }
}
