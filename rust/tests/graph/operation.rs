//! `rust/src/graph/operation.rs`: the one operation type - an order, a
//! quote or an execution by its kind over one holder - its undated entry,
//! the book-control facts it carries typed, and the update action.

use smol_str::SmolStr;
use yggdryl::graph::{
    BookRef, Element, Event, Market, MarketOperation, MarketOperationData,
    MarketOperationEventData, MdUpdateAction, Operation, OperationEntry, OperationKind,
};
use yggdryl::{Ccy, Decimal18, Error, Side, State, TimeInForce, Unit, Uuid};

/// One filled order as a foreign caller would state it, finalized.
fn full_order() -> MarketOperationEventData {
    let mut data = MarketOperationEventData::at(1_700_000_000_000_000_000);
    data.set_crosscode("O-100".to_owned());
    data.set_srcuuids(vec![Uuid::from_v8(7)]);
    data.set_state(State::from_spelling("Filled").expect("a shipped state"));
    data.set_price(Decimal18::from_int(82));
    data.set_currency(Ccy::new("USD").expect("a currency"));
    data.set_quantity(Decimal18::from_int(10));
    data.set_unit(Unit::new("lot").expect("a unit"));
    data.set_side(Side::read("Buy").expect("a side"));
    data.set_lastpx(Some(Decimal18::from_int(81)));
    data.set_lastqty(Some(Decimal18::from_int(2)));
    data.set_tif(TimeInForce::from_spelling("GoodTillCancel"));
    data.set_tradable(Some(true));
    data.set_ticker(Some(SmolStr::new("BRN")));
    data.set_avgpx(Some(Decimal18::from_int(80)));
    data.set_cumqty(Some(Decimal18::from_int(4)));
    data.set_leavesqty(Some(Decimal18::from_int(6)));
    data.set_prevpx(Some(Decimal18::from_int(79)));
    data.set_prevqty(Some(Decimal18::from_int(12)));
    data.insert_altid("ORDERID", "O-100")
        .expect("a plain holder takes every key");
    data.insert_accountid("ACCOUNT", "ACC-1")
        .expect("a plain holder takes every key");
    data.finalize();
    data
}

#[test]
fn the_kinds_spell_themselves_and_read_back_ignoring_case() {
    for (kind, spelling, stored) in [
        (OperationKind::Order, "order", 1),
        (OperationKind::Quote, "quote", 2),
        (OperationKind::Execution, "execution", 3),
        (OperationKind::Trade, "trade", 4),
    ] {
        assert_eq!(kind.as_str(), spelling);
        assert_eq!(kind as u8, stored);
        assert_eq!(OperationKind::read(spelling), Some(kind));
        assert_eq!(
            OperationKind::read(&spelling.to_ascii_uppercase()),
            Some(kind)
        );
    }
    assert_eq!(
        OperationKind::read("snapshot"),
        None,
        "a snapshot is a control, not a kind"
    );
    assert_eq!(OperationKind::read(""), None);
    assert!(OperationKind::Order < OperationKind::Trade);
}

#[test]
fn a_trade_root_is_refused_as_an_operation_and_as_an_entry() {
    let error = Operation::new(OperationKind::Trade, full_order()).unwrap_err();
    assert!(
        matches!(&error, Error::InvalidRecord { path, .. } if path == "$.operationkind"),
        "{error}"
    );
    let error =
        OperationEntry::new(OperationKind::Trade, MarketOperationData::default()).unwrap_err();
    assert!(
        matches!(&error, Error::InvalidRecord { path, .. } if path == "$.operationkind"),
        "{error}"
    );
    for kind in [
        OperationKind::Order,
        OperationKind::Quote,
        OperationKind::Execution,
    ] {
        assert_eq!(Operation::new(kind, full_order()).unwrap().kind(), kind);
        assert_eq!(
            OperationEntry::new(kind, MarketOperationData::default())
                .unwrap()
                .kind(),
            kind
        );
    }
}

#[test]
fn the_kind_says_whether_an_operation_is_an_execution_whatever_its_state() {
    let source = full_order();
    let order = Operation::order(source.clone());
    assert!(
        source.is_execution(),
        "the holder reads its lifecycle state"
    );
    assert!(!order.is_execution(), "the kind wins over a filled state");
    assert!(!Operation::quote(source.clone()).is_execution());
    let unknown = MarketOperationEventData::at(23);
    assert!(!unknown.is_execution());
    assert!(Operation::execution(unknown).is_execution());
    assert_eq!(order.kind(), OperationKind::Order);
    assert_eq!(
        Operation::quote(source.clone()).kind(),
        OperationKind::Quote
    );
    assert_eq!(
        Operation::execution(source).kind(),
        OperationKind::Execution
    );
    let default = Operation::default();
    assert_eq!(default.kind(), OperationKind::Order);
    assert_eq!(default.get_currunix(), 0);
}

#[test]
fn an_operation_delegates_every_reading_to_its_data_and_hands_it_back() {
    let source = full_order();
    let operation = Operation::order(source.clone());
    assert_eq!(operation.data(), &source);
    assert_eq!(operation.get_lastpx(), source.get_lastpx());
    assert_eq!(operation.get_lastqty(), source.get_lastqty());
    assert_eq!(operation.get_tif(), source.get_tif());
    assert_eq!(operation.get_tradable(), source.get_tradable());
    assert_eq!(operation.get_ticker(), source.get_ticker());
    assert_eq!(operation.get_avgpx(), source.get_avgpx());
    assert_eq!(operation.get_cumqty(), source.get_cumqty());
    assert_eq!(operation.get_leavesqty(), source.get_leavesqty());
    assert_eq!(operation.get_prevpx(), source.get_prevpx());
    assert_eq!(operation.get_prevqty(), source.get_prevqty());
    assert_eq!(operation.get_unit(), source.get_unit());
    assert_eq!(operation.get_altids(), source.get_altids());
    assert_eq!(operation.get_accountids(), source.get_accountids());
    assert_eq!(operation.get_curruuid(), source.get_curruuid());
    assert_eq!(operation.into_data(), source);
}

#[test]
fn an_entry_is_the_operation_undated_and_dates_again_at_an_instant() {
    let source = full_order();
    let mut operation = Operation::order(source.clone());
    operation.finalize();
    assert_ne!(
        operation.get_curruuid(),
        source.get_curruuid(),
        "the kind digests into the operation's identity"
    );

    let entry = operation.clone().entry();
    assert_eq!(entry.kind(), OperationKind::Order);
    assert_eq!(entry.get_price(), source.get_price());
    assert_eq!(entry.get_quantity(), source.get_quantity());
    assert_eq!(entry.get_ticker(), Some("BRN"));
    assert_eq!(entry.get_tif(), source.get_tif());
    assert_eq!(entry.get_altids(), source.get_altids());
    assert_eq!(entry.get_crosscode(), "O-100");
    // The entry keeps the identity the operation derived - its kind is in
    // it - not the holder's own.
    let expected = MarketOperationData::from(operation.data());
    assert_eq!(entry.data(), &expected);
    assert_ne!(entry.get_curruuid(), source.get_curruuid());

    let redated = entry.clone().at(1_700_000_000_000_000_000);
    assert_eq!(redated.kind(), OperationKind::Order);
    assert_eq!(redated.get_currunix(), 1_700_000_000_000_000_000);
    assert_eq!(redated.get_price(), source.get_price());
    assert_eq!(
        redated.get_state(),
        &State::unknown(),
        "the clocks and the lifecycle stayed behind"
    );
    assert_eq!(redated.get_seqnum(), 0);
    assert_eq!(redated.book(), None, "the book control stayed behind too");
    assert_eq!(
        redated.get_curruuid(),
        redated.time_uuid().expect("an identity"),
        "dated, so finalized"
    );
    // Back, the same entry states the same facts: finalized alike, the
    // two are one, whatever clocks each was undated from.
    let mut back = redated.clone().entry();
    back.finalize();
    let mut again = entry.clone();
    again.finalize();
    assert_eq!(back, again, "and back to the same entry");

    // The kind travels both ways.
    let execution = Operation::execution(source).entry();
    assert_eq!(execution.kind(), OperationKind::Execution);
    assert!(execution.at(5).is_execution());
    assert_eq!(OperationEntry::default().kind(), OperationKind::Order);
}

#[test]
fn the_book_control_rides_typed_beside_the_operation_and_digests_into_it() {
    let bare = Operation::order(full_order());
    assert_eq!(bare.book(), None);
    assert_eq!(bare.action(), None);
    assert_eq!(bare.scope(), "");
    assert!(!bare.is_full_snapshot());

    // A control stating nothing is no control.
    let mut stated = bare.clone();
    stated.set_book(Some(BookRef::default()));
    assert_eq!(stated.book(), None);
    assert!(!BookRef::default().is_stated());

    let book = BookRef {
        action: Some(MdUpdateAction::Snapshot),
        scope: Some(SmolStr::new("PRIMARY")),
        position: Some(2),
        entry_px: Some(Decimal18::from_int(82)),
        entry_size: Some(Decimal18::from_int(10)),
    };
    assert!(book.is_stated());
    let mut controlled = bare.clone().with_book(book.clone());
    assert_eq!(controlled.book(), Some(&book));
    assert_eq!(controlled.action(), Some(MdUpdateAction::Snapshot));
    assert_eq!(controlled.scope(), "PRIMARY");
    assert!(controlled.is_full_snapshot());
    assert_eq!(
        controlled.get_curruuid(),
        bare.get_curruuid(),
        "setting the control does not refinalize"
    );
    controlled.finalize();
    let mut finalized_bare = bare.clone();
    finalized_bare.finalize();
    assert_ne!(
        controlled.get_curruuid(),
        finalized_bare.get_curruuid(),
        "the control is part of what the operation states"
    );
    // Undated, the control stays behind: the two entries finalize alike.
    let mut controlled_entry = controlled.clone().entry();
    controlled_entry.finalize();
    let mut bare_entry = finalized_bare.clone().entry();
    bare_entry.finalize();
    assert_eq!(controlled_entry, bare_entry);
    controlled.set_book(None);
    assert_eq!(controlled.book(), None);
}

#[test]
fn the_update_action_reads_the_fix_code_the_name_and_the_legacy_snapshot() {
    for (action, code, name) in [
        (MdUpdateAction::New, "0", "new"),
        (MdUpdateAction::Change, "1", "change"),
        (MdUpdateAction::Delete, "2", "delete"),
        (MdUpdateAction::DeleteThru, "3", "deletethru"),
        (MdUpdateAction::DeleteFrom, "4", "deletefrom"),
        (MdUpdateAction::Overlay, "5", "overlay"),
        (MdUpdateAction::Snapshot, "snapshot", "snapshot"),
    ] {
        assert_eq!(action.as_str(), code);
        assert_eq!(MdUpdateAction::read(code), Some(action));
        assert_eq!(MdUpdateAction::read(name), Some(action));
        assert_eq!(
            MdUpdateAction::read(&name.to_ascii_uppercase()),
            Some(action)
        );
        assert_eq!(MdUpdateAction::read(&format!(" {code} ")), Some(action));
    }
    assert_eq!(
        MdUpdateAction::read("SNAPSHOT"),
        Some(MdUpdateAction::Snapshot)
    );
    assert_eq!(MdUpdateAction::read("6"), None);
    assert_eq!(MdUpdateAction::read(""), None);
    assert!(MdUpdateAction::Change.is_partial());
    assert!(MdUpdateAction::Overlay.is_partial());
    assert!(!MdUpdateAction::New.is_partial());
    assert!(MdUpdateAction::DeleteThru.is_range_delete());
    assert!(MdUpdateAction::DeleteFrom.is_range_delete());
    assert!(!MdUpdateAction::Delete.is_range_delete());
    assert_eq!(MdUpdateAction::New as u8, 0);
    assert_eq!(MdUpdateAction::Snapshot as u8, 6);
}

#[test]
fn an_operation_follows_and_merges_through_its_holder_and_keeps_its_kind() {
    let mut first = MarketOperationEventData::at(1_000_000);
    first.set_crosscode("O-100".to_owned());
    first.set_price(Decimal18::from_int(80));
    first.finalize();
    let first = Operation::order(first);

    let mut second = MarketOperationEventData::at(2_000_000);
    second.set_crosscode("O-100".to_owned());
    second.set_price(Decimal18::from_int(81));
    second.finalize();
    let second = Operation::order(second)
        .with_previous(&first)
        .expect("the later order follows");
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevpx(), Some(Decimal18::from_int(80)));
    assert_eq!(second.get_seqnum(), 1);
    assert_eq!(second.kind(), OperationKind::Order);

    // Two kinds over one holder are two operations, and never merge.
    let mut quote = Operation::quote(second.data().clone());
    quote.finalize();
    assert_ne!(quote.get_curruuid(), second.get_curruuid());
    assert!(second.clone().merge_with(&quote).is_none());
    let mut restated = second.clone();
    restated.set_srcuuids(vec![Uuid::from_v8(9)]);
    let merged = second
        .clone()
        .merge_with(&restated)
        .expect("another statement of the same order adds its source");
    assert_eq!(merged.kind(), OperationKind::Order);
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(9)]);
}
