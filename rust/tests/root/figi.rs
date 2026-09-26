//! `rust/src/figi.rs`: the Financial Instrument Global Identifier, `figi`.
//!
//! A registered code closed by its own check digit, so a value is an
//! identifier or is refused, never a typo stored as a security. What the
//! generic code invariants cannot pin is the check itself, the prefix rule,
//! the case fold, and the canonical spelling a column is held to; those are
//! here.

mod securities {

    use yggdryl::Figi;

    #[test]
    fn a_figi_is_twelve_characters_with_its_own_check_and_prefix_rules() {
        let official = Figi::new("bbg000blnq16").expect("the OpenFIGI vector");
        assert_eq!(official.as_str(), "BBG000BLNQ16");
        assert!(Figi::is_valid("BBG000BLNQ16"));
        assert!(
            Figi::is_valid("BCG000000005"),
            "a permitted consonant prefix"
        );
        assert!(!Figi::is_canonical("bbg000blnq16"));
        for invalid in [
            "BBG000BLNQ1",
            "BBG000BLNQ160",
            "BBG000BLNQ1A",
            "BAG000BLNQ16",
            "BSG000BLNQ16",
            "BBX000BLNQ16",
            "BBG000BLNQ17",
            "B?G000BLNQ16",
        ] {
            assert!(Figi::new(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn direct_figi_serde_validates_and_canonicalizes_the_identifier() {
        let canonical = Figi::new("BBG000BLNQ16").unwrap();
        let rendered = serde_json::to_string(&canonical).unwrap();
        assert_eq!(rendered, r#""BBG000BLNQ16""#);
        assert_eq!(serde_json::from_str::<Figi>(&rendered).unwrap(), canonical);
        assert_eq!(
            serde_json::from_str::<Figi>(r#""bbg000blnq16""#).unwrap(),
            canonical
        );
        for invalid in [r#""""#, r#""BBG000BLNQ17""#, r#""BBG000BLNQ160""#] {
            assert!(
                serde_json::from_str::<Figi>(invalid).is_err(),
                "{invalid} bypassed FIGI validation"
            );
        }
    }
}
