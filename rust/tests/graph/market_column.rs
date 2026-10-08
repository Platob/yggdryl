//! `rust/src/graph/market_column.rs`: the thirty-six columns every market
//! element is stated in, each stating back exactly the fact it read.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use yggdryl::IdKey;
use yggdryl::graph::{Element, Market, MarketColumn, OrderEvent};
use yggdryl::{
    Ccy, Cfi, DataType, Decimal, Field, IdType, Identifier, Identifiers, Mic, Scalar, Side,
    StructType, TimeUnit, Timezone, Unit,
};

fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

/// One security identifier of `kind` - a type's name or its FIX source code -
/// from `base`, validated by its type.
fn securityid(kind: &str, code: &str) -> Identifier {
    Identifier::new(
        IdKey::base(IdType::from_security_source(kind).unwrap()),
        code,
    )
    .unwrap()
}

#[test]
fn market_columns_round_trip_every_optional_band() {
    let mut source = OrderEvent::at(10);
    source.set_price(Some(decimal("101.25")), true);
    source.set_stoppx(Some(decimal("99.5")), true);
    source.set_displayqty(Some(Decimal::from_int(2)), true);
    source.set_hiddenqty(Some(Decimal::from_int(5)), true);
    source.set_cxlqty(Some(Decimal::from_int(1)), true);
    source.set_currency(Ccy::new("USD").unwrap(), true);
    source.set_origccy(Ccy::new("CHF").unwrap(), true);
    source.set_quantity(Some(Decimal::from_int(7)), true);
    source.set_unit(Unit::new("share").unwrap(), true);
    source.set_side(Side::read("Buy").unwrap(), true);
    source
        .insert_securityid(securityid("BLOOMBERG", "BBG000B9XRY4"))
        .unwrap();
    source
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .unwrap();
    source.set_cficode(Some(Cfi::new("ESVUFR").unwrap()), true);
    source.set_miccode(Some(Mic::new("XNAS").unwrap()), true);
    source.set_execunix(Some(1_650_000_000_000_000_000), true);
    source.set_lastpx(Some(decimal("101")), true);
    source.set_lastqty(Some(Decimal::from_int(2)), true);
    source.set_avgpx(Some(decimal("100.5")), true);
    source.set_cumqty(Some(Decimal::from_int(3)), true);
    source.set_leavesqty(Some(Decimal::from_int(4)), true);
    source.set_prevpx(Some(decimal("100")), true);
    source.set_prevqty(Some(Decimal::from_int(8)), true);
    source.set_spotrate(Some(decimal("100.75")), true);
    source.set_forwardpoints(Some(decimal("0.5")), true);
    source.set_bidpx(Some(decimal("100.25")), true);
    source.set_bidqty(Some(Decimal::from_int(6)), true);
    source.set_bidccy(Some(Ccy::new("USD").unwrap()), true);
    source.set_askpx(Some(decimal("100.5")), true);
    source.set_askqty(Some(Decimal::from_int(7)), true);
    source.set_askccy(Some(Ccy::new("EUR").unwrap()), true);
    source.set_fxrates(
        BTreeMap::from([
            (Ccy::new("EUR").unwrap(), decimal("1.085")),
            (Ccy::new("GBP").unwrap(), decimal("0.86")),
        ]),
        true,
    );
    source.set_ticker(Some(SmolStr::new("IBM")), true);
    source.set_strikepx(Some(decimal("4600.5")), true);
    source.set_metadata(
        Some(BTreeMap::from([(
            SmolStr::new("Feed"),
            SmolStr::new("PRIMARY"),
        )])),
        true,
    );
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
    assert_eq!(restored.get_stoppx(), source.get_stoppx());
    assert_eq!(restored.get_currency(), source.get_currency());
    assert_eq!(restored.get_origccy().as_str(), "CHF");
    assert_eq!(restored.get_quantity(), source.get_quantity());
    assert_eq!(restored.get_displayqty(), source.get_displayqty());
    assert_eq!(restored.get_hiddenqty(), source.get_hiddenqty());
    assert_eq!(restored.get_cxlqty(), source.get_cxlqty());
    assert_eq!(restored.get_unit(), source.get_unit());
    assert_eq!(restored.get_side(), source.get_side());
    assert_eq!(restored.get_securityids(), source.get_securityids());
    assert_eq!(restored.get_isincode(), Some("US0378331005"));
    assert_eq!(restored.get_fxrates(), source.get_fxrates());
    assert_eq!(
        restored.get_securityids().get(&IdType::Bloomberg),
        Some("BBG000B9XRY4")
    );
    assert_eq!(
        restored.get_securityids().get(&IdType::Cusip),
        Some("037833100"),
        "the CUSIP the ISIN carries was derived at finalization and travels as a column"
    );
    assert_eq!(restored.get_cficode(), source.get_cficode());
    assert_eq!(restored.get_miccode(), source.get_miccode());
    assert_eq!(restored.get_execunix(), Some(1_650_000_000_000_000_000));
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
    assert_eq!(restored.get_strikepx(), Some(decimal("4600.5")));
    assert_eq!(restored.get_metadata(), source.get_metadata());
    assert_eq!(restored.get_metadata()["Feed"], "PRIMARY");
}

#[test]
fn a_null_clears_an_optional_fact_and_leaves_a_required_one_stated() {
    let mut element = OrderEvent::default();
    element.set_price(Some(decimal("1")), true);
    element.set_currency(Ccy::new("EUR").unwrap(), true);
    element.set_origccy(Ccy::new("USD").unwrap(), true);
    element.set_quantity(Some(Decimal::from_int(2)), true);
    element.set_unit(Unit::new("bbl").unwrap(), true);
    element.set_side(Side::read("Sell").unwrap(), true);
    element.set_lastpx(Some(decimal("3")), true);
    element.set_execunix(Some(4), true);
    element.set_ticker(Some(SmolStr::new("BRN")), true);
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
    // The origin is optional: a null clears it, and the origin read is the
    // currency again.
    assert!(element.get_origccy().is_none());
    assert_eq!(element.origin_currency().as_str(), "EUR");
    assert_eq!(element.get_quantity(), None);
    assert_eq!(element.get_unit(), &Unit::none());
    assert_eq!(element.get_side(), Side::Sell);
    assert_eq!(element.get_lastpx(), None);
    assert_eq!(element.get_execunix(), None);
    assert_eq!(element.get_ticker(), None);
    assert_eq!(element.get_securityids(), &Identifiers::new());
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
        [
            "marketdatakind",
            "marketdatatype",
            "currency",
            "unit",
            "side"
        ]
    );
}

#[test]
fn market_column_schema_has_one_owner_and_order() {
    let fields = MarketColumn::fields().unwrap();
    assert_eq!(fields.len(), 36);
    assert_eq!(
        fields.iter().map(|field| field.name()).collect::<Vec<_>>(),
        [
            "marketdatakind",
            "marketdatatype",
            "price",
            "stoppx",
            "currency",
            "origccy",
            "quantity",
            "displayqty",
            "hiddenqty",
            "unit",
            "side",
            "securityids",
            "isincode",
            "cficode",
            "miccode",
            "execunix",
            "lastpx",
            "lastqty",
            "avgpx",
            "cumqty",
            "leavesqty",
            "cxlqty",
            "prevpx",
            "prevqty",
            "spotrate",
            "forwardpoints",
            "bidpx",
            "bidqty",
            "bidccy",
            "askpx",
            "askqty",
            "askccy",
            "fxrates",
            "ticker",
            "strikepx",
            "metadata",
        ]
    );
    // The order terms and the bid and the ask a quote or a book states:
    // decimals and codes, null where none is stated.
    for column in [
        MarketColumn::StopPx,
        MarketColumn::StrikePx,
        MarketColumn::DisplayQty,
        MarketColumn::HiddenQty,
        MarketColumn::CxlQty,
        MarketColumn::BidPx,
        MarketColumn::BidQty,
        MarketColumn::AskPx,
        MarketColumn::AskQty,
    ] {
        assert_eq!(column.datatype(), DataType::Decimal);
        assert!(column.nullable());
    }
    for column in [
        MarketColumn::OrigCcy,
        MarketColumn::BidCcy,
        MarketColumn::AskCcy,
    ] {
        assert_eq!(column.datatype(), DataType::Ccy);
        assert!(column.nullable());
    }
    assert_eq!(MarketColumn::IsinCode.datatype(), DataType::Isin);
    assert_eq!(MarketColumn::IsinCode.display(), "ISIN Code");
    // When the element last executed: a market fact, the event clocks'
    // nanosecond UTC datatype, null where none is known.
    assert_eq!(
        MarketColumn::ExecUnix.datatype(),
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        }
    );
    assert!(MarketColumn::ExecUnix.nullable());
    assert_eq!(
        MarketColumn::of_name("ExecUnix"),
        Some(MarketColumn::ExecUnix)
    );
    // Target currency to the rate to divide by: keys and rates required.
    assert_eq!(
        MarketColumn::FxRates.datatype(),
        DataType::map(
            Field::new(
                "entries",
                DataType::Struct(
                    StructType::from_fields(vec![
                        Field::new("key", DataType::Ccy, false),
                        Field::new("value", DataType::Decimal, false),
                    ])
                    .unwrap()
                ),
                false,
            ),
            true,
        )
        .unwrap()
    );
    assert_eq!(MarketColumn::FxRates.display(), "FX Rates");
    assert_eq!(MarketColumn::SecurityIds.datatype(), Identifiers::dtype());
    assert_eq!(MarketColumn::SecurityIds.name(), "securityids");
    assert_eq!(MarketColumn::SecurityIds.display(), "Security IDs");
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
        "timeinforce",
        "tradable",
        "cusipcode",
        "sedolcode",
        "bloombergcode",
        "figicode",
        "askunit",
        "symbolticker",
        "identifiers",
        "partyids",
        "secaltids",
    ] {
        assert_eq!(MarketColumn::of_name(gone), None, "{gone}");
    }
}

/// `isincode` is a projection of `securityids`: recording it fills an
/// absent `isin`, and - the lenient door - leaves a different one standing.
#[test]
fn recording_an_isincode_fills_an_absent_isin_and_leaves_another_alone() {
    let mut blank = OrderEvent::at(1);
    MarketColumn::IsinCode.record(&mut blank, &Scalar::from("US0378331005"));
    assert_eq!(blank.get_isincode(), Some("US0378331005"));
    assert_eq!(
        MarketColumn::IsinCode.fact(&blank),
        Some(Scalar::Isin(yggdryl::Isin::new("US0378331005").unwrap()))
    );

    let mut listed = OrderEvent::at(1);
    listed
        .insert_securityid(securityid("ISIN", "GB0002634946"))
        .unwrap();
    MarketColumn::IsinCode.record(&mut listed, &Scalar::from("US0378331005"));
    assert_eq!(listed.get_isincode(), Some("GB0002634946"));
    MarketColumn::IsinCode.record(&mut listed, &Scalar::Null);
    assert_eq!(listed.get_isincode(), Some("GB0002634946"));
}
