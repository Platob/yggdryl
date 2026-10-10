//! `rust/src/forex.rs`: the ISO 4217 currency pair as one registered code,
//! and the reading of a venue's FX symbol into one.
//!
//! The code is two currencies the crate lists and a solidus, in one stored
//! spelling however a feed wrote it; the symbol reader is the one detector
//! of an FX pair with a tenor or a RIC's `=` around it. These pin the
//! spellings accepted, the refusals named, and the datatype that holds a
//! column of pairs.

mod value {
    use yggdryl::{Error, Forex, FxSymbol, FxTenor};

    #[test]
    fn what_is_not_two_distinct_currencies_is_refused_naming_the_text() {
        for text in [
            // An inner space is a symbol's, never a code's.
            "EUR USD",
            // The legs differ.
            "EUR/EUR",
            "USDUSD",
            // No currency, and the testing code, name no leg.
            "XXX/USD",
            "XTS/EUR",
            "USD/XXX",
            // Not a currency this crate lists.
            "ABC/USD",
            "EUR/ABC",
            // A tenor makes a symbol, not a code.
            "EUR/USD 1M",
            "EURUSD=",
            // The wrong shape.
            "",
            "EUR",
            "EUR/",
            "EUR//USD",
            "EU/USD",
            "EUR/US",
            "EUR+USD",
            "E1R/USD",
            "EUR/USDX",
        ] {
            let refused = Forex::new(text).unwrap_err();
            assert!(
                matches!(refused, Error::InvalidDataType { kind: "forex", .. }),
                "{text}: {refused}"
            );
            let message = refused.to_string();
            assert!(message.contains("CCY/CCY"), "{text}: {message}");
            assert!(message.contains(&format!("{text:?}")), "{text}: {message}");
            assert!(!Forex::is_canonical(text), "{text}");
        }
    }

    #[test]
    fn every_accepted_spelling_canonicalizes_to_the_one_stored_pair() {
        for text in [
            "EUR/USD",
            "eur/usd",
            "Eur/Usd",
            "EURUSD",
            "eurusd",
            "EUR-USD",
            "EUR.USD",
            "EUR_USD",
            " EUR/USD ",
            "\tEURUSD\n",
            "EUR/USD\0\0",
        ] {
            let pair = Forex::new(text).unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!(pair.as_str(), "EUR/USD", "{text:?}");
            assert_eq!(pair.storage().as_str(), "EUR/USD");
            assert_eq!(pair.to_string(), "EUR/USD");
            assert_eq!(pair, Forex::new("EUR/USD").unwrap());
        }
        // Only the stored spelling is canonical.
        assert!(Forex::is_canonical("EUR/USD"));
        for text in ["eur/usd", "EURUSD", "EUR-USD", " EUR/USD", "EUR/USD "] {
            assert!(!Forex::is_canonical(text), "{text:?}");
        }
    }

    #[test]
    fn the_legs_are_the_base_and_the_quote_and_a_metal_is_a_leg() {
        let pair = Forex::new("GBP/JPY").unwrap();
        assert_eq!(pair.base().as_str(), "GBP");
        assert_eq!(pair.quote().as_str(), "JPY");
        assert!(!pair.is_metal());
        for text in ["XAU/USD", "XAG/EUR", "USD/XPT", "XPD/JPY"] {
            let metal = Forex::new(text).unwrap();
            assert!(metal.is_metal(), "{text}");
        }
        assert!(!Forex::new("USD/CHF").unwrap().is_metal());
    }

    #[test]
    fn direct_serde_is_the_text_and_canonicalizes_on_the_way_in() {
        let pair = Forex::new("EUR/USD").unwrap();
        let rendered = serde_json::to_string(&pair).unwrap();
        assert_eq!(rendered, r#""EUR/USD""#);
        assert_eq!(serde_json::from_str::<Forex>(&rendered).unwrap(), pair);
        assert_eq!(serde_json::from_str::<Forex>(r#""eurusd""#).unwrap(), pair);
        assert!(serde_json::from_str::<Forex>(r#""EUR/EUR""#).is_err());
    }

    #[test]
    fn a_symbol_reads_into_the_pair_the_tenor_and_the_settlement_type() {
        let spot = |settltype: Option<&'static str>| (FxTenor::Spot, settltype);
        let forward = |settltype: &'static str| (FxTenor::Forward, Some(settltype));
        let unstated = (FxTenor::Unstated, None);
        for (symbol, pair, (tenor, settltype)) in [
            ("EURUSD", "EUR/USD", unstated),
            ("EUR/USD", "EUR/USD", unstated),
            ("eur-usd", "EUR/USD", unstated),
            ("EUR-USD 1M", "EUR/USD", forward("M1")),
            ("EUR/USD 12m", "EUR/USD", forward("M12")),
            ("EURUSD 2Y", "EUR/USD", forward("Y2")),
            ("EURUSD 3W", "EUR/USD", forward("W3")),
            ("EURUSD 10D", "EUR/USD", forward("D10")),
            ("EURUSD SN", "EUR/USD", forward("C")),
            ("EURUSD SW", "EUR/USD", forward("W1")),
            ("EURUSD BROKEN", "EUR/USD", forward("B")),
            ("GBPUSD SPOT", "GBP/USD", spot(Some("0"))),
            ("GBPUSD sp", "GBP/USD", spot(Some("0"))),
            ("GBPUSD TOD", "GBP/USD", spot(Some("1"))),
            ("GBPUSD TOM", "GBP/USD", spot(Some("2"))),
            ("EURGBP=", "EUR/GBP", spot(None)),
            ("EURGBP=R", "EUR/GBP", spot(None)),
            ("EUR/USD Curncy", "EUR/USD", unstated),
            ("EUR/USD ON", "EUR/USD", unstated),
            ("EUR/USD TN", "EUR/USD", unstated),
            ("  EUR/USD   1M  ", "EUR/USD", forward("M1")),
            ("XAU/USD", "XAU/USD", unstated),
        ] {
            let read =
                FxSymbol::from_symbol(symbol).unwrap_or_else(|| panic!("{symbol:?} names a pair"));
            assert_eq!(read.forex.as_str(), pair, "{symbol:?}");
            assert_eq!(read.tenor, tenor, "{symbol:?}");
            assert_eq!(read.settltype.as_deref(), settltype, "{symbol:?}");
        }
        assert!(FxSymbol::from_symbol("XAU/USD").unwrap().forex.is_metal());
        // A single currency's RIC, a tenor glued to one leg, a suffix nothing
        // names, and a RIC with anything but `R` after its `=` name no pair.
        for symbol in [
            "EUR=",
            "EUR1M=",
            "EUR/USD XYZ",
            "EURUSD=X",
            "EURUSD 1M 2M",
            "EURUSD 1234D",
            "EURUSD 1Q",
            "EUR USD",
            "EUR/EUR",
            "XXX/USD",
            "AAPL",
            "",
        ] {
            assert_eq!(
                FxSymbol::from_symbol(symbol).map(|held| held.forex),
                None,
                "{symbol:?}"
            );
        }
        // Trailing whitespace is trimmed: the pair alone, with no tenor.
        assert_eq!(
            FxSymbol::from_symbol("EURUSD ").map(|held| held.tenor),
            Some(FxTenor::Unstated)
        );
    }
}

mod datatype {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, Forex, ForexField, Scalar,
        Serie,
    };

    fn eurusd() -> Scalar {
        Scalar::Forex(Forex::new("EUR/USD").unwrap())
    }

    fn text(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    #[test]
    fn a_forex_is_the_newest_registered_code() {
        assert_eq!(DataType::Forex.id(), DataTypeId::Forex);
        assert_eq!(DataTypeId::Forex.as_u8(), 0x7f);
        assert_eq!(DataTypeId::Forex.as_str(), "forex");
        assert_eq!(DataTypeId::from_str("forex").unwrap(), DataTypeId::Forex);
        assert_eq!(DataType::Forex.kind(), DataTypeKind::Code);
        assert!(DataType::Forex.is_code());
        assert!(!DataType::Forex.is_enum());
        assert!(!DataType::Forex.is_string());
        assert_eq!(DataType::Forex.code_name(), Some("forex"));
        assert_eq!(DataType::Forex.code_width(), Some(7));
        assert_eq!(DataType::Forex.fixed_byte_width(), None);
        assert_eq!(DataType::forex(), DataType::Forex);
        assert!(DataType::CODES.contains(&("forex", DataType::Forex, 7)));
        assert_eq!(
            DataType::from_logical_name("forex").unwrap(),
            DataType::Forex
        );
        // A pair is a code: it packs into an integer at its width.
        assert_eq!(
            DataType::Forex.ascii_packed(b"EUR/USD").unwrap(),
            DataType::fixed_ascii(7)
                .unwrap()
                .ascii_packed(b"EUR/USD")
                .unwrap()
        );

        // The datatype's own wire is the one spelling, and it round-trips.
        let json = DataType::Forex.into_json().unwrap();
        assert_eq!(json, r#"{"type":"forex"}"#);
        assert_eq!(DataType::from_json(&json).unwrap(), DataType::Forex);
        let rendered = serde_json::to_string(&DataType::Forex).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&rendered).unwrap(),
            DataType::Forex
        );
        assert_eq!(DataType::from_str("forex").unwrap(), DataType::Forex);
        assert_eq!(DataType::Forex.to_string(), "forex");
    }

    #[test]
    fn a_forex_field_is_the_typed_marker_over_the_variant() {
        let field = Field::new("forex", DataType::Forex, true);
        let typed = ForexField::unit("forex", true);
        assert_eq!(typed.dtype(), &DataType::Forex);
        assert_eq!(typed.to_field(), field);
        let shared = DataType::Forex.shared_field().unwrap();
        assert_eq!(shared.dtype(), &DataType::Forex);
    }

    #[test]
    fn a_forex_value_is_the_canonical_pair_under_its_own_identity() {
        let value = DataType::Forex.scalar(Scalar::from("EURUSD")).unwrap();
        assert_eq!(value, eurusd());
        assert!(value.is_code());
        assert_eq!(value.id(), DataTypeId::Forex);
        assert_eq!(value.kind(), "forex");
        assert_eq!(value.as_str(), Some("EUR/USD"));
        assert_eq!(value.dtype().unwrap(), DataType::Forex);
        assert_eq!(DataType::Forex.scalar(eurusd()).unwrap(), eurusd());
        // Every spelling the value door reads lands as the one stored pair,
        // whitespace included, which no width refuses.
        for spelling in ["eur/usd", "EUR-USD", " EUR/USD ", "EUR.USD"] {
            assert_eq!(
                DataType::Forex.scalar(Scalar::from(spelling)).unwrap(),
                eurusd(),
                "{spelling:?}"
            );
        }
        assert_eq!(
            DataType::Forex
                .scalar(Scalar::from(b"EURUSD".to_vec()))
                .unwrap(),
            eurusd()
        );

        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(wire, r#"{"type":"forex","value":"EUR/USD"}"#);
        assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);
        assert_eq!(
            serde_json::from_str::<Scalar>(r#"{"type":"forex","value":"eurusd"}"#).unwrap(),
            value
        );
        assert!(serde_json::from_str::<Scalar>(r#"{"type":"forex","value":"EUR USD"}"#).is_err());

        // The text is not the code, and the same bytes under another code
        // are another value.
        assert_ne!(value, Scalar::from("EUR/USD"));
        assert_ne!(
            value,
            DataType::Bbg.scalar(Scalar::from("EUR/USD")).unwrap()
        );
        // A refusal through the datatype door names the rule.
        let refused = DataType::Forex
            .scalar(Scalar::from("EUR/EUR"))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("CCY/CCY"), "{refused}");
        assert!(DataType::Forex.scalar(Scalar::from(1_i64)).is_err());
    }

    #[test]
    fn a_forex_has_no_default_and_an_empty_text_is_absence() {
        let refused = DataType::Forex.default_value().unwrap_err().to_string();
        assert!(refused.contains("forex"), "{refused}");
        assert_eq!(DataType::Forex.scalar("").unwrap(), Scalar::Null);
        let strict = || ArrowCastOptions::new().with_safe(false);
        let nullable = Field::new("forex", DataType::Forex, true);
        let landed =
            Serie::from_arrow_array(Some(&nullable), text(&["EUR/USD", ""]), strict()).unwrap();
        assert_eq!(landed.scalar(0).unwrap(), eurusd());
        assert_eq!(landed.scalar(1).unwrap(), Scalar::Null);
    }

    #[test]
    fn a_forex_column_crosses_arrow_as_text_under_its_own_extension() {
        let field = Field::new("forex", DataType::Forex, false);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.forex");
        assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

        let serie = Serie::from_scalars(field.clone(), [eurusd()]).unwrap();
        assert!(matches!(serie, Serie::Forex(_)));
        let stored = serie.require_arrow_array().unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(cells.value(0), "EUR/USD");
        assert_eq!(cells.value_length(0), 7);
        let back = Serie::from_arrow_array(
            Some(&field),
            Arc::clone(&stored),
            ArrowCastOptions::default(),
        )
        .unwrap();
        assert_eq!(back.scalar(0).unwrap(), eurusd());
        // The same bytes under another name are the text they are.
        let foreign = arrow_schema::Field::new("forex", ArrowDataType::Int32, true).with_metadata(
            [
                (
                    "ARROW:extension:name".to_owned(),
                    "yggdryl.forex".to_owned(),
                ),
                ("ARROW:extension:metadata".to_owned(), String::new()),
            ]
            .into(),
        );
        assert_eq!(
            Field::from_arrow_field(&foreign).unwrap().dtype(),
            &DataType::Int32
        );
    }

    #[test]
    fn a_cast_into_a_forex_column_lands_every_spelling_as_the_canonical_pair() {
        let field = Field::new("forex", DataType::Forex, true);
        let strict = || ArrowCastOptions::new().with_safe(false);

        let cast = Serie::from_arrow_array(
            Some(&field),
            text(&["EURUSD", "eur-usd", "EUR/USD", "XAU_USD"]),
            strict(),
        )
        .unwrap();
        let cells = cast.require_arrow_array().unwrap();
        let cells = cells.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(
            cells.iter().collect::<Vec<_>>(),
            [
                Some("EUR/USD"),
                Some("EUR/USD"),
                Some("EUR/USD"),
                Some("XAU/USD")
            ]
        );
        assert_eq!(cast.scalar(1).unwrap(), eurusd());
        // A column already holding the canonical spelling is shared, not
        // copied: the `Arc` around the leaf is new, the buffers the caller's.
        let canonical = text(&["EUR/USD", "GBP/JPY"]);
        let landed =
            Serie::from_arrow_array(Some(&field), Arc::clone(&canonical), strict()).unwrap();
        assert!(
            landed
                .require_arrow_array()
                .unwrap()
                .to_data()
                .ptr_eq(&canonical.to_data())
        );

        // A symbol, a pair of one currency and a stranger are refused naming
        // the row, the column and the rule; the safe cast answers null and
        // keeps the rest.
        for stranger in ["EUR/USD 1M", "EUR/EUR", "ABC/USD", "EUR USD"] {
            let refused =
                Serie::from_arrow_array(Some(&field), text(&["EUR/USD", stranger]), strict())
                    .unwrap_err()
                    .to_string();
            assert!(
                refused.contains("row 1 of column forex"),
                "{stranger}: {refused}"
            );
            let safe = Serie::from_arrow_array(
                Some(&field),
                text(&["EUR/USD", stranger]),
                ArrowCastOptions::new().with_safe(true),
            )
            .unwrap();
            assert_eq!(safe.scalar(0).unwrap(), eurusd(), "{stranger}");
            assert_eq!(safe.scalar(1).unwrap(), Scalar::Null, "{stranger}");
        }
        // Text a forex column casts to is the canonical pair.
        let back = cast
            .cast(&Field::new("forex", DataType::utf8(), true), strict())
            .unwrap();
        assert_eq!(back.scalar(3).unwrap(), Scalar::from("XAU/USD"));
    }
}

mod codecs {
    use arrow_array::RecordBatch;
    use yggdryl::expression::Filter;
    use yggdryl::{DataType, DataTypeId, DigestAlgorithm, Forex, Scalar, Serie, StructType};

    fn pair(text: &str) -> Scalar {
        Scalar::Forex(Forex::new(text).unwrap())
    }

    #[test]
    fn a_forex_crosses_the_value_stream_the_digest_and_the_structured_codecs() {
        let value = pair("EUR/USD");
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[1], DataTypeId::Forex.as_u8());
        assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
        // A digest reads the text under the code's tag: the same text as a
        // string, or another pair, is another value.
        assert_ne!(
            value.digest(DigestAlgorithm::Xxh3),
            Scalar::from("EUR/USD").digest(DigestAlgorithm::Xxh3)
        );
        assert_ne!(value.stable_hash(), pair("GBP/USD").stable_hash());
        assert_eq!(value.stable_hash(), pair("eurusd").stable_hash());

        // Every structured codec writes the canonical pair.
        let row = Scalar::from_struct([("pair", value.clone())]).unwrap();
        assert_eq!(
            yggdryl::into_json_scalar(&row).unwrap(),
            r#"{"pair":"EUR/USD"}"#
        );
        assert_eq!(
            yggdryl::into_toml_scalar(&row).unwrap(),
            "\"pair\" = \"EUR/USD\"\n"
        );
        assert!(yggdryl::into_yaml_scalar(&row).unwrap().contains("EUR/USD"));
        assert_eq!(
            yggdryl::into_xml_scalar(&Scalar::from_struct([("row", row)]).unwrap()).unwrap(),
            "<row><pair>EUR/USD</pair></row>"
        );
        let variant = value.clone().into_variant().unwrap();
        assert_eq!(variant.scalar().unwrap(), Scalar::from("EUR/USD"));
        assert_eq!(
            yggdryl::media::partition::partition_text(&value).unwrap(),
            "EUR/USD"
        );
    }

    #[test]
    fn a_forex_filters_by_any_spelling_of_its_pair_in_an_expression() {
        let root = DataType::from(
            StructType::from_fields([DataType::Forex.nullable_field("pair")]).unwrap(),
        )
        .required_field("row");
        let column = Serie::from_scalars(
            root.fields()[0].clone(),
            [pair("EUR/USD"), pair("GBP/USD"), pair("XAU/USD")],
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let batch =
            RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column]).unwrap();
        for (clause, kept) in [
            ("pair = 'EUR/USD'", 1),
            // A constant coerces into the operand it meets: any spelling of
            // the pair reads as the pair.
            ("pair = 'eurusd'", 1),
            ("pair in ('EUR-USD', 'xau_usd')", 2),
            ("pair = forex 'GBPUSD'", 1),
        ] {
            let filter: Filter = clause.parse().unwrap();
            assert_eq!(
                filter.apply_arrow_batch(&batch).unwrap().num_rows(),
                kept,
                "{clause}"
            );
        }
    }
}

/// The wire contract of `forex` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_forex_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("forex").unwrap();
    let value = dtype.scalar("EUR/USD").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x7f).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x7f);
    assert_eq!(dtype.id().as_str(), "forex");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "forex");
    assert_eq!(value.kind(), "forex");
    assert_eq!(value.id().as_u8(), 0x7f);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(7));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("EUR/USD")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "forex");
    assert_eq!(Field::from_str("value forex").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("forex").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["forex"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.forex"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.forex")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["EUR/USD"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"forex"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"forex"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"forex"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"forex"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"forex","value":"EUR/USD"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"forex","value":"EUR/USD"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 127, 0, 7, 69, 85, 82, 47, 85, 83, 68]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("EUR/USD")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 16790735672855145637);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(16790735672855145637)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(14416976940411698840)
    );
    assert_eq!(dtype.stable_hash(), 15833788041979850508);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `forex`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_forex_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("forex").unwrap();
        let value = dtype.scalar("EUR/USD").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            9848593824053419392
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            412333908725082328
        );
    }
}
