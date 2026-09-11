//! Numeric FIX keys and group plans share the capture's branch resolution.

use super::path;

use std::sync::Arc;

use yggdryl::{DataType, Field, FixBranch, FixCategory, FixCodec, FixRegistry, Scalar};

fn scalar(name: &str, tag: i32, dtype: DataType, branch: &FixBranch) -> Field {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field.as_fix_mut().set_branch(branch).unwrap();
    field
}

fn group(
    registry: &mut FixRegistry,
    name: &str,
    counter: i32,
    member: Field,
    branch: &FixBranch,
) -> Field {
    let mut component = DataType::from_fields([member])
        .unwrap()
        .required_field(format!("{name}Entry"));
    component.as_fix_mut().set_branch(branch).unwrap();
    registry
        .create_definition(FixCategory::Components, component.clone())
        .unwrap();
    let mut group = DataType::list(component.clone()).nullable_field(name);
    group.as_fix_mut().set_branch(branch).unwrap();
    group.as_fix_mut().set_counter(counter).unwrap();
    group.as_fix_mut().set_component(component.name()).unwrap();
    registry
        .create_definition(FixCategory::Groups, group.clone())
        .unwrap();
    group
}

fn registry(scoped: bool) -> Arc<FixRegistry> {
    let standard = FixBranch::STANDARD;
    let mut registry = FixRegistry::from_fields([
        scalar("MsgType", 35, DataType::Utf8, &standard),
        scalar("Symbol", 55, DataType::Utf8, &standard),
        scalar("CheckSum", 10, DataType::Utf8, &standard),
    ])
    .unwrap();
    for (branch, name, dtype) in [
        ("alpha", "Alpha", DataType::Int32),
        ("beta", "Beta", DataType::Utf8),
    ] {
        let branch = FixBranch::from_str(branch).unwrap();
        let counter = scalar(&format!("No{name}Rows"), 6000, DataType::Int32, &branch);
        let member = scalar(&format!("{name}ID"), 6001, dtype.clone(), &branch);
        let tail = scalar(&format!("{name}Value"), 6002, dtype, &branch);
        registry
            .add_fields([counter.clone(), member.clone(), tail.clone()])
            .unwrap();
        let group = group(&mut registry, &format!("{name}Rows"), 6000, member, &branch);
        if scoped {
            let mut message = DataType::from_fields([counter, group, tail])
                .unwrap()
                .required_field(format!("{name}Message"));
            message.as_fix_mut().set_branch(&branch).unwrap();
            message.as_fix_mut().set_msgtype("X").unwrap();
            registry
                .create_definition(FixCategory::Messages, message)
                .unwrap();
        }
    }
    let alpha = FixBranch::from_str("alpha").unwrap();
    let counter = scalar("NoAlphaOnlyRows", 6100, DataType::Int32, &alpha);
    let member = scalar("AlphaOnlyID", 6101, DataType::Int32, &alpha);
    registry.add_fields([counter, member.clone()]).unwrap();
    group(&mut registry, "AlphaOnlyRows", 6100, member, &alpha);
    Arc::new(registry)
}

#[test]
fn numeric_scalars_and_groups_follow_the_pinned_branch() {
    let wire = b"35=X|6000=1|6001=42|6002=7|55=AAPL|10=0|";
    for scoped in [false, true] {
        let registry = registry(scoped);
        for (branch, name, dtype, member, tail) in [
            (
                "alpha",
                "Alpha",
                DataType::Int32,
                Scalar::from(42_i32),
                Scalar::from(7_i32),
            ),
            (
                "beta",
                "Beta",
                DataType::Utf8,
                Scalar::from("42"),
                Scalar::from("7"),
            ),
        ] {
            let branch = FixBranch::from_str(branch).unwrap();
            let codec = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);
            let message = codec.parse_fix_line(wire).unwrap();
            assert_eq!(
                message.by_name(&format!("No{name}Rows")).unwrap(),
                &Scalar::from(1_i32)
            );
            assert_eq!(
                message
                    .by_path(&path(&format!("{name}Rows[0].{name}ID")))
                    .unwrap(),
                &member
            );
            assert_eq!(message.by_name(&format!("{name}Value")).unwrap(), &tail);
            assert_eq!(
                message
                    .as_field()
                    .field_by_path(&format!("{name}Value"))
                    .unwrap()
                    .dtype(),
                &dtype
            );
            assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
            assert_eq!(message.into_bytes(b'|'), wire);
        }
    }
}

#[test]
fn a_pinned_branch_does_not_borrow_another_venues_numeric_counter() {
    let codec = FixCodec::new(registry(false)).with_branch(&FixBranch::from_str("beta").unwrap());
    let wire = b"6100=1|6101=42|55=AAPL|";
    let message = codec.parse_fix_line(wire).unwrap();
    assert!(message.get_by_name("NoAlphaOnlyRows").is_none());
    assert!(message.get_by_name("AlphaOnlyRows").is_none());
    assert!(message.get_by_name("AlphaOnlyID").is_none());
    assert_eq!(message.by_name("6100").unwrap(), &Scalar::from("1"));
    assert_eq!(message.by_name("6101").unwrap(), &Scalar::from("42"));
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
    assert_eq!(message.into_bytes(b'|'), wire);
}
