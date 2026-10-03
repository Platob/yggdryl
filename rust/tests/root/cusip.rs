//! `rust/src/cusip.rs`: the CUSIP securities identifier beside `isin`.
//!
//! A registered code held by its shape and ranked by its own check digit,
//! so a typo is a value every merge replaces by a closing one rather than
//! a refusal. What the generic code invariants in `coded` cannot pin is the
//! check itself, the case fold, and the canonical spelling a column is held
//! to; those are here, once per identifier, with the ISIN rule as the
//! reference.

mod securities {

    use yggdryl::{CodeValue, Cusip};

    #[test]
    fn a_cusip_is_nine_characters_ranked_by_its_check_digit() {
        // Six of issuer, two of issue, one check digit: the modulus-10
        // double-add-double digit of the eight before it.
        let apple = Cusip::new("037833100").unwrap();
        assert_eq!(apple.as_str(), "037833100");
        assert_eq!(apple.issuer(), "037833");
        assert_eq!(apple.issue(), "10");
        assert_eq!(apple.check_digit(), 0);
        assert_eq!(apple.to_string(), "037833100");
        // A letter reads as ten plus its alphabet position.
        assert_eq!(Cusip::new("38259P508").unwrap().check_digit(), 8);
        assert_eq!(Cusip::new("594918104").unwrap().check_digit(), 4);
        assert_eq!(Cusip::closing_digit("03783310"), Some(0));
        assert_eq!(Cusip::closing_digit("38259P50"), Some(8));
        assert_eq!(Cusip::closing_digit("59491810"), Some(4));
        // The readings answer without building a value: canonical is the
        // upper-case shape, closing the check digit.
        assert!(Cusip::is_closed("037833100"));
        assert!(!Cusip::is_closed("38259p508"), "lower case closes nothing");
        assert!(Cusip::is_canonical("38259P508"));
        assert!(!Cusip::is_canonical("38259p508"));
        assert!(Cusip::is_canonical("037833101"), "a typo is a spelling");

        // Lower case is the upper case it spells, and stores as that.
        assert_eq!(
            Cusip::new("38259p508").unwrap(),
            Cusip::new("38259P508").unwrap()
        );
        assert_eq!(Cusip::new("38259p508").unwrap().as_str(), "38259P508");

        // One digit off is a typo: a value that does not close, of rank
        // zero, which a closing one replaces whichever leads.
        let typo = Cusip::new("037833101").unwrap();
        assert!(!Cusip::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        assert!(!typo.is_real());
        assert_eq!(apple.rank(), 1);
        assert!(apple.is_real());
        assert_eq!(<Cusip as CodeValue>::MAX_RANK, 1);
        assert_eq!(typo.clone().merge_with(&apple), apple);
        assert_eq!(apple.clone().merge_with(&typo), apple);
        let other_typo = Cusip::new("037833102").unwrap();
        assert_eq!(typo.clone().merge_with(&other_typo), typo);
        let microsoft = Cusip::new("594918104").unwrap();
        assert_eq!(apple.clone().merge_with(&microsoft), apple);
        // The wrong length, in both directions.
        let short = Cusip::new("03783310").unwrap_err().to_string();
        assert!(short.contains("expected nine characters"), "{short}");
        let long = Cusip::new("0378331000").unwrap_err().to_string();
        assert!(long.contains("at most 9 bytes"), "{long}");
        // A character outside the alphanumerics, a closing letter, and a body
        // the rule cannot close.
        let punctuated = Cusip::new("03783*100").unwrap_err().to_string();
        assert!(punctuated.contains("eight alphanumerics"), "{punctuated}");
        let letter = Cusip::new("03783310A").unwrap_err().to_string();
        assert!(letter.contains("closing check digit"), "{letter}");
        assert_eq!(Cusip::closing_digit("03783*10"), None);
        assert_eq!(Cusip::closing_digit("0378331"), None);
        assert_eq!(Cusip::closing_digit("38259p50"), None);
    }
}
