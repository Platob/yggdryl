use yggdryl::graph::{Element, MarketColumn, MarketElement, MarketEventData};
use yggdryl::{BloombergCode, Currency, Decimal18, Scalar, Side};

#[test]
fn market_columns_round_trip_every_optional_band() {
    let mut source = MarketEventData::at(10);
    source.set_marketoperationid(Some(14));
    source.set_price("101.25".parse().unwrap());
    source.set_currency(Currency::new("USD").unwrap());
    source.set_quantity(Decimal18::from_int(7));
    source.set_unit("share".to_owned());
    source.set_side(Side::read("Buy").unwrap());
    source.set_bloombergcode(Some(BloombergCode::new("BBG000B9XRY4").unwrap()));
    source.set_lastpx(Some("101".parse().unwrap()));
    source.set_lastqty(Some(Decimal18::from_int(2)));
    source.set_avgpx(Some("100.5".parse().unwrap()));
    source.set_cumqty(Some(Decimal18::from_int(3)));
    source.set_leavesqty(Some(Decimal18::from_int(4)));
    source.set_tif(Some("DAY".to_owned()));
    source.set_tradable(Some(true));
    source.set_symbolticker(Some("IBM".to_owned()));
    source.set_prevpx(Some("100".parse().unwrap()));
    source.set_prevqty(Some(Decimal18::from_int(8)));
    source.fill_lanes();
    source.finalize();

    let row: Vec<Scalar> = MarketColumn::ALL
        .into_iter()
        .map(|column| column.fact(&source).unwrap_or(Scalar::Null))
        .collect();
    let mut restored = MarketEventData::default();
    for (column, value) in MarketColumn::ALL.into_iter().zip(&row) {
        column.record(&mut restored, value);
    }

    assert_eq!(restored.get_marketoperationid(), Some(14));
    assert_eq!(restored.get_price(), source.get_price());
    assert_eq!(restored.get_bloombergcode(), source.get_bloombergcode());
    assert_eq!(restored.get_lastpx(), source.get_lastpx());
    assert_eq!(restored.get_lastqty(), source.get_lastqty());
    assert_eq!(restored.get_avgpx(), source.get_avgpx());
    assert_eq!(restored.get_cumqty(), source.get_cumqty());
    assert_eq!(restored.get_leavesqty(), source.get_leavesqty());
    assert_eq!(restored.get_tif(), source.get_tif());
    assert_eq!(restored.get_tradable(), source.get_tradable());
    assert_eq!(restored.get_symbolticker(), source.get_symbolticker());
    assert_eq!(restored.get_prevpx(), source.get_prevpx());
    assert_eq!(restored.get_prevqty(), source.get_prevqty());
    assert_eq!(restored.get_bidpx(), source.get_bidpx());
    assert_eq!(restored.get_askpx(), source.get_askpx());
}

#[test]
fn market_column_schema_has_one_owner_and_order() {
    let fields = MarketColumn::fields().unwrap();
    assert_eq!(fields.len(), 31);
    assert_eq!(fields.first().unwrap().name(), "marketoperationid");
    assert_eq!(fields[1].name(), "price");
    assert_eq!(fields[3].name(), "quantity");
    assert_eq!(fields.last().unwrap().name(), "askunit");
    assert_eq!(MarketColumn::of_name("Price"), Some(MarketColumn::Price));
    assert_eq!(
        MarketColumn::of_name("Quantity"),
        Some(MarketColumn::Quantity)
    );
    assert_eq!(MarketColumn::of_name("px"), None);
    assert_eq!(MarketColumn::of_name("qty"), None);
    assert_eq!(
        MarketColumn::of_name("MarketOperationID"),
        Some(MarketColumn::MarketOperationId)
    );
    assert_eq!(
        MarketColumn::of_name("BloombergCode"),
        Some(MarketColumn::BloombergCode)
    );
}
