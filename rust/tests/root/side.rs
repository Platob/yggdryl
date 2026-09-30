//! `rust/src/side.rs`: FIX's side of a trade as a one-byte enum stored as
//! the `int32` code of its member, read by spelling or code and refused
//! where neither names one.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, EnumValue, Field,
    Scalar, Serie, Side, StringEnum, StructType, Value,
};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// Every side: its wire code, its stored four-letter code, the name it was
/// stored under before the codes - read, never written - and the name the
/// specification gives it, in the order of the codes.
const SIDES: [(Option<char>, &str, &str, &str, Side); 18] = [
    (None, "UNKN", "UNKNOWN", "Unknown", Side::Unknown),
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

#[test]
fn a_side_is_one_byte_whose_code_is_the_position_of_its_wire_character() {
    assert_eq!(std::mem::size_of::<Side>(), 1);
    assert_eq!(std::mem::size_of::<Option<Side>>(), 1);
    assert_eq!(Side::default(), Side::Unknown);
    assert_eq!(Side::ALL.len(), 18);
    assert_eq!(<Side as EnumValue>::ALL, Side::ALL);
    assert_eq!(<Side as EnumValue>::KIND, "side");
    assert_eq!(<Side as EnumValue>::EXTENSION_NAME, "yggdryl.side");

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
    // The members order as the wire codes do, `1`..=`9` then `A`..=`H`.
    let mut ordered: Vec<Side> = SIDES.iter().map(|(_, _, _, _, side)| *side).collect();
    ordered.reverse();
    ordered.sort_unstable();
    assert_eq!(ordered, Side::ALL);
    // The string listing of the same vocabulary is the eighteen stored
    // codes, sorted: what a fixed-ASCII column may declare it holds.
    let mut listed: Vec<&str> = SIDES.iter().map(|(_, stored, _, _, _)| *stored).collect();
    listed.sort_unstable();
    assert_eq!(listed.as_slice(), StringEnum::SIDES);
}

#[test]
fn every_vocabulary_reaches_one_side_and_a_wire_code_never_folds() {
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
    // A cross, `OPPO`, `ASDF`, `UNDI` and a side stated as none
    // take no lane.
    for side in [
        Side::Unknown,
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
fn a_side_stated_as_none_merges_to_the_other_and_anything_stated_stands() {
    assert_eq!(Side::Unknown.merge_with(Side::Buy), Side::Buy);
    assert_eq!(Side::Unknown.merge_with(Side::Unknown), Side::Unknown);
    assert_eq!(Side::Buy.merge_with(Side::Sell), Side::Buy);
    assert_eq!(Side::Buy.merge_with(Side::Unknown), Side::Buy);
}

#[test]
fn a_side_serializes_as_its_stored_name_and_reads_back_by_code_or_spelling() {
    assert_eq!(serde_json::to_string(&Side::SShort).unwrap(), "\"SSHT\"");
    assert_eq!(serde_json::to_string(&Side::Unknown).unwrap(), "\"UNKN\"");
    // Every vocabulary reads back - the name stored before the codes
    // included - and so does the stored code, through a borrowed and an
    // owned door.
    for (spelling, side) in [
        ("\"SSHT\"", Side::SShort),
        ("\"SSHORT\"", Side::SShort),
        ("\"UNKN\"", Side::Unknown),
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
    assert_eq!(scalar, DataType::Side.scalar(Scalar::from("1")).unwrap());
    assert_eq!(scalar, DataType::Side.scalar(Scalar::from(1_i32)).unwrap());
    assert_eq!(scalar, DataType::Side.scalar(Scalar::from(1_i64)).unwrap());
    assert_eq!(scalar, DataType::Side.scalar(Scalar::from("Buy")).unwrap());
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
    assert!(DataType::Side.scalar(Scalar::from(18_i32)).is_err());
    assert!(DataType::Side.scalar(Scalar::from("Z")).is_err());
    assert!(
        DataType::Side
            .scalar(Scalar::State(yggdryl::State::New))
            .is_err()
    );
    // A side spells its stored name into a text column; it packs into no
    // integer, because its column holds its code already.
    assert_eq!(
        DataType::utf8().scalar(scalar.clone()).unwrap(),
        Scalar::from("BUYS")
    );
    assert!(DataType::Side.ascii_packed(b"BUYS").is_err());
}

#[test]
fn a_side_is_a_datatype_of_the_enum_family() {
    assert_eq!(DataType::Side.id(), DataTypeId::Side);
    assert_eq!(DataTypeId::Side.as_u8(), 0xc3);
    assert_eq!(DataTypeId::from_u8(0x75), None, "the code byte is retired");
    assert_eq!(DataTypeId::Side.as_str(), "side");
    assert_eq!(DataTypeId::Side.kind(), DataTypeKind::Enum);
    assert!(DataTypeKind::Enum.contains(DataTypeId::Side));
    assert!(DataType::Side.is_enum());
    assert!(!DataType::Side.is_code());
    assert!(!DataType::Side.is_string());
    assert_eq!(DataType::Side.code_width(), None);
    assert_eq!(DataType::Side.code_name(), None);
    assert_eq!(DataType::Side.fixed_byte_width(), None);
    assert_eq!(DataType::from_str("side").unwrap(), DataType::Side);
    assert_eq!(DataType::Side.to_string(), "side");
    assert_eq!(
        DataType::Side.default_value().unwrap(),
        Scalar::Side(Side::Unknown)
    );
    assert!(
        DataType::CODES.iter().all(|(name, _, _)| *name != "side"),
        "a side is no registered code"
    );
    assert!(StringEnum::PREBUILT.iter().any(|(name, _)| *name == "side"));
}

#[test]
fn a_column_is_uint8_codes_under_the_side_extension() {
    let field = Field::new("side", DataType::Side, true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.side");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

    let values = [
        Scalar::Side(Side::Buy),
        Scalar::Null,
        Scalar::Side(Side::SellUnd),
    ];
    let serie = Serie::from_scalars(field.clone(), values.clone()).unwrap();
    assert!(matches!(serie, Serie::Side(_)));
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
    let field = Field::new("side", DataType::Side, false);
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
    assert_eq!(landed.scalar(3).unwrap(), Scalar::Side(Side::SShort));
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

    let nullable = Field::new("side", DataType::Side, true);
    let landed = Serie::from_arrow_array(
        Some(&nullable),
        Arc::new(Int64Array::from(vec![Some(2), None, Some(17)])) as ArrayRef,
        strict(),
    )
    .unwrap();
    assert_eq!(landed.scalar(0).unwrap(), Scalar::Side(Side::Sell));
    assert_eq!(landed.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(landed.scalar(2).unwrap(), Scalar::Side(Side::SellUnd));
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
    for side in [Side::Unknown, Side::Buy, Side::SellUnd] {
        let value = Scalar::Side(side);
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[1], DataTypeId::Side.as_u8());
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
    let value = Scalar::Side(Side::Buy);
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
    use yggdryl::expression::{Expression, Filter};

    let root =
        DataType::from(StructType::from_fields([DataType::Side.nullable_field("side")]).unwrap())
            .required_field("row");
    let sides = [Side::Buy, Side::Sell, Side::SShort, Side::Cross];
    let column = Serie::from_scalars(
        root.fields()[0].clone(),
        sides
            .iter()
            .map(|side| Scalar::Side(*side))
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
    // A closed vocabulary with no empty member: its default is `UNKN`,
    // code zero as every enum leaf's, so a named row leaving out a required
    // side defaults at the value door rather than failing on the empty text.
    assert_eq!(
        DataType::Side.default_value().unwrap(),
        Scalar::Side(Side::Unknown)
    );
    assert!(
        DataType::Side
            .is_default_value(&Scalar::Side(Side::Unknown))
            .unwrap()
    );
    let root = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Side.required_field("side"),
        ])
        .unwrap(),
    )
    .required_field("row");
    assert_eq!(
        root.scalar(Scalar::from_struct([("id", Scalar::from(1_i64))]).unwrap())
            .unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::Side(Side::Unknown)])
    );
}

#[test]
fn there_is_no_member_meaning_no_answer_and_null_is_how_a_row_says_it() {
    // A row whose line does not say a side has none, and the crate already
    // spells "no answer" one way: `UNKN` is what a value that must state a
    // side states where none was said, as a state's `UNKNOWN` is, and
    // never what a column says for an absent one.
    assert!(Side::ALL.contains(&Side::Unknown));
    assert!(Side::from_spelling("NONE").is_none());

    let field = Field::new("side", DataType::Side, true);
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
            StructType::from_fields([Field::new("side", DataType::Side, false)]).unwrap(),
        ),
        false,
    );
    assert!(
        required
            .validate_value(&Scalar::from_sequence([Scalar::Null]))
            .is_err()
    );
}
