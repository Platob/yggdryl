//! `rust/src/marketdatatype.rs`: the type of its kind a market element is -
//! an order, quote, trade, book entry, trade report, quote request, mass
//! cancel or market data request type - as an enum stored as the `uint16`
//! code of its member, read off FIX's typing fields and refused by name
//! where nothing names a member.

use arrow_array::Array;
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeId, DataTypeKind, EnumValue, Field, MarketDataKind,
    MarketDataType, Scalar, Serie,
};

#[test]
fn every_member_is_its_code_its_stored_name_and_its_fix_value() {
    assert_eq!(MarketDataType::default(), MarketDataType::Unknown);
    assert_eq!(MarketDataType::Unknown.code(), 0);
    assert_eq!(<MarketDataType as EnumValue>::KIND, "marketdatatype");
    assert_eq!(
        <MarketDataType as EnumValue>::EXTENSION_NAME,
        "yggdryl.marketdatatype"
    );
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
        DataType::MarketDataType
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

#[test]
fn the_datatype_is_an_enum_over_uint16_codes() {
    assert_eq!(DataType::MarketDataType.id(), DataTypeId::MarketDataType);
    assert_eq!(DataTypeId::MarketDataType.kind(), DataTypeKind::Enum);
    assert!(DataType::MarketDataType.is_enum());
    assert_eq!(
        DataType::from_str("marketdatatype").unwrap(),
        DataType::MarketDataType
    );
    assert_eq!(DataType::MarketDataType.to_string(), "marketdatatype");
    assert_eq!(
        DataType::MarketDataType.default_value().unwrap(),
        Scalar::MarketDataType(MarketDataType::Unknown)
    );
    assert_eq!(
        DataType::MarketDataType
            .scalar(Scalar::from("ORDLIMIT"))
            .unwrap(),
        Scalar::MarketDataType(MarketDataType::OrdLimit)
    );
    assert_eq!(
        DataType::MarketDataType
            .scalar(Scalar::from(102_i32))
            .unwrap(),
        Scalar::MarketDataType(MarketDataType::OrdLimit)
    );
    let wire = serde_json::to_string(&Scalar::MarketDataType(MarketDataType::TrdBlock)).unwrap();
    assert_eq!(
        serde_json::from_str::<Scalar>(&wire).unwrap(),
        Scalar::MarketDataType(MarketDataType::TrdBlock)
    );

    let field = Field::new("marketdatatype", DataType::MarketDataType, true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(
        arrow.metadata()["ARROW:extension:name"],
        "yggdryl.marketdatatype"
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let values = [
        Scalar::MarketDataType(MarketDataType::OrdLimit),
        Scalar::Null,
        Scalar::MarketDataType(MarketDataType::BookBid),
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
