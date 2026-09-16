//! The complete shipped identifier declarations, including absences.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yggdryl::FixCategory;

#[test]
fn every_shipped_component_keeps_its_literal_identifier_membership() {
    let registry = super::committed_registry();
    let mut expected = BTreeMap::new();
    for line in include_str!("identifier_dictionary.snapshot")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let (name, identifiers) = line.split_once('\t').expect("name and declaration");
        assert!(expected.insert(name, identifiers).is_none(), "{name}");
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix/components");
    let shipped: BTreeSet<String> = std::fs::read_dir(root)
        .expect("the generated component directory")
        .map(|entry| {
            entry
                .expect("a component document")
                .path()
                .file_stem()
                .expect("a component name")
                .to_str()
                .expect("an ASCII component name")
                .to_owned()
        })
        .collect();
    assert_eq!(shipped.len(), 928);
    assert_eq!(
        shipped.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        expected.keys().copied().collect()
    );

    let mut declarations = 0;
    let mut identifier_count = 0;
    for (name, identifiers) in expected {
        let component = registry
            .definition(FixCategory::Components, name)
            .expect("a shipped component resolves");
        let expected = if identifiers == "-" {
            Vec::new()
        } else {
            declarations += 1;
            identifiers.split(',').collect::<Vec<_>>()
        };
        let actual = component.as_fix().identifiers().collect::<Vec<_>>();
        assert_eq!(actual, expected, "{name}");
        assert_eq!(
            component.as_metadata().get("fix:identifiers"),
            (identifiers != "-").then_some(identifiers),
            "{name}: absence is not an empty property"
        );
        identifier_count += actual.len();

        let direct = component
            .dtype()
            .as_fields()
            .expect("a component is a Struct");
        let selected = direct
            .iter()
            .filter(|member| actual.contains(&member.name()))
            .collect::<Vec<_>>();
        assert_eq!(
            selected
                .iter()
                .map(|member| member.name())
                .collect::<Vec<_>>(),
            actual,
            "{name}: names are canonical direct members in declaration order"
        );
        assert!(
            selected.iter().all(|member| !member.dtype().is_nested()),
            "{name}: a nested identifier escaped the boundary"
        );
    }
    assert_eq!((declarations, identifier_count), (109, 316));
    for group in registry.definitions(FixCategory::Groups) {
        assert!(
            group.as_fix().identifiers().next().is_none(),
            "{}: declarations belong to the occurrence component",
            group.name()
        );
    }
}

#[test]
fn the_five_message_identifier_lists_are_literal() {
    let registry = super::committed_registry();
    let cases: [(&str, &[&str]); 5] = [
        (
            "newordersingle",
            &[
                "clordid",
                "secondaryclordid",
                "allocid",
                "quoteid",
                "reforderid",
                "refclordid",
            ],
        ),
        (
            "executionreport",
            &[
                "orderid",
                "secondaryorderid",
                "secondaryclordid",
                "secondaryexecid",
                "clordid",
                "origclordid",
                "quoterespid",
                "listid",
                "execid",
                "execrefid",
                "allocid",
                "reforderid",
                "refclordid",
            ],
        ),
        (
            "tradecapturereport",
            &[
                "tradereportid",
                "tradeid",
                "secondarytradeid",
                "firmtradeid",
                "secondaryfirmtradeid",
                "origtradeid",
                "origsecondarytradeid",
                "tradereportrefid",
                "secondarytradereportrefid",
                "secondarytradereportid",
                "execid",
                "execrefid",
                "secondaryexecid",
            ],
        ),
        (
            "quote",
            &[
                "quotereqid",
                "quoteid",
                "secondaryquoteid",
                "quoterespid",
                "reforderid",
            ],
        ),
        (
            "allocationinstruction",
            &["allocid", "secondaryallocid", "refallocid"],
        ),
    ];
    for (name, expected) in cases {
        let component = registry
            .definition(FixCategory::Components, name)
            .expect("a shipped message component");
        assert!(component.as_fix().msgtype().is_some(), "{name}");
        assert_eq!(
            component.as_fix().identifiers().collect::<Vec<_>>(),
            expected,
            "{name}"
        );
    }
}
