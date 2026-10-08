//! `rust/src/market.rs`: the market extension point - what a registered
//! kind is to the four root enums, and what the register answers for the
//! core's own four enum kinds and refuses for a name, a byte or a tag no
//! claim holds. The kinds a crate claims are pinned in
//! `rust/tests/market_register.rs`, which owns its process. The seventeen
//! codes are the core's own, flat variants, and no claim's: each is pinned
//! in its own file beside this one.

use arrow_schema::DataType as ArrowDataType;
use yggdryl::market::{kind_for_extension, kind_named, kind_of, kinds};
use yggdryl::{DataType, DataTypeId, DataTypeKind, Field, MarketStorage, Scalar, Serie};

#[test]
fn the_core_claims_its_own_four_enum_kinds_in_byte_order() {
    let claimed = kinds();
    let names: Vec<&str> = claimed.iter().map(|kind| kind.name).collect();
    assert_eq!(
        names,
        ["marketdatakind", "side", "marketdatatype", "timeinforce"]
    );
    for kind in claimed {
        assert_eq!(
            DataType::from_str(kind.name).unwrap(),
            kind.dtype(),
            "{}",
            kind.name
        );
        assert!(std::ptr::eq(kind_of(kind.id).unwrap(), kind));
        assert!(std::ptr::eq(kind_named(kind.name).unwrap(), kind));
        assert!(std::ptr::eq(
            kind_for_extension(kind.extension_name).unwrap(),
            kind
        ));
        assert_eq!(kind.extension_name, format!("yggdryl.{}", kind.name));
        assert_eq!(kind.id.as_str(), kind.name);
        assert_eq!(kind.id.arrow_extension_name(), Some(kind.extension_name));
        assert_eq!(kind.id.kind(), DataTypeKind::Enum);
        assert!(!kind.members.is_empty());
        assert!(
            kind.members
                .windows(2)
                .all(|pair| pair[0].code < pair[1].code)
        );
        assert!(
            kind.members
                .iter()
                .all(|member| kind.storage.fits(member.code))
        );
        assert!(!kind.id.is_core());
        assert!(kind.id.is_registered());
    }
    // The listing is the core's ninety-two - the seventeen codes among
    // them - with the four kinds spliced in after `state`.
    assert_eq!(DataTypeId::ALL.len(), 92);
    assert_eq!(DataTypeId::all().len(), 96);
    let all = DataTypeId::all();
    let state = all.iter().position(|id| *id == DataTypeId::State).unwrap();
    assert_eq!(
        &all[state + 1..],
        [0xc2, 0xc3, 0xc4, 0xc5].map(DataTypeId::market)
    );
    // A code is no kind: the core's own, at its own byte.
    assert!(DataTypeId::ALL.contains(&DataTypeId::Isin));
    assert!(DataTypeId::Isin.is_core());
    assert!(kind_of(DataTypeId::Isin).is_none());
    assert!(kind_named("isin").is_none());
    assert!(kind_for_extension("yggdryl.isin").is_none());
    assert_eq!(DataType::Isin.id(), DataTypeId::Isin);
}

#[test]
fn a_name_a_byte_and_a_tag_no_claim_answers_are_refused_naming_the_registration() {
    assert!(DataTypeId::from_u8(0xcf).is_none());
    assert!(kind_of(DataTypeId::market(0xcf)).is_none());
    assert!(kind_named("nosuchkind").is_none());
    assert!(kind_for_extension("yggdryl.nosuchkind").is_none());
    for refused in [
        DataType::from_str("nosuchkind").unwrap_err().to_string(),
        DataTypeId::from_str("nosuchkind").unwrap_err().to_string(),
        Field::from_str("value nosuchkind").unwrap_err().to_string(),
        serde_json::from_str::<DataType>(r#"{"type":"nosuchkind"}"#)
            .unwrap_err()
            .to_string(),
        serde_json::from_str::<Scalar>(r#"{"type":"nosuchkind","value":"X"}"#)
            .unwrap_err()
            .to_string(),
        Scalar::decode_value_bytes(&[0, 0xcf, 1, 0, 0, 0])
            .unwrap_err()
            .to_string(),
    ] {
        assert!(
            refused.contains("no registered datatype answers"),
            "{refused}"
        );
        assert!(refused.contains("call its `install()`"), "{refused}");
    }
    // The Arrow fallback stays lossless: an unregistered name imports as
    // its storage, exactly as any foreign extension does.
    let foreign = arrow_schema::Field::new("value", ArrowDataType::UInt8, true).with_metadata(
        [(
            "ARROW:extension:name".to_owned(),
            "yggdryl.nosuchkind".to_owned(),
        )]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        Field::from_arrow_field(&foreign).unwrap().dtype(),
        &DataType::UInt8
    );
}

#[test]
fn a_market_type_compares_as_its_kind_states() {
    let side = DataType::from_str("side").unwrap();
    let DataType::Market(kind) = &side else {
        panic!("a registered kind");
    };
    assert_eq!(kind.name(), "side");
    assert_eq!(kind.id().as_u8(), 0xc3);
    assert_eq!(kind.storage(), MarketStorage::Code8);
    assert_eq!(kind.kind().dtype_rank, 53);
    assert_eq!(kind.kind().shape, 28);
    assert_eq!(kind.kind().value_rank, 29);
    assert_eq!(side.clone().nullable_field("value").dtype(), &side);
    // A kind stated twice is one kind: the byte is the identity.
    assert_eq!(DataType::side(), side);
    assert_ne!(DataType::from_str("timeinforce").unwrap(), side);
    assert!(
        side < DataType::from_str("marketdatatype").unwrap(),
        "rank 53 before 74"
    );
    // A code orders among the datatypes by its own rank, as a variant.
    assert!(DataType::Country < DataType::Isin, "rank 30 before 58");
    assert!(DataType::Cfi < side, "rank 33 before 53");
}

/// The order across the market kinds and `State` the split keeps (D19): the
/// seventeen codes share value rank 18 and order by identifier byte then
/// text, each enum kind keeps its own rank - `state` 27, `marketdatakind`
/// 28, `side` 29, `marketdatatype` 30, `timeinforce` 31 - and orders by
/// code within it. The rank is wire-visible: it is the order of an Arrow
/// dictionary's values and of any caller's sort. Pinned on `2ae975674`.
#[test]
fn the_market_kinds_and_state_order_by_rank_then_by_identity() {
    use arrow_array::types::Int32Type;
    use arrow_array::{Array, DictionaryArray, StringArray};

    // (value, rank, identifier byte), ascending.
    let ascending: [(Scalar, u8, u8); 22] = [
        (
            DataType::from_str("lei")
                .unwrap()
                .scalar("HWUPKR0MPOU8FGXBT394")
                .unwrap(),
            18,
            0x6b,
        ),
        (
            DataType::from_str("bic")
                .unwrap()
                .scalar("DEUTDEFFXXX")
                .unwrap(),
            18,
            0x6c,
        ),
        (
            DataType::from_str("elf").unwrap().scalar("2HBR").unwrap(),
            18,
            0x6d,
        ),
        (
            DataType::from_str("dti")
                .unwrap()
                .scalar("X9J9K872S")
                .unwrap(),
            18,
            0x6e,
        ),
        (
            DataType::from_str("fisn")
                .unwrap()
                .scalar("ACME CORP/SH")
                .unwrap(),
            18,
            0x6f,
        ),
        (
            DataType::from_str("country").unwrap().scalar("FR").unwrap(),
            18,
            0x71,
        ),
        (
            DataType::from_str("ccy").unwrap().scalar("USD").unwrap(),
            18,
            0x72,
        ),
        (
            DataType::from_str("mic").unwrap().scalar("XPAR").unwrap(),
            18,
            0x73,
        ),
        (
            DataType::from_str("cfi").unwrap().scalar("ESVUFR").unwrap(),
            18,
            0x74,
        ),
        (
            DataType::from_str("isin")
                .unwrap()
                .scalar("US0378331005")
                .unwrap(),
            18,
            0x78,
        ),
        (
            DataType::from_str("cusip")
                .unwrap()
                .scalar("037833100")
                .unwrap(),
            18,
            0x79,
        ),
        (
            DataType::from_str("sedol")
                .unwrap()
                .scalar("B0YBKJ7")
                .unwrap(),
            18,
            0x7a,
        ),
        (
            DataType::from_str("bbg")
                .unwrap()
                .scalar("AAPL US Equity")
                .unwrap(),
            18,
            0x7b,
        ),
        (
            DataType::from_str("figi")
                .unwrap()
                .scalar("BBG000BLNQ16")
                .unwrap(),
            18,
            0x7c,
        ),
        (
            DataType::from_str("unit").unwrap().scalar("MWh").unwrap(),
            18,
            0x7d,
        ),
        (
            DataType::from_str("ric").unwrap().scalar("VOD.L").unwrap(),
            18,
            0x7e,
        ),
        (
            DataType::from_str("forex")
                .unwrap()
                .scalar("EUR/USD")
                .unwrap(),
            18,
            0x7f,
        ),
        (
            DataType::from_str("state")
                .unwrap()
                .scalar("PENDING_NEW")
                .unwrap(),
            27,
            0xc1,
        ),
        (
            DataType::from_str("marketdatakind")
                .unwrap()
                .scalar("ORDR")
                .unwrap(),
            28,
            0xc2,
        ),
        (
            DataType::from_str("side").unwrap().scalar("BUYS").unwrap(),
            29,
            0xc3,
        ),
        (
            DataType::from_str("marketdatatype")
                .unwrap()
                .scalar("ORDLIMIT")
                .unwrap(),
            30,
            0xc4,
        ),
        (
            DataType::from_str("timeinforce")
                .unwrap()
                .scalar("GTC")
                .unwrap(),
            31,
            0xc5,
        ),
    ];
    for pair in ascending.windows(2) {
        assert!(pair[0].0 < pair[1].0, "{:?} !< {:?}", pair[0].0, pair[1].0);
        assert!(pair[0].1 <= pair[1].1);
    }
    // The values order by identifier byte and the datatypes by their rank
    // (`rust/tests/root/datatype.rs`), and the two disagree for the five
    // reference-data codes, whose bytes precede every other code's and whose
    // ranks follow them: a wire fact the split keeps as it is.
    let fisn = &ascending[4].0;
    let country = &ascending[5].0;
    assert!(fisn < country && fisn.dtype().unwrap() > country.dtype().unwrap());
    let isin = &ascending[9].0;
    assert!(country < isin && country.dtype().unwrap() < isin.dtype().unwrap());
    for (value, rank, byte) in &ascending {
        assert_eq!(value.id().as_u8(), *byte, "{value:?}");
        assert_eq!(*rank, if value.is_code() { 18 } else { *rank });
    }
    // Below the codes: text (5) and every container (11-13); above them a
    // URL (20), and above every code every enum kind (27-31).
    let text = Scalar::from("ZZZ");
    let url = Scalar::from(yggdryl::Url::from_str("https://example.com/a").unwrap());
    let sequence = Scalar::from_sequence([]);
    let first_code = &ascending[0].0;
    let last_code = &ascending[16].0;
    let first_enum = &ascending[17].0;
    assert!(text < *first_code && sequence < *first_code);
    assert!(*last_code < url && url < *first_enum);
    assert!(last_code < first_enum);
    // Within one code the text orders; within one enum the code does.
    let isin = DataType::from_str("isin").unwrap();
    let side = DataType::from_str("side").unwrap();
    let state = DataType::from_str("state").unwrap();
    assert!(isin.scalar("FR0000120271").unwrap() < isin.scalar("US0378331005").unwrap());
    assert!(side.scalar("BUYS").unwrap() < side.scalar("SELL").unwrap());
    assert!(state.scalar("PENDING_NEW").unwrap() < state.scalar("UPDATED").unwrap());
    // A dictionary over a code column lays its values out in that order,
    // which is what an Arrow reader sees.
    let field = Field::from_str("value dictionary<int32, isin>").unwrap();
    let column = Serie::from_scalars(
        field,
        [
            isin.scalar("US0378331005").unwrap(),
            isin.scalar("FR0000120271").unwrap(),
            isin.scalar("US0378331005").unwrap(),
        ],
    )
    .unwrap();
    let array = column.into_arrow_array().unwrap();
    let dictionary = array
        .as_any()
        .downcast_ref::<DictionaryArray<Int32Type>>()
        .expect("a dictionary array");
    let values = dictionary
        .values()
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("utf8 values");
    assert_eq!(
        values.iter().collect::<Vec<_>>(),
        [Some("FR0000120271"), Some("US0378331005")]
    );
    assert_eq!(dictionary.keys().values(), &[1, 0, 1]);
}
