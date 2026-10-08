//! `rust/src/mic.rs`: ISO 10383's market identifier code, and what its
//! registry states of each code - the operating MIC, whether it is a
//! segment, the country.

use yggdryl::{Country, Mic};

fn mic(text: &str) -> Mic {
    Mic::new(text).unwrap()
}

/// A segment answers the operating MIC of its market and an operating MIC
/// itself; an expired segment still answers, because a capture names venues
/// that have since closed.
#[test]
fn a_segment_answers_its_market_and_an_operating_mic_itself() {
    let segment = mic("XNGS");
    assert_eq!(segment.operating(), Some(mic("XNAS")));
    assert!(segment.is_segment());
    for operating in ["XNAS", "XLON", "XETR", "XSWX", "XXXX"] {
        let held = mic(operating);
        assert_eq!(held.operating(), Some(held.clone()), "{operating}");
        assert!(!held.is_segment(), "{operating}");
    }
    // `NBXO` expired under `XBXO`, which has since become a segment of
    // `XNAS`: the chain ends at the operating MIC.
    let expired = mic("NBXO");
    assert_eq!(expired.operating(), Some(mic("XNAS")));
    assert!(expired.is_segment());
}

/// A code answers the country ISO 10383 places it in; a venue of no single
/// country - the registry's own `ZZ` - answers none, as a code the registry
/// never assigned does.
#[test]
fn a_code_answers_the_country_its_registry_places_it_in() {
    for (code, country) in [
        ("XLON", "GB"),
        ("XETR", "DE"),
        ("XSWX", "CH"),
        ("XNAS", "US"),
        ("XNGS", "US"),
    ] {
        assert_eq!(
            mic(code).country(),
            Some(Country::new(country).unwrap()),
            "{code}"
        );
    }
    for nowhere in ["XOFF", "XXXX"] {
        assert_eq!(mic(nowhere).country(), None, "{nowhere}");
    }
}

/// A code the registry never assigned - a venue's own short code, a
/// lower-case spelling - answers no operating MIC, no segment and no
/// country.
#[test]
fn a_code_the_registry_never_assigned_answers_nothing() {
    for unknown in ["QQQQ", "xnas", "S", ""] {
        let held = mic(unknown);
        assert_eq!(held.operating(), None, "{unknown}");
        assert!(!held.is_segment(), "{unknown}");
        assert_eq!(held.country(), None, "{unknown}");
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::mic::mics;
    use yggdryl::{Country, Mic, StringEnum};

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_mic_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("mic").unwrap();
        let value = dtype.scalar("XPAR").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            7962632459274133690
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            15558487542905319998
        );
    }

    use super::mic;

    /// The generated table is sorted by the MIC with each code once - what
    /// the binary search needs - every MIC and operating MIC four upper-case
    /// letters or digits, every operating MIC a row naming itself, and every
    /// row what the three accessors answer; every common venue the logical
    /// `mic` enum lists is a registered code.
    #[test]
    fn the_generated_registry_table_is_sorted_unique_and_what_the_accessors_answer() {
        let rows = mics();
        assert!(
            rows.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "sorted by the MIC, each code once"
        );
        for &(code, operating, country) in rows {
            assert!(Mic::is_iso(code), "{code}");
            assert!(Mic::is_iso(operating), "{code}: {operating}");
            assert!(
                country.is_empty()
                    || (country.len() == 2
                        && country.bytes().all(|byte| byte.is_ascii_uppercase())),
                "{code}: {country}"
            );
            let held = mic(code);
            assert_eq!(held.operating(), Some(mic(operating)), "{code}");
            assert_eq!(held.is_segment(), code != operating, "{code}");
            // The row's country, where it is a listed one: `ZZ` is none.
            let listed = Country::new(country).unwrap();
            assert_eq!(
                held.country(),
                listed.is_listed().then_some(listed),
                "{code}"
            );
            assert_eq!(
                mic(operating).operating(),
                Some(mic(operating)),
                "{code}: {operating}"
            );
        }
        for code in StringEnum::MICS {
            assert!(mic(code).operating().is_some(), "{code}");
        }
    }
}

/// The wire contract of `mic` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_mic_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("mic").unwrap();
    let value = dtype.scalar("XPAR").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x73).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x73);
    assert_eq!(dtype.id().as_str(), "mic");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "mic");
    assert_eq!(value.kind(), "mic");
    assert_eq!(value.id().as_u8(), 0x73);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(4));
    assert_eq!(value.code_storage().map(|held| held.as_str()), Some("XPAR"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "mic");
    assert_eq!(Field::from_str("value mic").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("mic").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["mic", "exchange"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.mic"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.mic")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["XPAR"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"mic"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"mic"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"mic"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"mic"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"mic","value":"XPAR"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"mic","value":"XPAR"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 115, 0, 4, 88, 80, 65, 82]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("XPAR")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 342426546139583233);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(342426546139583233)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(3139147835418930419)
    );
    assert_eq!(dtype.stable_hash(), 5490350541240551306);
}
