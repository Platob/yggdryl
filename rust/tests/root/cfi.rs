//! `rust/src/cfi.rs`: the coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

mod coded {

    use arrow_array::{Array, StringArray};

    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

    #[test]
    fn a_cfi_stores_the_six_characters_it_is_and_nothing_beside_them() {
        let cfi = Field::new("classification", DataType::Cfi, false);
        let stored = Serie::from_scalars(cfi.clone(), [Scalar::from("ESVUFR")])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        let cells = stored.as_any().downcast_ref::<StringArray>().unwrap();

        assert_eq!(cells.value(0), "ESVUFR");
        assert_eq!(cells.value_length(0), 6);
        assert_eq!(
            Serie::from_arrow_array(Some(&cfi), stored, ArrowCastOptions::default())
                .unwrap()
                .scalar(0)
                .unwrap(),
            DataType::Cfi.scalar(Scalar::from("ESVUFR")).unwrap()
        );
        // A width of six bytes is spellable and is still not a CFI code.
        assert_eq!(
            DataType::from_str("fixed_ascii(6)").unwrap(),
            DataType::fixed_ascii(6).unwrap()
        );
        assert_ne!(DataType::Cfi, DataType::fixed_ascii(6).unwrap());
    }
}

mod merge {
    use yggdryl::{Cfi, CodeValue};

    fn filled(mine: &str, theirs: &str) -> String {
        Cfi::new(mine)
            .unwrap()
            .merge_with(&Cfi::new(theirs).unwrap())
            .as_str()
            .to_owned()
    }

    #[test]
    fn an_unclassified_code_yields_whole_to_a_classified_one() {
        // All `X`, or a letter no group accepts: it says nothing, so the
        // other statement stands whole.
        assert_eq!(filled("XXXXXX", "ESVUFR"), "ESVUFR");
        assert_eq!(filled("XXXXXX", "JFTXFN"), "JFTXFN");
        assert_eq!(filled("ESZUFR", "ESVUFR"), "ESVUFR");
        assert_eq!(filled("XXXXXX", "ESXXXX"), "ESXXXX");
        // Nothing better beside it: this one stays.
        assert_eq!(filled("XXXXXX", "XXXXXX"), "XXXXXX");
        assert_eq!(filled("XXXXXX", "EXXXXX"), "XXXXXX");
        // A classified code never takes an unclassified one.
        assert_eq!(filled("ESVUFR", "XXXXXX"), "ESVUFR");
        assert_eq!(filled("ESXXXX", "XXXXXX"), "ESXXXX");
    }

    #[test]
    fn one_instrument_fills_position_by_position_and_two_keep_this_one() {
        assert_eq!(filled("ESVXXX", "ESXUFR"), "ESVUFR");
        assert_eq!(
            filled("ESVUFR", "ESNUFR"),
            "ESVUFR",
            "a contradicted letter keeps the lead"
        );
        assert_eq!(filled("ESNXXX", "ESVUFR"), "ESNXXX");
        assert_eq!(filled("ESVUFR", "DBFNFB"), "ESVUFR");
        assert_eq!(filled("JFTXFX", "JFTXXN"), "JFTXFN");
    }

    #[test]
    fn refined_fills_the_leads_unknown_letters_from_one_instrument_and_nothing_else() {
        let refined =
            |lead: &str, other: &str| Cfi::refined(lead, other).map(|code| code.to_string());
        for (lead, other, expected) in [
            ("ESXXXX", "ESVUFR", Some("ESVUFR")),
            ("ESVUFR", "ESXXXX", Some("ESVUFR")),
            ("ESVXXX", "ESXUFR", Some("ESVUFR")),
            ("ESVUFR", "ESVUFR", Some("ESVUFR")),
            ("JFTXFX", "JFTXXN", Some("JFTXFN")),
            ("XXXXXX", "ESVUFR", Some("ESVUFR")),
            ("ESZUFR", "ESVTFR", Some("ESVTFR")),
            ("ESVUFR", "XXXXXX", Some("ESVUFR")),
            ("ESXXXX", "EXXXXX", Some("ESXXXX")),
            ("XXXXXX", "XXXXXX", None),
            ("XXXXXX", "EXXXXX", None),
            ("ESVUFR", "ESNUFR", None),
            ("ESVTFR", "ESVUFR", None),
            ("ESXXXX", "DBXXXX", None),
            ("ESXXXX", "EPXXXX", None),
        ] {
            assert_eq!(
                refined(lead, other).as_deref(),
                expected,
                "{lead} refined by {other}"
            );
        }
    }
}

mod groups {
    use yggdryl::Cfi;

    #[test]
    fn a_non_deliverable_fx_forward_and_swap_classify() {
        // ISO 10962:2021, groups JF and SF: position 6 (delivery) lists `N`,
        // non-deliverable, beside physical and cash.
        for code in ["JFTXFN", "JFRXFN", "SFXXXN", "SFAXXN", "SFCXXN"] {
            assert!(Cfi::is_classified(code), "{code}");
            assert!(Cfi::is_detailed(code), "{code}");
        }
        for code in ["JFTXFP", "JFTXFC", "JFTXCC", "SFCXXP", "IFXXXP"] {
            assert!(Cfi::is_classified(code), "{code}");
        }
        // `Z` is no delivery at all, and a spot is always physical.
        assert!(!Cfi::is_classified("JFTXFZ"));
        assert!(!Cfi::is_classified("IFXXXN"));
    }
}

/// The wire contract of `cfi` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_cfi_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("cfi").unwrap();
    let value = dtype.scalar("ESVUFR").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x74).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x74);
    assert_eq!(dtype.id().as_str(), "cfi");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "cfi");
    assert_eq!(value.kind(), "cfi");
    assert_eq!(value.id().as_u8(), 0x74);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(6));
    assert_eq!(
        value.code_storage().map(|held| held.as_str()),
        Some("ESVUFR")
    );

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "cfi");
    assert_eq!(Field::from_str("value cfi").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("cfi").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["cfi"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.cfi"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.cfi")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["ESVUFR"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"cfi"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"cfi"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"cfi"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"cfi"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"cfi","value":"ESVUFR"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"cfi","value":"ESVUFR"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 116, 0, 6, 69, 83, 86, 85, 70, 82]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("ESVUFR")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 15220031798906910930);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(15220031798906910930)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(2038433015491442551)
    );
    assert_eq!(dtype.stable_hash(), 14195364590669508576);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `cfi`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_cfi_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("cfi").unwrap();
        let value = dtype.scalar("ESVUFR").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            13224266511966418983
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            15672035853136880531
        );
    }
}
