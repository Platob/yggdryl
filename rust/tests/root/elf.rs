//! `rust/src/elf.rs`: the ISO 20275 Entity Legal Form code, `elf`.
//!
//! A registered code held by its shape - four letters or digits - with no
//! check character and no placeholder, so every well-shaped value ranks the
//! same. What the generic code invariants cannot pin is the shape, the case
//! fold and the refusals; those are here.

mod legal_forms {

    use yggdryl::{CodeValue, DataType, Elf, Scalar};

    #[test]
    fn the_shape_is_the_refusal_and_names_the_code_and_the_reason() {
        for (invalid, reason) in [
            ("", "four characters"),
            ("2HB", "four characters"),
            ("2HBR5", "at most 4"),
            ("2H-R", "four letters or digits"),
            ("2H R", "four letters or digits"),
        ] {
            let error = Elf::new(invalid).expect_err(invalid).to_string();
            assert!(error.contains(reason), "{invalid:?}: {error}");
        }
        let error = Elf::new("2H-R").unwrap_err().to_string();
        assert!(error.contains("elf"), "{error}");
        assert!(error.contains("2H-R"), "{error}");
    }

    #[test]
    fn a_legal_form_folds_to_upper_case_and_every_one_ranks_the_same() {
        let gmbh = Elf::new("2hbr").unwrap();
        assert_eq!(gmbh.as_str(), "2HBR");
        assert_eq!(gmbh, Elf::new("2HBR").unwrap());
        assert!(Elf::is_canonical("2HBR"));
        assert!(!Elf::is_canonical("2hbr"));
        assert_eq!(<Elf as CodeValue>::MAX_RANK, 1);
        for code in ["2HBR", "8888", "9999", "ZZZZ"] {
            assert_eq!(Elf::new(code).unwrap().rank(), 1, "{code}");
        }
    }

    #[test]
    fn the_datatype_reads_its_name_and_checks_values_by_the_type() {
        let dtype = DataType::from_str("elf").unwrap();
        assert_eq!(dtype, DataType::elf());
        assert_eq!(dtype.to_string(), "elf");
        assert!(dtype.is_code());
        assert_eq!(dtype.code_width(), Some(4));
        let stored = dtype.scalar(Scalar::from("2hbr")).unwrap();
        assert_eq!(stored, Scalar::Elf(Elf::new("2HBR").unwrap()));
        assert!(dtype.scalar(Scalar::from("2HB")).is_err());
    }

    #[test]
    fn direct_elf_serde_validates_the_shape_and_canonicalizes_the_code() {
        let canonical = Elf::new("2HBR").unwrap();
        assert_eq!(serde_json::to_string(&canonical).unwrap(), r#""2HBR""#);
        assert_eq!(serde_json::from_str::<Elf>(r#""2hbr""#).unwrap(), canonical);
        for invalid in [r#""""#, r#""2HB""#, r#""2H-R""#] {
            assert!(
                serde_json::from_str::<Elf>(invalid).is_err(),
                "{invalid} bypassed ELF validation"
            );
        }
    }
}

/// The wire contract of `elf` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_elf_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("elf").unwrap();
    let value = dtype.scalar("2HBR").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x6d).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x6d);
    assert_eq!(dtype.id().as_str(), "elf");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "elf");
    assert_eq!(value.kind(), "elf");
    assert_eq!(value.id().as_u8(), 0x6d);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(4));
    assert_eq!(value.code_storage().map(|held| held.as_str()), Some("2HBR"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "elf");
    assert_eq!(Field::from_str("value elf").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("elf").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["elf"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.elf"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.elf")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["2HBR"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"elf"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"elf"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"elf"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"elf"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"elf","value":"2HBR"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"elf","value":"2HBR"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 109, 0, 4, 50, 72, 66, 82]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("2HBR")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 11939408946376585611);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(11939408946376585611)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(11960936095069473263)
    );
    assert_eq!(dtype.stable_hash(), 16995795851994045872);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `elf`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_elf_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("elf").unwrap();
        let value = dtype.scalar("2HBR").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            901839621449666086
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            13104432080116389324
        );
    }
}
