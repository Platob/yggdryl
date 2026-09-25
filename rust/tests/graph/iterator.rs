//! `rust/src/graph/iterator.rs`: the one walk over market operation events:
//! each event chained to the live one under its cross identity - or, where
//! that is alive under nothing, under a name a live event goes by - the
//! alive set kept as the lifecycle moves, and the caller's word on the order
//! taken or the order made.

use yggdryl::graph::{Element, Event, EventIterator, ExecutionEvent, Operation, OrderEvent};
use yggdryl::{State, Uuid};

use super::element::filled;

/// One nanosecond count per millisecond: a derived identity opens with the
/// microsecond its instant falls in, and the instants below are spaced a
/// whole millisecond apart, so no two of them ever share one.
const MS: i64 = 1_000_000;

/// The alternate-identifier keys the walk's fixtures go by: the order's own
/// identifier, which a following event carries forward, and the client and
/// execution identifiers, which name an event without following.
const ORDER_ID: &str = "ORDERID";
const CL_ORD_ID: &str = "CLORDID";
const EXEC_ID: &str = "EXECID";

/// An instant a derived identity holds: `ms` milliseconds after one
/// evening in November 2023, UTC.
fn at(ms: i64) -> i64 {
    1_700_000_000_000_000_000 + ms * MS
}

/// The milliseconds an instant stands after [`at`]'s origin.
fn ms(unix: i64) -> i64 {
    (unix - at(0)) / MS
}

/// One event of the thing `order` identifies across its life, at `ms`.
fn incarnation(order: &str, ms: i64) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.finalize();
    event
}

/// An event of `order` at `ms` going by one name, so two events at one
/// instant are two events.
fn named(order: &str, ms: i64, scheme: &str, name: &str) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event
        .insert_altid(scheme, name)
        .expect("a plain holder takes every key");
    event.finalize();
    event
}

/// An event at `ms` going by `names` and stating no cross code of its own.
fn anonymous(ms: i64, names: &[(&str, &str)]) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    for (scheme, name) in names {
        event
            .insert_altid(scheme, name)
            .expect("a plain holder takes every key");
    }
    event.finalize();
    event
}

/// Where each event the walk yields stands: its instant, its place, and the
/// instant of the predecessor it names, in milliseconds.
fn places(walk: impl Iterator<Item = OrderEvent>) -> Vec<(i64, u64, Option<i64>)> {
    walk.map(|event| {
        (
            ms(event.get_currunix()),
            event.get_seqnum(),
            event.get_prevunix().map(ms),
        )
    })
    .collect()
}

/// The chains alive after a walk, by cross code and the live instant, in
/// one order.
fn alive(walk: &EventIterator<OrderEvent, std::vec::IntoIter<OrderEvent>>) -> Vec<(String, i64)> {
    let mut alive = walk
        .alive()
        .map(|held| (held.get_crosscode().to_owned(), ms(held.get_currunix())))
        .collect::<Vec<_>>();
    alive.sort_unstable();
    alive
}

#[test]
fn a_sorted_walk_chains_each_element_to_the_live_one_under_its_identity() {
    let mut first = incarnation("O-100", 10);
    first.set_creaunix(Some(at(5)));
    first.insert_altid(ORDER_ID, "O-100").unwrap();
    first.finalize();
    let arrived = vec![
        first,
        incarnation("O-900", 15),
        incarnation("O-100", 20),
        incarnation("O-100", 30),
    ];
    let mut walk = EventIterator::new(arrived, true);
    let first = walk.next().expect("the first");
    assert_eq!((first.get_seqnum(), first.get_prevuuid()), (0, None));
    let other = walk
        .next()
        .expect("another order's, under its own identity");
    assert_eq!((other.get_seqnum(), other.get_prevuuid()), (0, None));
    assert_eq!(other.get_crosscode(), "O-900");
    let second = walk.next().expect("the second incarnation");
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevunix(), Some(at(10)));
    assert_eq!(second.get_seqnum(), 1);
    assert_eq!(second.get_crossuuid(), first.get_crossuuid());
    // Enriched by the element's own reading: the lifecycle carried forward,
    // the order's own identifier with it, and the identity re-derived
    // around them.
    assert_eq!(second.get_creaunix(), Some(at(5)));
    assert_eq!(second.get_altids().get(ORDER_ID), Some("O-100"));
    assert_eq!(
        second.get_curruuid(),
        second.time_uuid().expect("an identity")
    );
    let third = walk.next().expect("the third incarnation");
    assert_eq!(third.get_prevuuid(), Some(second.get_curruuid()));
    assert_eq!(third.get_seqnum(), 2);
    assert_eq!(third.get_creaunix(), Some(at(5)));
    // A walk over the walked answers the same chain.
    assert!(walk.next().is_none());
    let walked = vec![first.clone(), other.clone(), second.clone(), third.clone()];
    let again: Vec<OrderEvent> = EventIterator::new(walked, true).collect();
    assert_eq!(again, [first, other, second, third]);
    // Both orders are still alive, the latest incarnation of each.
    assert_eq!(
        alive(&walk),
        [("O-100".to_owned(), 30), ("O-900".to_owned(), 15)]
    );
}

#[test]
fn an_unsorted_walk_sorts_by_the_elements_own_order_first_and_stably() {
    let arrived = vec![
        incarnation("O-100", 30),
        incarnation("O-100", 10),
        incarnation("O-100", 20),
        // Two at one instant are neither after nor before each other, and
        // keep the order they arrived in.
        named("O-100", 40, EXEC_ID, "E-5"),
        named("O-100", 40, EXEC_ID, "E-4"),
    ];
    let walk = EventIterator::new(arrived, false);
    let walked: Vec<OrderEvent> = walk.collect();
    assert_eq!(
        places(walked.iter().cloned()),
        [
            (10, 0, None),
            (20, 1, Some(10)),
            (30, 2, Some(20)),
            (40, 3, Some(30)),
            (40, 4, Some(40)),
        ]
    );
    assert_eq!(walked[3].get_altids().get(EXEC_ID), Some("E-5"));
    assert_eq!(walked[4].get_altids().get(EXEC_ID), Some("E-4"));
    assert_eq!(walked[4].get_prevuuid(), Some(walked[3].get_curruuid()));
}

#[test]
fn an_element_that_ended_retires_its_identity_and_a_later_one_starts_afresh() {
    let mut done = incarnation("O-100", 20);
    done.set_state(filled());
    done.finalize();
    let arrived = vec![incarnation("O-100", 10), done, incarnation("O-100", 30)];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the first");
    let done = walk.next().expect("the fill");
    assert_eq!(done.get_seqnum(), 1, "the ending element still follows");
    assert_eq!(walk.alive().count(), 0, "and nothing is alive after it");
    let fresh = walk.next().expect("the late one");
    assert_eq!((fresh.get_seqnum(), fresh.get_prevuuid()), (0, None));
    assert_eq!(walk.alive().count(), 1);

    // An element past its expiration ended the same way; one that expires
    // later than it happened is still alive.
    let mut expired = incarnation("O-100", 20);
    expired.set_exprtime(Some(at(20)));
    let mut open = incarnation("O-100", 30);
    open.set_exprtime(Some(at(31)));
    let arrived = vec![
        incarnation("O-100", 10),
        expired,
        open,
        incarnation("O-100", 40),
    ];
    let walked: Vec<_> = EventIterator::new(arrived, true).collect();
    assert_eq!(
        places(walked.clone().into_iter()),
        [
            (10, 0, None),
            (20, 1, Some(10)),
            (30, 0, None),
            (31, 1, Some(30)),
            (40, 0, None),
        ]
    );
    assert_eq!(walked[3].get_state(), &State::read("expired").unwrap());
    assert_eq!(
        (walked[4].get_seqnum(), walked[4].get_prevuuid()),
        (0, None),
        "the emitted expiry retired the identity"
    );
}

/// One partial fill of `order` at `ms`: an execution, whatever state it
/// reached, because the leaf's kind says so.
fn executed(order: &str, ms: i64) -> ExecutionEvent {
    let mut event = ExecutionEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.set_state(State::read("PartiallyFilled").unwrap());
    event.finalize();
    event
}

#[test]
fn the_walk_dates_each_execution_and_keeps_an_explicit_clock() {
    // An execution is dated by its own instant where it states no clock;
    // carrying the latest clock onto a non-execution successor is the
    // trait's reading, pinned over a foreign event in
    // `rust/tests/graph/element.rs`.
    let execution = executed("E-1", 10);
    let mut explicit = executed("E-1", 20);
    explicit.set_execunix(Some(at(15)));
    explicit.set_recdunix(Some(at(16)));
    explicit.finalize();
    let later_execution = executed("E-1", 30);

    let walked: Vec<_> = EventIterator::new([execution, explicit, later_execution], true).collect();
    assert_eq!(
        walked[0].get_execunix(),
        Some(at(10)),
        "the first event is filled"
    );
    assert_eq!(
        walked[0].get_recdunix(),
        None,
        "recording is never inferred"
    );
    assert_eq!(
        walked[1].get_execunix(),
        Some(at(15)),
        "an explicit instant is kept"
    );
    assert_eq!(walked[1].get_recdunix(), Some(at(16)));
    assert_eq!(
        walked[2].get_execunix(),
        Some(at(30)),
        "a later execution replaces the carried clock"
    );
    assert_eq!(
        walked[2].get_recdunix(),
        None,
        "following does not inherit the predecessor's recording clock"
    );
}

#[test]
fn replay_keeps_each_execution_clock_and_the_chain() {
    let first: Vec<_> =
        EventIterator::new([executed("E-1", 10), executed("E-1", 20)], true).collect();
    assert_eq!(first[0].get_execunix(), Some(at(10)));
    assert!(first[1].get_state().is_execution());
    assert_eq!(first[1].get_execunix(), Some(at(20)));

    let replayed: Vec<_> = EventIterator::new(first, true).collect();
    assert_eq!(replayed[1].get_execunix(), Some(at(20)));
    assert_eq!(replayed[1].get_seqnum(), 1);
    assert_eq!(replayed[1].get_prevuuid(), Some(replayed[0].get_curruuid()));
}

#[test]
fn an_element_with_no_cross_identity_stands_under_its_own() {
    // Two elements that are nothing elsewhere follow nothing: each is its
    // own identity, and nothing arrives under it but a restatement.
    let mut first = OrderEvent::at(at(10));
    first.finalize();
    assert_eq!(first.get_crossuuid(), first.get_curruuid());
    let mut second = OrderEvent::at(at(20));
    second.finalize();
    // A restatement of the first: the same event, said again.
    let arrived = vec![first.clone(), second, first.clone()];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the first");
    let second = walk.next().expect("the second");
    assert_eq!((second.get_seqnum(), second.get_prevuuid()), (0, None));
    // The restatement arrives under the identity the first arrived under,
    // so it restates it: the first one's place - none - is its own, its
    // identity the first one's, and it stands as the live one.
    let restated = walk.next().expect("the restatement");
    assert_eq!(
        (
            restated.get_currunix(),
            restated.get_seqnum(),
            restated.get_prevuuid()
        ),
        (at(10), 0, None)
    );
    assert_eq!(restated.get_curruuid(), first.get_curruuid());
    assert_eq!(restated, first);
    assert_eq!(walk.alive().count(), 2);
    assert!(walk.alive().any(|event| *event == first));
}

#[test]
fn a_twin_of_the_live_element_restates_it_and_the_chain_grows_by_nothing() {
    // One message a capture logged at two hops: the same instant, the same
    // content, arriving under the identity the live element arrived under.
    let fill = incarnation("O-100", 20);
    let arrived = vec![
        incarnation("O-100", 10),
        fill.clone(),
        fill,
        incarnation("O-100", 30),
    ];
    let mut walk = EventIterator::new(arrived, true);
    let first = walk.next().expect("the order");
    let second = walk.next().expect("the fill");
    assert_eq!(
        (second.get_seqnum(), second.get_prevuuid()),
        (1, Some(first.get_curruuid()))
    );
    let twin = walk.next().expect("the fill, logged again");
    assert_eq!(twin.get_seqnum(), 1, "the chain grows by nothing");
    assert_eq!(twin.get_prevuuid(), second.get_prevuuid());
    assert_eq!(twin.get_curruuid(), second.get_curruuid());
    assert_eq!(twin, second);
    // The next one follows the twin, which is to say the fill.
    let third = walk.next().expect("the third");
    assert_eq!(
        (third.get_seqnum(), third.get_prevuuid()),
        (2, Some(second.get_curruuid()))
    );
    assert_eq!(walk.alive().count(), 1);

    // A twin that knows more of the lifecycle adds what it knows to the
    // live one's place: a clock is never in the identity, so the twin
    // still arrives under it, and the identity stays where it was.
    let mut dated = incarnation("O-100", 20);
    dated.set_creaunix(Some(at(5)));
    let arrived = vec![incarnation("O-100", 10), incarnation("O-100", 20), dated];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the order");
    let second = walk.next().expect("the fill");
    assert_eq!(second.get_creaunix(), None);
    let twin = walk.next().expect("the fill, dated");
    assert_eq!(twin.get_prevuuid(), second.get_prevuuid());
    assert_eq!(twin.get_seqnum(), second.get_seqnum());
    assert_eq!(twin.get_creaunix(), Some(at(5)));
    assert_eq!(twin.get_curruuid(), second.get_curruuid());
    assert_eq!(
        walk.alive().next().map(Event::get_creaunix),
        Some(Some(at(5))),
        "the live one knows what the twin added"
    );
    // A statement going by a name the live one does not is another
    // statement, not a twin: the names an operation goes by are part of
    // what it states, so it arrives under an identity of its own and is
    // the one after the fill.
    let named = named("O-100", 20, EXEC_ID, "E-2");
    let arrived = vec![incarnation("O-100", 10), incarnation("O-100", 20), named];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the order");
    let second = walk.next().expect("the fill");
    let successor = walk.next().expect("the fill, named");
    assert_eq!(successor.get_prevuuid(), Some(second.get_curruuid()));
    assert_eq!(successor.get_seqnum(), 2);
    assert_eq!(successor.get_altids().get(EXEC_ID), Some("E-2"));
    assert_eq!(
        walk.alive().next().map(Element::get_curruuid),
        Some(successor.get_curruuid())
    );

    // On a grid, source statements stay source statements. The owned views
    // are separate and a twin changes no chain place in either one.
    let first = incarnation("O-100", 10);
    let arrived = vec![
        first.clone(),
        first,
        incarnation("O-100", 15),
        incarnation("O-100", 20),
    ];
    let walk = EventIterator::new(arrived, true).with_snapshot_ns(10 * MS);
    assert_eq!(
        walk.map(|event| (event.get_seqnum(), event.get_snapunix().map(ms)))
            .collect::<Vec<_>>(),
        [
            (0, None),
            (0, None),
            (0, Some(10)),
            (1, None),
            (2, None),
            (2, Some(20)),
        ]
    );
}

#[test]
fn an_element_under_no_live_identity_follows_the_live_one_it_shares_a_name_with() {
    // An order placed under a `ClOrdID`, and a report of it that spells no
    // `OrderID` of its own: the report's cross element is its own identity,
    // alive under nothing, so the name it shares with the live order is
    // the chain it belongs to.
    let order = named("O-100", 10, CL_ORD_ID, "C-1");
    let report = anonymous(20, &[(CL_ORD_ID, "C-1"), (EXEC_ID, "E-1")]);
    assert_ne!(report.get_crossuuid(), order.get_crossuuid());
    let mut walk = EventIterator::new(vec![order, report], true);
    let order = walk.next().expect("the order");
    let report = walk.next().expect("the report");
    assert_eq!(report.get_prevuuid(), Some(order.get_curruuid()));
    assert_eq!(report.get_seqnum(), 1);
    // Followed, the report carries the chain's cross code and stands as
    // the live one under the chain's identity, its own names with it.
    assert_eq!(report.get_crosscode(), "O-100");
    assert_eq!(report.get_crossuuid(), order.get_crossuuid());
    assert_eq!(walk.alive().count(), 1);
    let live = walk.alive().next().expect("the report stands");
    assert_eq!(live.get_currunix(), at(20));
    assert_eq!(live.get_altids().get(EXEC_ID), Some("E-1"));

    // A name no live element goes by starts a chain of its own, and an
    // element whose own identity is alive stays under it whatever names
    // it shares.
    let stranger = anonymous(30, &[(CL_ORD_ID, "C-9")]);
    let other = named("O-900", 40, CL_ORD_ID, "C-1");
    let mut walk = EventIterator::new(
        vec![
            named("O-100", 10, CL_ORD_ID, "C-1"),
            named("O-900", 15, CL_ORD_ID, "C-2"),
            stranger,
            other,
        ],
        true,
    );
    walk.next().expect("the order");
    let other_first = walk.next().expect("the other order");
    let stranger = walk.next().expect("the stranger");
    assert_eq!((stranger.get_seqnum(), stranger.get_prevuuid()), (0, None));
    assert_eq!(stranger.get_crosscode(), "");
    let other = walk.next().expect("the other order's second");
    assert_eq!(other.get_prevuuid(), Some(other_first.get_curruuid()));
    assert_eq!(other.get_crosscode(), "O-900");
    assert_eq!(walk.alive().count(), 3);

    // A chain that ended took its names with it: a later report spelling
    // only the name starts afresh.
    let mut fill = named("O-100", 20, CL_ORD_ID, "C-1");
    fill.set_state(filled());
    fill.finalize();
    let late = anonymous(30, &[(CL_ORD_ID, "C-1")]);
    let mut walk = EventIterator::new(vec![named("O-100", 10, CL_ORD_ID, "C-1"), fill, late], true);
    walk.next().expect("the order");
    walk.next().expect("the fill");
    assert_eq!(walk.alive().count(), 0);
    let late = walk.next().expect("the late report");
    assert_eq!((late.get_seqnum(), late.get_prevuuid()), (0, None));
    assert_eq!(late.get_crosscode(), "");
    assert_eq!(late.get_crossuuid(), late.get_curruuid());
}

#[test]
fn an_element_before_the_live_one_is_yielded_as_it_came_and_changes_nothing() {
    // The caller called the walk sorted, and one element arrives late.
    let arrived = vec![
        incarnation("O-100", 10),
        incarnation("O-100", 30),
        incarnation("O-100", 20),
        incarnation("O-100", 40),
    ];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the first");
    let third = walk.next().expect("the third");
    assert_eq!(third.get_seqnum(), 1);
    let stray = walk.next().expect("the late one");
    assert_eq!((stray.get_seqnum(), stray.get_prevuuid()), (0, None));
    // The live element is still the third, and the fourth follows it.
    let fourth = walk.next().expect("the fourth");
    assert_eq!(fourth.get_prevuuid(), Some(third.get_curruuid()));
    assert_eq!(fourth.get_seqnum(), 2);
}

#[test]
fn the_walk_states_its_size_and_is_fused() {
    let arrived = || vec![incarnation("O-100", 10), incarnation("O-100", 20)];
    let mut streamed = EventIterator::new(arrived(), true);
    assert_eq!(streamed.size_hint(), (2, None));
    streamed.next();
    assert_eq!(streamed.size_hint(), (1, None));
    let mut sorted = EventIterator::new(arrived(), false);
    assert_eq!(sorted.size_hint(), (2, None));
    assert_eq!(sorted.by_ref().count(), 2);
    assert_eq!(sorted.size_hint(), (0, None));
    assert!(sorted.next().is_none());
    assert!(sorted.next().is_none());
    // A walk over an empty source is alive to nothing.
    let mut empty = EventIterator::new(Vec::<OrderEvent>::new(), false);
    assert!(empty.next().is_none());
    assert_eq!(empty.alive().count(), 0);
}

#[test]
fn a_grid_starts_no_earlier_than_the_first_fact_and_zero_preserves_source_stamps() {
    // The grid is aligned on the epoch. A first observation just before its
    // boundary becomes live at that observation, then is copied at zero;
    // it is never copied into the earlier step where it did not yet exist.
    let mut before = OrderEvent::at(-1);
    before.set_crosscode("O-100".to_owned());
    before.finalize();
    let mut after = OrderEvent::at(1);
    after.set_crosscode("O-900".to_owned());
    after.finalize();
    let walked: Vec<_> = EventIterator::new(vec![before, after], true)
        .with_snapshot_ns(10)
        .collect();
    assert_eq!(walked[0].get_snapunix(), None);
    assert_eq!(
        walked
            .iter()
            .filter_map(|event| event.get_snapunix())
            .collect::<Vec<_>>(),
        [0]
    );

    // Without a grid the snapshot instant is left as it came, and a step
    // of no width is no grid.
    let mut stamped = incarnation("O-100", 10);
    stamped.set_snapunix(Some(at(5)));
    let walk = EventIterator::new(vec![stamped], true).with_snapshot_ns(0);
    assert_eq!(walk.snapshot_ns(), None);
    assert_eq!(
        walk.map(|event| event.get_snapunix()).collect::<Vec<_>>(),
        [Some(at(5))]
    );
}

#[test]
fn the_walk_yields_the_callers_own_copy_and_reads_any_event() {
    // What the walk yields is the caller's: the live element is a clone the
    // walk keeps, and a caller may change what it was handed without
    // moving the walk.
    let arrived = vec![incarnation("O-100", 10), incarnation("O-100", 20)];
    let mut walk = EventIterator::new(arrived, true);
    let mut first = walk.next().expect("the first");
    first.set_curruuid(Uuid::from_v8(1));
    let second = walk.next().expect("the second");
    assert_ne!(second.get_prevuuid(), Some(Uuid::from_v8(1)));
    assert_eq!(
        walk.alive().next().map(Element::get_curruuid),
        Some(second.get_curruuid())
    );
}

#[test]
fn a_deadline_emits_one_expired_snapshot_and_retires_the_live_identity() {
    let mut order = incarnation("O-100", 10);
    order.set_execunix(Some(at(8)));
    order.set_recdunix(Some(at(9)));
    order.set_exprtime(Some(at(20)));
    let original = order.clone();
    let walked: Vec<_> = EventIterator::new(
        [order, incarnation("O-900", 30), incarnation("O-100", 40)],
        true,
    )
    .collect();

    assert_eq!(
        walked
            .iter()
            .map(|event| ms(event.get_currunix()))
            .collect::<Vec<_>>(),
        [10, 20, 30, 40]
    );
    assert_eq!(walked[0], original, "an emitted snapshot stays immutable");
    let expired = &walked[1];
    assert_eq!(
        expired.get_state(),
        &State::from_spelling("expired").unwrap()
    );
    assert_eq!(expired.get_exprtime(), Some(at(20)));
    assert_eq!(
        expired.get_execunix(),
        Some(at(8)),
        "expiry carries the lifecycle's latest execution"
    );
    assert_eq!(expired.get_recdunix(), None, "expiry is a new event");
    assert_eq!(expired.get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(expired.get_seqnum(), 1);
    assert_eq!(expired.get_crossuuid(), walked[0].get_crossuuid());
    assert_eq!(
        (walked[3].get_seqnum(), walked[3].get_prevuuid()),
        (0, None),
        "the later incarnation starts after expiry"
    );
}

#[test]
fn deadlines_precede_equal_time_sources_and_views_and_eof_drains_in_order() {
    let mut first = incarnation("O-100", 10);
    first.set_exprtime(Some(at(20)));
    let mut second = incarnation("O-900", 20);
    second.set_exprtime(Some(at(30)));
    let walked: Vec<_> = EventIterator::new([first, second], true)
        .with_snapshot_ns(10 * MS)
        .collect();

    assert_eq!(
        walked
            .iter()
            .map(|event| (
                ms(event.get_currunix()),
                event.get_snapunix().map(ms),
                event.get_crosscode(),
                event.get_state().is_failed(),
            ))
            .collect::<Vec<_>>(),
        [
            (10, None, "O-100", false),
            (10, Some(10), "O-100", false),
            (20, None, "O-100", true),
            (20, None, "O-900", false),
            (20, Some(20), "O-900", false),
            (30, None, "O-900", true),
        ]
    );
}

#[test]
fn eof_keeps_the_deadline_horizon_for_nonexpiring_neighbors() {
    let mut finite = incarnation("O-100", 10);
    finite.set_exprtime(Some(at(20)));
    let forever = incarnation("O-900", 10);
    let walked: Vec<_> = EventIterator::new([finite, forever], true)
        .with_snapshot_ns(10 * MS)
        .collect();

    assert!(
        walked.iter().any(|event| {
            event.get_crosscode() == "O-100"
                && event.get_currunix() == at(20)
                && event.get_state().is_failed()
        }),
        "the finite identity expires at the horizon"
    );
    assert_eq!(
        walked
            .iter()
            .filter(|event| event.get_snapunix() == Some(at(20)))
            .map(|event| event.get_crosscode())
            .collect::<Vec<_>>(),
        ["O-900"],
        "the neighbor still living at that horizon gets its view"
    );
    assert!(
        walked
            .iter()
            .all(|event| event.get_snapunix().is_none_or(|tick| tick <= at(20))),
        "a nonexpiring identity does not extend the finite source"
    );
}

#[test]
fn replacing_or_ending_a_generation_removes_its_stale_deadline() {
    let mut first = incarnation("O-100", 10);
    first.set_exprtime(Some(at(20)));
    let mut replacement = incarnation("O-100", 15);
    replacement.set_exprtime(Some(at(40)));
    let mut canceled = incarnation("O-100", 30);
    canceled.set_state(State::from_spelling("canceled").unwrap());
    canceled.finalize();

    let walked: Vec<_> =
        EventIterator::new([first.clone(), replacement.clone(), canceled], true).collect();
    assert_eq!(
        walked
            .iter()
            .map(|event| ms(event.get_currunix()))
            .collect::<Vec<_>>(),
        [10, 15, 30],
        "neither replaced deadline survives the terminal generation"
    );

    let walked: Vec<_> = EventIterator::new([first, replacement], true).collect();
    assert_eq!(
        walked
            .iter()
            .map(|event| ms(event.get_currunix()))
            .collect::<Vec<_>>(),
        [10, 15, 40],
        "the replacement owns the one remaining deadline"
    );
    assert_eq!(walked[2].get_prevuuid(), Some(walked[1].get_curruuid()));

    let mut later = incarnation("O-200", 10);
    later.set_exprtime(Some(at(40)));
    let mut shortened = incarnation("O-200", 15);
    shortened.set_exprtime(Some(at(20)));
    let walked: Vec<_> = EventIterator::new([later, shortened], true).collect();
    assert_eq!(
        walked
            .iter()
            .map(|event| ms(event.get_currunix()))
            .collect::<Vec<_>>(),
        [10, 15, 20],
        "an explicitly replaced deadline may move earlier"
    );
}

#[test]
fn a_grid_copies_every_living_identity_at_each_crossed_tick() {
    let mut first = incarnation("O-100", 10);
    first.set_exprtime(Some(at(35)));
    let arrived = [first, incarnation("O-900", 15), incarnation("O-100", 25)];
    let walked: Vec<_> = EventIterator::new(arrived, true)
        .with_snapshot_ns(10 * MS)
        .collect();

    let sources: Vec<_> = walked
        .iter()
        .filter(|event| event.get_snapunix().is_none() && !event.get_state().is_failed())
        .collect();
    assert_eq!(
        sources
            .iter()
            .map(|event| ms(event.get_currunix()))
            .collect::<Vec<_>>(),
        [10, 15, 25],
        "source events keep their stated snapshot fact"
    );
    let mut snapshots: Vec<_> = walked
        .iter()
        .filter_map(|event| {
            event
                .get_snapunix()
                .map(|tick| (ms(tick), event.get_crosscode(), ms(event.get_currunix())))
        })
        .collect();
    snapshots.sort_unstable();
    assert_eq!(
        snapshots,
        [
            (10, "O-100", 10),
            (20, "O-100", 10),
            (20, "O-900", 15),
            (30, "O-100", 25),
            (30, "O-900", 15),
        ]
    );
    for snapshot in walked.iter().filter(|event| event.get_snapunix().is_some()) {
        let source = sources
            .iter()
            .rev()
            .find(|source| {
                source.get_crosscode() == snapshot.get_crosscode()
                    && source.get_currunix() == snapshot.get_currunix()
            })
            .expect("the living source copied at this tick");
        assert_eq!(snapshot.get_curruuid(), source.get_curruuid());
        assert_eq!(snapshot.get_seqnum(), source.get_seqnum());
    }
    assert!(walked.iter().all(|event| {
        event
            .get_snapunix()
            .is_none_or(|tick| tick <= event.get_exprtime().unwrap_or(i64::MAX))
    }));
    assert_eq!(
        walked.last().map(|event| ms(event.get_currunix())),
        Some(35),
        "EOF stops the grid at the greatest finite deadline"
    );
}

/// The name index a caller only ever sees the result of.
///
/// An event arriving under no live identity finds its chain through a name a
/// live event goes by - one of its alternate identifiers - and the two maps
/// that make that lookup are the walk's own; settling and retiring an
/// identity directly is what pins that retiring a name forgets exactly the
/// records it opened.
#[cfg(feature = "internals")]
mod naming {
    use yggdryl::graph::{Element, EventIterator, Operation, OrderEvent};
    use yggdryl::internals::graph_iterator::{
        name_records, named_identities, named_identity, named_schemes, retire, settle,
    };

    fn named(cross: &str, unix: i64, scheme: &str, name: &str) -> OrderEvent {
        let mut event = OrderEvent::at(unix);
        event.set_crosscode(cross.to_owned());
        event
            .insert_altid(scheme, name)
            .expect("a plain holder takes every key");
        event.finalize();
        event
    }

    #[test]
    fn retiring_the_last_name_removes_its_whole_index() {
        let event = named("A", 1, "VENUEORDERID", "A-1");
        let identity = event.get_crossuuid();
        let mut walk = EventIterator::new(Vec::<OrderEvent>::new(), true);
        settle(&mut walk, identity, &event, event.get_curruuid());
        assert_eq!(named_schemes(&walk), 1);
        assert_eq!(named_identities(&walk), 1);

        assert!(retire(&mut walk, identity), "the live identity retires");
        assert_eq!(named_schemes(&walk), 0);
        assert_eq!(named_identities(&walk), 0);
    }

    #[test]
    fn alternating_name_ownership_keeps_one_reverse_record() {
        let first = named("A", 1, "VENUEORDERID", "SHARED");
        let second = named("B", 2, "VENUEORDERID", "SHARED");
        let first_identity = first.get_crossuuid();
        let second_identity = second.get_crossuuid();
        let mut walk = EventIterator::new(Vec::<OrderEvent>::new(), true);

        for turn in 0..64 {
            let (identity, event) = if turn % 2 == 0 {
                (first_identity, &first)
            } else {
                (second_identity, &second)
            };
            settle(&mut walk, identity, event, event.get_curruuid());
            assert_eq!(
                named_identity(&walk, "VENUEORDERID", "SHARED"),
                Some(identity),
                "the latest owner remains the lookup target"
            );
            assert_eq!(
                name_records(&walk),
                1,
                "one current lookup retains one reverse ownership record"
            );
        }

        assert!(retire(&mut walk, first_identity), "the first owner retires");
        assert!(
            retire(&mut walk, second_identity),
            "the second owner retires"
        );
        assert_eq!(named_schemes(&walk), 0);
        assert_eq!(named_identities(&walk), 0);
    }
}

/// The walk over the one value every boundary crosses as: an execution
/// follows the order it fills across kinds, its twin restates it and the
/// chain grows by nothing, and a value the walk does not chain - a book
/// side - is yielded where it is read and changes nothing.
#[test]
fn a_market_data_walk_chains_across_operation_kinds_and_passes_the_rest_through() {
    use yggdryl::Side;
    use yggdryl::graph::{BookSide, MarketData};

    let fill = executed("O-500", 20);
    let arrived = vec![
        MarketData::from(incarnation("O-500", 10)),
        MarketData::from(BookSide::new(Side::Buy).unwrap()),
        MarketData::from(fill.clone()),
        MarketData::from(fill),
        MarketData::from(incarnation("O-500", 30)),
    ];
    let mut walk = EventIterator::new(arrived, true);
    let order = walk.next().expect("the order");
    let side = walk.next().expect("the side, as it came");
    assert!(side.as_book_side().is_some());
    let second = walk.next().expect("the fill");
    let leaf = second
        .as_execution_event()
        .expect("the fill keeps its kind");
    assert_eq!(
        (leaf.get_seqnum(), leaf.get_prevuuid()),
        (1, Some(order.get_curruuid()))
    );
    let twin = walk.next().expect("the fill, logged again");
    assert_eq!(twin, second, "the chain grows by nothing");
    let third = walk.next().expect("the order's next statement");
    assert_eq!(
        (
            third.as_order_event().unwrap().get_seqnum(),
            third.as_order_event().unwrap().get_prevuuid()
        ),
        (2, Some(second.get_curruuid()))
    );
    assert_eq!(walk.alive().count(), 1);
}
