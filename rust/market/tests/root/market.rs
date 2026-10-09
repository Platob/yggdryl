//! `rust/src/market.rs` over the four enum kinds `yggdryl-market` claims:
//! what each is to the four root enums once installed - its byte, name,
//! extension name, datatype, field and ranks, the ones the core reserves
//! for it.
//! What the register answers and refuses with no kind claimed is
//! `rust/tests/root/market.rs`'s, and a claim's own refusals
//! `rust/market/tests/market_register.rs`'s, which owns its process.

use yggdryl::market::{kind_for_extension, kind_named, kind_of, kinds};
use yggdryl::{DataType, DataTypeId, DataTypeKind, Field, MarketStorage, Scalar, Serie};
use yggdryl_market::{MarketDataKind, MarketDataType, Side, TimeInForce};

#[test]
fn the_market_crate_claims_the_four_enum_kinds_in_byte_order() {
    crate::install::installed();
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
}

/// The datatype and the field a kind states of itself are what the register
/// holds for it: the four enum kinds each answer through their own associated
/// items, and `DataType` holds no constructor of any of them.
#[test]
fn each_enum_kind_states_the_datatype_and_field_the_register_holds_for_it() {
    crate::install::installed();
    for (name, dtype, field) in [
        (
            "marketdatakind",
            MarketDataKind::dtype(),
            MarketDataKind::field("value"),
        ),
        ("side", Side::dtype(), Side::field("value")),
        (
            "marketdatatype",
            MarketDataType::dtype(),
            MarketDataType::field("value"),
        ),
        (
            "timeinforce",
            TimeInForce::dtype(),
            TimeInForce::field("value"),
        ),
    ] {
        let claimed = kind_named(name).unwrap();
        assert_eq!(dtype, claimed.dtype(), "{name}");
        assert_eq!(dtype, DataType::from_str(name).unwrap(), "{name}");
        assert_eq!(dtype.id(), claimed.id, "{name}");
        assert_eq!(field, claimed.field("value", true), "{name}");
        assert!(field.is_nullable(), "{name}");
        assert_eq!(field.dtype(), &dtype, "{name}");
    }
}

#[test]
fn a_market_type_compares_as_its_kind_states() {
    crate::install::installed();
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
    assert_eq!(Side::dtype(), side);
    assert_ne!(DataType::from_str("timeinforce").unwrap(), side);
    assert!(
        side < DataType::from_str("marketdatatype").unwrap(),
        "rank 53 before 74"
    );
    // A code orders among the datatypes by its own rank, as a variant.
    assert!(DataType::Country < DataType::Isin, "rank 30 before 58");
    assert!(DataType::Cfi < side, "rank 33 before 53");
}

/// The order across the market kinds and `State`: the seventeen codes
/// share value rank 18 and order by identifier byte then text, each enum
/// kind keeps its own rank - `state` 27, `marketdatakind` 28, `side` 29,
/// `marketdatatype` 30, `timeinforce` 31 - and orders by code within it.
/// The rank is wire-visible: it is the order of an Arrow dictionary's
/// values and of any caller's sort.
#[test]
fn the_market_kinds_and_state_order_by_rank_then_by_identity() {
    crate::install::installed();
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
    // ranks follow them: a wire fact that stays as it is.
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

/// The core's market doors over a claimed kind: an enum door reads the
/// kind's spelling as its member, and a code adopted under the kind is that
/// kind's member. The doors over the core's own leaves are
/// `rust/tests/root/implementer.rs`'s.
#[test]
fn the_enum_doors_read_a_claimed_kind_as_its_member() {
    use yggdryl::implementer::{adopt_market_code, read_enum_spelling};
    use yggdryl_market::SIDE_KIND;

    crate::install::installed();
    assert_eq!(
        read_enum_spelling(&Side::dtype(), "Buy").unwrap(),
        Scalar::from(Side::Buy)
    );
    assert!(read_enum_spelling(&Side::dtype(), "zzzz").is_err());
    assert_eq!(adopt_market_code(&SIDE_KIND, 1), Scalar::from(Side::Buy));
}
