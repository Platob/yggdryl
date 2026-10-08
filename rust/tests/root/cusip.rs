//! `rust/src/cusip.rs`: the CUSIP securities identifier beside `isin`.
//!
//! A registered code held by its shape and ranked by its own check digit,
//! so a typo is a value every merge replaces by a closing one rather than
//! a refusal. What the generic code invariants in `coded` cannot pin is the
//! check itself, the case fold, and the canonical spelling a column is held
//! to; those are here, once per identifier, with the ISIN rule as the
//! reference.

mod securities {

    use yggdryl::{CodeValue, Cusip};

    #[test]
    fn a_cusip_is_nine_characters_ranked_by_its_check_digit() {
        // Six of issuer, two of issue, one check digit: the modulus-10
        // double-add-double digit of the eight before it.
        let apple = Cusip::new("037833100").unwrap();
        assert_eq!(apple.as_str(), "037833100");
        assert_eq!(apple.issuer(), "037833");
        assert_eq!(apple.issue(), "10");
        assert_eq!(apple.check_digit(), 0);
        assert_eq!(apple.to_string(), "037833100");
        // A letter reads as ten plus its alphabet position.
        assert_eq!(Cusip::new("38259P508").unwrap().check_digit(), 8);
        assert_eq!(Cusip::new("594918104").unwrap().check_digit(), 4);
        assert_eq!(Cusip::closing_digit("03783310"), Some(0));
        assert_eq!(Cusip::closing_digit("38259P50"), Some(8));
        assert_eq!(Cusip::closing_digit("59491810"), Some(4));
        // The readings answer without building a value: canonical is the
        // upper-case shape, closing the check digit.
        assert!(Cusip::is_closed("037833100"));
        assert!(!Cusip::is_closed("38259p508"), "lower case closes nothing");
        assert!(Cusip::is_canonical("38259P508"));
        assert!(!Cusip::is_canonical("38259p508"));
        assert!(Cusip::is_canonical("037833101"), "a typo is a spelling");

        // Lower case is the upper case it spells, and stores as that.
        assert_eq!(
            Cusip::new("38259p508").unwrap(),
            Cusip::new("38259P508").unwrap()
        );
        assert_eq!(Cusip::new("38259p508").unwrap().as_str(), "38259P508");

        // One digit off is a typo: a value that does not close, of rank
        // zero, which a closing one replaces whichever leads.
        let typo = Cusip::new("037833101").unwrap();
        assert!(!Cusip::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        assert!(!typo.is_real());
        assert_eq!(apple.rank(), 1);
        assert!(apple.is_real());
        assert_eq!(<Cusip as CodeValue>::MAX_RANK, 1);
        assert_eq!(typo.clone().merge_with(&apple), apple);
        assert_eq!(apple.clone().merge_with(&typo), apple);
        let other_typo = Cusip::new("037833102").unwrap();
        assert_eq!(typo.clone().merge_with(&other_typo), typo);
        let microsoft = Cusip::new("594918104").unwrap();
        assert_eq!(apple.clone().merge_with(&microsoft), apple);
        // The wrong length, in both directions.
        let short = Cusip::new("03783310").unwrap_err().to_string();
        assert!(short.contains("expected nine characters"), "{short}");
        let long = Cusip::new("0378331000").unwrap_err().to_string();
        assert!(long.contains("at most 9 bytes"), "{long}");
        // A character outside the alphanumerics, a closing letter, and a body
        // the rule cannot close.
        let punctuated = Cusip::new("03783*100").unwrap_err().to_string();
        assert!(punctuated.contains("eight alphanumerics"), "{punctuated}");
        let letter = Cusip::new("03783310A").unwrap_err().to_string();
        assert!(letter.contains("closing check digit"), "{letter}");
        assert_eq!(Cusip::closing_digit("03783*10"), None);
        assert_eq!(Cusip::closing_digit("0378331"), None);
        assert_eq!(Cusip::closing_digit("38259p50"), None);
    }
}

/// The wire contract of `cusip` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_cusip_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("cusip").unwrap();
    let value = dtype.scalar("037833100").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x79).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x79);
    assert_eq!(dtype.id().as_str(), "cusip");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "cusip");
    assert_eq!(value.kind(), "cusip");
    assert_eq!(value.id().as_u8(), 0x79);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(9));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("037833100")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "cusip");
    assert_eq!(Field::from_str("value cusip").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("cusip").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["cusip"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.cusip"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.cusip")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["037833100"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"cusip"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"cusip"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"cusip"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"cusip"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"cusip","value":"037833100"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"cusip","value":"037833100"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 121, 0, 9, 48, 51, 55, 56, 51, 51, 49, 48, 48]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("037833100"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 10982021773620609182);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(10982021773620609182)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(8059308612240845317)
    );
    assert_eq!(dtype.stable_hash(), 4885984059231586804);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `cusip`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_cusip_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("cusip").unwrap();
        let value = dtype.scalar("037833100").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            6306177423754601266
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            13933794664134666544
        );
    }
}
