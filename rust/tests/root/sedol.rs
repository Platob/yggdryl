//! `rust/src/sedol.rs`: the two securities identifiers beside `isin`:
//! `cusip` and `sedol`.
//!
//! Each is a registered code closed by its own check digit, so a value is an
//! identifier or is refused, never a typo stored as a security. What the
//! generic code invariants in `coded` cannot pin is the check itself, the
//! case fold, and the canonical spelling a column is held to; those are
//! here, once per identifier, with the ISIN rule as the reference.

mod securities {

    use yggdryl::Sedol;

    #[test]
    fn a_sedol_is_seven_characters_closed_by_its_check_digit() {
        // Six alphanumerics weighted 1, 3, 1, 7, 3, 9 and the modulus-10 digit
        // that closes the weighted sum.
        let held = Sedol::new("B0YBKJ7").unwrap();
        assert_eq!(held.as_str(), "B0YBKJ7");
        assert_eq!(held.check_digit(), 7);
        assert_eq!(held.to_string(), "B0YBKJ7");
        assert_eq!(Sedol::new("0263494").unwrap().check_digit(), 4);
        assert_eq!(Sedol::new("B1F3M59").unwrap().check_digit(), 9);
        assert_eq!(Sedol::new("2046251").unwrap().check_digit(), 1);
        assert_eq!(Sedol::closing_digit("B0YBKJ"), Some(7));
        assert_eq!(Sedol::closing_digit("026349"), Some(4));
        assert_eq!(Sedol::closing_digit("B1F3M5"), Some(9));
        assert!(Sedol::is_valid("B0YBKJ7"));
        assert!(Sedol::is_valid("b0ybkj7"));
        assert!(Sedol::is_canonical("B0YBKJ7"));
        assert!(!Sedol::is_canonical("b0ybkj7"));

        // Lower case is the upper case it spells, and stores as that.
        assert_eq!(Sedol::new("b0ybkj7").unwrap(), held);
        assert_eq!(Sedol::new("b0ybkj7").unwrap().as_str(), "B0YBKJ7");

        // One digit off is a typo, and the refusal names the identifier.
        let refused = Sedol::new("B0YBKJ8").unwrap_err().to_string();
        assert!(refused.contains("sedol"), "{refused}");
        assert!(
            refused.contains("check digit does not close the identifier"),
            "{refused}"
        );
        assert!(!Sedol::is_valid("B0YBKJ8"));
        // The wrong length, in both directions.
        let short = Sedol::new("B0YBKJ").unwrap_err().to_string();
        assert!(short.contains("expected seven characters"), "{short}");
        let long = Sedol::new("B0YBKJ70").unwrap_err().to_string();
        assert!(long.contains("at most 7 bytes"), "{long}");
        // A character outside the alphanumerics, a closing letter, and a body
        // the rule cannot close.
        let punctuated = Sedol::new("B0Y-KJ7").unwrap_err().to_string();
        assert!(punctuated.contains("six alphanumerics"), "{punctuated}");
        let letter = Sedol::new("B0YBKJZ").unwrap_err().to_string();
        assert!(letter.contains("closing check digit"), "{letter}");
        assert_eq!(Sedol::closing_digit("B0Y-KJ"), None);
        assert_eq!(Sedol::closing_digit("B0YBK"), None);
        assert_eq!(Sedol::closing_digit("b0ybkj"), None);
    }
}
