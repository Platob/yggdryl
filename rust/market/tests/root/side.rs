//! `rust/market/src/side.rs`: FIX's side of a trade as a one-byte enum stored as
//! the `int32` code of its member, read by spelling or code and refused
//! where neither names one.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, EnumValue, Field,
    MarketSerie, MarketType, Scalar, Serie, StringEnum, StructType, Value,
};
use yggdryl_market::{SIDE_KIND, Side};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// Every side: its wire code, its stored four-letter code, the name it was
/// stored under before the codes - read, never written - and the name the
/// specification gives it, in the order of the codes.
const SIDES: [(Option<char>, &str, &str, &str, Side); 18] = [
    (None, "UKNW", "UNKNOWN", "Unknown", Side::Unknown),
    (Some('1'), "BUYS", "BUY", "Buy", Side::Buy),
    (Some('2'), "SELL", "SELL", "Sell", Side::Sell),
    (Some('3'), "BUYM", "BUYMINUS", "BuyMinus", Side::BuyMinus),
    (Some('4'), "SELP", "SELLPLUS", "SellPlus", Side::SellPlus),
    (Some('5'), "SSHT", "SSHORT", "SellShort", Side::SShort),
    (
        Some('6'),
        "SSEX",
        "SSHORTEX",
        "SellShortExempt",
        Side::SShortEx,
    ),
    (Some('7'), "UNDI", "UNDISC", "Undisclosed", Side::Undisc),
    (Some('8'), "CROS", "CROSS", "Cross", Side::Cross),
    (Some('9'), "CRSH", "CROSSSH", "CrossShort", Side::CrossSh),
    (
        Some('A'),
        "CRSX",
        "CROSSSHX",
        "CrossShortExempt",
        Side::CrossShX,
    ),
    (Some('B'), "ASDF", "ASDEF", "AsDefined", Side::AsDef),
    (Some('C'), "OPPO", "OPPOSITE", "Opposite", Side::Opposite),
    (Some('D'), "SUBS", "SUBSCR", "Subscribe", Side::Subscr),
    (Some('E'), "REDM", "REDEEM", "Redeem", Side::Redeem),
    (Some('F'), "LEND", "LEND", "Lend", Side::Lend),
    (Some('G'), "BORR", "BORROW", "Borrow", Side::Borrow),
    (
        Some('H'),
        "SELU",
        "SELLUND",
        "SellUndisclosed",
        Side::SellUnd,
    ),
];

#[test]
fn a_code_or_a_spelling_that_names_no_side_is_refused_by_name() {
    crate::install::installed();
    assert_eq!(Side::from_code(18), None);
    assert!(Side::read_code(-1).is_err());
    let refused = Side::read_code(4_242).unwrap_err().to_string();
    assert!(refused.contains("4242"), "{refused}");
    assert!(refused.contains("side"), "{refused}");
    assert!(Side::read_code(i64::from(i32::MAX) + 1).is_err());
    assert!(Side::try_from(18_u8).is_err());

    // A spelling that names no side is refused naming the datatype, and no
    // width bounds the spelling: a long name is refused for naming nothing.
    assert_eq!(Side::from_spelling("X"), None);
    assert_eq!(Side::from_spelling(""), None);
    for spelling in ["X", "Z", "TOOLONGSIDE", "BUYX"] {
        let refused = Side::read(spelling).unwrap_err().to_string();
        assert!(refused.contains("side"), "{refused}");
        assert!(refused.contains(spelling), "{refused}");
    }
    // A stored code is an integer and never text: `10` spells nothing,
    // because `1`..=`9` are FIX wire codes and a number read as text would
    // answer the wrong side for one of the two vocabularies.
    assert_eq!(Side::from_spelling("10"), None);
    assert_eq!(Side::from_spelling("17"), None);
}

/// `UKNW` is the zero member's four-letter spelling, and `UNKN`, the
/// spelling it was stored under before, names no side at any door - the
/// stored name, the folded alias table and the word patterns alike - so a
/// document written under it is rebuilt by its writer, never read back as
/// the member; a stored column holds the code `0` and reads unchanged.
#[test]
fn the_retired_spelling_unkn_names_no_side() {
    crate::install::installed();
    assert_eq!(Side::from_spelling("UKNW"), Some(Side::Unknown));
    assert_eq!(Side::from_spelling("uknw"), Some(Side::Unknown));
    assert_eq!(Side::from_spelling("Unknown"), Some(Side::Unknown));
    for spelling in ["UNKN", "unkn", "Unkn"] {
        assert_eq!(Side::from_spelling(spelling), None, "{spelling}");
        let refused = Side::read(spelling).unwrap_err().to_string();
        assert!(refused.contains("side"), "{refused}");
        assert!(refused.contains(spelling), "{refused}");
    }
    assert!(serde_json::from_str::<Side>("\"UNKN\"").is_err());
    assert_eq!(
        serde_json::from_str::<Side>("\"UKNW\"").unwrap(),
        Side::Unknown
    );
    assert!(Side::dtype().scalar(Scalar::from("UNKN")).is_err());
    assert_eq!(
        Side::dtype().scalar(Scalar::from("UKNW")).unwrap(),
        Scalar::from(Side::Unknown)
    );
}

#[test]
fn a_side_is_one_byte_whose_code_is_the_position_of_its_wire_character() {
    crate::install::installed();
    assert_eq!(std::mem::size_of::<Side>(), 1);
    assert_eq!(std::mem::size_of::<Option<Side>>(), 1);
    assert_eq!(Side::default(), Side::Unknown);
    assert_eq!(Side::ALL.len(), 19);
    assert_eq!(<Side as EnumValue>::ALL, Side::ALL);
    assert_eq!(Side::NAME, "side");
    assert_eq!(Side::EXTENSION_NAME, "yggdryl.side");

    for (index, (code, stored, former, _, side)) in SIDES.iter().enumerate() {
        assert_eq!(*side as usize, index, "{stored}");
        assert_eq!(side.code(), u8::try_from(index).unwrap(), "{stored}");
        assert_eq!(Side::from_code(side.code()), Some(*side), "{stored}");
        assert_eq!(Side::read_code(i64::from(side.code())).unwrap(), *side);
        assert_eq!(Side::try_from(side.code()).unwrap(), *side);
        assert_eq!(u8::from(*side), side.code());
        assert_eq!(side.fix_code(), *code, "{stored}");
        assert_eq!(side.as_str(), *stored);
        assert_eq!(side.to_string(), *stored);
        assert_eq!(Side::from_name(stored), Some(*side));
        if former != stored {
            assert_eq!(
                Side::from_name(former),
                None,
                "{former} is read, never stored"
            );
        }
        assert!(!side.description().is_empty(), "{stored}");
        assert_eq!(Side::ALL[index], *side);
    }
    // The members order as the wire codes do, `1`..=`9` then `A`..=`H`,
    // and `BOTH`, which has no wire code, stands last.
    let mut ordered: Vec<Side> = SIDES.iter().map(|(_, _, _, _, side)| *side).collect();
    ordered.reverse();
    ordered.sort_unstable();
    assert_eq!(ordered, &Side::ALL[..18]);
    assert_eq!(Side::ALL[18], Side::Both);
    // The string listing of the same vocabulary is the nineteen stored
    // codes, sorted: what a fixed-ASCII column may declare it holds.
    let mut listed: Vec<&str> = SIDES.iter().map(|(_, stored, _, _, _)| *stored).collect();
    listed.push(Side::Both.as_str());
    listed.sort_unstable();
    assert_eq!(listed.as_slice(), yggdryl_market::SIDES);
    assert!(StringEnum::prebuilt_values("side").contains(&"BOTH"));
}

#[test]
fn every_vocabulary_reaches_one_side_and_a_wire_code_never_folds() {
    crate::install::installed();
    for (code, stored, former, name, side) in SIDES {
        assert_eq!(Side::from_spelling(stored), Some(side), "{stored}");
        assert_eq!(Side::from_spelling(former), Some(side), "{former}");
        assert_eq!(
            Side::from_spelling(&former.to_ascii_lowercase()),
            Some(side),
            "{former}"
        );
        assert_eq!(Side::from_spelling(name), Some(side), "{name}");
        assert_eq!(
            Side::from_spelling(&name.to_ascii_lowercase()),
            Some(side),
            "{name}"
        );
        assert_eq!(Side::read(stored).unwrap(), side);
        assert_eq!(Side::new(name).unwrap(), side);
        assert_eq!(Side::new(String::from(stored)).unwrap(), side);
        if let Some(code) = code {
            let wire = String::from(code);
            assert_eq!(Side::from_spelling(&wire), Some(side), "{wire}");
            // A wire letter does not fold: `a` is not `A`.
            if code.is_ascii_alphabetic() {
                assert_eq!(
                    Side::from_spelling(&wire.to_ascii_lowercase()),
                    None,
                    "{wire}"
                );
            }
        }
    }
    // A name folds the way every name in this crate folds.
    for spelling in ["sell_short", "SELL SHORT", "Sell-Short", "sellshort"] {
        assert_eq!(
            Side::from_spelling(spelling),
            Some(Side::SShort),
            "{spelling}"
        );
    }
    assert_eq!(Side::read("SellShortExempt").unwrap(), Side::SShortEx);
}

#[test]
fn a_side_is_a_bid_an_ask_or_neither() {
    crate::install::installed();
    let bid = [Side::Buy, Side::BuyMinus];
    let ask = [
        Side::Sell,
        Side::SellPlus,
        Side::SShort,
        Side::SShortEx,
        Side::SellUnd,
    ];
    for (_, stored, _, _, side) in SIDES {
        assert_eq!(side.is_bid(), bid.contains(&side), "{stored}");
        assert_eq!(side.is_ask(), ask.contains(&side), "{stored}");
        assert!(!(side.is_bid() && side.is_ask()), "{stored}");
    }
    // A cross, `OPPO`, `ASDF`, `UNDI`, a side stated as none and both
    // sides at once take no lane.
    for side in [
        Side::Unknown,
        Side::Both,
        Side::Cross,
        Side::CrossSh,
        Side::CrossShX,
        Side::AsDef,
        Side::Opposite,
        Side::Undisc,
        Side::Subscr,
        Side::Redeem,
        Side::Lend,
        Side::Borrow,
    ] {
        assert!(!side.is_bid() && !side.is_ask(), "{side}");
    }
}

#[test]
fn both_sides_at_once_is_its_own_member_with_no_wire_code() {
    crate::install::installed();
    let both = Side::Both;
    assert_eq!(both.code(), 99);
    assert_eq!(both.as_str(), "BOTH");
    assert_eq!(both.to_string(), "BOTH");
    assert!(!both.description().is_empty());
    assert_eq!(Side::from_code(99), Some(both));
    assert_eq!(Side::read_code(99).unwrap(), both);
    assert_eq!(Side::try_from(99_u8).unwrap(), both);
    assert_eq!(u8::from(both), 99);
    // No message carries it, so it has no wire character and takes neither
    // leg.
    assert_eq!(both.fix_code(), None);
    assert!(!both.is_bid() && !both.is_ask());
    // Its stored code, folded, and the word for it read; a stored code is
    // an integer and never text.
    for spelling in ["BOTH", "both", "Both", "two-sided", "TwoSided"] {
        assert_eq!(Side::from_spelling(spelling), Some(both), "{spelling}");
    }
    assert_eq!(Side::from_spelling("99"), None);
    // It is stated, so it stands over another side and a side stated as
    // none takes it.
    assert_eq!(both.merge_with(Side::Buy), both);
    assert_eq!(Side::Unknown.merge_with(both), both);
    // It serializes as its stored name and reads back by code or spelling.
    assert_eq!(serde_json::to_string(&both).unwrap(), "\"BOTH\"");
    assert_eq!(serde_json::from_str::<Side>("\"BOTH\"").unwrap(), both);
    assert_eq!(serde_json::from_str::<Side>("99").unwrap(), both);
    let value = Scalar::from(both);
    assert_eq!(
        Scalar::decode_value_bytes(&value.into_value_bytes()).unwrap(),
        value
    );
    assert_eq!(Side::dtype().scalar(Scalar::from(99_i32)).unwrap(), value);
}

#[test]
fn a_side_stated_as_none_merges_to_the_other_and_anything_stated_stands() {
    crate::install::installed();
    assert_eq!(Side::Unknown.merge_with(Side::Buy), Side::Buy);
    assert_eq!(Side::Unknown.merge_with(Side::Unknown), Side::Unknown);
    assert_eq!(Side::Buy.merge_with(Side::Sell), Side::Buy);
    assert_eq!(Side::Buy.merge_with(Side::Unknown), Side::Buy);
}

#[test]
fn a_side_serializes_as_its_stored_name_and_reads_back_by_code_or_spelling() {
    crate::install::installed();
    assert_eq!(serde_json::to_string(&Side::SShort).unwrap(), "\"SSHT\"");
    assert_eq!(serde_json::to_string(&Side::Unknown).unwrap(), "\"UKNW\"");
    // Every vocabulary reads back - the name stored before the codes
    // included - and so does the stored code, through a borrowed and an
    // owned door.
    for (spelling, side) in [
        ("\"SSHT\"", Side::SShort),
        ("\"SSHORT\"", Side::SShort),
        ("\"UKNW\"", Side::Unknown),
        ("\"5\"", Side::SShort),
        ("5", Side::SShort),
        ("\"sell_short\"", Side::SShort),
        ("\"UNKNOWN\"", Side::Unknown),
        ("0", Side::Unknown),
        ("2", Side::Sell),
        ("\"SELL\"", Side::Sell),
    ] {
        assert_eq!(serde_json::from_str::<Side>(spelling).unwrap(), side);
        let parsed: serde_json::Value = serde_json::from_str(spelling).unwrap();
        assert_eq!(serde_json::from_value::<Side>(parsed).unwrap(), side);
    }
    let refused = serde_json::from_str::<Side>("\"X\"")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("side"), "{refused}");
    assert!(serde_json::from_str::<Side>("18").is_err());

    // And a scalar carries the side, not the text and not the integer.
    let scalar = Scalar::from(Side::Buy);
    assert_eq!(scalar, Side::dtype().scalar(Scalar::from("1")).unwrap());
    assert_eq!(scalar, Side::dtype().scalar(Scalar::from(1_i32)).unwrap());
    assert_eq!(scalar, Side::dtype().scalar(Scalar::from(1_i64)).unwrap());
    assert_eq!(scalar, Side::dtype().scalar(Scalar::from("Buy")).unwrap());
    assert_eq!(scalar.as_str(), Some("BUYS"));
    assert_eq!(scalar.enum_code(), Some(1));
    assert_eq!(scalar.enum_name(), Some("BUYS"));
    assert!(scalar.is_enum());
    assert!(!scalar.is_code());
    assert_eq!(Side::from_scalar(&scalar), Some(&Side::Buy));
    assert_ne!(scalar, Scalar::from("BUYS"));
    assert_ne!(scalar, Scalar::from(1_i32));
    // The scalar's structural wire names the leaf and spells the name.
    let wire = serde_json::to_string(&scalar).unwrap();
    assert_eq!(wire, r#"{"type":"side","value":"BUYS"}"#);
    assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), scalar);
    // A code of another enum leaf and a spelling that names no side are
    // refused by the value door, naming the datatype.
    assert!(Side::dtype().scalar(Scalar::from(18_i32)).is_err());
    assert!(Side::dtype().scalar(Scalar::from("Z")).is_err());
    assert!(
        Side::dtype()
            .scalar(Scalar::State(yggdryl::State::New))
            .is_err()
    );
    // A side spells its stored name into a text column; it packs into no
    // integer, because its column holds its code already.
    assert_eq!(
        DataType::utf8().scalar(scalar.clone()).unwrap(),
        Scalar::from("BUYS")
    );
    assert!(Side::dtype().ascii_packed(b"BUYS").is_err());
}

#[test]
fn a_side_is_a_datatype_of_the_enum_family() {
    crate::install::installed();
    assert_eq!(Side::dtype().id(), Side::ID);
    assert_eq!(Side::ID.as_u8(), 0xc3);
    assert_eq!(DataTypeId::from_u8(0x75), None, "the code byte is retired");
    assert_eq!(Side::ID.as_str(), "side");
    assert_eq!(Side::ID.kind(), DataTypeKind::Enum);
    assert!(DataTypeKind::Enum.contains(Side::ID));
    assert!(Side::dtype().is_enum());
    assert!(!Side::dtype().is_code());
    assert!(!Side::dtype().is_string());
    assert_eq!(Side::dtype().code_width(), None);
    assert_eq!(Side::dtype().code_name(), None);
    assert_eq!(Side::dtype().fixed_byte_width(), None);
    assert_eq!(DataType::from_str("side").unwrap(), Side::dtype());
    assert_eq!(Side::dtype().to_string(), "side");
    assert_eq!(
        Side::dtype().default_value().unwrap(),
        Scalar::from(Side::Unknown)
    );
    assert!(
        DataType::CODES.iter().all(|(name, _, _)| *name != "side"),
        "a side is no registered code"
    );
    assert!(
        StringEnum::prebuilt()
            .iter()
            .any(|(name, _)| *name == "side")
    );
}

/// A kind states its own datatype and a nullable field of it - inherent items
/// of the enum, where `DataType` once held a constructor per kind - and both
/// are the descriptor's own, so a crate declaring a kind gets the same pair.
#[test]
fn a_side_states_its_own_datatype_and_a_nullable_field_of_it() {
    crate::install::installed();
    let dtype = Side::dtype();
    assert_eq!(dtype, DataType::Market(MarketType::new(&SIDE_KIND)));
    assert_eq!(dtype, SIDE_KIND.dtype());
    assert_eq!(dtype, DataType::from_str("side").unwrap());
    assert_eq!(dtype.id(), Side::ID);
    assert_eq!(dtype.to_string(), "side");
    assert!(dtype.is_enum());

    let field = Side::field("side");
    assert_eq!(field.name(), "side");
    assert!(field.is_nullable(), "a kind's field is nullable");
    assert_eq!(field.dtype(), &dtype);
    assert_eq!(field, SIDE_KIND.field("side", true));
    // Any text names it; nullability is the field's, so the required one is
    // the descriptor's alone.
    assert_eq!(Side::field(String::from("a side")).name(), "a side");
    assert!(!SIDE_KIND.field("side", false).is_nullable());
}

#[test]
fn a_column_is_uint8_codes_under_the_side_extension() {
    crate::install::installed();
    let field = Field::new("side", Side::dtype(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.side");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

    let values = [
        Scalar::from(Side::Buy),
        Scalar::Null,
        Scalar::from(Side::SellUnd),
    ];
    let serie = Serie::from_scalars(field.clone(), values.clone()).unwrap();
    assert!(matches!(serie, Serie::Market(MarketSerie::Code8(_))));
    let array = serie.require_arrow_array().unwrap();
    let codes = array
        .as_any()
        .downcast_ref::<arrow_array::UInt8Array>()
        .unwrap();
    assert_eq!(codes.values().as_ref(), [1, 0, 17]);
    assert!(codes.is_null(1));
    let back = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default()).unwrap();
    for (index, value) in values.iter().enumerate() {
        assert_eq!(&back.scalar(index).unwrap(), value, "row {index}");
    }

    // The text a side column used to be stored as is a foreign field wearing
    // the name now, and imports as the text it is.
    let foreign = arrow_schema::Field::new("side", ArrowDataType::Utf8, true).with_metadata(
        [
            ("ARROW:extension:name".to_owned(), "yggdryl.side".to_owned()),
            ("ARROW:extension:metadata".to_owned(), String::new()),
        ]
        .into(),
    );
    assert_eq!(
        Field::from_arrow_field(&foreign).unwrap().dtype(),
        &DataType::utf8()
    );
}

#[test]
fn text_and_integers_land_as_sides_and_a_stranger_is_refused_by_row() {
    crate::install::installed();
    let field = Field::new("side", Side::dtype(), false);
    // A `utf8` column of the stored name, a wire code and the specification's
    // name lands as the codes they name.
    let landed = Serie::from_arrow_array(
        Some(&field),
        Arc::new(StringArray::from(vec!["Buy", "1", "SELL", "SellShort"])) as ArrayRef,
        strict(),
    )
    .unwrap();
    let codes = landed.require_arrow_array().unwrap();
    let codes = codes
        .as_any()
        .downcast_ref::<arrow_array::UInt8Array>()
        .unwrap();
    assert_eq!(codes.values().as_ref(), [1, 1, 2, 5]);
    assert_eq!(landed.scalar(3).unwrap(), Scalar::from(Side::SShort));
    let text = landed
        .cast(&Field::new("side", DataType::utf8(), false), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    let names = text.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(
        names.iter().collect::<Vec<_>>(),
        [Some("BUYS"), Some("BUYS"), Some("SELL"), Some("SSHT")]
    );

    let nullable = Field::new("side", Side::dtype(), true);
    let landed = Serie::from_arrow_array(
        Some(&nullable),
        Arc::new(Int64Array::from(vec![Some(2), None, Some(17)])) as ArrayRef,
        strict(),
    )
    .unwrap();
    assert_eq!(landed.scalar(0).unwrap(), Scalar::from(Side::Sell));
    assert_eq!(landed.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(landed.scalar(2).unwrap(), Scalar::from(Side::SellUnd));
    let refused = Serie::from_arrow_array(
        Some(&nullable),
        Arc::new(Int32Array::from(vec![1, 18])) as ArrayRef,
        strict(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    let safe = Serie::from_arrow_array(
        Some(&nullable),
        Arc::new(Int32Array::from(vec![1, 18])) as ArrayRef,
        ArrowCastOptions::new().with_safe(true),
    )
    .unwrap();
    assert_eq!(safe.scalar(1).unwrap(), Scalar::Null);
    let refused = Serie::from_arrow_array(
        Some(&nullable),
        Arc::new(StringArray::from(vec!["BUY", "X"])) as ArrayRef,
        strict(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 1"), "{refused}");
}

#[test]
fn a_side_crosses_the_value_stream_the_digest_and_the_structured_codecs() {
    crate::install::installed();
    for side in [Side::Unknown, Side::Buy, Side::SellUnd] {
        let value = Scalar::from(side);
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[1], Side::ID.as_u8());
        // The canonical four bytes, whatever width a column stores.
        assert_eq!(bytes[2..], i32::from(side.code()).to_le_bytes());
        assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
        // A digest reads the code under the leaf's tag: a member's name is
        // free to change, and a state or an integer of the same code is
        // another value.
        assert_ne!(
            value.digest(DigestAlgorithm::Xxh3),
            Scalar::from(side.code()).digest(DigestAlgorithm::Xxh3),
            "{side}"
        );
        assert_ne!(
            value.digest(DigestAlgorithm::Xxh3),
            Scalar::State(yggdryl::State::from_code(u16::from(side.code())).unwrap_or_default())
                .digest(DigestAlgorithm::Xxh3),
            "{side}"
        );
    }
    let value = Scalar::from(Side::Buy);
    let row = Scalar::from_struct([("side", value.clone())]).unwrap();
    assert_eq!(
        yggdryl::into_json_scalar(&row).unwrap(),
        r#"{"side":"BUYS"}"#
    );
    assert_eq!(
        yggdryl::into_toml_scalar(&row).unwrap(),
        "\"side\" = \"BUYS\"\n"
    );
    assert!(yggdryl::into_yaml_scalar(&row).unwrap().contains("BUYS"));
    assert_eq!(
        yggdryl::into_xml_scalar(&Scalar::from_struct([("row", row)]).unwrap()).unwrap(),
        "<row><side>BUYS</side></row>"
    );
    assert_eq!(
        value.into_variant().unwrap().scalar().unwrap(),
        Scalar::from("BUYS")
    );
    assert_eq!(
        yggdryl::media::partition::partition_text(&value).unwrap(),
        "BUYS"
    );
}

#[test]
fn a_side_filters_and_casts_by_its_member_in_an_expression() {
    crate::install::installed();
    use yggdryl::expression::{Expression, Filter};

    let root =
        DataType::from(StructType::from_fields([Side::dtype().nullable_field("side")]).unwrap())
            .required_field("row");
    let sides = [Side::Buy, Side::Sell, Side::SShort, Side::Cross];
    let column = Serie::from_scalars(
        root.fields()[0].clone(),
        sides
            .iter()
            .map(|side| Scalar::from(*side))
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .require_arrow_array()
    .unwrap();
    let batch =
        arrow_array::RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column])
            .unwrap();
    for (clause, kept) in [
        ("side = 'BUYS'", 1),
        ("side = 'BUY'", 1),
        ("side in ('1', 'SellShort')", 2),
        ("side = side 'CROS'", 1),
        ("cast('2' as side) = side", 1),
    ] {
        let filter: Filter = clause.parse().unwrap();
        assert_eq!(
            filter.apply_arrow_batch(&batch).unwrap().num_rows(),
            kept,
            "{clause}"
        );
    }
    for (clause, kept) in [
        ("where side < 3", 2),
        ("where side >= 5", 2),
        ("where cast(side as int64) = 8", 1),
    ] {
        let expression: Expression = clause.parse().unwrap();
        assert_eq!(
            expression.apply_arrow_batch(&batch).unwrap().num_rows(),
            kept,
            "{clause}"
        );
    }
}

#[test]
fn the_canonical_default_is_the_side_stated_as_none() {
    crate::install::installed();
    // A closed vocabulary with no empty member: its default is `UKNW`,
    // code zero as every enum leaf's, so a named row leaving out a required
    // side defaults at the value door rather than failing on the empty text.
    assert_eq!(
        Side::dtype().default_value().unwrap(),
        Scalar::from(Side::Unknown)
    );
    assert!(
        Side::dtype()
            .is_default_value(&Scalar::from(Side::Unknown))
            .unwrap()
    );
    let root = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            Side::dtype().required_field("side"),
        ])
        .unwrap(),
    )
    .required_field("row");
    assert_eq!(
        root.scalar(Scalar::from_struct([("id", Scalar::from(1_i64))]).unwrap())
            .unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(Side::Unknown)])
    );
}

#[test]
fn there_is_no_member_meaning_no_answer_and_null_is_how_a_row_says_it() {
    crate::install::installed();
    // A row whose line does not say a side has none, and the crate already
    // spells "no answer" one way: `UKNW` is what a value that must state a
    // side states where none was said, as a state's `UNKNOWN` is, and
    // never what a column says for an absent one.
    assert!(Side::ALL.contains(&Side::Unknown));
    assert!(Side::from_spelling("NONE").is_none());

    let field = Field::new("side", Side::dtype(), true);
    let row = Field::new(
        "row",
        DataType::from(StructType::from_fields([field.clone()]).unwrap()),
        false,
    );
    let value = row
        .canonicalize_value(Scalar::from_sequence([Scalar::Null]))
        .unwrap();
    row.validate_value(&value).unwrap();
    assert!(value.as_sequence().unwrap()[0].is_null());
    // A required one refuses the same null, so the nullability is the field's
    // and not the datatype's.
    let required = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("side", Side::dtype(), false)]).unwrap(),
        ),
        false,
    );
    assert!(
        required
            .validate_value(&Scalar::from_sequence([Scalar::Null]))
            .is_err()
    );
}

/// The wire contract of `side`, pinned so that no byte of it moves: the
/// identifier byte and name, the Arrow extension and its storage in both
/// directions, the serde tags, the value-stream bytes, the canonical
/// digests and the names that reach it. Generated from the facts the tree
/// answered, never typed by hand.
#[test]
fn the_side_wire_contracts_are_pinned() {
    crate::install::installed();
    use std::sync::Arc;

    use arrow_array::{ArrayRef, UInt8Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors a claimed kind is read by: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("side").unwrap();
    let value = dtype.scalar("BUYS").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0xc3).unwrap());
    assert_eq!(dtype.id().as_u8(), 0xc3);
    assert_eq!(dtype.id().as_str(), "side");
    assert_eq!(dtype.id().kind(), DataTypeKind::Enum);
    assert_eq!(dtype.to_string(), "side");
    assert_eq!(value.kind(), "side");
    assert_eq!(value.id().as_u8(), 0xc3);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(value.enum_code(), Some(1_u16));
    assert_eq!(value.enum_name(), Some("BUYS"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "side");
    assert_eq!(Field::from_str("value side").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("side").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["side"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.side"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.side")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(UInt8Array::from(vec![1_u8]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(serde_json::to_string(&dtype).unwrap(), r#"{"type":"side"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"side"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"side"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"side"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"side","value":"BUYS"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"side","value":"BUYS"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 195, 1, 0, 0, 0]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype.encode_value_bytes(&Scalar::from("BUYS")).unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 10873967693893398222);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(10873967693893398222)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(5798505129261801236)
    );
    assert_eq!(dtype.stable_hash(), 8287515046321296587);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `side`, which no caller names.

    /// The std `Hash` feed - the value's rank then its identity, the
    /// datatype's `Shape` position - read through the door that feeds it to
    /// XXH3, pinned so no persisted digest moves.
    #[test]
    fn the_side_std_hash_feed_is_pinned() {
        crate::install::installed();
        use yggdryl::DataType;

        let dtype = DataType::from_str("side").unwrap();
        let value = dtype.scalar("BUYS").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            14287857958782127812
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            5956444352070576251
        );
    }
}
