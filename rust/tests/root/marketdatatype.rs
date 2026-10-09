//! `rust/src/marketdatatype.rs`: the type of its kind a market element is -
//! an order, quote, trade, book entry, trade report, quote request, mass
//! cancel or market data request type - as an enum stored as the `uint16`
//! code of its member, read off FIX's typing fields and refused by name
//! where nothing names a member.

use arrow_array::Array;
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeKind, Field, MARKETDATATYPE_KIND, MarketDataKind,
    MarketDataType, MarketType, Scalar, Serie,
};

#[test]
fn every_member_is_its_code_its_stored_name_and_its_fix_value() {
    assert_eq!(MarketDataType::default(), MarketDataType::Unknown);
    assert_eq!(MarketDataType::Unknown.code(), 0);
    assert_eq!(MarketDataType::NAME, "marketdatatype");
    assert_eq!(MarketDataType::EXTENSION_NAME, "yggdryl.marketdatatype");
    let mut previous = None;
    for member in MarketDataType::ALL.iter().copied() {
        // In code order, each set in its hundreds.
        assert!(previous < Some(member.code()), "{member}");
        previous = Some(member.code());
        assert_eq!(MarketDataType::from_code(member.code()), Some(member));
        assert_eq!(MarketDataType::from_name(member.as_str()), Some(member));
        assert_eq!(
            MarketDataType::from_spelling(&member.as_str().to_ascii_lowercase()),
            Some(member)
        );
        assert!(!member.description().is_empty(), "{member}");
        // A member standing for one FIX value reads back from it.
        if let Some((tag, wire)) = member.fix_code() {
            assert_eq!(
                MarketDataType::from_fix(tag, wire),
                Some(member),
                "{member}"
            );
            assert_eq!(
                member.code() / 100,
                match tag {
                    40 => 1,
                    537 => 2,
                    828 => 3,
                    269 => 4,
                    856 => 5,
                    303 => 6,
                    530 => 7,
                    263 => 8,
                    other => panic!("{member} types by {other}"),
                }
            );
        }
    }
    assert_eq!(MarketDataType::OrdLimit.as_str(), "ORDLIMIT");
    assert_eq!(MarketDataType::OrdMarket.as_str(), "ORDMKT");
    assert_eq!(MarketDataType::OrdLimit.fix_code(), Some((40, "2")));
    assert_eq!(MarketDataType::TrdBlock.fix_code(), Some((828, "1")));
    assert_eq!(MarketDataType::Unknown.fix_code(), None);
    assert_eq!(MarketDataType::OrdOther.fix_code(), None);
}

#[test]
fn a_fix_value_no_member_names_reads_as_its_sets_catch_all() {
    assert_eq!(
        MarketDataType::from_fix(40, "Z"),
        Some(MarketDataType::OrdOther)
    );
    assert_eq!(
        MarketDataType::from_fix(537, "9"),
        Some(MarketDataType::QuoOther)
    );
    assert_eq!(
        MarketDataType::from_fix(828, "999"),
        Some(MarketDataType::TrdOther)
    );
    assert_eq!(
        MarketDataType::from_fix(269, "z"),
        Some(MarketDataType::BookOther)
    );
    // A field that types nothing, and a value under the wrong field.
    assert_eq!(MarketDataType::from_fix(54, "1"), None);
    assert_eq!(
        MarketDataType::from_fix(40, "0"),
        Some(MarketDataType::OrdOther)
    );
}

#[test]
fn a_spelling_is_a_stored_name_or_a_name_only_one_set_gives() {
    assert_eq!(
        MarketDataType::from_spelling("Limit"),
        Some(MarketDataType::OrdLimit)
    );
    assert_eq!(
        MarketDataType::from_spelling("market_if_touched"),
        Some(MarketDataType::OrdMarketIfTouched)
    );
    assert_eq!(
        MarketDataType::from_spelling("BLOCK TRADE"),
        Some(MarketDataType::TrdBlock)
    );
    assert_eq!(
        MarketDataType::from_spelling("Tradeable"),
        Some(MarketDataType::QuoTradeable)
    );
    // `Counter` names a quote and an order selection: neither, by that name.
    assert_eq!(MarketDataType::from_spelling("Counter"), None);
    assert_eq!(
        MarketDataType::from_spelling("QUOCOUNTER"),
        Some(MarketDataType::QuoCounter)
    );
    // A wire value is no spelling, and a stored code is an integer.
    assert_eq!(MarketDataType::from_spelling("2"), None);
    assert_eq!(MarketDataType::from_spelling("102"), None);
    let refused = MarketDataType::read("NOTATYPE").unwrap_err().to_string();
    assert!(refused.contains("marketdatatype"), "{refused}");
    assert!(refused.contains("NOTATYPE"), "{refused}");
    assert!(MarketDataType::read_code(103).is_ok());
    assert!(MarketDataType::read_code(198).is_err());
}

/// `UKNW` is the zero member's four-letter spelling, and `UNKN`, the
/// spelling it was stored under before, names no type at any door, so a
/// dictionary whose `marketdatatypecodeset` still names it is rebuilt,
/// never read back as the member; a stored column holds the code `0` and
/// reads unchanged.
#[test]
fn the_retired_spelling_unkn_names_no_type() {
    assert_eq!(
        MarketDataType::from_spelling("UKNW"),
        Some(MarketDataType::Unknown)
    );
    assert_eq!(
        MarketDataType::from_spelling("uknw"),
        Some(MarketDataType::Unknown)
    );
    for spelling in ["UNKN", "unkn", "Unkn"] {
        assert_eq!(MarketDataType::from_spelling(spelling), None, "{spelling}");
        let refused = MarketDataType::read(spelling).unwrap_err().to_string();
        assert!(refused.contains("marketdatatype"), "{refused}");
        assert!(refused.contains(spelling), "{refused}");
    }
    assert!(serde_json::from_str::<MarketDataType>("\"UNKN\"").is_err());
    assert_eq!(
        serde_json::from_str::<MarketDataType>("\"UKNW\"").unwrap(),
        MarketDataType::Unknown
    );
    assert!(
        MarketDataType::dtype()
            .scalar(Scalar::from("UNKN"))
            .is_err()
    );
}

#[test]
fn a_kind_names_the_fields_that_type_it_first() {
    assert_eq!(MarketDataType::fix_tags(MarketDataKind::Order), [40]);
    assert_eq!(
        MarketDataType::fix_tags(MarketDataKind::Quotation),
        [537, 40]
    );
    assert_eq!(
        MarketDataType::fix_tags(MarketDataKind::Execution),
        [828, 40]
    );
    assert_eq!(MarketDataType::fix_tags(MarketDataKind::Trade), [828, 40]);
    assert_eq!(
        MarketDataType::fix_tags(MarketDataKind::Book),
        [269, 828, 537, 40]
    );
    assert_eq!(
        yggdryl::MARKETDATATYPE_FIX_TAGS,
        [40, 537, 828, 269, 856, 303, 530, 263]
    );
}

/// A message type with a field of its own is typed by it first: a trade
/// capture report by what the report is, a quote request, a mass cancel and
/// a market data request by what each asks; every other message type by its
/// kind's fields.
#[test]
fn a_message_type_names_its_own_typing_field_before_its_kind() {
    assert_eq!(
        MarketDataType::fix_tags_of("AE", MarketDataKind::Trade),
        [856, 828, 40]
    );
    assert_eq!(
        MarketDataType::fix_tags_of("R", MarketDataKind::Quotation),
        [303, 537, 40]
    );
    assert_eq!(
        MarketDataType::fix_tags_of("q", MarketDataKind::Order),
        [530]
    );
    assert_eq!(
        MarketDataType::fix_tags_of("V", MarketDataKind::Book),
        [263]
    );
    assert_eq!(
        MarketDataType::fix_tags_of("D", MarketDataKind::Order),
        [40]
    );
    for (tag, wire, member) in [
        (856, "0", MarketDataType::TrptSubmit),
        (856, "6", MarketDataType::TrptCancel),
        (856, "15", MarketDataType::TrptAllegedBreak),
        (303, "2", MarketDataType::QrqAutomatic),
        (530, "7", MarketDataType::McxAll),
        (530, "C", MarketDataType::McxUnderlyingIssuer),
        (263, "1", MarketDataType::MdrSubscribe),
    ] {
        assert_eq!(
            MarketDataType::from_fix(tag, wire),
            Some(member),
            "{tag}={wire}"
        );
        assert_eq!(member.fix_code(), Some((tag, wire)));
    }
    for (tag, other) in [
        (856, MarketDataType::TrptOther),
        (303, MarketDataType::QrqOther),
        (530, MarketDataType::McxOther),
        (263, MarketDataType::MdrOther),
    ] {
        assert_eq!(MarketDataType::from_fix(tag, "ZZ"), Some(other));
    }
    assert_eq!(
        MarketDataType::from_spelling("cancel all orders"),
        Some(MarketDataType::McxAll)
    );
    assert_eq!(
        MarketDataType::from_spelling("SnapshotAndUpdates"),
        Some(MarketDataType::MdrSubscribe)
    );
}

/// A kind states its own datatype and a nullable field of it - inherent items
/// of the enum, where `DataType` once held a constructor per kind - and both
/// are the descriptor's own, so a crate declaring a kind gets the same pair.
#[test]
fn the_marketdatatype_states_its_own_datatype_and_a_nullable_field_of_it() {
    let dtype = MarketDataType::dtype();
    assert_eq!(
        dtype,
        DataType::Market(MarketType::new(&MARKETDATATYPE_KIND))
    );
    assert_eq!(dtype, MARKETDATATYPE_KIND.dtype());
    assert_eq!(dtype, DataType::from_str("marketdatatype").unwrap());
    assert_eq!(dtype.id(), MarketDataType::ID);
    assert_eq!(dtype.to_string(), "marketdatatype");
    assert!(dtype.is_enum());

    let field = MarketDataType::field("marketdatatype");
    assert_eq!(field.name(), "marketdatatype");
    assert!(field.is_nullable(), "a kind's field is nullable");
    assert_eq!(field.dtype(), &dtype);
    assert_eq!(field, MARKETDATATYPE_KIND.field("marketdatatype", true));
    // Any text names it; nullability is the field's, so the required one is
    // the descriptor's alone.
    assert_eq!(
        MarketDataType::field(String::from("a marketdatatype")).name(),
        "a marketdatatype"
    );
    assert!(
        !MARKETDATATYPE_KIND
            .field("marketdatatype", false)
            .is_nullable()
    );
}

#[test]
fn the_datatype_is_an_enum_over_uint16_codes() {
    assert_eq!(MarketDataType::dtype().id(), MarketDataType::ID);
    assert_eq!(MarketDataType::ID.kind(), DataTypeKind::Enum);
    assert!(MarketDataType::dtype().is_enum());
    assert_eq!(
        DataType::from_str("marketdatatype").unwrap(),
        MarketDataType::dtype()
    );
    assert_eq!(MarketDataType::dtype().to_string(), "marketdatatype");
    assert_eq!(
        MarketDataType::dtype().default_value().unwrap(),
        Scalar::from(MarketDataType::Unknown)
    );
    assert_eq!(
        MarketDataType::dtype()
            .scalar(Scalar::from("ORDLIMIT"))
            .unwrap(),
        Scalar::from(MarketDataType::OrdLimit)
    );
    assert_eq!(
        MarketDataType::dtype()
            .scalar(Scalar::from(102_i32))
            .unwrap(),
        Scalar::from(MarketDataType::OrdLimit)
    );
    let wire = serde_json::to_string(&Scalar::from(MarketDataType::TrdBlock)).unwrap();
    assert_eq!(
        serde_json::from_str::<Scalar>(&wire).unwrap(),
        Scalar::from(MarketDataType::TrdBlock)
    );

    let field = Field::new("marketdatatype", MarketDataType::dtype(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(
        arrow.metadata()["ARROW:extension:name"],
        "yggdryl.marketdatatype"
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let values = [
        Scalar::from(MarketDataType::OrdLimit),
        Scalar::Null,
        Scalar::from(MarketDataType::BookBid),
    ];
    let serie = Serie::from_scalars(field.clone(), values.clone()).unwrap();
    let array = serie.require_arrow_array().unwrap();
    let codes = array
        .as_any()
        .downcast_ref::<arrow_array::UInt16Array>()
        .unwrap();
    assert_eq!(codes.values().as_ref(), [102, 0, 400]);
    assert!(codes.is_null(1));
    let back = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default()).unwrap();
    for (index, value) in values.iter().enumerate() {
        assert_eq!(&back.scalar(index).unwrap(), value, "row {index}");
    }
}

/// The wire contract of `marketdatatype` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_marketdatatype_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, UInt16Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("marketdatatype").unwrap();
    let value = dtype.scalar("ORDLIMIT").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0xc4).unwrap());
    assert_eq!(dtype.id().as_u8(), 0xc4);
    assert_eq!(dtype.id().as_str(), "marketdatatype");
    assert_eq!(dtype.id().kind(), DataTypeKind::Enum);
    assert_eq!(dtype.to_string(), "marketdatatype");
    assert_eq!(value.kind(), "marketdatatype");
    assert_eq!(value.id().as_u8(), 0xc4);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(value.enum_code(), Some(102_u16));
    assert_eq!(value.enum_name(), Some("ORDLIMIT"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "marketdatatype");
    assert_eq!(
        Field::from_str("value marketdatatype").unwrap().dtype(),
        &dtype
    );
    assert_eq!(
        DataType::from_logical_name("marketdatatype").unwrap(),
        dtype
    );
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["marketdatatype"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(
        dtype.id().arrow_extension_name(),
        Some("yggdryl.marketdatatype")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.marketdatatype")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(UInt16Array::from(vec![102_u16]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"marketdatatype"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"marketdatatype"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"marketdatatype"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"marketdatatype"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"marketdatatype","value":"ORDLIMIT"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"marketdatatype","value":"ORDLIMIT"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 196, 102, 0, 0, 0]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("ORDLIMIT")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 1473889680286530346);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(1473889680286530346)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(6287968742111782408)
    );
    assert_eq!(dtype.stable_hash(), 3177951029087061660);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `marketdatatype`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_marketdatatype_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("marketdatatype").unwrap();
        let value = dtype.scalar("ORDLIMIT").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            1847531576414769836
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            5766283677918373446
        );
    }
}
