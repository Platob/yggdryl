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
