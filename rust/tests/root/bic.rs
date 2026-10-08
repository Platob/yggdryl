//! `rust/src/bic.rs`: the ISO 9362 Business Identifier Code, `bic`.
//!
//! A registered code held by its shape - a party prefix, a country, a
//! location and an optional branch - and ranked by its country, since a BIC
//! carries no check character. What the generic code invariants cannot pin
//! is the 2014 alphanumeric prefix, the two lengths kept as stated, the
//! readings of the four parts, SWIFT's `XK`, and the refusals; those are
//! here.

mod parties {

    use yggdryl::{Bic, CodeValue, DataType, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "eight or eleven characters"),
            ("DEUTDEF", "eight or eleven characters"),
            ("DEUTDEFF5", "eight or eleven characters"),
            ("DEUTDEFF50", "eight or eleven characters"),
            ("DEUTDEFF5000", "at most 11"),
            ("DEU-DEFF", "party prefix of letters or digits"),
            ("DEUT1EFF", "two-letter country code"),
            ("DEUTDEF-", "location of letters or digits"),
            ("DEUTDEFF5-0", "branch of letters or digits"),
            ("DEUT DEFF", "eight or eleven characters"),
        ] {
            let error = Bic::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Bic::new("DEUT1EFF").unwrap_err().to_string();
        assert!(error.contains("bic"), "{error}");
        assert!(error.contains("DEUT1EFF"), "{error}");
    }

    #[test]
    fn eight_and_eleven_characters_are_kept_as_stated() {
        let primary = Bic::new("DEUTDEFF").unwrap();
        let spelled = Bic::new("DEUTDEFFXXX").unwrap();
        assert_eq!(primary.as_str(), "DEUTDEFF");
        assert_eq!(spelled.as_str(), "DEUTDEFFXXX");
        assert_ne!(primary, spelled, "no branch is folded away");
        assert!(primary.is_primary_office());
        assert!(spelled.is_primary_office());
        assert_eq!(primary.branch(), None);
        assert_eq!(spelled.branch(), Some("XXX"));
        let branch = Bic::new("deutdeff500").unwrap();
        assert_eq!(branch.as_str(), "DEUTDEFF500");
        assert_eq!(branch.party_prefix(), "DEUT");
        assert_eq!(branch.country(), "DE");
        assert_eq!(branch.location(), "FF");
        assert_eq!(branch.branch(), Some("500"));
        assert!(!branch.is_primary_office());
        assert!(Bic::is_canonical("DEUTDEFF500"));
        assert!(!Bic::is_canonical("deutdeff500"));
        assert!(!Bic::is_canonical("DEUTDEFF50"));
    }

    #[test]
    fn a_prefix_may_hold_digits_since_iso_9362_2014() {
        let code = Bic::new("1234DEFF").unwrap();
        assert_eq!(code.party_prefix(), "1234");
        assert!(code.is_real());
    }

    #[test]
    fn the_rank_is_the_country_listed_or_kosovo() {
        assert_eq!(<Bic as CodeValue>::MAX_RANK, 1);
        assert_eq!(Bic::new("DEUTDEFF").unwrap().rank(), 1);
        assert_eq!(Bic::new("RBKOXKPR").unwrap().rank(), 1, "SWIFT's XK");
        let unlisted = Bic::new("DEUTZZFF").unwrap();
        assert_eq!(unlisted.rank(), 0);
        let listed = Bic::new("DEUTDEFF").unwrap();
        assert_eq!(unlisted.clone().merge_with(&listed), listed);
        assert_eq!(listed.clone().merge_with(&unlisted), listed);
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("bic").unwrap();
        assert_eq!(dtype, DataType::bic());
        assert_eq!(dtype.to_string(), "bic");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(11));
        let stored = dtype.scalar(Scalar::from("deutdeff")).unwrap();
        assert_eq!(stored, Scalar::Bic(Bic::new("DEUTDEFF").unwrap()));
        assert!(dtype.scalar(Scalar::from("DEUTDEFF5")).is_err());
    }

    #[test]
    fn direct_bic_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Bic::new("DEUTDEFF").unwrap();
        let rendered = serde_json::to_string(&canonical).unwrap();
        assert_eq!(rendered, r#""DEUTDEFF""#);
        assert_eq!(
            serde_json::from_str::<Bic>(r#""deutdeff""#).unwrap(),
            canonical
        );
        for invalid in [r#""""#, r#""DEUTDEFF5""#, r#""DEUT1EFF""#] {
            assert!(
                serde_json::from_str::<Bic>(invalid).is_err(),
                "{invalid} bypassed BIC validation"
            );
        }
    }
}

/// The wire contract of `bic` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_bic_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("bic").unwrap();
    let value = dtype.scalar("DEUTDEFFXXX").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x6c).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x6c);
    assert_eq!(dtype.id().as_str(), "bic");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "bic");
    assert_eq!(value.kind(), "bic");
    assert_eq!(value.id().as_u8(), 0x6c);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(11));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("DEUTDEFFXXX")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "bic");
    assert_eq!(Field::from_str("value bic").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("bic").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["bic"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.bic"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.bic")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["DEUTDEFFXXX"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"bic"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"bic"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"bic"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"bic"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"bic","value":"DEUTDEFFXXX"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"bic","value":"DEUTDEFFXXX"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(
        bytes,
        [0, 108, 0, 11, 68, 69, 85, 84, 68, 69, 70, 70, 88, 88, 88]
    );
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("DEUTDEFFXXX"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 9214906311216571244);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(9214906311216571244)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(3174523691552793290)
    );
    assert_eq!(dtype.stable_hash(), 1761858626127621558);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `bic`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_bic_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("bic").unwrap();
        let value = dtype.scalar("DEUTDEFFXXX").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            5714203061965085177
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            16708399131106159899
        );
    }
}
