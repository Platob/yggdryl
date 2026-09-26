//! `rust/src/graph/market_column.rs`: the nineteen columns every market
//! element is stated in, each stating back exactly the fact it read.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use yggdryl::graph::{Element, Market, MarketColumn, OrderEvent};
use yggdryl::securityid::{SecType, SecurityId, SecurityIds};
use yggdryl::{Ccy, Cfi, DataType, Decimal, Mic, Scalar, Side, Unit};

fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn securityid(key: &str, code: &str) -> SecurityId {
    SecurityId::new(SecType::read(key).unwrap(), code).unwrap()
}

#[test]
fn market_columns_round_trip_every_optional_band() {
    let mut source = OrderEvent::at(10);
    source.set_price(Some(decimal("101.25")));
    source.set_currency(Ccy::new("USD").unwrap());
    source.set_quantity(Some(Decimal::from_int(7)));
    source.set_unit(Unit::new("share").unwrap());
    source.set_side(Side::read("Buy").unwrap());
    source
        .insert_securityid(securityid("BLOOMBERG", "BBG000B9XRY4"))
        .unwrap();
    source
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .unwrap();
    source.set_cficode(Some(Cfi::new("ESVUFR").unwrap()));
    source.set_miccode(Some(Mic::new("XNAS").unwrap()));
    source.set_lastpx(Some(decimal("101")));
    source.set_lastqty(Some(Decimal::from_int(2)));
    source.set_avgpx(Some(decimal("100.5")));
    source.set_cumqty(Some(Decimal::from_int(3)));
    source.set_leavesqty(Some(Decimal::from_int(4)));
    source.set_prevpx(Some(decimal("100")));
    source.set_prevqty(Some(Decimal::from_int(8)));
    source.set_spotrate(Some(decimal("100.75")));
    source.set_forwardpoints(Some(decimal("0.5")));
    source.set_ticker(Some(SmolStr::new("IBM")));
    source.set_metadata(Some(BTreeMap::from([(
        SmolStr::new("Feed"),
        SmolStr::new("PRIMARY"),
    )])));
    source.finalize();

    let row: Vec<Scalar> = MarketColumn::ALL
        .into_iter()
        .map(|column| column.fact(&source).expect("every fact is stated"))
        .collect();
    for (column, value) in MarketColumn::ALL.into_iter().zip(&row) {
        column
            .field()
            .expect("a field")
            .scalar(value.clone())
            .expect("the fact fits the column");
    }
    let mut restored = OrderEvent::default();
    for (column, value) in MarketColumn::ALL.into_iter().zip(&row) {
        column.record(&mut restored, value);
    }

    assert_eq!(restored.get_price(), source.get_price());
    assert_eq!(restored.get_currency(), source.get_currency());
    assert_eq!(restored.get_quantity(), source.get_quantity());
    assert_eq!(restored.get_unit(), source.get_unit());
    assert_eq!(restored.get_side(), source.get_side());
    assert_eq!(restored.get_securityids(), source.get_securityids());
    assert_eq!(
        restored.get_securityids().get("BLOOMBERG"),
        Some("BBG000B9XRY4")
    );
    assert_eq!(
        restored.get_securityids().get("CUSIP"),
        Some("037833100"),
        "the CUSIP the ISIN carries was derived at finalization and travels as a column"
    );
    assert_eq!(restored.get_cficode(), source.get_cficode());
    assert_eq!(restored.get_miccode(), source.get_miccode());
    assert_eq!(restored.get_lastpx(), source.get_lastpx());
    assert_eq!(restored.get_lastqty(), source.get_lastqty());
    assert_eq!(restored.get_avgpx(), source.get_avgpx());
    assert_eq!(restored.get_cumqty(), source.get_cumqty());
    assert_eq!(restored.get_leavesqty(), source.get_leavesqty());
    assert_eq!(restored.get_prevpx(), source.get_prevpx());
    assert_eq!(restored.get_prevqty(), source.get_prevqty());
    assert_eq!(restored.get_spotrate(), source.get_spotrate());
    assert_eq!(restored.get_forwardpoints(), source.get_forwardpoints());
    assert_eq!(restored.get_ticker(), Some("IBM"));
    assert_eq!(restored.get_metadata(), source.get_metadata());
    assert_eq!(restored.get_metadata()["Feed"], "PRIMARY");
}

#[test]
fn a_null_clears_an_optional_fact_and_leaves_a_required_one_stated() {
    let mut element = OrderEvent::default();
    element.set_price(Some(decimal("1")));
    element.set_currency(Ccy::new("EUR").unwrap());
    element.set_quantity(Some(Decimal::from_int(2)));
    element.set_unit(Unit::new("bbl").unwrap());
    element.set_side(Side::read("Sell").unwrap());
    element.set_lastpx(Some(decimal("3")));
    element.set_ticker(Some(SmolStr::new("BRN")));
    element
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .unwrap();
    for column in MarketColumn::ALL {
        column.record(&mut element, &Scalar::Null);
    }
    // A required column is never null, so a null is no reading of it and
    // the stated fact stands - except the unit, a code that spells none,
    // for which a null is that spelling; an optional one is cleared, and
    // the price and the quantity are optional: a null cell is a price the
    // element no longer states, never a zero.
    assert_eq!(element.get_price(), None);
    assert_eq!(element.get_currency().as_str(), "EUR");
    assert_eq!(element.get_quantity(), None);
    assert_eq!(element.get_unit(), &Unit::none());
    assert_eq!(element.get_side(), Side::Sell);
    assert_eq!(element.get_lastpx(), None);
    assert_eq!(element.get_ticker(), None);
    assert_eq!(element.get_securityids(), &SecurityIds::default());
    for column in MarketColumn::ALL {
        let fact = column.fact(&element);
        if column.nullable() {
            assert_eq!(fact, None, "{}", column.name());
        } else {
            assert!(fact.is_some(), "{} is stated", column.name());
        }
    }
    // And an element stating nothing states its three required facts as
    // nothing - an empty code, an unknown side - and no price or quantity
    // at all: those two columns are nullable and answer `None`.
    let bare = OrderEvent::default();
    assert!(MarketColumn::Price.nullable() && MarketColumn::Quantity.nullable());
    assert_eq!(MarketColumn::Price.fact(&bare), None);
    assert_eq!(MarketColumn::Quantity.fact(&bare), None);
    for column in MarketColumn::ALL
        .into_iter()
        .filter(|column| !column.nullable())
    {
        assert!(
            column.fact(&bare).is_some(),
            "{} is stated, if only as nothing",
            column.name()
        );
    }
    assert_eq!(bare.get_unit(), &Unit::none());
    assert_eq!(bare.get_currency(), &Ccy::none());
    assert_eq!(bare.get_side(), Side::Unknown);
    assert_eq!(
        MarketColumn::ALL
            .into_iter()
            .filter(|column| !column.nullable())
            .map(|column| column.name())
            .collect::<Vec<_>>(),
        ["currency", "unit", "side"]
    );
}

#[test]
fn market_column_schema_has_one_owner_and_order() {
    let fields = MarketColumn::fields().unwrap();
    assert_eq!(fields.len(), 19);
    assert_eq!(
        fields.iter().map(|field| field.name()).collect::<Vec<_>>(),
        [
            "price",
            "currency",
            "quantity",
            "unit",
            "side",
            "securityids",
            "cficode",
            "miccode",
            "lastpx",
            "lastqty",
            "avgpx",
            "cumqty",
            "leavesqty",
            "prevpx",
            "prevqty",
            "spotrate",
            "forwardpoints",
            "ticker",
            "metadata",
        ]
    );
    assert_eq!(MarketColumn::SecurityIds.datatype(), SecurityIds::dtype());
    assert_eq!(MarketColumn::Ticker.datatype(), DataType::utf8());
    assert_eq!(MarketColumn::Unit.datatype(), DataType::Unit);
    assert_eq!(MarketColumn::Side.datatype(), DataType::Side);
    assert_eq!(MarketColumn::of_name("Price"), Some(MarketColumn::Price));
    assert_eq!(
        MarketColumn::of_name("Quantity"),
        Some(MarketColumn::Quantity)
    );
    assert_eq!(MarketColumn::of_name("px"), None);
    assert_eq!(MarketColumn::of_name("qty"), None);
    assert_eq!(
        MarketColumn::of_name("SecurityIDs"),
        Some(MarketColumn::SecurityIds)
    );
    assert_eq!(MarketColumn::of_name("Ticker"), Some(MarketColumn::Ticker));
    // What left the market: the operation's own columns, the five code
    // columns folded into `securityids`, the lanes and the old ticker name.
    for gone in [
        "marketoperationid",
        "tif",
        "tradable",
        "isincode",
        "cusipcode",
        "sedolcode",
        "bloombergcode",
        "figicode",
        "bidpx",
        "askunit",
        "symbolticker",
        "identifiers",
    ] {
        assert_eq!(MarketColumn::of_name(gone), None, "{gone}");
    }
}
