//! `rust/src/bbg.rs`: the Bloomberg identifier, `bbg`, as one registered
//! code.
//!
//! The one code whose width is a bound rather than a shape: a ticker, a
//! market and a yellow key with spaces between them, or a FIGI, held to
//! thirty-two bytes with its case and its inner spaces kept.

mod value {
    use yggdryl::{Bbg, Error};

    #[test]
    fn an_empty_or_oversized_identifier_is_refused_naming_why() {
        assert!(matches!(
            Bbg::new(""),
            Err(Error::InvalidDataType { kind: "bbg", .. })
        ));
        assert_eq!(
            Bbg::new("").unwrap_err().to_string(),
            "invalid bbg datatype: expected a securities identifier, got \"\""
        );
        assert_eq!(
            Bbg::new("B".repeat(33)).unwrap_err().to_string(),
            "invalid record value at $: expected ASCII text of at most 32 bytes, got 33 bytes"
        );
        assert!(Bbg::new("AAPL\u{a0}US").is_err());
    }

    #[test]
    fn case_and_inner_spaces_are_kept_and_only_nul_padding_is_trimmed() {
        let apple = Bbg::new("aapl US Equity").unwrap();
        assert_eq!(apple.as_str(), "aapl US Equity");
        assert_eq!(apple.storage().as_str(), "aapl US Equity");
        assert_eq!(apple.to_string(), "aapl US Equity");
        assert_ne!(apple, Bbg::new("AAPL US Equity").unwrap());
        assert_eq!(
            Bbg::new("SPX Index\0\0").unwrap(),
            Bbg::new("SPX Index").unwrap()
        );
        let widest = "B".repeat(32);
        assert_eq!(Bbg::new(&widest).unwrap().as_str(), widest);

        assert!(Bbg::is_canonical("AAPL US Equity"));
        assert!(Bbg::is_canonical("BBG000B9XRY4"));
        assert!(!Bbg::is_canonical(""));
        assert!(!Bbg::is_canonical("SPX Index\0"));
        assert!(!Bbg::is_canonical(&"B".repeat(33)));
    }

    #[test]
    fn direct_serde_is_the_text() {
        let apple = Bbg::new("AAPL US Equity").unwrap();
        let rendered = serde_json::to_string(&apple).unwrap();
        assert_eq!(rendered, r#""AAPL US Equity""#);
        assert_eq!(serde_json::from_str::<Bbg>(&rendered).unwrap(), apple);
    }
}

mod datatype {
    use std::sync::Arc;

    use arrow_array::{Array, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, Bbg, BbgField, DataType, DataTypeId, DataTypeKind, Field, Scalar, Serie,
    };

    fn apple() -> Scalar {
        Scalar::Bbg(Bbg::new("AAPL US Equity").unwrap())
    }

    #[test]
    fn bbg_is_a_registered_code_the_grammar_spells_one_way() {
        assert_eq!(DataType::bbg(), DataType::Bbg);
        assert_eq!(DataType::from_str("bbg").unwrap(), DataType::Bbg);
        assert_eq!(DataType::from_str("BBG").unwrap(), DataType::Bbg);
        assert_eq!(DataType::from_logical_name("Bbg").unwrap(), DataType::Bbg);
        assert_eq!(DataType::Bbg.to_string(), "bbg");
        assert_eq!(DataType::Bbg.name(), "bbg");

        assert_eq!(DataType::Bbg.id(), DataTypeId::Bbg);
        assert_eq!(DataTypeId::Bbg.as_u8(), 0x7b);
        assert_eq!(DataTypeId::Bbg.as_str(), "bbg");
        assert_eq!(DataType::Bbg.kind(), DataTypeKind::Code);
        assert!(DataType::Bbg.is_code());
        assert_eq!(DataType::Bbg.code_name(), Some("bbg"));
        assert_eq!(DataType::Bbg.code_width(), Some(32));
        assert_eq!(DataType::Bbg.fixed_byte_width(), None);
        assert!(
            DataType::CODES
                .iter()
                .any(|entry| entry == &("bbg", DataType::Bbg, 32))
        );

        let json = DataType::Bbg.into_json().unwrap();
        assert_eq!(json, r#"{"type":"bbg"}"#);
        assert_eq!(DataType::from_json(&json).unwrap(), DataType::Bbg);
    }

    #[test]
    fn a_bbg_value_is_the_code_under_its_own_identity() {
        let value = DataType::Bbg
            .scalar(Scalar::from("AAPL US Equity"))
            .unwrap();
        assert_eq!(value, apple());
        assert_eq!(value.kind(), "bbg");
        assert_eq!(value.id(), DataTypeId::Bbg);
        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(wire, r#"{"type":"bbg","value":"AAPL US Equity"}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);

        // No neutral member: the default is refused and an empty text is
        // absence.
        assert!(DataType::Bbg.default_value().is_err());
        assert_eq!(DataType::Bbg.scalar("").unwrap(), Scalar::Null);

        let typed = BbgField::unit("bbg", true);
        assert_eq!(typed.to_field(), Field::new("bbg", DataType::Bbg, true));
    }

    #[test]
    fn a_bbg_column_crosses_arrow_as_text_under_its_own_extension() {
        let field = Field::new("bbg", DataType::Bbg, false);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.bbg");
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

        let stored = Serie::from_scalars(field.clone(), [apple()])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(cells.value(0), "AAPL US Equity");
        let back = Serie::from_arrow_array(
            Some(&field),
            Arc::clone(&stored),
            ArrowCastOptions::default(),
        )
        .unwrap();
        assert_eq!(back.scalar(0).unwrap(), apple());
    }
}
