//! `rust/src/dti.rs`: the ISO 24165 Digital Token Identifier, `dti`.
//!
//! A registered code held by its shape - nine symbols of a thirty-symbol
//! alphabet, never opening with `0` - and ranked by ISO 7064 hybrid MOD
//! 31,30, so a typo is a value every merge replaces by a closing one rather
//! than a refusal. What the generic code invariants cannot pin is the
//! alphabet, the check itself, the case fold and the refusals; those are
//! here.

mod tokens {

    use yggdryl::{CodeValue, DataType, Dti, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "nine characters"),
            ("X9J9K872", "nine characters"),
            ("X9J9K872S0", "at most 9"),
            ("09J9K872S", "first character other than 0"),
            ("A9J9K872S", "consonants other than Y"),
            ("X9J9K8Y2S", "consonants other than Y"),
            ("X9J9K8-2S", "consonants other than Y"),
        ] {
            let error = Dti::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Dti::new("A9J9K872S").unwrap_err().to_string();
        assert!(error.contains("dti"), "{error}");
        assert!(error.contains("A9J9K872S"), "{error}");
    }

    #[test]
    fn the_check_character_closes_under_hybrid_mod_31_30() {
        let token = Dti::new("X9J9K872S").unwrap();
        assert!(Dti::is_closed("X9J9K872S"));
        assert_eq!(Dti::closing_character("X9J9K872"), Some('S'));
        assert_eq!(token.check_character(), 'S');
        assert_eq!(token.rank(), 1);
        assert!(token.is_real());
        assert_eq!(<Dti as CodeValue>::MAX_RANK, 1);
        // Not eight upper-case symbols of the alphabet: nothing closes it.
        assert_eq!(Dti::closing_character("X9J9K87"), None);
        assert_eq!(Dti::closing_character("x9j9k872"), None);
        assert_eq!(Dti::closing_character("A9J9K872"), None);
    }

    #[test]
    fn the_check_is_the_rule_and_a_code_it_does_not_close_ranks_as_a_typo() {
        let typo = Dti::new("X9J9K872T").unwrap();
        assert!(!Dti::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        let token = Dti::new("X9J9K872S").unwrap();
        assert_eq!(typo.clone().merge_with(&token), token);
        assert_eq!(token.clone().merge_with(&typo), token);
        // A registry code whose stored character the algorithm does not
        // give - the hybrid check over `4H95J0R2` gives `T` - is a typo
        // too: there is no exception table.
        assert_eq!(Dti::closing_character("4H95J0R2"), Some('T'));
        assert_eq!(Dti::new("4H95J0R2X").unwrap().rank(), 0);
    }

    #[test]
    fn lower_case_folds_once() {
        let token = Dti::new("x9j9k872s").unwrap();
        assert_eq!(token.as_str(), "X9J9K872S");
        assert!(Dti::is_canonical("X9J9K872S"));
        assert!(!Dti::is_canonical("x9j9k872s"));
        assert!(!Dti::is_closed("x9j9k872s"), "lower case closes nothing");
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("dti").unwrap();
        assert_eq!(dtype, DataType::dti());
        assert_eq!(dtype.to_string(), "dti");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(9));
        let stored = dtype.scalar(Scalar::from("x9j9k872s")).unwrap();
        assert_eq!(stored, Scalar::Dti(Dti::new("X9J9K872S").unwrap()));
        assert!(dtype.scalar(Scalar::from("09J9K872S")).is_err());
    }

    #[test]
    fn direct_dti_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Dti::new("X9J9K872S").unwrap();
        assert_eq!(serde_json::to_string(&canonical).unwrap(), r#""X9J9K872S""#);
        assert_eq!(
            serde_json::from_str::<Dti>(r#""x9j9k872s""#).unwrap(),
            canonical
        );
        for invalid in [r#""""#, r#""X9J9K872""#, r#""A9J9K872S""#] {
            assert!(
                serde_json::from_str::<Dti>(invalid).is_err(),
                "{invalid} bypassed DTI validation"
            );
        }
    }
}
