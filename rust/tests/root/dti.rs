//! `rust/src/dti.rs`: the ISO 24165 Digital Token Identifier, `dti`.
//!
//! A registered code held by its shape - nine symbols of a thirty-symbol
//! alphabet, never opening with `0` - and ranked by ISO 7064 hybrid MOD
//! 31,30, so a typo is a value every merge replaces by a closing one rather
//! than a refusal. What the generic code invariants cannot pin is the
//! alphabet, the check itself, the case fold and the refusals; those are
//! here.

mod tokens {

    use yggdryl::{CodeValue, DataType, Dti, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "nine characters"),
            ("X9J9K872", "nine characters"),
            ("X9J9K872S0", "at most 9"),
            ("09J9K872S", "first character other than 0"),
            ("A9J9K872S", "consonants other than Y"),
            ("X9J9K8Y2S", "consonants other than Y"),
            ("X9J9K8-2S", "consonants other than Y"),
        ] {
            let error = Dti::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Dti::new("A9J9K872S").unwrap_err().to_string();
        assert!(error.contains("dti"), "{error}");
        assert!(error.contains("A9J9K872S"), "{error}");
    }

    #[test]
    fn the_check_character_closes_under_hybrid_mod_31_30() {
        let token = Dti::new("X9J9K872S").unwrap();
        assert!(Dti::is_closed("X9J9K872S"));
        assert_eq!(Dti::closing_character("X9J9K872"), Some('S'));
        assert_eq!(token.check_character(), 'S');
        assert_eq!(token.rank(), 1);
        assert!(token.is_real());
        assert_eq!(<Dti as CodeValue>::MAX_RANK, 1);
        // Not eight upper-case symbols of the alphabet: nothing closes it.
        assert_eq!(Dti::closing_character("X9J9K87"), None);
        assert_eq!(Dti::closing_character("x9j9k872"), None);
        assert_eq!(Dti::closing_character("A9J9K872"), None);
    }

    #[test]
    fn the_check_is_the_rule_and_a_code_it_does_not_close_ranks_as_a_typo() {
        let typo = Dti::new("X9J9K872T").unwrap();
        assert!(!Dti::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        let token = Dti::new("X9J9K872S").unwrap();
        assert_eq!(typo.clone().merge_with(&token), token);
        assert_eq!(token.clone().merge_with(&typo), token);
        // A registry code whose stored character the algorithm does not
        // give - the hybrid check over `4H95J0R2` gives `T` - is a typo
        // too: there is no exception table.
        assert_eq!(Dti::closing_character("4H95J0R2"), Some('T'));
        assert_eq!(Dti::new("4H95J0R2X").unwrap().rank(), 0);
    }

    #[test]
    fn lower_case_folds_once() {
        let token = Dti::new("x9j9k872s").unwrap();
        assert_eq!(token.as_str(), "X9J9K872S");
        assert!(Dti::is_canonical("X9J9K872S"));
        assert!(!Dti::is_canonical("x9j9k872s"));
        assert!(!Dti::is_closed("x9j9k872s"), "lower case closes nothing");
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("dti").unwrap();
        assert_eq!(dtype, DataType::dti());
        assert_eq!(dtype.to_string(), "dti");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(9));
        let stored = dtype.scalar(Scalar::from("x9j9k872s")).unwrap();
        assert_eq!(stored, Scalar::Dti(Dti::new("X9J9K872S").unwrap()));
        assert!(dtype.scalar(Scalar::from("09J9K872S")).is_err());
    }

    #[test]
    fn direct_dti_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Dti::new("X9J9K872S").unwrap();
        assert_eq!(serde_json::to_string(&canonical).unwrap(), r#""X9J9K872S""#);
        assert_eq!(
            serde_json::from_str::<Dti>(r#""x9j9k872s""#).unwrap(),
            canonical
        );
        for invalid in [r#""""#, r#""X9J9K872""#, r#""A9J9K872S""#] {
            assert!(
                serde_json::from_str::<Dti>(invalid).is_err(),
                "{invalid} bypassed DTI validation"
            );
        }
    }
}

/// The wire contract of `dti` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_dti_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("dti").unwrap();
    let value = dtype.scalar("X9J9K872S").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x6e).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x6e);
    assert_eq!(dtype.id().as_str(), "dti");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "dti");
    assert_eq!(value.kind(), "dti");
    assert_eq!(value.id().as_u8(), 0x6e);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(9));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("X9J9K872S")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "dti");
    assert_eq!(Field::from_str("value dti").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("dti").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["dti"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.dti"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.dti")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["X9J9K872S"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"dti"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"dti"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"dti"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"dti"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"dti","value":"X9J9K872S"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"dti","value":"X9J9K872S"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 110, 0, 9, 88, 57, 74, 57, 75, 56, 55, 50, 83]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("X9J9K872S"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 17644871332366537191);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(17644871332366537191)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(12119103888037027857)
    );
    assert_eq!(dtype.stable_hash(), 6614075628612812762);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `dti`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_dti_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("dti").unwrap();
        let value = dtype.scalar("X9J9K872S").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            14893911022073648066
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            11483144012303516221
        );
    }
}
