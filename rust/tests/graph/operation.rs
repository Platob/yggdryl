//! `rust/src/graph/operation.rs`: the operation leaves - an order, a quote
//! or an execution, undated ([`Order`], [`Quote`], [`Execution`]) or dated
//! ([`OrderEvent`], [`QuoteEvent`], [`ExecutionEvent`]) by the sealed kind
//! they are generic over - the book-control facts a dated one carries typed,
//! and the update action.

use std::collections::BTreeMap;

use smol_str::SmolStr;
use yggdryl::graph::{
    BookRef, Element, Event, Execution, ExecutionEvent, ExecutionKind, Market, MarketKind,
    MdUpdateAction, Operation, OperationEvent, OperationKind, Order, OrderEvent, OrderKind, Quote,
    QuoteEvent, QuoteKind,
};
use yggdryl::{Ccy, CfiCode, Decimal, Side, State, TimeInForce, Unit, Uuid};

/// One filled order as a foreign caller would state it, finalized.
fn full<K: OperationKind>() -> OperationEvent<K> {
    let mut data = OperationEvent::<K>::at(1_700_000_000_000_000_000);
    data.set_crosscode("O-100".to_owned());
    data.set_srcuuids(vec![Uuid::from_v8(7)]);
    data.set_state(State::from_spelling("Filled").expect("a shipped state"));
    data.set_price(Some(Decimal::from_int(82)));
    data.set_currency(Ccy::new("USD").expect("a currency"));
    data.set_quantity(Some(Decimal::from_int(10)));
    data.set_unit(Unit::new("lot").expect("a unit"));
    data.set_side(Side::read("Buy").expect("a side"));
    data.set_lastpx(Some(Decimal::from_int(81)));
    data.set_lastqty(Some(Decimal::from_int(2)));
    data.set_tif(TimeInForce::from_spelling("GoodTillCancel"));
    data.set_tradable(Some(true));
    data.set_ticker(Some(SmolStr::new("BRN")));
    data.set_avgpx(Some(Decimal::from_int(80)));
    data.set_cumqty(Some(Decimal::from_int(4)));
    data.set_leavesqty(Some(Decimal::from_int(6)));
    data.set_prevpx(Some(Decimal::from_int(79)));
    data.set_prevqty(Some(Decimal::from_int(12)));
    data.insert_altid("ORDERID", "O-100")
        .expect("an operation takes every key");
    data.insert_accountid("ACCOUNT", "ACC-1")
        .expect("an operation takes every key");
    data.finalize();
    data
}

fn full_order() -> OrderEvent {
    full()
}

#[test]
fn each_kind_marker_names_its_leaf_and_the_word_it_digests_under() {
    assert_eq!(OrderKind::KIND, MarketKind::Order);
    assert_eq!(QuoteKind::KIND, MarketKind::Quote);
    assert_eq!(ExecutionKind::KIND, MarketKind::Execution);
    // The undated and the dated leaf of one kind name the same word: the
    // one an operation's identity digests.
    for (element, event, word) in [
        (Order::new().kind(), OrderEvent::at(0).kind(), "order"),
        (Quote::new().kind(), QuoteEvent::at(0).kind(), "quote"),
        (
            Execution::new().kind(),
            ExecutionEvent::at(0).kind(),
            "execution",
        ),
    ] {
        assert_eq!(element, event);
        assert_eq!(element.as_str(), word);
    }
}

#[test]
fn the_kind_says_whether_an_operation_is_an_execution_whatever_its_state() {
    let order = full_order();
    assert!(!order.is_execution(), "the kind wins over a filled state");
    assert!(!QuoteEvent::from(&order).is_execution());
    let unknown = ExecutionEvent::at(23);
    assert!(unknown.is_execution());
    assert!(!OrderEvent::at(23).is_execution());
    assert_eq!(order.kind(), MarketKind::Order);
    assert_eq!(QuoteEvent::from(&order).kind(), MarketKind::Quote);
    assert_eq!(ExecutionEvent::from(&order).kind(), MarketKind::Execution);
    let default = OrderEvent::default();
    assert_eq!(default.kind(), MarketKind::Order);
    assert_eq!(default.get_currunix(), 0);
}

#[test]
fn an_operation_of_another_kind_copies_every_fact_and_no_book_control() {
    let source = full_order().with_book(BookRef {
        action: Some(MdUpdateAction::New),
        ..BookRef::default()
    });
    let operation = QuoteEvent::from(&source);
    assert_eq!(operation.book(), None, "the control is the leaf's own");
    assert_eq!(operation.get_currunix(), source.get_currunix());
    assert_eq!(operation.get_state(), source.get_state());
    assert_eq!(operation.get_price(), source.get_price());
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
    assert_eq!(
        operation.get_curruuid(),
        source.get_curruuid(),
        "copied as stated, not refinalized"
    );
    let mut finalized = operation.clone();
    finalized.finalize();
    assert_ne!(
        finalized.get_curruuid(),
        source.get_curruuid(),
        "the kind digests into the operation's identity"
    );
}

#[test]
fn an_element_is_the_operation_undated_and_dates_again_at_an_instant() {
    let operation = full_order();
    let element = operation.clone().into_element();
    assert_eq!(element.kind(), MarketKind::Order);
    assert_eq!(element.get_price(), operation.get_price());
    assert_eq!(element.get_quantity(), operation.get_quantity());
    assert_eq!(element.get_ticker(), Some("BRN"));
    assert_eq!(element.get_tif(), operation.get_tif());
    assert_eq!(element.get_altids(), operation.get_altids());
    assert_eq!(element.get_crosscode(), "O-100");
    // The element keeps the identity the operation derived - its kind is in
    // it - until it is finalized as an element.
    assert_eq!(element.get_curruuid(), operation.get_curruuid());
    let mut undated = element.clone();
    undated.finalize();
    assert_ne!(
        undated.get_curruuid(),
        operation.get_curruuid(),
        "an undated element digests no clock"
    );
    assert_eq!(
        undated.get_curruuid(),
        Uuid::from_v8(u128::from(undated.get_currhashcode())),
        "an undated element's identity is its code"
    );

    let redated = element.clone().at(1_700_000_000_000_000_000);
    assert_eq!(redated.kind(), MarketKind::Order);
    assert_eq!(redated.get_currunix(), 1_700_000_000_000_000_000);
    assert_eq!(redated.get_price(), operation.get_price());
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
    // Back, the same element states the same facts: finalized alike, the
    // two are one, whatever clocks each was undated from.
    let mut back = redated.clone().into_element();
    back.finalize();
    assert_eq!(back, undated, "and back to the same element");

    // The kind travels both ways.
    let execution = ExecutionEvent::from(&operation).into_element();
    assert_eq!(execution.kind(), MarketKind::Execution);
    assert!(execution.at(5).is_execution());
    assert_eq!(Order::default().kind(), MarketKind::Order);
}

#[test]
fn the_same_facts_as_two_kinds_are_two_elements() {
    let mut order = Order::new();
    order.set_crosscode("O-100".to_owned());
    order.set_price(Some(Decimal::from_int(82)));
    order.finalize();
    let mut quote = Quote::new();
    quote.set_crosscode("O-100".to_owned());
    quote.set_price(Some(Decimal::from_int(82)));
    quote.finalize();
    assert_ne!(order.get_curruuid(), quote.get_curruuid());
    assert_eq!(order.get_crossuuid(), quote.get_crossuuid());
    // An element states no order: it is never after another.
    assert!(!order.is_after(&order.clone()));
}

#[test]
fn the_book_control_rides_typed_beside_the_operation_and_digests_into_it() {
    let bare = full_order();
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
        entry_px: Some(Decimal::from_int(82)),
        entry_size: Some(Decimal::from_int(10)),
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
    // Undated, the control stays behind: the two elements finalize alike.
    let mut controlled_element = controlled.clone().into_element();
    controlled_element.finalize();
    let mut bare_element = finalized_bare.clone().into_element();
    bare_element.finalize();
    assert_eq!(controlled_element, bare_element);
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
fn every_update_action_lists_itself_once_in_declaration_order_and_round_trips() {
    assert_eq!(
        MdUpdateAction::ALL.map(MdUpdateAction::as_str),
        ["0", "1", "2", "3", "4", "5", "snapshot"]
    );
    for action in MdUpdateAction::ALL {
        assert_eq!(MdUpdateAction::read(action.as_str()), Some(action));
    }
}

#[test]
fn an_operation_follows_and_merges_and_keeps_its_kind() {
    let mut first = OrderEvent::at(1_000_000);
    first.set_crosscode("O-100".to_owned());
    first.set_price(Some(Decimal::from_int(80)));
    first.finalize();

    let mut second = OrderEvent::at(2_000_000);
    second.set_crosscode("O-100".to_owned());
    second.set_price(Some(Decimal::from_int(81)));
    second.finalize();
    let second = second
        .with_previous(&first)
        .expect("the later order follows");
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevpx(), Some(Decimal::from_int(80)));
    assert_eq!(second.get_seqnum(), 1);
    assert_eq!(second.kind(), MarketKind::Order);

    // The same facts as a quote are another operation.
    let mut quote = QuoteEvent::from(&second);
    quote.finalize();
    assert_ne!(quote.get_curruuid(), second.get_curruuid());
    let mut restated = second.clone();
    restated.set_srcuuids(vec![Uuid::from_v8(9)]);
    let merged = second
        .clone()
        .merge_with(&restated)
        .expect("another statement of the same order adds its source");
    assert_eq!(merged.kind(), MarketKind::Order);
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(9)]);
}

#[test]
fn an_element_follows_and_merges_through_its_facts() {
    let mut first = Order::new();
    first.set_crosscode("O-100".to_owned());
    first.set_price(Some(Decimal::from_int(80)));
    first.finalize();
    let mut next = Order::new();
    next.set_crosscode("O-999".to_owned());
    next.finalize();
    let next = next.with_previous(&first).expect("an element follows");
    assert_eq!(next.get_crosscode(), "O-100", "it adopts the cross code");
    assert!(!next.is_after(&first) && !first.is_before(&next));

    let mut restated = first.clone();
    restated.set_srcuuids(vec![Uuid::from_v8(9)]);
    let merged = first
        .clone()
        .merge_with(&restated)
        .expect("another statement of the same element adds its source");
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(9)]);
    assert_eq!(merged.kind(), MarketKind::Order);
}

#[test]
fn merging_an_undated_element_lets_this_statement_lead() {
    // With no instant to say which statement is later, this one leads:
    // its price, quantity and unit stand, and each code is the better of
    // the two with this one first.
    let mut this = Order::new();
    this.set_crosscode("T-1".to_owned());
    this.set_price(Some(Decimal::parse("82.5").expect("a decimal")));
    this.set_quantity(Some(Decimal::from_int(1_000)));
    this.set_cficode(Some(CfiCode::new("ESXXXR").expect("a CFI")));
    this.finalize();
    let mut other = this.clone();
    other.set_price(Some(Decimal::from_int(83)));
    other.set_unit(Unit::new("bbl").expect("a unit"));
    other.set_currency(Ccy::new("USD").expect("a currency"));
    other.set_side(Side::read("1").expect("a side"));
    other.set_cficode(Some(CfiCode::new("ESVUFR").expect("a CFI")));
    other.set_metadata(Some(BTreeMap::from([(
        SmolStr::new("Feed"),
        SmolStr::new("OTHER"),
    )])));
    let merged = this.clone().merge_with(&other).expect("the same element");
    assert_eq!(
        merged.get_price(),
        Some(Decimal::parse("82.5").expect("a decimal"))
    );
    assert_eq!(
        merged.get_unit(),
        &Unit::none(),
        "this element's unit stands, stated or not"
    );
    assert_eq!(
        merged.get_currency().as_str(),
        "USD",
        "unknown takes the other"
    );
    assert_eq!(merged.get_side().as_str(), "BUY");
    assert_eq!(merged.get_cficode().map(CfiCode::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_metadata()["Feed"],
        "OTHER",
        "a key this statement leaves unstated is taken"
    );
    // Finalized: the identity is what the merged element states.
    assert_eq!(
        merged.get_curruuid(),
        Uuid::from_v8(u128::from(merged.get_currhashcode()))
    );
    assert_ne!(merged.get_curruuid(), this.get_curruuid());
    // Another element does not merge, and nothing new answers nothing.
    let mut stranger = Order::new();
    stranger.set_crosscode("T-2".to_owned());
    stranger.finalize();
    assert!(this.clone().merge_with(&stranger).is_none());
    assert!(this.clone().merge_with(&this).is_none());
}

/// The book control a dated operation carries is one pointer: boxed, and
/// absent on every operation that is not a market-data entry.
#[test]
fn a_boxed_book_control_is_one_pointer() {
    assert_eq!(std::mem::size_of::<Option<Box<BookRef>>>(), 8);
}

/// The two operation leaves' sizes. They are the sizes the holders they
/// wrap were pinned at when the slim `Market` trait landed - 864 bytes of
/// facts undated, 1040 dated - the dated leaf adding the boxed control,
/// padded to 1056. It moved from the one generic envelope's 1024 when the
/// four holders came back: a leaf holds only its role's facts.
#[test]
fn the_operation_leaves_are_the_sizes_of_the_facts_they_hold() {
    use std::mem::size_of;
    assert_eq!((size_of::<Order>(), size_of::<OrderEvent>()), (864, 1056));
    assert_eq!(size_of::<Quote>(), size_of::<Order>());
    assert_eq!(size_of::<ExecutionEvent>(), size_of::<OrderEvent>());
}
