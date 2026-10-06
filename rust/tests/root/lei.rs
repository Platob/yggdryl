//! `rust/src/lei.rs`: the ISO 17442 Legal Entity Identifier, `lei`.
//!
//! A registered code held by its shape - eighteen letters or digits and two
//! decimal check digits - and ranked by ISO 7064 MOD 97-10, so a typo is a
//! value every merge replaces by a closing one rather than a refusal. What
//! the generic code invariants cannot pin is the check itself, the 2020
//! reading of positions five and six, the case fold and the refusals; those
//! are here.

mod entities {

    use yggdryl::{CodeValue, DataType, Lei, Scalar};

    /// Published LEIs and the digits that close their first eighteen
    /// characters: Apple, Deutsche Bank and two of GLEIF's own.
    const PUBLISHED: [(&str, u8); 4] = [
        ("HWUPKR0MPOU8FGXBT394", 94),
        ("7LTWFZYICNSX8D621K86", 86),
        ("5493001KJTIIGC8Y1R12", 12),
        ("506700GE1G29325QX363", 63),
    ];

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "twenty"),
            ("HWUPKR0MPOU8FGXBT39", "twenty characters"),
            ("HWUPKR0MPOU8FGXBT3945", "at most 20"),
            ("HWUPKR0MPOU8FGXBT3A4", "two closing check digits"),
            ("HWUPKR0MPOU8FGXBT39X", "two closing check digits"),
            ("HWUPKR0MPOU8FGXBT-94", "eighteen letters or digits"),
            ("HWUPKR0MPOU8FGX T394", "eighteen letters or digits"),
        ] {
            let error = Lei::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Lei::new("HWUPKR0MPOU8FGXBT3A4").unwrap_err().to_string();
        assert!(error.contains("lei"), "{error}");
        assert!(error.contains("HWUPKR0MPOU8FGXBT3A4"), "{error}");
    }

    #[test]
    fn published_identifiers_close_under_mod_97_10() {
        for (lei, digits) in PUBLISHED {
            let code = Lei::new(lei).expect(lei);
            assert_eq!(code.as_str(), lei);
            assert!(Lei::is_closed(lei), "{lei}");
            assert_eq!(code.check_digits(), digits);
            assert_eq!(Lei::closing_digits(&lei[..18]), Some(digits), "{lei}");
            assert_eq!(code.rank(), 1);
            assert!(code.is_real());
        }
        assert_eq!(<Lei as CodeValue>::MAX_RANK, 1);
        assert_eq!(Lei::new("5493001KJTIIGC8Y1R12").unwrap().prefix(), "5493");
        // Not eighteen upper-case letters or digits: no digits close it.
        assert_eq!(Lei::closing_digits("HWUPKR0MPOU8FGXBT"), None);
        assert_eq!(Lei::closing_digits("hwupkr0mpou8fgxbt3"), None);
        assert_eq!(Lei::closing_digits("HWUPKR0MPOU8FGXB-3"), None);
    }

    #[test]
    fn positions_five_and_six_are_part_of_the_entity_code() {
        // ISO 17442:2012 reserved them as `00`; ISO 17442-1:2020 did not,
        // and Apple's (`KR`) and Deutsche Bank's (`FZ`) codes say so.
        for lei in ["HWUPKR0MPOU8FGXBT394", "7LTWFZYICNSX8D621K86"] {
            assert_ne!(&lei[4..6], "00");
            assert!(Lei::new(lei).is_ok(), "{lei}");
            assert!(Lei::is_closed(lei), "{lei}");
        }
    }

    #[test]
    fn lower_case_folds_once_and_a_typo_ranks_below_its_closing_code() {
        let apple = Lei::new("hwupkr0mpou8fgxbt394").unwrap();
        assert_eq!(apple.as_str(), "HWUPKR0MPOU8FGXBT394");
        assert!(!Lei::is_canonical("hwupkr0mpou8fgxbt394"));
        assert!(
            !Lei::is_closed("hwupkr0mpou8fgxbt394"),
            "lower case closes nothing"
        );
        assert!(
            Lei::is_canonical("HWUPKR0MPOU8FGXBT395"),
            "a typo is a spelling"
        );
        let typo = Lei::new("HWUPKR0MPOU8FGXBT395").unwrap();
        assert!(!Lei::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        assert_eq!(typo.clone().merge_with(&apple), apple);
        assert_eq!(apple.clone().merge_with(&typo), apple);
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("lei").unwrap();
        assert_eq!(dtype, DataType::lei());
        assert_eq!(dtype.to_string(), "lei");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(20));
        let stored = dtype.scalar(Scalar::from("hwupkr0mpou8fgxbt394")).unwrap();
        assert_eq!(
            stored,
            Scalar::Lei(Lei::new("HWUPKR0MPOU8FGXBT394").unwrap())
        );
        assert_eq!(stored.dtype().unwrap(), DataType::lei());
        assert!(dtype.scalar(Scalar::from("HWUPKR0MPOU8FGXBT3")).is_err());
    }

    #[test]
    fn direct_lei_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Lei::new("HWUPKR0MPOU8FGXBT394").unwrap();
        let rendered = serde_json::to_string(&canonical).unwrap();
        assert_eq!(rendered, r#""HWUPKR0MPOU8FGXBT394""#);
        assert_eq!(serde_json::from_str::<Lei>(&rendered).unwrap(), canonical);
        assert_eq!(
            serde_json::from_str::<Lei>(r#""hwupkr0mpou8fgxbt394""#).unwrap(),
            canonical
        );
        assert_eq!(
            serde_json::from_str::<Lei>(r#""HWUPKR0MPOU8FGXBT395""#)
                .unwrap()
                .rank(),
            0
        );
        for invalid in [
            r#""""#,
            r#""HWUPKR0MPOU8FGXBT39""#,
            r#""HWUPKR0MPOU8FGXBT3A4""#,
        ] {
            assert!(
                serde_json::from_str::<Lei>(invalid).is_err(),
                "{invalid} bypassed LEI validation"
            );
        }
    }
}
