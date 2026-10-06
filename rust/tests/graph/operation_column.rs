//! `rust/src/graph/operation_column.rs`: the five columns every operation
//! on the market is stated in beside the market's thirty-five, each
//! stating back exactly the fact it read.

use yggdryl::IdKey;

use yggdryl::graph::{Operation, OperationColumn, OrderEvent};
use yggdryl::{DataType, IdType, Identifier, Identifiers, Scalar, TimeInForce};

/// One identifier of a plain holder: a value of `kind` from `fix`.
fn identifier(kind: IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::base(kind), value).unwrap()
}

/// One party: a value of `role` from `base`.
fn party(role: IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::base(role), value).unwrap()
}

#[test]
fn operation_columns_round_trip_every_fact() {
    let mut source = OrderEvent::at(10);
    source.set_ordqty(Some(yggdryl::Decimal::from_int(100)), true);
    source.set_timeinforce(TimeInForce::from_spelling("DAY"), true);
    source.set_tradable(Some(true), true);
    source
        .insert_identifier(identifier(IdType::OrderId, "O-1"))
        .unwrap();
    source
        .insert_identifier(identifier(IdType::ClOrdId, "C-1"))
        .unwrap();
    source
        .insert_partyid(party(IdType::CustomerAccount, "ACC-1"))
        .unwrap();

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

    assert_eq!(restored.get_ordqty(), Some(yggdryl::Decimal::from_int(100)));
    assert_eq!(
        restored.get_timeinforce().map(|held| held.as_str()),
        Some("DAY")
    );
    assert_eq!(restored.get_tradable(), Some(true));
    assert_eq!(restored.get_identifiers(), source.get_identifiers());
    assert_eq!(
        restored.get_identifiers().get(&IdType::OrderId),
        Some("O-1")
    );
    assert_eq!(restored.get_partyids(), source.get_partyids());
    assert_eq!(
        restored.get_partyids().get(&IdType::CustomerAccount),
        Some("ACC-1")
    );
}

#[test]
fn a_null_clears_and_nothing_stated_is_none() {
    let mut operation = OrderEvent::at(7);
    operation.set_tradable(Some(false), true);
    operation.set_ordqty(Some(yggdryl::Decimal::from_int(5)), true);
    operation
        .insert_identifier(identifier(IdType::OrderId, "O-1"))
        .unwrap();
    operation
        .insert_partyid(party(IdType::CustomerAccount, "ACC-1"))
        .unwrap();
    for column in OperationColumn::ALL {
        column.record(&mut operation, &Scalar::Null);
    }
    assert_eq!(operation.get_tradable(), None);
    assert!(operation.get_identifiers().is_empty());
    assert!(operation.get_partyids().is_empty());
    for column in OperationColumn::ALL {
        assert_eq!(column.fact(&operation), None, "{}", column.name());
        assert!(column.nullable(), "{}", column.name());
    }
}

#[test]
fn a_flag_cell_reads_as_every_flag_does_and_an_unreadable_cell_leaves_the_fact() {
    let mut operation = OrderEvent::at(7);
    for (cell, expected) in [
        (Scalar::from(true), Some(true)),
        (Scalar::from("yes"), Some(true)),
        (Scalar::from("Y"), Some(true)),
        (Scalar::from("1"), Some(true)),
        (Scalar::from(false), Some(false)),
        (Scalar::from("N"), Some(false)),
        (Scalar::from(" off "), Some(false)),
        (Scalar::from("true"), Some(true)),
    ] {
        OperationColumn::Tradable.record(&mut operation, &cell);
        assert_eq!(operation.get_tradable(), expected, "{cell:?}");
    }
    // A cell no flag spells is incompatible, so the fact stands: it never
    // clears, as the doc promises and as the market columns do.
    operation.set_tradable(Some(false), true);
    for cell in [Scalar::from("maybe"), Scalar::from(7_i64)] {
        OperationColumn::Tradable.record(&mut operation, &cell);
        assert_eq!(operation.get_tradable(), Some(false), "{cell:?}");
    }
    operation.set_ordqty(Some(yggdryl::Decimal::from_int(5)), true);
    OperationColumn::OrdQty.record(&mut operation, &Scalar::from("many"));
    assert_eq!(
        operation.get_ordqty(),
        Some(yggdryl::Decimal::from_int(5)),
        "an unreadable quantity leaves the fact"
    );
    // Only a null clears.
    OperationColumn::Tradable.record(&mut operation, &Scalar::Null);
    assert_eq!(operation.get_tradable(), None);
}

#[test]
fn operation_column_schema_has_one_owner_and_order() {
    let fields = OperationColumn::fields().unwrap();
    assert_eq!(fields.len(), 5);
    assert_eq!(
        fields.iter().map(|field| field.name()).collect::<Vec<_>>(),
        [
            "ordqty",
            "timeinforce",
            "tradable",
            "identifiers",
            "partyids"
        ]
    );
    // What it ordered: the crate's decimal.
    assert_eq!(OperationColumn::OrdQty.datatype(), DataType::Decimal);
    assert_eq!(
        OperationColumn::TimeInForce.datatype(),
        DataType::TimeInForce
    );
    assert_eq!(
        OperationColumn::Identifiers.datatype(),
        Identifiers::dtype(),
        "the identifiers are a sorted map from the key src:type to the source, type, value row"
    );
    assert_eq!(OperationColumn::Identifiers.name(), "identifiers");
    assert_eq!(OperationColumn::Identifiers.display(), "Identifiers");
    // The parties an operation names - its accounts, traders, firms and
    // users - are an operation fact, a serie of identifiers whose type is
    // the role; the old account map is none, and neither is a lane: a book
    // states its price levels under `bidlimits` and `asklimits`.
    assert_eq!(
        OperationColumn::of_name("partyids"),
        Some(OperationColumn::PartyIds)
    );
    assert_eq!(OperationColumn::PartyIds.datatype(), Identifiers::dtype());
    assert_eq!(OperationColumn::PartyIds.name(), "partyids");
    assert_eq!(OperationColumn::PartyIds.display(), "Party IDs");
    assert_eq!(OperationColumn::of_name("parties"), None);
    assert_eq!(OperationColumn::of_name("altids"), None);
    assert_eq!(OperationColumn::of_name("accountids"), None);
    assert_eq!(OperationColumn::of_name("userids"), None);
    assert_eq!(OperationColumn::of_name("bid"), None);
    assert_eq!(OperationColumn::of_name("ask"), None);
    // The category is the row's `marketdatakind`, never an operation's.
    assert_eq!(OperationColumn::of_name("MarketOperationID"), None);
    // The column is named as FIX's `TimeInForce(59)` is; the old short name
    // reaches nothing.
    assert_eq!(
        OperationColumn::of_name("timeinforce"),
        Some(OperationColumn::TimeInForce)
    );
    assert_eq!(OperationColumn::of_name("tif"), None);
    assert_eq!(OperationColumn::of_name("bidpx"), None);
    assert_eq!(
        OperationColumn::of_name("identifiers"),
        Some(OperationColumn::Identifiers)
    );
    assert_eq!(OperationColumn::TimeInForce.display(), "Time In Force");
}
