//! `rust/src/bic.rs`: the ISO 9362 Business Identifier Code, `bic`.
//!
//! A registered code held by its shape - a party prefix, a country, a
//! location and an optional branch - and ranked by its country, since a BIC
//! carries no check character. What the generic code invariants cannot pin
//! is the 2014 alphanumeric prefix, the two lengths kept as stated, the
//! readings of the four parts, SWIFT's `XK`, and the refusals; those are
//! here.

mod parties {

    use yggdryl::{Bic, CodeValue, DataType, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "eight or eleven characters"),
            ("DEUTDEF", "eight or eleven characters"),
            ("DEUTDEFF5", "eight or eleven characters"),
            ("DEUTDEFF50", "eight or eleven characters"),
            ("DEUTDEFF5000", "at most 11"),
            ("DEU-DEFF", "party prefix of letters or digits"),
            ("DEUT1EFF", "two-letter country code"),
            ("DEUTDEF-", "location of letters or digits"),
            ("DEUTDEFF5-0", "branch of letters or digits"),
            ("DEUT DEFF", "eight or eleven characters"),
        ] {
            let error = Bic::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Bic::new("DEUT1EFF").unwrap_err().to_string();
        assert!(error.contains("bic"), "{error}");
        assert!(error.contains("DEUT1EFF"), "{error}");
    }

    #[test]
    fn eight_and_eleven_characters_are_kept_as_stated() {
        let primary = Bic::new("DEUTDEFF").unwrap();
        let spelled = Bic::new("DEUTDEFFXXX").unwrap();
        assert_eq!(primary.as_str(), "DEUTDEFF");
        assert_eq!(spelled.as_str(), "DEUTDEFFXXX");
        assert_ne!(primary, spelled, "no branch is folded away");
        assert!(primary.is_primary_office());
        assert!(spelled.is_primary_office());
        assert_eq!(primary.branch(), None);
        assert_eq!(spelled.branch(), Some("XXX"));
        let branch = Bic::new("deutdeff500").unwrap();
        assert_eq!(branch.as_str(), "DEUTDEFF500");
        assert_eq!(branch.party_prefix(), "DEUT");
        assert_eq!(branch.country(), "DE");
        assert_eq!(branch.location(), "FF");
        assert_eq!(branch.branch(), Some("500"));
        assert!(!branch.is_primary_office());
        assert!(Bic::is_canonical("DEUTDEFF500"));
        assert!(!Bic::is_canonical("deutdeff500"));
        assert!(!Bic::is_canonical("DEUTDEFF50"));
    }

    #[test]
    fn a_prefix_may_hold_digits_since_iso_9362_2014() {
        let code = Bic::new("1234DEFF").unwrap();
        assert_eq!(code.party_prefix(), "1234");
        assert!(code.is_real());
    }

    #[test]
    fn the_rank_is_the_country_listed_or_kosovo() {
        assert_eq!(<Bic as CodeValue>::MAX_RANK, 1);
        assert_eq!(Bic::new("DEUTDEFF").unwrap().rank(), 1);
        assert_eq!(Bic::new("RBKOXKPR").unwrap().rank(), 1, "SWIFT's XK");
        let unlisted = Bic::new("DEUTZZFF").unwrap();
        assert_eq!(unlisted.rank(), 0);
        let listed = Bic::new("DEUTDEFF").unwrap();
        assert_eq!(unlisted.clone().merge_with(&listed), listed);
        assert_eq!(listed.clone().merge_with(&unlisted), listed);
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("bic").unwrap();
        assert_eq!(dtype, DataType::bic());
        assert_eq!(dtype.to_string(), "bic");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(11));
        let stored = dtype.scalar(Scalar::from("deutdeff")).unwrap();
        assert_eq!(stored, Scalar::Bic(Bic::new("DEUTDEFF").unwrap()));
        assert!(dtype.scalar(Scalar::from("DEUTDEFF5")).is_err());
    }

    #[test]
    fn direct_bic_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Bic::new("DEUTDEFF").unwrap();
        let rendered = serde_json::to_string(&canonical).unwrap();
        assert_eq!(rendered, r#""DEUTDEFF""#);
        assert_eq!(
            serde_json::from_str::<Bic>(r#""deutdeff""#).unwrap(),
            canonical
        );
        for invalid in [r#""""#, r#""DEUTDEFF5""#, r#""DEUT1EFF""#] {
            assert!(
                serde_json::from_str::<Bic>(invalid).is_err(),
                "{invalid} bypassed BIC validation"
            );
        }
    }
}
