//! `rust/src/sedol.rs`: the SEDOL securities identifier beside `isin`.
//!
//! A registered code held by its shape and ranked by its own check digit,
//! so a typo is a value every merge replaces by a closing one rather than
//! a refusal. What the generic code invariants in `coded` cannot pin is the
//! check itself, the case fold, and the canonical spelling a column is held
//! to; those are here, once per identifier, with the ISIN rule as the
//! reference.

mod securities {

    use yggdryl::{CodeValue, Sedol};

    #[test]
    fn a_sedol_is_seven_characters_ranked_by_its_check_digit() {
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
        // The readings answer without building a value: canonical is the
        // upper-case shape, closing the check digit.
        assert!(Sedol::is_closed("B0YBKJ7"));
        assert!(!Sedol::is_closed("b0ybkj7"), "lower case closes nothing");
        assert!(Sedol::is_canonical("B0YBKJ7"));
        assert!(!Sedol::is_canonical("b0ybkj7"));
        assert!(Sedol::is_canonical("B0YBKJ8"), "a typo is a spelling");

        // Lower case is the upper case it spells, and stores as that.
        assert_eq!(Sedol::new("b0ybkj7").unwrap(), held);
        assert_eq!(Sedol::new("b0ybkj7").unwrap().as_str(), "B0YBKJ7");

        // One digit off is a typo: a value that does not close, of rank
        // zero, which a closing one replaces whichever leads.
        let typo = Sedol::new("B0YBKJ8").unwrap();
        assert!(!Sedol::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        assert!(!typo.is_real());
        assert_eq!(held.rank(), 1);
        assert!(held.is_real());
        assert_eq!(<Sedol as CodeValue>::MAX_RANK, 1);
        assert_eq!(typo.clone().merge_with(&held), held);
        assert_eq!(held.clone().merge_with(&typo), held);
        let other = Sedol::new("0263494").unwrap();
        assert_eq!(held.clone().merge_with(&other), held);
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

/// The wire contract of `sedol` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_sedol_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("sedol").unwrap();
    let value = dtype.scalar("B0YBKJ7").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x7a).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x7a);
    assert_eq!(dtype.id().as_str(), "sedol");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "sedol");
    assert_eq!(value.kind(), "sedol");
    assert_eq!(value.id().as_u8(), 0x7a);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(7));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("B0YBKJ7")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "sedol");
    assert_eq!(Field::from_str("value sedol").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("sedol").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["sedol"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.sedol"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.sedol")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["B0YBKJ7"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"sedol"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"sedol"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"sedol"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"sedol"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"sedol","value":"B0YBKJ7"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"sedol","value":"B0YBKJ7"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 122, 0, 7, 66, 48, 89, 66, 75, 74, 55]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("B0YBKJ7")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 6105491412071589058);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(6105491412071589058)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(8130441686835634469)
    );
    assert_eq!(dtype.stable_hash(), 12480796736920323934);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `sedol`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_sedol_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("sedol").unwrap();
        let value = dtype.scalar("B0YBKJ7").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            14244502231422953335
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            8969406193618307452
        );
    }
}
