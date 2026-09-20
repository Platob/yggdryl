//! A statement: one value over the four products that state something to
//! a ladder, a market event exactly as each arm is, whose walk readings
//! are the arm's own and refuse across arms.

use yggdryl::graph::{Element, Event, MarketElement};
use yggdryl::market::{ExecutionData, Folded, OrderData, QuoteData, Statement, TradeData};
use yggdryl::{Decimal18, State, Uuid};

fn order(unix: i64, code: &str, qty: i64, source: u128) -> OrderData {
    let mut order = OrderData::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_qty(Decimal18::from_int(qty));
    order.set_state(State::read("New").expect("a state"));
    order.set_srcuuids(vec![Uuid::from_v8(source)]);
    order.finalize();
    order
}

fn fill(unix: i64, code: &str, qty: i64, source: u128) -> ExecutionData {
    let mut fill = ExecutionData::at(unix);
    fill.set_crosscode(code.to_owned());
    fill.set_qty(Decimal18::from_int(qty));
    fill.set_srcuuids(vec![Uuid::from_v8(source)]);
    fill.finalize();
    fill
}

#[test]
fn a_statement_is_the_arm_it_holds() {
    let placed = order(10, "O-1", 100, 1);
    let quote = QuoteData::at(11);
    let filled = fill(12, "O-1", 40, 2);
    let trade = TradeData::at(13);
    let statements = [
        Statement::from(placed.clone()),
        Statement::from(quote),
        Statement::from(filled.clone()),
        Statement::from(trade),
    ];
    assert!(statements[0].is_maker() && statements[1].is_maker());
    assert!(statements[2].is_print() && statements[3].is_print());
    assert_eq!(statements[0].get_curruuid(), placed.get_curruuid());
    assert_eq!(statements[0].get_crosscode(), "O-1");
    assert_eq!(statements[2].get_qty(), Decimal18::from_int(40));
    assert_eq!(statements[2].get_srcuuids(), [Uuid::from_v8(2)]);
    assert_eq!(statements[0].event(), placed.event());
    assert!(statements[2].is_after(&statements[0]), "ordered by instant");
    // A mutator reaches the arm's own event.
    let mut moved = statements[0].clone();
    moved.set_qty(Decimal18::from_int(7));
    moved.finalize();
    assert_eq!(moved.get_qty(), Decimal18::from_int(7));
    assert_ne!(moved.get_curruuid(), placed.get_curruuid(), "settled again");
    let Statement::Order(inner) = moved else {
        panic!("still an order")
    };
    assert_eq!(inner.get_qty(), Decimal18::from_int(7));
}

#[test]
fn the_walk_readings_are_the_arms_own_and_refuse_across_arms() {
    let first = Statement::from(order(10, "O-1", 100, 1));
    let second = Statement::from(order(20, "O-1", 60, 2));
    let print = Statement::from(fill(20, "O-1", 40, 3));
    // Within an arm: the holder's own following, merging and restating.
    let followed = second
        .clone()
        .with_previous(&first)
        .expect("an order follows an order");
    assert_eq!(followed.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(followed.get_seqnum(), 1);
    let twin = Statement::from(order(10, "O-1", 100, 9));
    let merged = first
        .clone()
        .merge_with(&twin)
        .expect("two statements of one order fold");
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(9)]);
    let restated = twin.clone().restating(&followed);
    assert_eq!(restated.get_seqnum(), followed.get_seqnum());
    // Across arms: nothing follows, nothing merges, a restatement stands.
    assert!(print.clone().with_previous(&first).is_none());
    assert!(first.clone().merge_with(&print).is_none());
    assert_eq!(print.clone().restating(&first), print);
}

#[test]
fn a_mixed_stream_folds_a_twin_print_and_leaves_the_order_beside_it() {
    let statements = [
        Statement::from(order(10, "O-1", 100, 1)),
        Statement::from(fill(10, "O-1", 40, 2)),
        Statement::from(fill(10, "O-1", 40, 3)),
        Statement::from(order(20, "O-1", 60, 4)),
    ];
    let folded: Vec<Statement> = Folded::new(statements.map(Ok))
        .collect::<yggdryl::Result<_>>()
        .expect("every statement folds");
    assert_eq!(
        folded.len(),
        3,
        "the twin print folded into the one it restates"
    );
    assert!(folded[0].is_maker() && folded[1].is_print() && folded[2].is_maker());
    assert_eq!(
        folded[1].get_srcuuids(),
        [Uuid::from_v8(2), Uuid::from_v8(3)]
    );
}
