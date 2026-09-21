//! `rust/src/fix/group_plan.rs`: the layout a repeating group is read by.
//!
//! A plan is compiled once per group definition and borrowed by every message
//! and every registry clone reading it, so what is pinned here is the sharing
//! as much as the layout. A caller sees the row a plan lays out and never the
//! plan, so it is reached through `yggdryl::internals`.

use yggdryl::internals::fix_catalog::{
    get_group_by_tag as registry_group, get_group_plan_by_tag as registry_plan,
};
use yggdryl::internals::fix_group_plan::{GroupPlan, tags_is_empty};
use yggdryl::internals::fix_msgtype::{
    from_field as msgtype_from_field, get_group_plan_by_tag as msgtype_plan,
};
use yggdryl::sequence::SequenceType;
use yggdryl::{DataType, Field, FixCategory, FixRegistry, Scalar, StructType};

fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
    let mut field = dtype.required_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn parties() -> Field {
    let subparty = StructType::from_fields([tagged("PartySubID", 523, DataType::utf8())])
        .map(DataType::from)
        .unwrap()
        .required_field("SubParty");
    let mut nested = DataType::large_list(subparty).required_field("SubParties");
    nested.as_fix_mut().set_counter(802).unwrap();
    let attribution = StructType::from_fields([tagged("PartyRole", 452, DataType::Int32)])
        .map(DataType::from)
        .unwrap()
        .required_field("Attribution");
    let item = StructType::from_fields([
        tagged("PartyID", 448, DataType::utf8()),
        attribution,
        tagged("NoPartySubIDs", 802, DataType::Int32),
        nested,
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("Party");
    let mut group = DataType::list(item).required_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group
}

#[test]
fn plans_flatten_components_preserve_list_width_and_project_nullable_members() {
    let source = parties();
    let plan = GroupPlan::from_field(&source).unwrap();
    assert_eq!(plan.columns_len(), 4);
    assert_eq!(plan.delimiter(), Some(448));
    assert_eq!(plan.tag_index(452), Some(1));
    assert_eq!(plan.tag_index(802), Some(2));
    assert!(plan.column(1).is_nullable());
    let (column, nested) = plan.nested(802).unwrap();
    assert_eq!(column, 3);
    assert!(matches!(
        nested.field().dtype(),
        DataType::Sequence(SequenceType::LargeList(_))
    ));
    assert_eq!(nested.tag_index(523), Some(0));
    let row = plan.row(vec![
        Scalar::from("broker"),
        Scalar::Null,
        Scalar::from(0_i32),
        Scalar::Null,
    ]);
    assert_eq!(
        row,
        Scalar::from_sequence([
            Scalar::from("broker"),
            Scalar::Null,
            Scalar::from(0_i32),
            Scalar::Null,
        ])
    );
    let DataType::Sequence(SequenceType::List(item)) = source.dtype() else {
        panic!("list")
    };
    assert!(!item.fields()[0].is_nullable());
}

#[test]
fn message_and_nested_scope_borrow_one_precompiled_plan() {
    let mut field = StructType::from_fields([parties()])
        .map(DataType::from)
        .unwrap()
        .required_field("Report");
    field.as_fix_mut().set_msgtype("R").unwrap();
    let message = msgtype_from_field(field).unwrap();
    let outer = msgtype_plan(&message, 453).unwrap();
    let nested = msgtype_plan(&message, 802).unwrap();
    assert!(std::ptr::eq(outer.nested(802).unwrap().1, nested));
    assert!(std::ptr::eq(outer, msgtype_plan(&message, 453).unwrap(),));
    let cloned = message.clone();
    assert!(std::ptr::eq(outer, msgtype_plan(&cloned, 453).unwrap(),));
}

#[test]
fn registry_clones_share_plans_and_replacements_recompile_once() {
    let mut registry =
        FixRegistry::from_fields([tagged("NoPartyIDs", 453, DataType::Int32)]).unwrap();
    registry
        .insert_definition(FixCategory::Groups, parties())
        .unwrap();
    let snapshot = registry.clone();
    let original = registry_plan(&snapshot, 453).unwrap();
    assert!(std::ptr::eq(
        original,
        registry_plan(&registry, 453).unwrap()
    ));
    let item = StructType::from_fields([tagged("PartyRole", 452, DataType::Int32)])
        .map(DataType::from)
        .unwrap()
        .required_field("Party");
    let mut replacement = DataType::list(item).required_field("Parties");
    replacement.as_fix_mut().set_counter(453).unwrap();
    registry
        .update_definition(FixCategory::Groups, replacement)
        .unwrap();
    let current = registry_plan(&registry, 453).unwrap();
    assert!(!std::ptr::eq(original, current));
    assert_eq!(original.delimiter(), Some(448));
    assert_eq!(current.delimiter(), Some(452));
}

#[test]
fn maps_keep_native_key_shape_and_declare_no_numeric_wire_layout() {
    let mut field = DataType::map_of(DataType::utf8(), DataType::utf8(), true)
        .unwrap()
        .nullable_field("nativeids");
    field.as_fix_mut().set_tag(65_090).unwrap();
    field.as_fix_mut().set_counter(65_090).unwrap();
    let plan = GroupPlan::from_field(&field).unwrap();
    let DataType::Mapping(map) = plan.field().dtype() else {
        panic!("the native Map layout is preserved")
    };
    assert!(map.keys_sorted());
    assert!(!map.entries().is_nullable());
    assert!(!map.entries().fields()[0].is_nullable());
    assert_eq!(plan.delimiter(), None);
    assert!(tags_is_empty(&plan));
    assert_eq!(
        plan.row(vec![Scalar::from("orderid"), Scalar::from("O-1")]),
        Scalar::from_sequence([Scalar::from("orderid"), Scalar::from("O-1")]),
    );
    let mut registry = FixRegistry::new();
    registry
        .insert_definition(FixCategory::Groups, field)
        .unwrap();
    assert!(registry_group(&registry, 65_090).is_some());
    assert!(registry_plan(&registry, 65_090).is_none());
}
