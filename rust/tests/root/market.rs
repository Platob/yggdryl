//! `rust/src/market.rs`: the market extension point - what the register
//! answers in a process no crate has claimed a kind in, and what it refuses
//! for a name, a byte or a tag no claim holds. The kinds `yggdryl-market`
//! claims are pinned in `rust/market/tests/root/market.rs`, and a claim's
//! own refusals in `rust/market/tests/market_register.rs`, which owns its
//! process. The seventeen codes are the core's own, flat variants, and no
//! claim's: each is pinned in its own file beside this one.

use arrow_schema::DataType as ArrowDataType;
use yggdryl::market::{kind_for_extension, kind_named, kind_of, kinds};
use yggdryl::{DataType, DataTypeId, Field, Scalar, Serie};

/// The core claims no kind: its listing is its own, the seventeen codes
/// among them, and a reserved kind is a name no claim answers until the
/// crate that owns it installs, refused naming that install.
#[test]
fn the_core_claims_no_kind_and_a_code_is_no_kind() {
    assert!(kinds().is_empty(), "nothing installed in this process");
    assert_eq!(DataTypeId::all(), DataTypeId::ALL);
    for reserved in ["marketdatakind", "side", "marketdatatype", "timeinforce"] {
        assert!(kind_named(reserved).is_none(), "{reserved}");
        let refused = DataType::from_str(reserved).unwrap_err().to_string();
        assert!(
            refused.contains("no registered datatype answers"),
            "{refused}"
        );
        assert!(
            refused.contains(&format!(
                "`{reserved}` is read only once the crate that claims it is installed \
                 (`yggdryl_market::install()`)"
            )),
            "{refused}"
        );
    }
    for byte in 0xc2..=0xc5 {
        assert!(DataTypeId::from_u8(byte).is_none(), "{byte:#04x}");
    }
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
        // No reserved kind's word: there is no install to name.
        assert!(!refused.contains("install"), "{refused}");
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

/// A reserved kind is claimed only at its byte, its name and its extension
/// name together, by the crate that owns it: in a process `yggdryl-market`
/// never installed in, a claim of `side` by another crate, or of one of its
/// keys without the others, is refused and registers nothing.
#[test]
fn a_reserved_kind_is_claimed_only_at_its_keys_by_its_owner() {
    use yggdryl::market::claim;
    use yggdryl::{MarketDescriptor, MarketMember, MarketStorage};

    const MEMBERS: &[MarketMember] = &[MarketMember {
        code: 0,
        name: "UKNW",
        description: "Nothing stated.",
    }];
    fn read_none(_: &str) -> yggdryl::Result<u16> {
        Ok(0)
    }
    const fn side_like(byte: u8, extension_name: &'static str) -> MarketDescriptor {
        MarketDescriptor {
            id: DataTypeId::market(byte),
            name: "side",
            extension_name,
            storage: MarketStorage::Code8,
            members: MEMBERS,
            value_rank: 29,
            dtype_rank: 53,
            shape: 28,
            read: read_none,
        }
    }
    static SIDE: MarketDescriptor = side_like(0xc3, "yggdryl.side");
    static OTHER_BYTE: MarketDescriptor = side_like(0xc9, "yggdryl.side");
    static OTHER_EXTENSION: MarketDescriptor = side_like(0xc3, "yggdryl.notside");
    for (kind, by, reason) in [
        (
            &SIDE,
            "another",
            "expected \"yggdryl-market\", the crate that owns the reserved kind `side`, got \"another\"",
        ),
        (
            &OTHER_BYTE,
            "yggdryl-market",
            "expected `side` at 0xc3 under \"yggdryl.side\"",
        ),
        (
            &OTHER_EXTENSION,
            "yggdryl-market",
            "got `side` at 0xc3 under \"yggdryl.notside\"",
        ),
    ] {
        let refused = claim(kind, by).unwrap_err().to_string();
        assert!(refused.contains(reason), "{refused}");
    }
    assert!(kind_named("side").is_none());
    assert!(kinds().is_empty());
}

/// The order across the codes and `State`: the seventeen codes share value
/// rank 18 and order by identifier byte then text, and `state` keeps its
/// own rank, 27, and orders by code within it. The rank is wire-visible: it
/// is the order of an Arrow dictionary's values and of any caller's sort.
/// The market kinds' ranks beside them are
/// `rust/market/tests/root/market.rs`'s.
#[test]
fn the_codes_and_state_order_by_rank_then_by_identity() {
    use arrow_array::types::Int32Type;
    use arrow_array::{Array, DictionaryArray, StringArray};

    // (value, rank, identifier byte), ascending.
    let ascending: [(Scalar, u8, u8); 18] = [
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
    // URL (20), and above every code the enum leaf (27).
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
    let state = DataType::from_str("state").unwrap();
    assert!(isin.scalar("FR0000120271").unwrap() < isin.scalar("US0378331005").unwrap());
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
