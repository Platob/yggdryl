//! `rust/src/ric.rs`: the Refinitiv Identification Code, LSEG's ticker-like
//! instrument code, as one registered code.
//!
//! No registry publishes the set and no check digit closes a code, so the
//! rule is its shape - one token of printable ASCII, at most thirty-two
//! bytes, its case kept - and these pin that rule, the exchange mnemonic a
//! code names, and the datatype that holds a column of them.

mod value {
    use yggdryl::{Error, Ric};

    #[test]
    fn a_ric_that_is_not_one_token_of_printable_ascii_is_refused_naming_why() {
        let reason = |text: &str| Ric::new(text).unwrap_err().to_string();

        // Empty: no code at all, refused by the RIC rule itself.
        assert!(matches!(
            Ric::new(""),
            Err(Error::InvalidDataType { kind: "ric", .. })
        ));
        assert_eq!(
            reason(""),
            "invalid ric datatype: expected a Refinitiv Identification Code, got \"\""
        );
        // A space or a control byte splits the token, wherever it stands.
        assert!(matches!(
            Ric::new("VOD L"),
            Err(Error::InvalidDataType { kind: "ric", .. })
        ));
        assert_eq!(
            reason("VOD L"),
            "invalid ric datatype: expected one token of printable ASCII, got 0x20 at 3 in \"VOD L\""
        );
        assert_eq!(
            reason("IBM\t.N"),
            "invalid ric datatype: expected one token of printable ASCII, got 0x09 at 3 in \"IBM\\t.N\""
        );
        assert_eq!(
            reason("IBM\u{7f}"),
            "invalid ric datatype: expected one token of printable ASCII, got 0x7F at 3 in \"IBM\\u{7f}\""
        );
        // Only NUL padding is trimmed: a trailing space is still a space.
        assert!(reason("VOD.L ").contains("0x20 at 5"));
        // The repertoire and the width are the shared ASCII rule's refusals.
        assert_eq!(
            reason("VOD.L\u{20ac}"),
            "invalid record value at $: expected ASCII text of at most 32 bytes, got a non-ASCII byte 0xE2 at 5"
        );
        assert_eq!(
            reason("IBM\0.N"),
            "invalid record value at $: expected ASCII text of at most 32 bytes, got a NUL byte at 3"
        );
        assert_eq!(
            reason(&"R".repeat(33)),
            "invalid record value at $: expected ASCII text of at most 32 bytes, got 33 bytes"
        );
    }

    #[test]
    fn every_documented_form_is_a_ric_and_reads_back_as_itself() {
        for text in [
            "IBM.N", "VOD.L", "0005.HK", "AAPL.OQ", ".SPX", "0#.FTSE", "EUR=", "ESc1",
        ] {
            let ric = Ric::new(text).unwrap_or_else(|error| panic!("{text}: {error}"));
            assert_eq!(ric.as_str(), text);
            assert_eq!(ric.storage().as_str(), text);
            assert_eq!(ric.to_string(), text);
            assert!(Ric::is_canonical(text), "{text}");
        }
        // Exactly the width fits.
        let widest = "R".repeat(32);
        assert_eq!(Ric::new(&widest).unwrap().as_str(), widest);
        assert!(Ric::is_canonical(&widest));
    }

    #[test]
    fn case_is_part_of_the_code_and_only_nul_padding_is_trimmed() {
        // A continuation future and the same letters upper-cased are two
        // codes: nothing folds.
        let future = Ric::new("ESc1").unwrap();
        assert_eq!(future.as_str(), "ESc1");
        assert_ne!(future, Ric::new("ESC1").unwrap());
        assert!(Ric::is_canonical("ESc1"));

        // The padding a fixed slot wrote is the slot's, not the code's.
        assert_eq!(Ric::new("VOD.L\0\0\0").unwrap(), Ric::new("VOD.L").unwrap());
        assert!(!Ric::is_canonical("VOD.L\0"));
        assert!(!Ric::is_canonical(""));
        assert!(!Ric::is_canonical("VOD L"));
        assert!(!Ric::is_canonical(&"R".repeat(33)));
    }

    #[test]
    fn the_exchange_code_is_the_mnemonic_after_a_tickers_last_period() {
        for (text, exchange) in [
            ("IBM.N", Some("N")),
            ("VOD.L", Some("L")),
            ("0005.HK", Some("HK")),
            ("AAPL.OQ", Some("OQ")),
            ("BRKb.N", Some("N")),
            ("A.B.C", Some("C")),
            // An index opens on its period, a chain carries `#`, a quote and
            // a continuation have no period at all, and a period with
            // nothing after it names nothing.
            (".SPX", None),
            ("0#.FTSE", None),
            ("EUR=", None),
            ("ESc1", None),
            ("IBM.", None),
        ] {
            assert_eq!(Ric::new(text).unwrap().exchange_code(), exchange, "{text}");
        }
        // The mnemonic is what resolves the market's MIC.
        let hong_kong = Ric::new("0005.HK").unwrap();
        assert_eq!(
            yggdryl::Mic::from_reuters_exchange_code(hong_kong.exchange_code().unwrap())
                .unwrap()
                .as_str(),
            "XHKG"
        );
    }

    #[test]
    fn direct_serde_is_the_text_and_validates_on_the_way_in() {
        let ric = Ric::new("AAPL.OQ").unwrap();
        let rendered = serde_json::to_string(&ric).unwrap();
        assert_eq!(rendered, r#""AAPL.OQ""#);
        assert_eq!(serde_json::from_str::<Ric>(&rendered).unwrap(), ric);
        // Case survives the wire as it survives the constructor.
        assert_eq!(
            serde_json::from_str::<Ric>(r#""ESc1""#).unwrap().as_str(),
            "ESc1"
        );
        for invalid in [r#""""#, r#""VOD L""#, r#""IBM\t.N""#] {
            let refused = serde_json::from_str::<Ric>(invalid)
                .unwrap_err()
                .to_string();
            assert!(
                refused.contains("invalid ric datatype"),
                "{invalid}: {refused}"
            );
        }
        let over = format!("\"{}\"", "R".repeat(33));
        assert!(serde_json::from_str::<Ric>(&over).is_err());
    }
}

mod datatype {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, Ric, RicField, Scalar, Serie,
    };

    fn text(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    fn apple() -> Scalar {
        Scalar::Ric(Ric::new("AAPL.OQ").unwrap())
    }

    #[test]
    fn ric_is_a_registered_code_the_grammar_spells_one_way() {
        assert_eq!(DataType::ric(), DataType::Ric);
        assert_eq!(DataType::from_str("ric").unwrap(), DataType::Ric);
        assert_eq!(DataType::from_str("RIC").unwrap(), DataType::Ric);
        assert_eq!(DataType::from_logical_name("Ric").unwrap(), DataType::Ric);
        assert_eq!(DataType::Ric.to_string(), "ric");
        assert_eq!(DataType::Ric.name(), "ric");

        // Identity: the discriminant, the family, the width, and the listing.
        assert_eq!(DataType::Ric.id(), DataTypeId::Ric);
        assert_eq!(DataTypeId::Ric.as_u8(), 0x7e);
        assert_eq!(DataTypeId::Ric.as_str(), "ric");
        assert_eq!(DataTypeId::from_str("ric").unwrap(), DataTypeId::Ric);
        assert_eq!(DataType::Ric.kind(), DataTypeKind::Code);
        assert!(DataType::Ric.is_code());
        assert!(!DataType::Ric.is_string());
        assert_eq!(DataType::Ric.code_name(), Some("ric"));
        assert_eq!(DataType::Ric.code_width(), Some(32));
        assert_eq!(DataType::Ric.fixed_byte_width(), None);
        assert_eq!(
            DataType::CODES.last(),
            Some(&("ric", DataType::Ric, 32)),
            "the newest code is listed last"
        );

        // The datatype's own wire is the one spelling, and it round-trips.
        let json = DataType::Ric.into_json().unwrap();
        assert_eq!(json, r#"{"type":"ric"}"#);
        assert_eq!(DataType::from_json(&json).unwrap(), DataType::Ric);
        let rendered = serde_json::to_string(&DataType::Ric).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&rendered).unwrap(),
            DataType::Ric
        );
    }

    #[test]
    fn a_ric_field_is_the_typed_marker_over_the_variant() {
        let field = Field::new("ric", DataType::Ric, true);
        let typed = RicField::unit("ric", true);
        assert_eq!(typed.dtype(), &DataType::Ric);
        assert_eq!(typed.to_field(), field);
        let shared = DataType::Ric.shared_field().unwrap();
        assert_eq!(shared.dtype(), &DataType::Ric);
    }

    #[test]
    fn a_ric_value_is_the_code_under_its_own_identity() {
        let value = DataType::Ric.scalar(Scalar::from("AAPL.OQ")).unwrap();
        assert_eq!(value, apple());
        assert!(value.is_code());
        assert_eq!(value.id(), DataTypeId::Ric);
        assert_eq!(value.kind(), "ric");
        assert_eq!(value.as_str(), Some("AAPL.OQ"));
        assert_eq!(value.dtype().unwrap(), DataType::Ric);
        assert_eq!(DataType::Ric.scalar(apple()).unwrap(), apple());
        // Case is kept through the datatype door too.
        assert_eq!(
            DataType::Ric.scalar(Scalar::from("ESc1")).unwrap().as_str(),
            Some("ESc1")
        );

        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(wire, r#"{"type":"ric","value":"AAPL.OQ"}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);
        assert!(serde_json::from_str::<Scalar>(r#"{"type":"ric","value":"AAPL OQ"}"#).is_err());

        // The text is not the code, and the same bytes under another code
        // are another value.
        assert_ne!(value, Scalar::from("AAPL.OQ"));
        assert_ne!(
            DataType::Ric.scalar(Scalar::from("SPX")).unwrap(),
            DataType::Bbg.scalar(Scalar::from("SPX")).unwrap()
        );
        // A refusal through the datatype door names the rule.
        let refused = DataType::Ric
            .scalar(Scalar::from("AAPL OQ"))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("printable ASCII"), "{refused}");
    }

    #[test]
    fn a_ric_has_no_default_and_an_empty_text_is_absence() {
        // No neutral member: the empty text names no instrument, so the
        // default is refused rather than invented, naming the code.
        let refused = DataType::Ric.default_value().unwrap_err().to_string();
        assert!(refused.contains("ric"), "{refused}");

        // An empty text entering a ric column is null, and the column's
        // nullability is what may refuse it.
        assert_eq!(DataType::Ric.scalar("").unwrap(), Scalar::Null);
        let strict = || ArrowCastOptions::new().with_safe(false);
        let nullable = Field::new("ric", DataType::Ric, true);
        let landed =
            Serie::from_arrow_array(Some(&nullable), text(&["AAPL.OQ", ""]), strict()).unwrap();
        assert_eq!(landed.scalar(0).unwrap(), apple());
        assert_eq!(landed.scalar(1).unwrap(), Scalar::Null);
    }

    #[test]
    fn a_ric_column_crosses_arrow_as_text_under_its_own_extension() {
        let field = Field::new("ric", DataType::Ric, false);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.ric");
        assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

        // A value is stored as exactly its own bytes and read back as itself.
        let stored = Serie::from_scalars(field.clone(), [apple()])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(cells.value(0), "AAPL.OQ");
        assert_eq!(cells.value_length(0), 7);
        let back = Serie::from_arrow_array(
            Some(&field),
            Arc::clone(&stored),
            ArrowCastOptions::default(),
        )
        .unwrap();
        assert_eq!(back.scalar(0).unwrap(), apple());
    }

    #[test]
    fn a_cast_into_a_ric_column_holds_it_to_the_canonical_spelling() {
        let field = Field::new("ric", DataType::Ric, true);
        let strict = || ArrowCastOptions::new().with_safe(false);

        let cast =
            Serie::from_arrow_array(Some(&field), text(&["AAPL.OQ", "ESc1"]), strict()).unwrap();
        assert_eq!(cast.scalar(0).unwrap(), apple());
        assert_eq!(
            cast.scalar(1).unwrap(),
            Scalar::Ric(Ric::new("ESc1").unwrap())
        );

        // A split token is refused naming the row, the column and the rule;
        // the safe cast answers null for it and keeps the rest.
        let refused = Serie::from_arrow_array(Some(&field), text(&["AAPL.OQ", "VOD L"]), strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("canonical spelling"), "{refused}");
        assert!(refused.contains("row 1 of column ric"), "{refused}");
        let safe = Serie::from_arrow_array(
            Some(&field),
            text(&["AAPL.OQ", "VOD L"]),
            ArrowCastOptions::new().with_safe(true),
        )
        .unwrap();
        assert_eq!(safe.scalar(0).unwrap(), apple());
        assert_eq!(safe.scalar(1).unwrap(), Scalar::Null);

        // One byte past the width is refused at the code's own width.
        let over = "R".repeat(33);
        let refused = Serie::from_arrow_array(Some(&field), text(&[over.as_str()]), strict())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 32 bytes"), "{refused}");
    }
}
