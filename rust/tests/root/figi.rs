//! `rust/src/figi.rs`: the Financial Instrument Global Identifier, `figi`.
//!
//! A registered code held by its shape - the consonants, the prefix the
//! standard reserves against an ISIN's, the `G` - and ranked by its own
//! check digit, so a typo is a value every merge replaces by a closing one
//! rather than a refusal. What the generic code invariants cannot pin is
//! the check itself, the prefix rule, the case fold, and the canonical
//! spelling a column is held to; those are here.

mod securities {

    use yggdryl::{CodeValue, Figi};

    #[test]
    fn a_figi_is_twelve_characters_with_its_own_check_and_prefix_rules() {
        let official = Figi::new("bbg000blnq16").expect("the OpenFIGI vector");
        assert_eq!(official.as_str(), "BBG000BLNQ16");
        assert!(Figi::is_closed("BBG000BLNQ16"));
        assert!(
            Figi::is_closed("BCG000000005"),
            "a permitted consonant prefix"
        );
        assert!(
            !Figi::is_closed("bbg000blnq16"),
            "lower case closes nothing"
        );
        assert!(!Figi::is_canonical("bbg000blnq16"));
        assert!(Figi::is_canonical("BBG000BLNQ17"), "a typo is a spelling");
        assert_eq!(official.rank(), 1);
        assert!(official.is_real());
        assert_eq!(<Figi as CodeValue>::MAX_RANK, 1);
        // One digit off is a typo: a value that does not close, of rank
        // zero, which a closing one replaces whichever leads.
        let typo = Figi::new("BBG000BLNQ17").unwrap();
        assert!(!Figi::is_closed(typo.as_str()));
        assert_eq!(typo.rank(), 0);
        assert_eq!(typo.clone().merge_with(&official), official);
        assert_eq!(official.clone().merge_with(&typo), official);
        // The shape stays the refusal: the length, a vowel, a reserved
        // prefix, the third letter, a closing letter, a byte outside ASCII
        // letters and digits.
        for invalid in [
            "BBG000BLNQ1",
            "BBG000BLNQ160",
            "BBG000BLNQ1A",
            "BAG000BLNQ16",
            "BSG000BLNQ16",
            "BBX000BLNQ16",
            "B?G000BLNQ16",
        ] {
            assert!(Figi::new(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn direct_figi_serde_validates_the_shape_and_canonicalizes_the_identifier() {
        let canonical = Figi::new("BBG000BLNQ16").unwrap();
        let rendered = serde_json::to_string(&canonical).unwrap();
        assert_eq!(rendered, r#""BBG000BLNQ16""#);
        assert_eq!(serde_json::from_str::<Figi>(&rendered).unwrap(), canonical);
        assert_eq!(
            serde_json::from_str::<Figi>(r#""bbg000blnq16""#).unwrap(),
            canonical
        );
        // A typo reads as the value it is; the shape is what the wire holds
        // to.
        assert_eq!(
            serde_json::from_str::<Figi>(r#""BBG000BLNQ17""#)
                .unwrap()
                .rank(),
            0
        );
        for invalid in [r#""""#, r#""BBG000BLNQ160""#, r#""BSG000BLNQ16""#] {
            assert!(
                serde_json::from_str::<Figi>(invalid).is_err(),
                "{invalid} bypassed FIGI validation"
            );
        }
    }
}

/// The wire contract of `figi` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_figi_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("figi").unwrap();
    let value = dtype.scalar("BBG000BLNQ16").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x7c).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x7c);
    assert_eq!(dtype.id().as_str(), "figi");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "figi");
    assert_eq!(value.kind(), "figi");
    assert_eq!(value.id().as_u8(), 0x7c);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(12));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("BBG000BLNQ16")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "figi");
    assert_eq!(Field::from_str("value figi").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("figi").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["figi"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.figi"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.figi")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["BBG000BLNQ16"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"figi"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"figi"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"figi"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"figi"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"figi","value":"BBG000BLNQ16"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"figi","value":"BBG000BLNQ16"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(
        bytes,
        [
            0, 124, 0, 12, 66, 66, 71, 48, 48, 48, 66, 76, 78, 81, 49, 54
        ]
    );
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("BBG000BLNQ16"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 11238263791772724216);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(11238263791772724216)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(1513786914167340168)
    );
    assert_eq!(dtype.stable_hash(), 12371612670941900531);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `figi`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_figi_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("figi").unwrap();
        let value = dtype.scalar("BBG000BLNQ16").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            12481983180732811358
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            4802680402299474529
        );
    }
}
