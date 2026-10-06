//! `rust/src/elf.rs`: the ISO 20275 Entity Legal Form code, `elf`.
//!
//! A registered code held by its shape - four letters or digits - with no
//! check character and no placeholder, so every well-shaped value ranks the
//! same. What the generic code invariants cannot pin is the shape, the case
//! fold and the refusals; those are here.

mod legal_forms {

    use yggdryl::{CodeValue, DataType, Elf, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "four characters"),
            ("2HB", "four characters"),
            ("2HBR5", "at most 4"),
            ("2H-R", "four letters or digits"),
            ("2H R", "four letters or digits"),
        ] {
            let error = Elf::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Elf::new("2H-R").unwrap_err().to_string();
        assert!(error.contains("elf"), "{error}");
        assert!(error.contains("2H-R"), "{error}");
    }

    #[test]
    fn a_legal_form_folds_to_upper_case_and_every_one_ranks_the_same() {
        let gmbh = Elf::new("2hbr").unwrap();
        assert_eq!(gmbh.as_str(), "2HBR");
        assert_eq!(gmbh, Elf::new("2HBR").unwrap());
        assert!(Elf::is_canonical("2HBR"));
        assert!(!Elf::is_canonical("2hbr"));
        assert_eq!(<Elf as CodeValue>::MAX_RANK, 1);
        for code in ["2HBR", "8888", "9999", "ZZZZ"] {
            assert_eq!(Elf::new(code).unwrap().rank(), 1, "{code}");
        }
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("elf").unwrap();
        assert_eq!(dtype, DataType::elf());
        assert_eq!(dtype.to_string(), "elf");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(4));
        let stored = dtype.scalar(Scalar::from("2hbr")).unwrap();
        assert_eq!(stored, Scalar::Elf(Elf::new("2HBR").unwrap()));
        assert!(dtype.scalar(Scalar::from("2HB")).is_err());
    }

    #[test]
    fn direct_elf_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Elf::new("2HBR").unwrap();
        assert_eq!(serde_json::to_string(&canonical).unwrap(), r#""2HBR""#);
        assert_eq!(serde_json::from_str::<Elf>(r#""2hbr""#).unwrap(), canonical);
        for invalid in [r#""""#, r#""2HB""#, r#""2H-R""#] {
            assert!(
                serde_json::from_str::<Elf>(invalid).is_err(),
                "{invalid} bypassed ELF validation"
            );
        }
    }
}
