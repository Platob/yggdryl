//! `rust/src/ccy.rs`: the currency code - ISO 4217's three letters, or a
//! digital-asset ticker past them, at most eight bytes.

use std::sync::Arc;

use arrow_array::{ArrayRef, StringArray};
use yggdryl::{ArrowCastOptions, Ccy, CodeValue, DataType, Scalar, Serie};

/// Every currency a feed states fits: ISO 4217's codes, and the tickers a
/// digital-asset venue writes up to the eight-byte bound, case and digits
/// kept as stated.
const HELD: [&str; 9] = [
    "USD", "XXX", "USDT", "USDC", "DOGE", "1INCH", "stETH", "WBTC", "BABYDOGE",
];

#[test]
fn a_currency_is_iso_4217_or_a_digital_asset_ticker_of_up_to_eight_bytes() {
    assert_eq!(<Ccy as CodeValue>::WIDTH, 8);
    assert_eq!(DataType::Ccy.code_width(), Some(8));
    for held in HELD {
        assert_eq!(Ccy::new(held).unwrap().as_str(), held, "{held}");
        assert_eq!(
            DataType::Ccy.scalar(held).unwrap(),
            Scalar::from(Ccy::new(held).unwrap()),
            "{held}"
        );
    }

    // A ninth byte is past the bound, whatever the bytes are.
    for over in ["BABYDOGE1", "TOOLONGCCY"] {
        let message = Ccy::new(over).unwrap_err().to_string();
        assert!(message.contains("at most 8 bytes"), "{over}: {message}");
        assert!(DataType::Ccy.scalar(over).is_err(), "{over}");
    }
}

#[test]
fn a_column_of_tickers_lands_as_the_text_it_is() {
    let field = DataType::Ccy.nullable_field("ccy");
    let cells: ArrayRef = Arc::new(StringArray::from(
        HELD.iter()
            .map(|held| Some(*held))
            .chain([None])
            .collect::<Vec<_>>(),
    ));
    let column = Serie::from_arrow_array(Some(&field), cells, ArrowCastOptions::new()).unwrap();
    assert_eq!(column.len(), HELD.len() + 1);
    for (row, held) in HELD.iter().enumerate() {
        assert_eq!(
            column.scalar(row).unwrap(),
            Scalar::from(Ccy::new(held).unwrap()),
            "{held}"
        );
    }
    assert!(column.is_null(HELD.len()).unwrap());

    // A value past the bound names its row.
    let over: ArrayRef = Arc::new(StringArray::from(vec!["USDT", "TOOLONGCCY"]));
    let message =
        Serie::from_arrow_array(Some(&field), over, ArrowCastOptions::new().with_safe(false))
            .unwrap_err()
            .to_string();
    assert!(message.contains("row 1"), "{message}");
    assert!(message.contains("at most 8 bytes"), "{message}");
}

#[test]
fn the_packed_integer_pads_a_currency_to_its_eight_bytes() {
    // ISO 4217's codes and the tickers past them pack into one ordering: the
    // order of the integers is the order of the text.
    let usd = DataType::Ccy.ascii_packed(b"USD").unwrap();
    let usdt = DataType::Ccy.ascii_packed(b"USDT").unwrap();
    assert_eq!(usd, 0x5553_4400_0000_0000);
    assert_eq!(usdt, 0x5553_4454_0000_0000);
    assert!(usd < usdt);
    assert_eq!(DataType::Ccy.ascii_value(usdt).unwrap(), "USDT");
    assert_eq!(
        DataType::Ccy.ascii_packed(b"BABYDOGE").unwrap(),
        DataType::fixed_ascii(8)
            .unwrap()
            .ascii_packed(b"BABYDOGE")
            .unwrap()
    );
}

/// The wire contract of `ccy` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_ccy_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("ccy").unwrap();
    let value = dtype.scalar("USD").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0x72).unwrap());
    assert_eq!(dtype.id().as_u8(), 0x72);
    assert_eq!(dtype.id().as_str(), "ccy");
    assert_eq!(dtype.id().kind(), DataTypeKind::Code);
    assert_eq!(dtype.to_string(), "ccy");
    assert_eq!(value.kind(), "ccy");
    assert_eq!(value.id().as_u8(), 0x72);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_code() && !dtype.is_enum());
    assert_eq!(dtype.code_width(), Some(8));
    assert_eq!(value.code_storage().map(|held| held.as_str()), Some("USD"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "ccy");
    assert_eq!(Field::from_str("value ccy").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("ccy").unwrap(), dtype);
    let logical: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["ccy"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.ccy"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.ccy")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(StringArray::from(vec!["USD"]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"ccy"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"ccy"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"ccy"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"ccy"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"ccy","value":"USD"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"ccy","value":"USD"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 114, 0, 3, 85, 83, 68]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("USD")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 579522367022846554);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(579522367022846554)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(820407550475414395)
    );
    assert_eq!(dtype.stable_hash(), 861021160962715385);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `ccy`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_ccy_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("ccy").unwrap();
        let value = dtype.scalar("USD").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            11170268193241562629
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            13726854045022095689
        );
    }
}
