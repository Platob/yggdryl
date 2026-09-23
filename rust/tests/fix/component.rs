//! `rust/src/fix/component.rs`: repeating groups and their occurrence
//! components, including the native mapping groups and their counters.

use std::sync::Arc;

use yggdryl::{DataType, Field, FixCategory, FixMsg, FixRegistry, Scalar, StructType, fix_schema};

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
    let schema = StructType::from_fields(fields)
        .map(DataType::from)
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
    // Every wire field is a leaf but the list of scalars the crate owns:
    // `srcuuids` is one column under one name, because a group's occurrence
    // is a Struct of members a wire states one tag at a time and it is not.
    assert!(
        super::definitions(&registry, FixCategory::Fields)
            .all(|field| !field.dtype().is_nested() || field.name() == "srcuuids")
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
        StructType::from_fields([DataType::utf8().nullable_field("id")])
            .map(DataType::from)
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
    let Some(map) = (stored.dtype()).as_mapping() else {
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
        .filter(|field| field.name() == "identifiers")
        .collect();
    assert_eq!(columns.len(), 1);
    let column = columns[0];
    assert!(column.is_nullable());
    assert_eq!(column.as_fix().tag().unwrap(), Some(65_020));
    assert_eq!(column.as_fix().counter().unwrap(), Some(65_020));
    let Some(map) = (column.dtype()).as_mapping() else {
        panic!("identifiers is a Map")
    };
    assert!(map.keys_sorted());
    assert!(registry.get_field_by_tag(65_020).is_none());
    assert_eq!(yggdryl::fix::GROUP_TAGS, [453, 454, 768, 1907]);
}

#[test]
fn native_mapping_survives_message_rows_and_arrow_in_both_directions() {
    let registry = Arc::new(FixRegistry::new());
    let group = registry.get_field_by_counter(65_020).unwrap().clone();
    let schema = StructType::from_fields([group])
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
    for pairs in [
        vec![],
        vec![
            (Scalar::from("clordid"), Scalar::from("O-1")),
            (Scalar::from("execid"), Scalar::from("E-1")),
        ],
    ] {
        let empty = pairs.is_empty();
        let mapping = Scalar::from_mapping(pairs).unwrap();
        let source = fresh(Arc::clone(&registry), &schema, vec![mapping.clone()]);
        // The identifiers are the event's own fact: a map stating none is
        // no fact, and one stating pairs answers them.
        assert_eq!(source.get_by_tag(65_020), (!empty).then(|| mapping.clone()));
        let schema = source.as_field();
        let row = source.as_value();
        let msg = FixMsg::from_row(Arc::clone(&registry), schema, row).unwrap();
        assert_eq!(&msg.into_row(schema).unwrap(), row);
        let array = yggdryl::Serie::from_scalars(schema.clone(), [row.clone()])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let restored = yggdryl::Serie::from_arrow_array(
            Some(schema),
            array,
            yggdryl::ArrowCastOptions::default(),
        )
        .unwrap()
        .scalar(0)
        .unwrap();
        assert_eq!(&restored, row);
        let restored_msg = FixMsg::from_row(Arc::clone(&registry), schema, &restored).unwrap();
        assert_eq!(&restored_msg.into_row(schema).unwrap(), row);
    }
}

#[test]
fn map_paths_distinguish_present_missing_and_absent_maps() {
    let registry = Arc::new(FixRegistry::new());
    let schema = StructType::from_fields([registry.get_field_by_counter(65_020).unwrap().clone()])
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
    let key = yggdryl::FieldPath::from_str("identifiers['clordid']").unwrap();
    let missing = yggdryl::FieldPath::from_str("identifiers['missing']").unwrap();
    let named_child = yggdryl::FieldPath::from_str("identifiers.clordid").unwrap();
    let indexed_child = yggdryl::FieldPath::from_str("identifiers[0]").unwrap();
    let value_field = registry.field_by_path(&key).unwrap();
    assert_eq!(value_field.dtype(), &DataType::utf8());
    assert!(value_field.is_nullable());
    assert_eq!(registry.field_by_path(&missing).unwrap(), value_field);
    for invalid in [&named_child, &indexed_child] {
        assert!(registry.get_field_by_path(invalid).is_none());
    }
    // The identifiers a message goes by are names beside values, so a key
    // stating nothing is a key it does not go by: present and missing are
    // the two answers a path has.
    let value = Scalar::from("O-1");
    let mapping = Scalar::from_mapping([(Scalar::from("clordid"), value.clone())]).unwrap();
    let message = fresh(Arc::clone(&registry), &schema, vec![mapping]);
    assert_eq!(message.get_by_path(&key), Some(value.clone()));
    assert_eq!(message.get("identifiers['clordid']"), Some(value));
    assert_eq!(message.get_by_path(&missing), None);
    for invalid in [&named_child, &indexed_child] {
        assert_eq!(message.get_by_path(invalid), None);
    }
    let nulled = Scalar::from_mapping([(Scalar::from("clordid"), Scalar::Null)]).unwrap();
    let message = fresh(Arc::clone(&registry), &schema, vec![nulled]);
    assert_eq!(message.get_by_path(&key), None);
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
    label.as_fix_mut().set_names(["Identifiers"]).unwrap();
    registry.insert(label.clone()).unwrap();
    let map = registry.get_field_by_counter(65_020).unwrap().clone();
    let schema = StructType::from_fields([label, map])
        .map(DataType::from)
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
    let root = yggdryl::FieldPath::from_str("Identifiers").unwrap();
    let path = yggdryl::FieldPath::from_str("identifiers['clordid']").unwrap();
    assert_eq!(
        registry.field_by_name("Identifiers").unwrap().name(),
        "label"
    );
    let declared = registry.field_by_path(&root).unwrap();
    assert_eq!(declared, registry.get_field_by_counter(65_020).unwrap());
    let Some(map) = (declared.dtype()).as_mapping() else {
        panic!("the canonical Map owns the resolved path")
    };
    assert_eq!(
        registry.field_by_path(&path).unwrap(),
        &map.entries().fields()[1]
    );
    assert_eq!(message.get_by_name("Identifiers"), Some(mapping("O-1")));
    assert_eq!(message.get_by_path(&root), Some(mapping("O-1")));
    assert_eq!(message.get_by_path(&path), Some(Scalar::from("O-1")));
    message.set("IDENTIFIERS", mapping("O-2")).unwrap();
    assert_eq!(message.get_by_path(&root), Some(mapping("O-2")));
    assert_eq!(message.get_by_path(&path), Some(Scalar::from("O-2")));
    assert_eq!(
        message.get_by_name("label"),
        Some(Scalar::from("stated-label"))
    );
    assert_eq!(
        message.as_field().fields().len(),
        1,
        "the one business field; the map is the event's own"
    );
}

#[test]
fn map_and_component_roots_remain_ambiguous_despite_scalar_aliases() {
    for scalar_alias in [false, true] {
        let mut registry = FixRegistry::new();
        if scalar_alias {
            let mut label = DataType::utf8().nullable_field("label");
            label.as_fix_mut().set_tag(9001).unwrap();
            label.as_fix_mut().set_names(["Identifiers"]).unwrap();
            registry.insert(label).unwrap();
        }
        let component = StructType::from_fields([DataType::utf8().nullable_field("note")])
            .map(DataType::from)
            .unwrap()
            .required_field("Identifiers");
        registry.insert(component).unwrap();
        assert!(registry.get_field_by_name("identifiers").is_some());
        for spelling in [
            "identifiers",
            "Identifiers",
            "identifiers['clordid']",
            "Identifiers.note",
        ] {
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
