//! `rust/src/graph/operation_column.rs`: the eight columns every market
//! operation is stated in beside the market's nineteen, each stating back
//! exactly the fact it read.

use yggdryl::graph::{Lane, Operation, OperationColumn, OperationEventData};
use yggdryl::idmap::IdMap;
use yggdryl::{Ccy, DataType, Decimal18, Scalar, TimeInForce, Unit};

fn decimal(text: &str) -> Decimal18 {
    text.parse().unwrap()
}

#[test]
fn operation_columns_round_trip_every_fact() {
    let mut source = OperationEventData::at(10);
    source.set_marketoperationid(Some(14));
    source.set_tif(TimeInForce::from_spelling("DAY"));
    source.set_tradable(Some(true));
    source.insert_accountid("ACCOUNT", "ACC-1").unwrap();
    source.insert_userid("SENDERSUBID", "trader").unwrap();
    source.insert_altid("ORDERID", "O-1").unwrap();
    source.insert_altid("CLORDID", "C-1").unwrap();
    source.set_bid(Some(Lane {
        price: Some(decimal("101.25")),
        spotrate: Some(decimal("101")),
        forwardpoints: Some(decimal("0.25")),
        currency: Some(Ccy::new("USD").unwrap()),
        quantity: Some(Decimal18::from_int(7)),
        unit: Some(Unit::new("share").unwrap()),
    }));
    source.set_ask(Some(Lane {
        price: Some(decimal("102")),
        ..Lane::default()
    }));

    let row: Vec<Scalar> = OperationColumn::ALL
        .into_iter()
        .map(|column| column.fact(&source).expect("every fact is stated"))
        .collect();
    for (column, value) in OperationColumn::ALL.into_iter().zip(&row) {
        column
            .field()
            .expect("a field")
            .scalar(value.clone())
            .expect("the fact fits the column");
    }
    let mut restored = OperationEventData::default();
    for (column, value) in OperationColumn::ALL.into_iter().zip(&row) {
        column.record(&mut restored, value);
    }

    assert_eq!(restored.get_marketoperationid(), Some(14));
    assert_eq!(restored.get_tif().map(TimeInForce::as_str), Some("0"));
    assert_eq!(restored.get_tradable(), Some(true));
    assert_eq!(restored.get_accountids(), source.get_accountids());
    assert_eq!(restored.get_userids(), source.get_userids());
    assert_eq!(restored.get_altids(), source.get_altids());
    assert_eq!(restored.get_altids().get("ORDERID"), Some("O-1"));
    assert_eq!(restored.get_bid(), source.get_bid());
    assert_eq!(restored.get_ask(), source.get_ask());
}

#[test]
fn a_null_clears_and_nothing_stated_is_none() {
    let mut operation = OperationEventData::at(7);
    operation.set_marketoperationid(Some(3));
    operation.set_tradable(Some(false));
    operation.insert_altid("ORDERID", "O-1").unwrap();
    operation.set_bid(Some(Lane {
        price: Some(decimal("1")),
        ..Lane::default()
    }));
    for column in OperationColumn::ALL {
        column.record(&mut operation, &Scalar::Null);
    }
    assert_eq!(operation.get_marketoperationid(), None);
    assert_eq!(operation.get_tradable(), None);
    assert!(operation.get_altids().is_empty());
    assert_eq!(operation.get_bid(), None);
    for column in OperationColumn::ALL {
        assert_eq!(column.fact(&operation), None, "{}", column.name());
        assert!(column.nullable(), "{}", column.name());
    }
    // A lane stating nothing is no lane.
    OperationColumn::Ask.record(
        &mut operation,
        &Scalar::from_sequence([
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
            Scalar::Null,
        ]),
    );
    assert_eq!(operation.get_ask(), None);
}

#[test]
fn operation_column_schema_has_one_owner_and_order() {
    let fields = OperationColumn::fields().unwrap();
    assert_eq!(fields.len(), 8);
    assert_eq!(
        fields.iter().map(|field| field.name()).collect::<Vec<_>>(),
        [
            "marketoperationid",
            "tif",
            "tradable",
            "accountids",
            "userids",
            "altids",
            "bid",
            "ask",
        ]
    );
    assert_eq!(
        OperationColumn::TimeInForce.datatype().unwrap(),
        DataType::TimeInForce
    );
    assert_eq!(
        OperationColumn::AltIds.datatype().unwrap(),
        IdMap::dtype(),
        "the three maps are sorted utf8 maps"
    );
    let lane = OperationColumn::Bid.datatype().unwrap();
    let DataType::Struct(lane_type) = &lane else {
        panic!("a lane is a struct, got {lane}")
    };
    assert_eq!(
        lane_type
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        OperationColumn::LANE_FIELDS
    );
    assert_eq!(OperationColumn::Ask.datatype().unwrap(), lane);
    assert_eq!(
        OperationColumn::of_name("MarketOperationID"),
        Some(OperationColumn::MarketOperationId)
    );
    assert_eq!(
        OperationColumn::of_name("tif"),
        Some(OperationColumn::TimeInForce)
    );
    assert_eq!(OperationColumn::of_name("timeinforce"), None);
    assert_eq!(OperationColumn::of_name("bidpx"), None);
    assert_eq!(OperationColumn::of_name("identifiers"), None);
    assert_eq!(OperationColumn::TimeInForce.display(), "Time In Force");
}
