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

/// The wire contract of `bbg` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_bbg_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("bbg").unwrap();
    let value = dtype.scalar("AAPL US Equity").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x7b).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x7b);
    assert_eq!(dtype.id().as_str(), "bbg");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "bbg");
    assert_eq!(value.kind(), "bbg");
    assert_eq!(value.id().as_u8(), 0x7b);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(32));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("AAPL US Equity")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "bbg");
    assert_eq!(Field::from_str("value bbg").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("bbg").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["bbg"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.bbg"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.bbg")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["AAPL US Equity"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"bbg"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"bbg"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"bbg"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"bbg"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"bbg","value":"AAPL US Equity"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"bbg","value":"AAPL US Equity"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(
        bytes,
        [
            0, 123, 0, 14, 65, 65, 80, 76, 32, 85, 83, 32, 69, 113, 117, 105, 116, 121
        ]
    );
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("AAPL US Equity"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 3640964093791871709);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(3640964093791871709)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(2134243375622617346)
    );
    assert_eq!(dtype.stable_hash(), 7575273193796255979);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `bbg`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_bbg_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("bbg").unwrap();
        let value = dtype.scalar("AAPL US Equity").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            15295320624850782262
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            885729296881078195
        );
    }
}
