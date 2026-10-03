//! `rust/src/figi.rs`: the Financial Instrument Global Identifier, `figi`.
//!
//! A registered code held by its shape - the consonants, the prefix the
//! standard reserves against an ISIN's, the `G` - and ranked by its own
//! check digit, so a typo is a value every merge replaces by a closing one
//! rather than a refusal. What the generic code invariants cannot pin is
//! the check itself, the prefix rule, the case fold, and the canonical
//! spelling a column is held to; those are here.

mod securities {

    use yggdryl::{CodeValue, Figi};

    #[test]
    fn a_figi_is_twelve_characters_with_its_own_check_and_prefix_rules() {
        let official = Figi::new("bbg000blnq16").expect("the OpenFIGI vector");
        assert_eq!(official.as_str(), "BBG000BLNQ16");
        assert!(Figi::is_closed("BBG000BLNQ16"));
        assert!(
            Figi::is_closed("BCG000000005"),
            "a permitted consonant prefix"
        );
        assert!(
            !Figi::is_closed("bbg000blnq16"),
            "lower case closes nothing"
        );
        assert!(!Figi::is_canonical("bbg000blnq16"));
        assert!(Figi::is_canonical("BBG000BLNQ17"), "a typo is a spelling");
        assert_eq!(official.rank(), 1);
        assert!(official.is_real());
        assert_eq!(<Figi as CodeValue>::MAX_RANK, 1);
        // One digit off is a typo: a value that does not close, of rank
        // zero, which a closing one replaces whichever leads.
        let typo = Figi::new("BBG000BLNQ17").unwrap();
        assert!(!Figi::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        assert_eq!(typo.clone().merge_with(&official), official);
        assert_eq!(official.clone().merge_with(&typo), official);
        // The shape stays the refusal: the length, a vowel, a reserved
        // prefix, the third letter, a closing letter, a byte outside ASCII
        // letters and digits.
        for invalid in [
            "BBG000BLNQ1",
            "BBG000BLNQ160",
            "BBG000BLNQ1A",
            "BAG000BLNQ16",
            "BSG000BLNQ16",
            "BBX000BLNQ16",
            "B?G000BLNQ16",
        ] {
            assert!(Figi::new(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn direct_figi_serde_validates_the_shape_and_canonicalizes_the_identifier() {
        let canonical = Figi::new("BBG000BLNQ16").unwrap();
        let rendered = serde_json::to_string(&canonical).unwrap();
        assert_eq!(rendered, r#""BBG000BLNQ16""#);
        assert_eq!(serde_json::from_str::<Figi>(&rendered).unwrap(), canonical);
        assert_eq!(
            serde_json::from_str::<Figi>(r#""bbg000blnq16""#).unwrap(),
            canonical
        );
        // A typo reads as the value it is; the shape is what the wire holds
        // to.
        assert_eq!(
            serde_json::from_str::<Figi>(r#""BBG000BLNQ17""#)
                .unwrap()
                .rank(),
            0
        );
        for invalid in [r#""""#, r#""BBG000BLNQ160""#, r#""BSG000BLNQ16""#] {
            assert!(
                serde_json::from_str::<Figi>(invalid).is_err(),
                "{invalid} bypassed FIGI validation"
            );
        }
    }
}
