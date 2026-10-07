//! `rust/src/fisn.rs`: the ISO 18774 Financial Instrument Short Name,
//! `fisn`.
//!
//! A registered code held by its shape - an issuer and a description either
//! side of the first `/`, at most thirty-five printable US-ASCII bytes -
//! with no check character, so every well-shaped value ranks the same. What
//! the generic code invariants cannot pin is the first-`/` split, what the
//! guidelines admit that a stricter reading would refuse, the case fold and
//! the refusals; those are here. It is also the newest code, last in the
//! listing.

mod short_names {

    use yggdryl::{CodeValue, DataType, Fisn, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "a '/' between the issuer"),
            ("ACME CORP SH", "a '/' between the issuer"),
            ("/SH", "an issuer name before the '/'"),
            ("ACME CORP/", "an instrument description after the '/'"),
            ("ACME\tCORP/SH", "printable characters"),
            ("ACME CORP/SH\u{7f}", "printable characters"),
            ("ACME CORPORATION INCORPORATED/ORD SH", "at most 35"),
            ("SOCIÉTÉ/SH", "non-ASCII"),
        ] {
            let error = Fisn::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Fisn::new("ACME CORP SH").unwrap_err().to_string();
        assert!(error.contains("fisn"), "{error}");
        assert!(error.contains("ACME CORP SH"), "{error}");
    }

    #[test]
    fn the_first_slash_splits_and_the_guidelines_examples_are_admitted() {
        let name = Fisn::new("acme corp/sh").unwrap();
        assert_eq!(name.as_str(), "ACME CORP/SH");
        assert_eq!(name.issuer(), "ACME CORP");
        assert_eq!(name.description(), "SH");
        // A second `/` and a dot belong to the description.
        let note = Fisn::new("XXXXXXXXX/4.7 CD 20260123 UNSEC/UNG").unwrap();
        assert_eq!(note.issuer(), "XXXXXXXXX");
        assert_eq!(note.description(), "4.7 CD 20260123 UNSEC/UNG");
        assert_eq!(
            Fisn::new("ACME/AMORT PN W/P/C").unwrap().description(),
            "AMORT PN W/P/C"
        );
        // A collective investment vehicle's issuer may run past fifteen.
        let fund = Fisn::new("ACME GLOBAL EQUITY F/UNITS").unwrap();
        assert_eq!(fund.issuer().len(), 20);
        // Exactly thirty-five bytes is the bound, not past it.
        let widest = "ACME CORPORATION INCORPORAT/ORD SHS";
        assert_eq!(widest.len(), 35);
        assert_eq!(Fisn::new(widest).unwrap().as_str(), widest);
        assert!(Fisn::is_canonical("ACME CORP/SH"));
        assert!(!Fisn::is_canonical("acme corp/sh"));
    }

    #[test]
    fn every_short_name_ranks_the_same() {
        assert_eq!(<Fisn as CodeValue>::MAX_RANK, 1);
        assert_eq!(Fisn::new("ACME CORP/SH").unwrap().rank(), 1);
    }

    #[test]
    fn the_datatype_reads_its_name_and_is_the_newest_code() {
        let dtype = DataType::from_str("fisn").unwrap();
        assert_eq!(dtype, DataType::fisn());
        assert_eq!(dtype.to_string(), "fisn");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(35));
        let stored = dtype.scalar(Scalar::from("acme corp/sh")).unwrap();
        assert_eq!(stored, Scalar::Fisn(Fisn::new("ACME CORP/SH").unwrap()));
        assert!(dtype.scalar(Scalar::from("ACME CORP")).is_err());
        assert_eq!(DataType::CODES.last(), Some(&("fisn", DataType::Fisn, 35)));
    }

    #[test]
    fn direct_fisn_serde_validates_the_shape_and_canonicalizes_the_name() {
        let canonical = Fisn::new("ACME CORP/SH").unwrap();
        assert_eq!(
            serde_json::to_string(&canonical).unwrap(),
            r#""ACME CORP/SH""#
        );
        assert_eq!(
            serde_json::from_str::<Fisn>(r#""acme corp/sh""#).unwrap(),
            canonical
        );
        for invalid in [r#""""#, r#""ACME CORP""#, r#""/SH""#] {
            assert!(
                serde_json::from_str::<Fisn>(invalid).is_err(),
                "{invalid} bypassed FISN validation"
            );
        }
    }
}

/// `Fisn::similarity`: one less the Levenshtein distance over the longer
/// length, symmetric, `1` for equal names, and never more than the length
/// bound the prefilter reads - so a pair the prefilter rejects at a
/// threshold is below it.
mod similarity {

    use yggdryl::Fisn;

    fn name(text: &str) -> Fisn {
        Fisn::new(text).unwrap()
    }

    #[test]
    fn identical_names_are_one_and_the_score_is_symmetric() {
        let apple = name("APPLE INC/SH");
        assert_eq!(apple.similarity(&apple), 1.0);
        assert_eq!(apple.similarity(&name("apple inc/sh")), 1.0, "both folded");
        for (left, right) in [
            ("APPLE INC/SH", "APPLE INC./SH"),
            ("HSBC HLDG/PAR VTG FPD 0.5", "HSBC HLDGS/ORD USD0.50"),
            ("A/B", "ACME CORPORATION INCORPORAT/ORD SHS"),
        ] {
            assert_eq!(
                name(left).similarity(&name(right)),
                name(right).similarity(&name(left)),
                "{left} and {right}"
            );
        }
    }

    #[test]
    fn one_edit_in_thirteen_passes_the_default_threshold() {
        let score = name("APPLE INC/SH").similarity(&name("APPLE INC./SH"));
        assert!((score - 12.0 / 13.0).abs() < 1e-12, "{score}");
        assert!(score >= yggdryl::IsinRegistry::DEFAULT_ECONOMIC_THRESHOLD);
        assert_eq!(name("ACME CORP/SH").similarity(&name("ACME CORP/SH")), 1.0);
        assert_eq!(
            name("AB/C").similarity(&name("XY/Z")),
            0.25,
            "three of four"
        );
    }

    /// The distance is at least the lengths' difference, so the length
    /// bound is an upper bound - reached where the longer name only adds -
    /// and a pair it puts below the threshold is below it.
    #[test]
    fn the_length_bound_is_never_passed() {
        let short = name("APPLE INC/SH");
        let long = name("APPLE INC/SH USD");
        assert_eq!(
            short.similarity(&long),
            0.75,
            "1 - 4 / 16, the bound reached"
        );
        for (left, right) in [
            ("APPLE INC/SH", "XYZ INC/SH USD"),
            ("A/B", "APPLE INC/SH"),
            ("APPLE INC./SH", "APPLE INC/SH"),
        ] {
            let (left, right) = (name(left), name(right));
            let (a, b) = (left.as_str().len() as f64, right.as_str().len() as f64);
            let bound = 1.0 - (a - b).abs() / a.max(b);
            assert!(left.similarity(&right) <= bound, "{left} and {right}");
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::fisn::{below_threshold, similarity};

    #[test]
    fn two_empty_texts_are_identical() {
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("", "AB/C"), 0.0);
        assert_eq!(similarity("AB/C", ""), 0.0);
    }

    /// The prefilter rejects exactly what the length bound puts below the
    /// threshold: four bytes in sixteen at `0.85` - the bound `0.75` - and
    /// not two in sixteen - the bound `0.875`, which the score may reach.
    #[test]
    fn the_prefilter_rejects_only_below_the_threshold() {
        assert!(below_threshold(12, 16, 0.85));
        assert!(below_threshold(16, 12, 0.85), "symmetric");
        assert!(!below_threshold(14, 16, 0.85));
        assert!(!below_threshold(12, 13, 0.85));
        assert!(!below_threshold(12, 16, 0.75), "at the bound, not below it");
        assert!(!below_threshold(16, 16, 1.0));
        assert!(below_threshold(15, 16, 1.0));
        // A text past the short name's width is scored all the same.
        let long = "A".repeat(40);
        assert_eq!(similarity(&long, &long), 1.0);
        assert_eq!(similarity(&long, &"A".repeat(20)), 0.5);
    }
}
