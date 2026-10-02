//! `rust/src/excel/formula/functions.rs`: the one private function registry.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::excel_formula_functions::{accepts, catalog, lookup};

    #[test]
    fn listed_signatures_match_the_durable_ui_fixture() {
        let fixture: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../fixtures/functions.json")).unwrap();
        let facts = catalog();
        let listed: Vec<_> = facts
            .iter()
            .filter(|fact| fact.signature.is_some())
            .collect();
        assert_eq!(listed.len(), 159);
        assert_eq!(facts.len(), 170);
        assert!(facts.windows(2).all(|pair| pair[0].name < pair[1].name));
        for item in &fixture {
            let name = item["name"].as_str().unwrap();
            let fact = lookup(name).unwrap();
            assert_eq!(fact.category, item["category"].as_str(), "{name}");
            assert_eq!(fact.signature, item["signature"].as_str(), "{name}");
            assert_eq!(fact.description, item["description"].as_str(), "{name}");
            assert!(fact.arity.is_some(), "{name}");
            assert_eq!(lookup(&name.to_ascii_lowercase()), Some(fact));
        }
        assert_eq!(fixture.len(), listed.len());
    }

    #[test]
    fn prefix_only_held_names_and_volatile_facts_have_one_owner() {
        let facts = catalog();
        assert_eq!(
            facts.iter().filter(|fact| !fact.prefix.is_empty()).count(),
            31
        );
        assert_eq!(
            facts
                .iter()
                .filter(|fact| fact.signature.is_some() && !fact.prefix.is_empty())
                .count(),
            20
        );
        assert_eq!(facts.iter().filter(|fact| fact.volatile).count(), 6);
        let held: Vec<_> = facts
            .iter()
            .filter(|fact| fact.signature.is_none())
            .map(|fact| fact.name)
            .collect();
        assert_eq!(
            held,
            [
                "AGGREGATE",
                "ANCHORARRAY",
                "FILTER",
                "LAMBDA",
                "LET",
                "RANDARRAY",
                "SEQUENCE",
                "SORT",
                "SORTBY",
                "UNIQUE",
                "XMATCH"
            ]
        );
        assert_eq!(lookup("FILTER").unwrap().prefix, "_xlfn._xlws.");
        assert_eq!(lookup("SORT").unwrap().prefix, "_xlfn._xlws.");
        assert_eq!(lookup("XLOOKUP").unwrap().prefix, "_xlfn.");
        assert!(lookup("NOT_A_FUNCTION").is_none());
    }

    #[test]
    fn typed_arity_distinguishes_fixed_optional_and_paired_repeats() {
        assert_eq!(accepts("PI", 0), Some(true));
        assert_eq!(accepts("PI", 1), Some(false));
        // Excel 16.0 build 20430.0 refuses the isolated IF(TRUE)/IF(FALSE)
        // files, while two arguments and a blank true arm open unchanged.
        assert_eq!(accepts("IF", 1), Some(false));
        assert_eq!(accepts("IF", 2), Some(true));
        assert_eq!(accepts("IF", 3), Some(true));
        assert_eq!(accepts("IF", 4), Some(false));
        // The native reference-form INDEX selects the second supplied area.
        assert_eq!(accepts("INDEX", 4), Some(true));
        assert_eq!(accepts("SUMIFS", 3), Some(true));
        assert_eq!(accepts("SUMIFS", 4), Some(false));
        assert_eq!(accepts("SUMIFS", 5), Some(true));
        assert_eq!(accepts("COUNTIFS", 2), Some(true));
        assert_eq!(accepts("COUNTIFS", 3), Some(false));
        assert_eq!(accepts("COUNTIFS", 4), Some(true));
        assert_eq!(accepts("AGGREGATE", 2), None);
    }
}
