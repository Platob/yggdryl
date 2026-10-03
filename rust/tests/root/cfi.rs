//! `rust/src/cfi.rs`: the coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod coded {

    use arrow_array::{Array, StringArray};

    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

    #[test]
    fn a_cfi_stores_the_six_characters_it_is_and_nothing_beside_them() {
        let cfi = Field::new("classification", DataType::Cfi, false);
        let stored = Serie::from_scalars(cfi.clone(), [Scalar::from("ESVUFR")])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();

        assert_eq!(cells.value(0), "ESVUFR");
        assert_eq!(cells.value_length(0), 6);
        assert_eq!(
            Serie::from_arrow_array(Some(&cfi), stored, ArrowCastOptions::default())
                .unwrap()
                .scalar(0)
                .unwrap(),
            DataType::Cfi.scalar(Scalar::from("ESVUFR")).unwrap()
        );
        // A width of six bytes is spellable and is still not a CFI code.
        assert_eq!(
            DataType::from_str("fixed_ascii(6)").unwrap(),
            DataType::fixed_ascii(6).unwrap()
        );
        assert_ne!(DataType::Cfi, DataType::fixed_ascii(6).unwrap());
    }
}

mod merge {
    use yggdryl::{Cfi, CodeValue};

    fn filled(mine: &str, theirs: &str) -> String {
        Cfi::new(mine)
            .unwrap()
            .merge_with(&Cfi::new(theirs).unwrap())
            .as_str()
            .to_owned()
    }

    #[test]
    fn an_unclassified_code_yields_whole_to_a_classified_one() {
        // All `X`, or a letter no group accepts: it says nothing, so the
        // other statement stands whole.
        assert_eq!(filled("XXXXXX", "ESVUFR"), "ESVUFR");
        assert_eq!(filled("XXXXXX", "JFTXFN"), "JFTXFN");
        assert_eq!(filled("ESZUFR", "ESVUFR"), "ESVUFR");
        assert_eq!(filled("XXXXXX", "ESXXXX"), "ESXXXX");
        // Nothing better beside it: this one stays.
        assert_eq!(filled("XXXXXX", "XXXXXX"), "XXXXXX");
        assert_eq!(filled("XXXXXX", "EXXXXX"), "XXXXXX");
        // A classified code never takes an unclassified one.
        assert_eq!(filled("ESVUFR", "XXXXXX"), "ESVUFR");
        assert_eq!(filled("ESXXXX", "XXXXXX"), "ESXXXX");
    }

    #[test]
    fn one_instrument_fills_position_by_position_and_two_keep_this_one() {
        assert_eq!(filled("ESVXXX", "ESXUFR"), "ESVUFR");
        assert_eq!(
            filled("ESVUFR", "ESNUFR"),
            "ESVUFR",
            "a contradicted letter keeps the lead"
        );
        assert_eq!(filled("ESNXXX", "ESVUFR"), "ESNXXX");
        assert_eq!(filled("ESVUFR", "DBFNFB"), "ESVUFR");
        assert_eq!(filled("JFTXFX", "JFTXXN"), "JFTXFN");
    }

    #[test]
    fn refined_fills_the_leads_unknown_letters_from_one_instrument_and_nothing_else() {
        let refined =
            |lead: &str, other: &str| Cfi::refined(lead, other).map(|code| code.to_string());
        for (lead, other, expected) in [
            ("ESXXXX", "ESVUFR", Some("ESVUFR")),
            ("ESVUFR", "ESXXXX", Some("ESVUFR")),
            ("ESVXXX", "ESXUFR", Some("ESVUFR")),
            ("ESVUFR", "ESVUFR", Some("ESVUFR")),
            ("JFTXFX", "JFTXXN", Some("JFTXFN")),
            ("XXXXXX", "ESVUFR", Some("ESVUFR")),
            ("ESZUFR", "ESVTFR", Some("ESVTFR")),
            ("ESVUFR", "XXXXXX", Some("ESVUFR")),
            ("ESXXXX", "EXXXXX", Some("ESXXXX")),
            ("XXXXXX", "XXXXXX", None),
            ("XXXXXX", "EXXXXX", None),
            ("ESVUFR", "ESNUFR", None),
            ("ESVTFR", "ESVUFR", None),
            ("ESXXXX", "DBXXXX", None),
            ("ESXXXX", "EPXXXX", None),
        ] {
            assert_eq!(
                refined(lead, other).as_deref(),
                expected,
                "{lead} refined by {other}"
            );
        }
    }
}

mod groups {
    use yggdryl::Cfi;

    #[test]
    fn a_non_deliverable_fx_forward_and_swap_classify() {
        // ISO 10962:2021, groups JF and SF: position 6 (delivery) lists `N`,
        // non-deliverable, beside physical and cash.
        for code in ["JFTXFN", "JFRXFN", "SFXXXN", "SFAXXN", "SFCXXN"] {
            assert!(Cfi::is_classified(code), "{code}");
            assert!(Cfi::is_detailed(code), "{code}");
        }
        for code in ["JFTXFP", "JFTXFC", "JFTXCC", "SFCXXP", "IFXXXP"] {
            assert!(Cfi::is_classified(code), "{code}");
        }
        // `Z` is no delivery at all, and a spot is always physical.
        assert!(!Cfi::is_classified("JFTXFZ"));
        assert!(!Cfi::is_classified("IFXXXN"));
    }
}
