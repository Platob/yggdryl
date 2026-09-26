//! `rust/src/cusip.rs`: the two securities identifiers beside `isin`:
//! `cusip` and `sedol`.
//!
//! Each is a registered code closed by its own check digit, so a value is an
//! identifier or is refused, never a typo stored as a security. What the
//! generic code invariants in `coded` cannot pin is the check itself, the
//! case fold, and the canonical spelling a column is held to; those are
//! here, once per identifier, with the ISIN rule as the reference.

mod securities {

    use yggdryl::Cusip;

    #[test]
    fn a_cusip_is_nine_characters_closed_by_its_check_digit() {
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
        // The rule answers without building a value, in either case.
        assert!(Cusip::is_valid("037833100"));
        assert!(Cusip::is_valid("38259p508"));
        assert!(Cusip::is_canonical("38259P508"));
        assert!(!Cusip::is_canonical("38259p508"));

        // Lower case is the upper case it spells, and stores as that.
        assert_eq!(
            Cusip::new("38259p508").unwrap(),
            Cusip::new("38259P508").unwrap()
        );
        assert_eq!(Cusip::new("38259p508").unwrap().as_str(), "38259P508");

        // One digit off is a typo, and the refusal names the identifier.
        let refused = Cusip::new("037833101").unwrap_err().to_string();
        assert!(refused.contains("cusip"), "{refused}");
        assert!(
            refused.contains("check digit does not close the identifier"),
            "{refused}"
        );
        assert!(!Cusip::is_valid("037833101"));
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
