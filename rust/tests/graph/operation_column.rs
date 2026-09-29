//! `rust/src/graph/operation_column.rs`: the four columns every operation
//! on the market is stated in beside the market's twenty-eight, each
//! stating back exactly the fact it read.

use yggdryl::graph::{Operation, OperationColumn, OrderEvent};
use yggdryl::idmap::IdMap;
use yggdryl::{DataType, Scalar, TimeInForce};

#[test]
fn operation_columns_round_trip_every_fact() {
    let mut source = OrderEvent::at(10);
    source.set_tif(TimeInForce::from_spelling("DAY"));
    source.set_tradable(Some(true));
    source.insert_altid("ORDERID", "O-1").unwrap();
    source.insert_altid("CLORDID", "C-1").unwrap();
    source.insert_accountid("CUSTOMERACCOUNT", "ACC-1").unwrap();

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
    let mut restored = OrderEvent::default();
    for (column, value) in OperationColumn::ALL.into_iter().zip(&row) {
        column.record(&mut restored, value);
    }

    assert_eq!(restored.get_tif().map(TimeInForce::as_str), Some("0"));
    assert_eq!(restored.get_tradable(), Some(true));
    assert_eq!(restored.get_altids(), source.get_altids());
    assert_eq!(restored.get_altids().get("ORDERID"), Some("O-1"));
    assert_eq!(restored.get_accountids(), source.get_accountids());
    assert_eq!(
        restored.get_accountids().get("CUSTOMERACCOUNT"),
        Some("ACC-1")
    );
}

#[test]
fn a_null_clears_and_nothing_stated_is_none() {
    let mut operation = OrderEvent::at(7);
    operation.set_tradable(Some(false));
    operation.insert_altid("ORDERID", "O-1").unwrap();
    operation
        .insert_accountid("CUSTOMERACCOUNT", "ACC-1")
        .unwrap();
    for column in OperationColumn::ALL {
        column.record(&mut operation, &Scalar::Null);
    }
    assert_eq!(operation.get_tradable(), None);
    assert!(operation.get_altids().is_empty());
    assert!(operation.get_accountids().is_empty());
    for column in OperationColumn::ALL {
        assert_eq!(column.fact(&operation), None, "{}", column.name());
        assert!(column.nullable(), "{}", column.name());
    }
}

#[test]
fn operation_column_schema_has_one_owner_and_order() {
    let fields = OperationColumn::fields().unwrap();
    assert_eq!(fields.len(), 4);
    assert_eq!(
        fields.iter().map(|field| field.name()).collect::<Vec<_>>(),
        ["tif", "tradable", "altids", "accountids"]
    );
    assert_eq!(
        OperationColumn::TimeInForce.datatype(),
        DataType::TimeInForce
    );
    assert_eq!(
        OperationColumn::AltIds.datatype(),
        IdMap::dtype(),
        "the alternate identifiers are a sorted utf8 map"
    );
    // The accounts an operation names are an operation fact, a sorted utf8
    // map keyed by role; a user is none, and neither is a lane: a book
    // states its price levels under `bidlimits` and `asklimits`.
    assert_eq!(
        OperationColumn::of_name("accountids"),
        Some(OperationColumn::AccountIds)
    );
    assert_eq!(OperationColumn::AccountIds.datatype(), IdMap::dtype());
    assert_eq!(OperationColumn::AccountIds.display(), "Account IDs");
    assert_eq!(OperationColumn::of_name("userids"), None);
    assert_eq!(OperationColumn::of_name("bid"), None);
    assert_eq!(OperationColumn::of_name("ask"), None);
    // The category is the row's `marketdatakind`, never an operation's.
    assert_eq!(OperationColumn::of_name("MarketOperationID"), None);
    assert_eq!(
        OperationColumn::of_name("tif"),
        Some(OperationColumn::TimeInForce)
    );
    assert_eq!(OperationColumn::of_name("timeinforce"), None);
    assert_eq!(OperationColumn::of_name("bidpx"), None);
    assert_eq!(OperationColumn::of_name("identifiers"), None);
    assert_eq!(OperationColumn::TimeInForce.display(), "Time In Force");
}
