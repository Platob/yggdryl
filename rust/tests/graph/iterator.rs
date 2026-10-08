//! `rust/src/graph/iterator.rs`: the one walk over market operation events:
//! each event chained to the live one under its cross identity - or, where
//! that is alive under nothing, under a name a live event goes by - the
//! alive set kept as the lifecycle moves, and the caller's word on the order
//! taken or the order made.

use yggdryl::IdKey;

use yggdryl::graph::{
    Element, Event, EventIterator, ExecutionEvent, Market, Operation, OrderEvent,
};
use yggdryl::{IdType, Identifier, State, Uuid};

use super::element::{crosshash, filled};

/// Asserts `event` derives its cross identity from the cross code it
/// stores: the cross hash the code's XXH3-64, the cross element the UUIDv8
/// over that hash.
fn assert_derived(event: &impl Element, what: &str) {
    assert_eq!(
        event.get_crosshashcode(),
        crosshash(event.get_crosscode()),
        "{what}: the cross hash"
    );
    assert_eq!(
        event.get_crossuuid(),
        Uuid::from_v8(u128::from(event.get_crosshashcode())),
        "{what}: the cross element"
    );
}

/// One identifier of a plain holder: a value of `kind` from `fix`.
fn identifier(kind: &IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::base(kind.clone()), value).unwrap()
}

/// One nanosecond count per millisecond: a derived identity opens with the
/// microsecond its instant falls in, and the instants below are spaced a
/// whole millisecond apart, so no two of them ever share one.
const MS: i64 = 1_000_000;

/// The identifier types the walk's fixtures go by: the order's own
/// identifier, which a following event carries forward, and the client and
/// execution identifiers, which name an event without following.
const ORDER_ID: IdType = IdType::OrderId;
const CL_ORD_ID: IdType = IdType::ClOrdId;
const EXEC_ID: IdType = IdType::ExecId;

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
fn named(order: &str, ms: i64, scheme: &IdType, name: &str) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event
        .insert_identifier(identifier(scheme, name))
        .expect("a plain holder takes every key");
    event.finalize();
    event
}

/// An event at `ms` going by `names` and stating no cross code of its own.
fn anonymous(ms: i64, names: &[(IdType, &str)]) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    for (scheme, name) in names {
        event
            .insert_identifier(identifier(scheme, name))
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
    first
        .insert_identifier(identifier(&ORDER_ID, "O-100"))
        .unwrap();
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
    assert_eq!(other.get_crosscode(), "10:0:O-900");
    let second = walk.next().expect("the second incarnation");
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevunix(), Some(at(10)));
    assert_eq!(second.get_seqnum(), 0);
    assert_eq!(second.get_crossuuid(), first.get_crossuuid());
    // Enriched by the element's own reading: the lifecycle carried forward,
    // the order's own identifier with it, and the identity re-derived
    // around them.
    assert_eq!(second.get_creaunix(), Some(at(5)));
    assert_eq!(second.get_identifiers().get(&ORDER_ID), Some("O-100"));
    assert_eq!(
        second.get_curruuid(),
        second.time_uuid().expect("an identity")
    );
    let third = walk.next().expect("the third incarnation");
    assert_eq!(third.get_prevuuid(), Some(second.get_curruuid()));
    assert_eq!(third.get_seqnum(), 0);
    assert_eq!(third.get_creaunix(), Some(at(5)));
    // A walk over the walked answers the same chain.
    assert!(walk.next().is_none());
    let walked = vec![first.clone(), other.clone(), second.clone(), third.clone()];
    let again: Vec<OrderEvent> = EventIterator::new(walked, true).collect();
    assert_eq!(again, [first, other, second, third]);
    // Both orders are still alive, the latest incarnation of each.
    assert_eq!(
        alive(&walk),
        [("10:0:O-100".to_owned(), 30), ("10:0:O-900".to_owned(), 15)]
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
        named("O-100", 40, &EXEC_ID, "E-5"),
        named("O-100", 40, &EXEC_ID, "E-4"),
    ];
    let walk = EventIterator::new(arrived, false);
    let walked: Vec<OrderEvent> = walk.collect();
    assert_eq!(
        places(walked.iter().cloned()),
        [
            (10, 0, None),
            (20, 0, Some(10)),
            (30, 0, Some(20)),
            (40, 0, Some(30)),
            (40, 1, Some(40)),
        ]
    );
    assert_eq!(walked[3].get_identifiers().get(&EXEC_ID), Some("E-5"));
    assert_eq!(walked[4].get_identifiers().get(&EXEC_ID), Some("E-4"));
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
    assert_eq!(done.get_seqnum(), 0, "the ending element still follows");
    assert_eq!(walk.alive().count(), 0, "and nothing is alive after it");
    let fresh = walk.next().expect("the late one");
    assert_eq!((fresh.get_seqnum(), fresh.get_prevuuid()), (0, None));
    assert_eq!(walk.alive().count(), 1);

    // An element past its expiration ended the same way; one that expires
    // later than it happened is still alive.
    let mut expired = incarnation("O-100", 20);
    expired.set_exprunix(Some(at(20)));
    let mut open = incarnation("O-100", 30);
    open.set_exprunix(Some(at(31)));
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
            (20, 0, Some(10)),
            (30, 0, None),
            (31, 0, Some(30)),
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
    explicit.set_execunix(Some(at(15)), true);
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
    assert_eq!(replayed[1].get_seqnum(), 0);
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
    // The walk states the creation the first left unstated: its own instant.
    first.set_creaunix(Some(at(10)));
    assert_eq!(restated, first);
    assert_eq!(walk.alive().count(), 2);
    assert!(walk.alive().any(|event| *event == first));
}

#[test]
fn the_walk_states_each_lifecycles_creation_and_never_replaces_a_stated_one() {
    // A chain whose events state no creation: the first is created at its
    // own instant, and every event after it at the chain's creation.
    let mut walk = EventIterator::new(
        vec![
            incarnation("O-1", 10),
            incarnation("O-1", 20),
            incarnation("O-1", 30),
        ],
        true,
    );
    let chain: Vec<OrderEvent> = walk.by_ref().collect();
    assert!(
        chain
            .iter()
            .all(|event| event.get_creaunix() == Some(at(10)))
    );
    // The instant is no part of what an event digests: stating it moved no
    // identity, and the chain keeps one cross element.
    let first = incarnation("O-1", 10);
    assert_eq!(chain[0].get_curruuid(), first.get_curruuid());
    assert!(
        chain
            .iter()
            .all(|event| event.get_crossuuid() == first.get_crossuuid())
    );

    // A stated creation stands, on the first event and on a later one; an
    // unstated one after it takes the earliest the fold kept.
    let mut stated = incarnation("O-2", 10);
    stated.set_creaunix(Some(at(5)));
    let mut later = incarnation("O-2", 20);
    later.set_creaunix(Some(at(1)));
    let walked: Vec<OrderEvent> =
        EventIterator::new(vec![stated, later, incarnation("O-2", 30)], true).collect();
    assert_eq!(
        walked
            .iter()
            .map(|event| event.get_creaunix())
            .collect::<Vec<_>>(),
        [Some(at(5)), Some(at(1)), Some(at(1))]
    );
}

#[test]
fn a_chain_with_no_cross_code_is_one_cross_element() {
    // An order stating no cross code is its own chain's identity, and a
    // report joining it by a name it shares carries that identity as its
    // cross element rather than its own.
    let order = anonymous(10, &[(CL_ORD_ID, "C-7")]);
    let report = anonymous(20, &[(CL_ORD_ID, "C-7"), (EXEC_ID, "E-7")]);
    let walked: Vec<OrderEvent> = EventIterator::new(vec![order, report], true).collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[0].get_crossuuid(), walked[0].get_curruuid());
    assert_eq!(walked[1].get_crossuuid(), walked[0].get_crossuuid());
    assert_ne!(walked[1].get_crossuuid(), walked[1].get_curruuid());
}

#[test]
fn a_chain_with_no_cross_code_keeps_one_cross_element_through_an_update_an_expiry_a_twin_and_a_stated_code()
 {
    use yggdryl::Side;

    let event = |ms: i64, code: Option<&str>, state: State, expiry: Option<i64>| {
        let mut event = OrderEvent::at(at(ms));
        if let Some(code) = code {
            event.set_crosscode(code.to_owned());
        }
        event.set_side(Side::Buy, true);
        event.set_state(state);
        event.set_exprunix(expiry);
        event
            .insert_identifier(identifier(&CL_ORD_ID, "C-7"))
            .unwrap();
        event.finalize();
        event
    };

    // A NEW over the live NEW walks UPDATED.
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![
            event(10, None, State::New, None),
            event(20, None, State::New, None),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[0].get_crossuuid(), walked[0].get_curruuid());
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(*walked[1].get_state(), State::Updated);
    assert_eq!(
        walked[1].get_crossuuid(),
        walked[0].get_crossuuid(),
        "the update"
    );

    // The expiry the walk emits at the deadline.
    let walked: Vec<OrderEvent> =
        EventIterator::new(vec![event(10, None, State::New, Some(at(30)))], true).collect();
    assert_eq!(walked.len(), 2);
    assert_eq!(*walked[1].get_state(), State::Expired);
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(
        walked[1].get_crossuuid(),
        walked[0].get_crossuuid(),
        "the expiry"
    );

    // A twin of a follower, restating it.
    let fill = event(20, None, State::PartiallyFilled, None);
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![event(10, None, State::New, None), fill.clone(), fill],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_crossuuid(), walked[0].get_crossuuid());
    assert_eq!(walked[2], walked[1], "the twin");

    // A follower joining by the name, stating a code of its own.
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![
            event(10, None, State::New, None),
            event(20, Some("O-9"), State::PartiallyFilled, None),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(
        walked[1].get_crossuuid(),
        walked[0].get_crossuuid(),
        "the stated code"
    );
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
        (0, Some(first.get_curruuid()))
    );
    let twin = walk.next().expect("the fill, logged again");
    assert_eq!(twin.get_seqnum(), 0, "the chain grows by nothing");
    assert_eq!(twin.get_prevuuid(), second.get_prevuuid());
    assert_eq!(twin.get_curruuid(), second.get_curruuid());
    assert_eq!(twin, second);
    // The next one follows the twin, which is to say the fill.
    let third = walk.next().expect("the third");
    assert_eq!(
        (third.get_seqnum(), third.get_prevuuid()),
        (0, Some(second.get_curruuid()))
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
    // Stating none, the fill is created when its chain was.
    assert_eq!(second.get_creaunix(), Some(at(10)));
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
    let named = named("O-100", 20, &EXEC_ID, "E-2");
    let arrived = vec![incarnation("O-100", 10), incarnation("O-100", 20), named];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the order");
    let second = walk.next().expect("the fill");
    let successor = walk.next().expect("the fill, named");
    assert_eq!(successor.get_prevuuid(), Some(second.get_curruuid()));
    // The named statement arrives at the fill's own instant, so it takes
    // the place after it rather than keeping its own.
    assert_eq!(successor.get_seqnum(), 1);
    assert_eq!(successor.get_identifiers().get(&EXEC_ID), Some("E-2"));
    assert_eq!(
        walk.alive().next().map(Element::get_curruuid),
        Some(successor.get_curruuid())
    );

    // On a grid, source statements stay source statements. The owned views
    // are separate and a twin changes no place in either one.
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
            (0, None),
            (0, None),
            (0, Some(20)),
        ]
    );
}

#[test]
fn a_statement_logged_again_after_its_chain_moved_on_at_its_instant_restates_it() {
    // An acknowledgement, the fill that moved its chain on at the same
    // instant, then the acknowledgement logged again: a statement of the
    // acknowledgement, not a step after the fill.
    let mut ack = named("O-100", 20, &EXEC_ID, "E-1");
    ack.set_state(State::New);
    ack.finalize();
    let mut fill = named("O-100", 20, &EXEC_ID, "E-2");
    fill.set_state(State::PartiallyFilled);
    fill.finalize();
    let arrived = vec![
        incarnation("O-100", 10),
        ack.clone(),
        fill,
        ack,
        incarnation("O-100", 30),
    ];
    let walked: Vec<OrderEvent> = EventIterator::new(arrived, true).collect();
    let [_, ack, fill, again, next] = walked.as_slice() else {
        panic!("five statements, not {}", walked.len())
    };
    assert_eq!(fill.get_prevuuid(), Some(ack.get_curruuid()));
    assert_eq!(again, ack, "the acknowledgement's own identity and place");
    // The chain stays where the fill moved it.
    assert_eq!(next.get_prevuuid(), Some(fill.get_curruuid()));
}

#[test]
fn a_statement_logged_again_after_the_step_that_ended_its_chain_at_its_instant_restates_it() {
    // An acknowledgement, the fill that ended its chain at the same
    // instant, then each logged again: statements of the two, not a chain
    // started afresh.
    let mut ack = named("O-100", 20, &EXEC_ID, "E-1");
    ack.set_state(State::New);
    ack.finalize();
    let mut fill = named("O-100", 20, &EXEC_ID, "E-2");
    fill.set_state(State::Filled);
    fill.finalize();
    let arrived = vec![
        incarnation("O-100", 10),
        ack.clone(),
        fill.clone(),
        ack.clone(),
        fill,
        incarnation("O-100", 30),
    ];
    let walked: Vec<OrderEvent> = EventIterator::new(arrived, true).collect();
    let [_, first, fill, again, filled_again, next] = walked.as_slice() else {
        panic!("six statements, not {}", walked.len())
    };
    assert_eq!(fill.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(again, first, "the acknowledgement's own identity and place");
    assert_eq!(filled_again, fill, "the fill's own identity and place");
    // The chain stays ended: the next statement starts one afresh.
    assert_eq!(next.get_prevuuid(), None);

    // The walk keeps them only while it reads that instant: one read after
    // a later instant is out of order, and restates nothing.
    let mut walk = EventIterator::new(
        vec![
            incarnation("O-100", 10),
            ack.clone(),
            stating("O-100", 20, State::Filled),
            incarnation("O-200", 30),
            ack,
        ],
        true,
    );
    let first = walk.nth(1).expect("the acknowledgement");
    let late = walk.nth(2).expect("the acknowledgement, read late");
    assert_eq!(late.get_prevuuid(), None);
    assert_ne!(late.get_curruuid(), first.get_curruuid());
}

/// A FIX message held as market data walks as itself: a twin of the live
/// message, and a statement logged again after its chain moved on at its
/// instant, restate the message they repeat through its own reading.
#[test]
fn a_market_data_walk_restates_a_fix_twin_and_a_fix_statement_logged_again() {
    use yggdryl::graph::MarketData;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind};

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
    let codec = FixCodec::new(std::sync::Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ));
    // Walked as market data, read back as the messages the walk yielded.
    let walked = |lines: &[&str]| -> Vec<FixMsg> {
        let arrived: Vec<MarketData> = lines
            .iter()
            .flat_map(|line| codec.parse_line(line.as_bytes()).expect("a readable line"))
            .map(|message| MarketData::from(message.expect("every frame parses")))
            .collect();
        EventIterator::new(arrived, true)
            .filter(|held| held.marketdatakind() == MarketDataKind::Order)
            .map(|held| FixMsg::try_from(held).expect("a FIX message walks as itself"))
            .collect()
    };
    let order =
        "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|";
    let ack = |sequence: u64| {
        format!(
            "8=FIX.4.4|35=8|49=T|56=S|34={sequence}|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|54=1|55=AAPL|10=0|"
        )
    };
    let fill = "8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:30.500|11=A1|37=O1|17=E1|150=F|39=1|14=10|151=90|31=10|32=10|54=1|55=AAPL|10=0|";

    // The acknowledgement delivered again under another sequence: a twin.
    let twin = walked(&[order, &ack(1), &ack(2)]);
    let [order_walked, first, again] = twin.as_slice() else {
        panic!("three order messages, not {}", twin.len())
    };
    assert_eq!(first.get_prevuuid(), Some(order_walked.get_curruuid()));
    assert_eq!(
        (again.get_curruuid(), again.get_prevuuid()),
        (first.get_curruuid(), first.get_prevuuid()),
        "the twin is the acknowledgement again"
    );

    // Delivered again after the fill that moved its chain on at its instant.
    let passed = walked(&[order, &ack(1), fill, &ack(3)]);
    let [_, first, fill, again] = passed.as_slice() else {
        panic!("four order messages, not {}", passed.len())
    };
    assert_eq!(fill.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(
        (again.get_curruuid(), again.get_prevuuid()),
        (first.get_curruuid(), first.get_prevuuid()),
        "the acknowledgement again, not a step after the fill"
    );
}

#[test]
fn an_element_under_no_live_identity_follows_the_live_one_it_shares_a_name_with() {
    // An order placed under a `ClOrdID`, and a report of it that spells no
    // `OrderID` of its own: the report's cross element is its own identity,
    // alive under nothing, so the name it shares with the live order is
    // the chain it belongs to.
    let order = named("O-100", 10, &CL_ORD_ID, "C-1");
    let report = anonymous(20, &[(CL_ORD_ID, "C-1"), (EXEC_ID, "E-1")]);
    assert_ne!(report.get_crossuuid(), order.get_crossuuid());
    let mut walk = EventIterator::new(vec![order, report], true);
    let order = walk.next().expect("the order");
    let report = walk.next().expect("the report");
    assert_eq!(report.get_prevuuid(), Some(order.get_curruuid()));
    assert_eq!(report.get_seqnum(), 0);
    // Followed, the report carries the chain's cross code and stands as
    // the live one under the chain's identity, its own names with it.
    assert_eq!(report.get_crosscode(), "10:0:O-100");
    assert_eq!(report.get_crossuuid(), order.get_crossuuid());
    assert_eq!(walk.alive().count(), 1);
    let live = walk.alive().next().expect("the report stands");
    assert_eq!(live.get_currunix(), at(20));
    assert_eq!(live.get_identifiers().get(&EXEC_ID), Some("E-1"));

    // A name no live element goes by starts a chain of its own, and an
    // element whose own identity is alive stays under it whatever names
    // it shares.
    let stranger = anonymous(30, &[(CL_ORD_ID, "C-9")]);
    let other = named("O-900", 40, &CL_ORD_ID, "C-1");
    let mut walk = EventIterator::new(
        vec![
            named("O-100", 10, &CL_ORD_ID, "C-1"),
            named("O-900", 15, &CL_ORD_ID, "C-2"),
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
    assert_eq!(other.get_crosscode(), "10:0:O-900");
    assert_eq!(walk.alive().count(), 3);

    // A chain that ended took its names with it - the value it was placed
    // under and the one a replace gave it alike: a later report spelling
    // either alone starts afresh.
    let mut replaced = named("O-100", 15, &CL_ORD_ID, "C-2");
    replaced
        .insert_identifier(identifier(&IdType::OrigClOrdId, "C-1"))
        .unwrap();
    replaced.finalize();
    let mut fill = named("O-100", 20, &CL_ORD_ID, "C-2");
    fill.set_state(filled());
    fill.finalize();
    let mut walk = EventIterator::new(
        vec![
            named("O-100", 10, &CL_ORD_ID, "C-1"),
            replaced,
            fill,
            anonymous(30, &[(CL_ORD_ID, "C-1")]),
            anonymous(40, &[(CL_ORD_ID, "C-2")]),
        ],
        true,
    );
    walk.next().expect("the order");
    let replaced = walk.next().expect("the replace");
    assert_eq!(replaced.get_crosscode(), "10:0:O-100");
    walk.next().expect("the fill");
    assert_eq!(walk.alive().count(), 0);
    #[cfg(feature = "internals")]
    assert_eq!(
        yggdryl::internals::graph_iterator::named_identities(&walk),
        0,
        "the ended chain holds no name"
    );
    for value in ["C-1", "C-2"] {
        let late = walk.next().expect("a late report");
        assert_eq!(
            (late.get_seqnum(), late.get_prevuuid()),
            (0, None),
            "{value}"
        );
        assert_eq!(late.get_crosscode(), "", "{value}");
        assert_eq!(late.get_crossuuid(), late.get_curruuid(), "{value}");
    }
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
    assert_eq!(third.get_seqnum(), 0);
    let stray = walk.next().expect("the late one");
    assert_eq!((stray.get_seqnum(), stray.get_prevuuid()), (0, None));
    // The live element is still the third, and the fourth follows it.
    let fourth = walk.next().expect("the fourth");
    assert_eq!(fourth.get_prevuuid(), Some(third.get_curruuid()));
    assert_eq!(fourth.get_seqnum(), 0);

    // A late element stating no side and another code of the chain's kind,
    // naming the chain by a name it goes by: it follows nothing and moves
    // the live element not at all, but it is one of the chain's statements,
    // so it stands under the chain's side, stored cross code and cross
    // element.
    use yggdryl::Side;
    let arrived = vec![
        sided("O-100", 10, Side::Buy, State::New, &[(CL_ORD_ID, "C-1")]),
        sided(
            "O-100",
            30,
            Side::Buy,
            State::PartiallyFilled,
            &[(CL_ORD_ID, "C-1")],
        ),
        sided(
            "O-777",
            20,
            Side::Unknown,
            State::PartiallyFilled,
            &[(CL_ORD_ID, "C-1")],
        ),
    ];
    assert_eq!(arrived[2].get_crosscode(), "10:0:O-777");
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the order");
    let live = walk.next().expect("the live one");
    let stray = walk.next().expect("the late one");
    assert_eq!(stray.get_prevuuid(), None, "it follows nothing");
    assert_eq!(stray.get_side(), Side::Buy, "the chain's side");
    assert_eq!(stray.get_crosscode(), "10:1:O-100", "the chain's code");
    assert_derived(&stray, "the late element");
    assert_eq!(stray.get_crossuuid(), live.get_crossuuid());
    assert_eq!(
        stray.get_curruuid(),
        stray.time_uuid().expect("an identity")
    );
    assert_eq!(
        walk.alive().collect::<Vec<_>>(),
        [&live],
        "the live element unchanged"
    );
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
    // The view at zero is dated at its tick and keeps the instant the event
    // it copies was stated at.
    assert_eq!(
        walked
            .iter()
            .filter_map(|event| event
                .get_snapunix()
                .map(|original| (event.get_currunix(), original)))
            .collect::<Vec<_>>(),
        [(0, -1)]
    );
    for view in walked.iter().filter(|event| event.get_snapunix().is_some()) {
        let mut settled = view.clone();
        settled.finalize();
        settled.set_crossuuid(view.get_crossuuid());
        assert_eq!(*view, settled, "a view is what settling it answers");
    }

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
    order.set_execunix(Some(at(8)), true);
    order.set_recdunix(Some(at(9)));
    order.set_exprunix(Some(at(20)));
    // The walk states the creation the order left unstated, and nothing else.
    let mut original = order.clone();
    original.set_creaunix(Some(at(10)));
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
    assert_eq!(expired.get_exprunix(), Some(at(20)));
    assert_eq!(
        expired.get_execunix(),
        Some(at(8)),
        "expiry carries the lifecycle's latest execution"
    );
    assert_eq!(expired.get_recdunix(), None, "expiry is a new event");
    assert_eq!(expired.get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(expired.get_seqnum(), 0);
    assert_eq!(expired.get_crossuuid(), walked[0].get_crossuuid());
    assert_eq!(expired.get_crosscode(), "10:0:O-100");
    assert_derived(expired, "the expiry");
    assert_eq!(
        (walked[3].get_seqnum(), walked[3].get_prevuuid()),
        (0, None),
        "the later incarnation starts after expiry"
    );
}

#[test]
fn deadlines_precede_equal_time_sources_and_views_and_eof_drains_in_order() {
    let mut first = incarnation("O-100", 10);
    first.set_exprunix(Some(at(20)));
    let mut second = incarnation("O-900", 20);
    second.set_exprunix(Some(at(30)));
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
            (10, None, "10:0:O-100", false),
            (10, Some(10), "10:0:O-100", false),
            (20, None, "10:0:O-100", true),
            (20, None, "10:0:O-900", false),
            (20, Some(20), "10:0:O-900", false),
            (30, None, "10:0:O-900", true),
        ]
    );
}

#[test]
fn the_expirations_of_one_deadline_take_its_places_in_order() {
    // Two orders sharing one deadline and a third at that very instant: the
    // walk hands the expirations over first - in the order of the
    // identities they retire - each at the next place of the deadline, and
    // keeps the place the source element came with.
    let mut first = incarnation("O-100", 10);
    first.set_exprunix(Some(at(20)));
    let mut second = incarnation("O-200", 15);
    second.set_exprunix(Some(at(20)));
    let walked: Vec<_> =
        EventIterator::new([first, second, incarnation("O-900", 20)], true).collect();

    assert_eq!(
        walked[2..]
            .iter()
            .map(|event| (
                ms(event.get_currunix()),
                event.get_state().is_failed(),
                event.get_seqnum()
            ))
            .collect::<Vec<_>>(),
        [(20, true, 0), (20, true, 1), (20, false, 0)]
    );
    assert_eq!(walked[4].get_crosscode(), "10:0:O-900");
}

#[test]
fn eof_keeps_the_deadline_horizon_for_nonexpiring_neighbors() {
    let mut finite = incarnation("O-100", 10);
    finite.set_exprunix(Some(at(20)));
    let forever = incarnation("O-900", 10);
    let walked: Vec<_> = EventIterator::new([finite, forever], true)
        .with_snapshot_ns(10 * MS)
        .collect();

    assert!(
        walked.iter().any(|event| {
            event.get_crosscode() == "10:0:O-100"
                && event.get_currunix() == at(20)
                && event.get_state().is_failed()
        }),
        "the finite identity expires at the horizon"
    );
    assert_eq!(
        walked
            .iter()
            .filter(|event| event.get_snapunix().is_some() && event.get_currunix() == at(20))
            .map(|event| event.get_crosscode())
            .collect::<Vec<_>>(),
        ["10:0:O-900"],
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
    first.set_exprunix(Some(at(20)));
    let mut replacement = incarnation("O-100", 15);
    replacement.set_exprunix(Some(at(40)));
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
    later.set_exprunix(Some(at(40)));
    let mut shortened = incarnation("O-200", 15);
    shortened.set_exprunix(Some(at(20)));
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
    first.set_exprunix(Some(at(35)));
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
            event.get_snapunix().map(|original| {
                (
                    ms(event.get_currunix()),
                    event.get_crosscode(),
                    ms(original),
                )
            })
        })
        .collect();
    snapshots.sort_unstable();
    // A view is the live event as of its tick: dated at it, and keeping the
    // instant the event it copies was stated at.
    assert_eq!(
        snapshots,
        [
            (10, "10:0:O-100", 10),
            (20, "10:0:O-100", 10),
            (20, "10:0:O-900", 15),
            (30, "10:0:O-100", 25),
            (30, "10:0:O-900", 15),
        ]
    );
    for snapshot in walked.iter().filter(|event| event.get_snapunix().is_some()) {
        let tick = snapshot.get_currunix();
        let source = sources
            .iter()
            .rev()
            .find(|source| {
                source.get_crosscode() == snapshot.get_crosscode() && source.get_currunix() <= tick
            })
            .expect("the living source copied at this tick");
        // Its content, its place and its cross element are the live event's;
        // its identity is the one its tick derives, so it is a row of its own.
        assert_eq!(snapshot.get_snapunix(), Some(source.get_currunix()));
        assert_eq!(snapshot.get_currhashcode(), source.get_currhashcode());
        assert_eq!(snapshot.get_seqnum(), source.get_seqnum());
        assert_eq!(snapshot.get_prevuuid(), source.get_prevuuid());
        assert_eq!(snapshot.get_crossuuid(), source.get_crossuuid());
        assert_derived(snapshot, "the view");
        assert_eq!(
            snapshot.get_curruuid() == source.get_curruuid(),
            tick == source.get_currunix()
        );
        // Only instants moved, and they feed no code: the view is stamped
        // again over the live code, and is what settling a copy of it
        // answers, under its chain's cross element.
        let mut settled = snapshot.clone();
        settled.finalize();
        settled.set_crossuuid(snapshot.get_crossuuid());
        assert_eq!(*snapshot, settled, "the view at {}", ms(tick));
    }
    assert!(walked.iter().all(|event| {
        event
            .get_snapunix()
            .is_none_or(|tick| tick <= event.get_exprunix().unwrap_or(i64::MAX))
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
/// live event goes by - one of its chain identities, or the chain's first
/// value a lineage field names under its base - and the two maps that make
/// that lookup are the walk's own; settling and retiring an identity directly
/// is what pins that a name has one live holder and that retiring it forgets
/// exactly the records it opened.
#[cfg(feature = "internals")]
mod naming {
    use yggdryl::graph::{Element, Event, EventIterator, Operation, OrderEvent};

    use super::{CL_ORD_ID, EXEC_ID, ORDER_ID, anonymous, identifier, named as named_at};
    use yggdryl::internals::graph_iterator::{
        name_records, named_chains, named_identities, named_schemes, retire, settle,
    };
    use yggdryl::internals::logging_warning::count;
    use yggdryl::{IdType, State};

    fn named(cross: &str, unix: i64, scheme: &IdType, name: &str) -> OrderEvent {
        let mut event = OrderEvent::at(unix);
        event.set_crosscode(cross.to_owned());
        event
            .insert_identifier(identifier(scheme, name))
            .expect("a plain holder takes every key");
        event.finalize();
        event
    }

    #[test]
    fn retiring_the_last_name_removes_its_whole_index() {
        let event = named("A", 1, &CL_ORD_ID, "A-1");
        let identity = event.get_crossuuid();
        let mut walk = EventIterator::new(Vec::<OrderEvent>::new(), true);
        settle(&mut walk, identity, &event, event.get_curruuid());
        assert_eq!(named_schemes(&walk), 1);
        assert_eq!(named_identities(&walk), 1);

        assert!(retire(&mut walk, identity), "the live identity retires");
        assert_eq!(named_schemes(&walk), 0);
        assert_eq!(named_identities(&walk), 0);
    }

    /// A name has one live holder: the first chain that stated it keeps it
    /// however often another settles stating it too, and holds the one
    /// reverse record; it is released when that chain ends, and the next
    /// chain stating it files it.
    #[test]
    fn a_name_keeps_its_first_live_holder() {
        let first = named("A", 1, &CL_ORD_ID, "SHARED");
        let second = named("B", 2, &CL_ORD_ID, "SHARED");
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
                named_chains(&walk, &CL_ORD_ID, "SHARED"),
                [first_identity],
                "the first holder keeps the name"
            );
            assert_eq!(name_records(&walk), 1, "one holder, one reverse record");
        }

        assert!(
            retire(&mut walk, first_identity),
            "the first holder retires"
        );
        assert!(named_chains(&walk, &CL_ORD_ID, "SHARED").is_empty());
        assert_eq!(named_schemes(&walk), 0, "the index holds nothing for it");
        settle(&mut walk, second_identity, &second, second.get_curruuid());
        assert_eq!(
            named_chains(&walk, &CL_ORD_ID, "SHARED"),
            [second_identity],
            "released, the next chain stating it files it"
        );
        assert!(
            retire(&mut walk, second_identity),
            "the second holder retires"
        );
        assert_eq!(named_schemes(&walk), 0);
        assert_eq!(named_identities(&walk), 0);
    }

    /// A per-report reference - an `ExecID(17)`, a `TrdMatchID(880)` -
    /// names no chain, so two orders whose reports share one stay two
    /// chains, and a chain of many reports holds one record per identity
    /// type it goes by, never one per report.
    #[test]
    fn a_per_report_reference_names_no_chain() {
        let trdmatchid = IdType::TrdMatchId;
        let report = |ms: i64, order: &str, exec: &str| {
            anonymous(
                ms,
                &[
                    (ORDER_ID, order),
                    (EXEC_ID, exec),
                    (trdmatchid.clone(), "M1"),
                ],
            )
        };
        let arrived = vec![
            named_at("O-1", 10, &ORDER_ID, "A"),
            named_at("O-2", 11, &ORDER_ID, "B"),
            report(20, "A", "E1"),
            report(21, "B", "E1"),
        ];
        let mut walk = EventIterator::new(arrived, true);
        let walked: Vec<OrderEvent> = walk.by_ref().collect();
        assert_eq!(walked[2].get_prevuuid(), Some(walked[0].get_curruuid()));
        assert_eq!(
            walked[3].get_prevuuid(),
            Some(walked[1].get_curruuid()),
            "B's report joins B, never A by the execution they share"
        );
        assert_eq!(walked[3].get_crosscode(), "10:0:O-2");
        assert_eq!(walk.alive().count(), 2);
        assert!(named_chains(&walk, &EXEC_ID, "E1").is_empty());
        assert!(named_chains(&walk, &trdmatchid, "M1").is_empty());
        assert_eq!(
            named_chains(&walk, &ORDER_ID, "A"),
            [walked[0].get_crossuuid()]
        );

        // One order, a client order identifier and sixty-four reports each
        // stating an execution of its own: two records, whatever the count.
        let mut order = named_at("O-1", 0, &ORDER_ID, "A");
        order
            .insert_identifier(identifier(&CL_ORD_ID, "C"))
            .unwrap();
        order.finalize();
        let reports = (1..=64).map(|step| {
            let mut report = report(step, "A", &format!("E{step}"));
            report.set_state(State::PartiallyFilled);
            report.finalize();
            report
        });
        let mut walk = EventIterator::new(std::iter::once(order).chain(reports), true);
        assert_eq!(walk.by_ref().count(), 65);
        assert_eq!(walk.alive().count(), 1);
        assert_eq!(name_records(&walk), 2, "orderid and clordid, once each");
        assert!(named_chains(&walk, &EXEC_ID, "E64").is_empty());
    }

    /// A replace keeps the old value a living identity of the chain beside
    /// the new one, filed under the base, and files the lineage field's raw
    /// type nowhere.
    #[test]
    fn a_replace_files_both_values_under_the_base_and_no_lineage_slot() {
        let walked = super::replace_chain();
        let walk = &walked.1;
        let chain = walked.0[0].get_crossuuid();
        assert_eq!(named_chains(walk, &CL_ORD_ID, "C1"), [chain]);
        assert_eq!(named_chains(walk, &CL_ORD_ID, "C2"), [chain]);
        assert!(named_chains(walk, &IdType::OrigClOrdId, "C1").is_empty());
    }

    /// The conflict an element citing two live chains is: warned under its
    /// kind each time it is stated, and the conflicted statement files none
    /// of the names it shares with the chains it cited.
    #[test]
    fn a_conflict_is_warned_and_files_no_name_another_chain_holds() {
        let warned = || {
            count(
                "yggdryl::graph::iterator",
                "lifecycle element cites two live chains: it stands under its own identity",
                "ORDR",
            )
        };
        let before = warned();
        let (walked, walk) = super::conflicted(&[]);
        assert!(warned() > before, "the conflict is warned");
        let (first, second) = (walked[0].get_crossuuid(), walked[1].get_crossuuid());
        assert_eq!(named_chains(&walk, &CL_ORD_ID, "C1"), [first]);
        assert_eq!(
            named_chains(&walk, &IdType::SecondaryClOrdId, "S2"),
            [second]
        );
        assert_eq!(walk.alive().count(), 3, "the conflicted statement's own");
    }
}

/// The walk over the one value every boundary crosses as: a chain holds one
/// market data kind, so an execution under its order's cross code is a chain
/// of its own and never follows the order; its twin restates it and the
/// chain grows by nothing, and a value the walk does not chain - an undated
/// order - is yielded where it is read and changes nothing.
#[test]
fn a_market_data_walk_chains_within_one_kind_and_passes_the_rest_through() {
    use yggdryl::graph::{MarketData, Order};

    let fill = executed("O-500", 20);
    let arrived = vec![
        MarketData::from(incarnation("O-500", 10)),
        MarketData::from(Order::new()),
        MarketData::from(fill.clone()),
        MarketData::from(fill),
        MarketData::from(incarnation("O-500", 30)),
    ];
    let mut walk = EventIterator::new(arrived, true);
    let order = walk.next().expect("the order");
    let undated = walk.next().expect("the undated order, as it came");
    assert!(undated.as_order().is_some());
    let second = walk.next().expect("the fill");
    let leaf = second
        .as_execution_event()
        .expect("the fill keeps its kind");
    assert_eq!(
        (leaf.get_seqnum(), leaf.get_prevuuid()),
        (0, None),
        "the fill follows no order"
    );
    let twin = walk.next().expect("the fill, logged again");
    assert_eq!(twin, second, "the chain grows by nothing");
    let third = walk.next().expect("the order's next statement");
    assert_eq!(
        (
            third.as_order_event().unwrap().get_seqnum(),
            third.as_order_event().unwrap().get_prevuuid()
        ),
        (0, Some(order.get_curruuid()))
    );
    assert_eq!(walk.alive().count(), 2, "the order's chain and the fill's");

    // Across leaves: a FIX message following a typed order of its kind, and
    // a typed order stating no side following a FIX message, each stand
    // under the chain's side, stored cross code and cross element, whatever
    // following the other leaf answered.
    use yggdryl::graph::Market;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, Side};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let folder = yggdryl::local::LocalFolder::new(root).expect("the local seed path");
    let codec = FixCodec::new(std::sync::Arc::new(
        FixRegistry::from_handle(&folder).expect("the committed dictionary loads"),
    ));
    let message = |line: &str| -> MarketData {
        let message: FixMsg = codec
            .parse_line(line.as_bytes())
            .expect("a readable line")
            .next()
            .expect("one frame")
            .expect("the frame parses");
        MarketData::from(message)
    };
    let typed = sided("T-1", 10, Side::Buy, State::New, &[(CL_ORD_ID, "A1")]);
    let fix = message(
        "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20231114-22:13:20.020|11=A1|55=AAPL|54=1|38=100|10=0|",
    );
    assert_eq!(fix.get_crosscode(), "10:1:A1");
    let walked: Vec<MarketData> =
        EventIterator::new(vec![MarketData::from(typed), fix], true).collect();
    assert!(walked[1].as_fix().is_some(), "the message keeps its leaf");
    assert_eq!(walked[1].get_side(), Side::Buy);
    assert_eq!(
        walked[1].get_crosscode(),
        "10:1:T-1",
        "the typed chain's code"
    );
    assert_derived(&walked[1], "a FIX follower of a typed order");
    assert_eq!(walked[1].get_crossuuid(), walked[0].get_crossuuid());

    let fix = message(
        "8=FIX.4.4|35=D|49=S|56=T|34=1|52=20231114-22:13:20.010|11=B1|55=AAPL|54=1|38=100|10=0|",
    );
    let unsided = sided("Z-9", 20, Side::Unknown, State::New, &[(CL_ORD_ID, "B1")]);
    assert_eq!(unsided.get_crosscode(), "10:0:Z-9");
    let walked: Vec<MarketData> =
        EventIterator::new(vec![fix, MarketData::from(unsided)], true).collect();
    let follower = walked[1]
        .as_order_event()
        .expect("the order keeps its leaf");
    assert_eq!(follower.get_side(), Side::Buy, "the FIX chain's side");
    assert_eq!(follower.get_crosscode(), "10:1:B1", "the FIX chain's code");
    assert_derived(follower, "a typed follower of a FIX message");
    assert_eq!(follower.get_crossuuid(), walked[0].get_crossuuid());
    assert_eq!(
        follower.get_curruuid(),
        follower.time_uuid().expect("an identity")
    );
}

/// One event of `order` at `ms` stating `state`.
fn stating(order: &str, ms: i64, state: State) -> OrderEvent {
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.set_state(state);
    event.finalize();
    event
}

/// The states a walk over `states`, one event each under one chain, yields.
fn walked_states(states: &[State]) -> Vec<State> {
    let arrived: Vec<OrderEvent> = states
        .iter()
        .enumerate()
        .map(|(index, state)| stating("O-1", 10 * (index as i64 + 1), *state))
        .collect();
    EventIterator::new(arrived, true)
        .map(|event| *event.get_state())
        .collect()
}

/// A `NEW` stated over a live element that is new, carrying on or
/// restated is that element updated; the rule reads what was stated, before
/// the fold keeps the higher rank.
#[test]
fn a_new_over_a_new_like_live_element_walks_as_updated() {
    for held in [State::New, State::Active, State::Running, State::Replaced] {
        assert_eq!(
            walked_states(&[held, State::New]),
            [held, State::Updated],
            "{held:?}"
        );
    }
    // Updated moves the identity: the flipped state is digested.
    let arrived = vec![
        stating("O-1", 10, State::New),
        stating("O-1", 20, State::New),
    ];
    let stated = arrived[1].get_currhashcode();
    let walked: Vec<OrderEvent> = EventIterator::new(arrived, true).collect();
    assert_eq!(*walked[1].get_state(), State::Updated);
    assert_ne!(walked[1].get_currhashcode(), stated);
    assert_eq!(
        walked[1].get_curruuid(),
        walked[1].time_uuid().expect("an identity")
    );
    // A walk over the walked answers the same chain.
    let again: Vec<OrderEvent> = EventIterator::new(walked.clone(), true).collect();
    assert_eq!(again, walked);
}

/// Only a live element that is itself new-like makes a `NEW` an update:
/// a `NEW` answering a pending new stays new, a chain already updated stays
/// updated, and later progress folds over it as it does over working.
#[test]
fn updated_is_only_a_new_over_a_new_like_element_and_progress_folds_over_it() {
    assert_eq!(
        walked_states(&[State::PendingNew, State::New]),
        [State::PendingNew, State::New]
    );
    assert_eq!(
        walked_states(&[State::New, State::New, State::New]),
        [State::New, State::Updated, State::Updated]
    );
    assert_eq!(
        walked_states(&[State::New, State::New, State::PartiallyFilled]),
        [State::New, State::Updated, State::PartiallyFilled]
    );
}

/// A twin of the live element restates it: the restating branch never
/// reaches the rule, so a restated `NEW` stays `NEW`.
#[test]
fn a_restating_twin_of_a_new_element_stays_new() {
    let first = stating("O-1", 10, State::New);
    let twin = first.clone();
    let walked: Vec<OrderEvent> = EventIterator::new(vec![first, twin], true).collect();
    assert!(walked.iter().all(|event| *event.get_state() == State::New));
}

/// An event of `order` taking `side` at `ms` in `state`, going by `names`.
fn sided(
    order: &str,
    ms: i64,
    side: yggdryl::Side,
    state: State,
    names: &[(IdType, &str)],
) -> OrderEvent {
    use yggdryl::graph::Market;
    let mut event = OrderEvent::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.set_side(side, true);
    event.set_state(state);
    for (scheme, name) in names {
        event.insert_identifier(identifier(scheme, name)).unwrap();
    }
    event.finalize();
    event
}

/// A15: chains are keyed by side - a buy and a sell under one code are two
/// chains, a name is alive on each side apart - and an element stating no
/// side joins the one side alive under its code or its name, taking that
/// side; where both sides are alive it joins neither.
#[test]
fn a_buy_and_a_sell_under_one_code_are_two_chains_and_a_sideless_element_joins_the_one_live_side() {
    use yggdryl::Side;
    use yggdryl::graph::Market;

    let buy = sided("C-1", 10, Side::Buy, State::New, &[(CL_ORD_ID, "C-1")]);
    let sell = sided("C-1", 20, Side::Sell, State::New, &[(CL_ORD_ID, "C-1")]);
    assert_eq!(buy.get_crosscode(), "10:1:C-1");
    assert_eq!(sell.get_crosscode(), "10:2:C-1");
    let replaced = sided("C-1", 30, Side::Sell, State::Replaced, &[]);
    let unsided = sided("C-1", 40, Side::Unknown, State::Canceled, &[]);
    let walked: Vec<OrderEvent> =
        EventIterator::new(vec![buy.clone(), sell.clone(), replaced, unsided], true).collect();
    // The sell's replacement follows the sell, not the buy.
    assert_eq!(walked[2].get_prevuuid(), Some(walked[1].get_curruuid()));
    // Both sides are alive, so the side-less cancel joins neither.
    assert_eq!(walked[3].get_prevuuid(), None);
    assert_eq!(walked[3].get_side(), Side::Unknown);
    assert_eq!(walked[3].get_crosscode(), "10:0:C-1");

    // One side alive: a side-less element joins it by its base code and
    // takes its side, and so its code.
    let unsided = sided("C-1", 30, Side::Unknown, State::Canceled, &[]);
    let walked: Vec<OrderEvent> = EventIterator::new(vec![buy.clone(), unsided], true).collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_side(), Side::Buy);
    assert_eq!(walked[1].get_crosscode(), "10:1:C-1");

    // By a name too: a side-less element under no code of its own joins
    // the one live side going by it, a sided one only its own side.
    let mut nameless = anonymous(30, &[(CL_ORD_ID, "C-1")]);
    nameless.set_state(State::Canceled);
    nameless.finalize();
    let walked: Vec<OrderEvent> = EventIterator::new(vec![buy.clone(), nameless], true).collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    let other = sided(
        "S-9",
        30,
        Side::Sell,
        State::Canceled,
        &[(CL_ORD_ID, "C-1")],
    );
    let walked: Vec<OrderEvent> = EventIterator::new(vec![buy, other], true).collect();
    assert_eq!(
        walked[1].get_prevuuid(),
        None,
        "a sell never joins a buy by name"
    );
}

/// A12: an execution is a chain of its own - it joins no live order by a
/// name it shares with it, so a fill never ends its order's chain.
#[test]
fn an_execution_joins_no_chain_by_a_name() {
    use yggdryl::Side;
    use yggdryl::graph::{Market, MarketData};

    let order = sided("O-1", 10, Side::Buy, State::New, &[(ORDER_ID, "O-1")]);
    let mut fill = ExecutionEvent::at(at(20));
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::Buy, true);
    fill.set_state(State::Filled);
    fill.insert_identifier(identifier(&ORDER_ID, "O-1"))
        .unwrap();
    fill.finalize();
    let mut walk = EventIterator::new(vec![MarketData::from(order), MarketData::from(fill)], true);
    let _ = walk.next();
    let fill = walk.next().unwrap();
    let fill = fill.as_execution_event().expect("the fill keeps its kind");
    assert_eq!(fill.get_prevuuid(), None);
    assert_eq!(walk.alive().count(), 1, "the order is still alive");
}

/// A live execution going by its order's name keeps the order's name
/// record: a name is alive once per side and category, so a report stating
/// only that name still finds the order.
#[test]
fn a_live_execution_keeps_its_orders_name_record() {
    use yggdryl::Side;
    use yggdryl::graph::{Market, MarketData};

    let order = sided("O-1", 10, Side::Buy, State::New, &[(ORDER_ID, "O-1")]);
    let mut fill = ExecutionEvent::at(at(20));
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::Buy, true);
    fill.set_state(State::PartiallyFilled);
    fill.insert_identifier(identifier(&ORDER_ID, "O-1"))
        .unwrap();
    fill.finalize();
    let report = sided(
        "R-1",
        30,
        Side::Buy,
        State::PartiallyFilled,
        &[(ORDER_ID, "O-1")],
    );
    let walked: Vec<MarketData> = EventIterator::new(
        vec![
            MarketData::from(order),
            MarketData::from(fill),
            MarketData::from(report),
        ],
        true,
    )
    .collect();
    let order = walked[0].as_order_event().expect("the order");
    let fill = walked[1].as_execution_event().expect("the fill");
    let report = walked[2].as_order_event().expect("the report");
    assert_eq!(fill.get_prevuuid(), None, "the fill is a chain of its own");
    assert_eq!(report.get_prevuuid(), Some(order.get_curruuid()));
    assert_eq!(report.get_crosscode(), order.get_crosscode());
}

/// The stored cross codes of one base split the chains: a side-less order
/// is the base alone under side 0, and it joins the one side its base is
/// alive on - taking that side's stored code and cross identity - but
/// never one of two, and never the chain of another category.
#[test]
fn an_unsided_order_joins_the_one_live_side_of_its_base_under_the_stored_code() {
    use yggdryl::Side;

    let buy = sided("ORD-1", 10, Side::Buy, State::New, &[]);
    let sell = sided("ORD-1", 15, Side::Sell, State::New, &[]);
    let unsided = sided("ORD-1", 20, Side::Unknown, State::Canceled, &[]);
    assert_eq!(
        [
            buy.get_crosscode(),
            sell.get_crosscode(),
            unsided.get_crosscode()
        ],
        ["10:1:ORD-1", "10:2:ORD-1", "10:0:ORD-1"]
    );
    assert_ne!(buy.get_crossuuid(), unsided.get_crossuuid());

    // One side alive: the side-less order is that chain's next statement.
    let walked: Vec<OrderEvent> =
        EventIterator::new(vec![buy.clone(), unsided.clone()], true).collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_side(), Side::Buy);
    assert_eq!(walked[1].get_crosscode(), "10:1:ORD-1");
    assert_eq!(walked[1].get_crossuuid(), buy.get_crossuuid());
    let walked: Vec<OrderEvent> =
        EventIterator::new(vec![sell.clone(), unsided.clone()], true).collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(
        walked[1].get_crosscode(),
        "10:2:ORD-1",
        "the one live side, whichever it is"
    );
    assert_eq!(walked[1].get_crossuuid(), sell.get_crossuuid());

    // Both sides alive: it joins neither and stays under side 0.
    let walked: Vec<OrderEvent> =
        EventIterator::new(vec![buy.clone(), sell.clone(), unsided.clone()], true).collect();
    assert_eq!(walked[2].get_prevuuid(), None);
    assert_eq!(walked[2].get_crosscode(), "10:0:ORD-1");
    assert_eq!(walked[2].get_crossuuid(), unsided.get_crossuuid());

    // A side-less quote of the same base joins no order: the category is in
    // the stored code, so a quote and an order are two chains.
    let mut quote = yggdryl::graph::QuoteEvent::at(at(20));
    quote.set_crosscode("ORD-1".to_owned());
    quote.set_state(State::Canceled);
    quote.finalize();
    assert_eq!(quote.get_crosscode(), "14:0:ORD-1");
    let walked: Vec<yggdryl::graph::MarketData> = EventIterator::new(
        vec![
            yggdryl::graph::MarketData::from(buy),
            yggdryl::graph::MarketData::from(quote),
        ],
        true,
    )
    .collect();
    let quote = walked[1]
        .as_quote_event()
        .expect("the quote keeps its kind");
    assert_eq!(
        quote.get_prevuuid(),
        None,
        "an order's base is no quote's chain"
    );
    assert_eq!(quote.get_crosscode(), "14:0:ORD-1");
}

/// A quote is unsided: every statement of one quote stores `14:0:`
/// whatever side it tags, so a bid statement and an offer statement under
/// one code are one chain, each following the one before. A name a quote
/// goes by is alive on the side it tags - FIX scopes an `MDEntryID(278)` by
/// its `MDEntryType(269)` - so under two codes a bid and an offer going by
/// one name are two chains; a statement tagging no side joins the one
/// quote alive under the name, and a tagged one the untagged quote holding
/// both legs.
#[test]
fn one_quote_identifier_is_one_chain_whatever_side_its_statements_state() {
    use yggdryl::Side;
    use yggdryl::graph::QuoteEvent;

    let quote = |code: &str, ms: i64, side: Side, px: &str| {
        let mut quote = QuoteEvent::at(at(ms));
        quote.set_crosscode(code.to_owned());
        quote.set_side(side, true);
        quote.set_price(Some(px.parse().unwrap()), true);
        quote
            .insert_identifier(identifier(&IdType::QuoteId, "Q1"))
            .unwrap();
        quote.finalize();
        quote
    };
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            quote("Q-1", 10, Side::Buy, "99"),
            quote("Q-1", 20, Side::Sell, "101"),
        ],
        true,
    )
    .collect();
    assert_eq!(walked.len(), 2);
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_crossuuid(), walked[0].get_crossuuid());
    assert_eq!(
        (walked[0].get_crosscode(), walked[1].get_crosscode()),
        ("14:0:Q-1", "14:0:Q-1")
    );
    assert_eq!(
        walked[1].get_side(),
        Side::Sell,
        "the tag is the statement's"
    );

    // Two codes going by one quote identifier, tagging two sides: two
    // chains, each its own code.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            quote("BID-1", 10, Side::Buy, "99"),
            quote("ASK-1", 20, Side::Sell, "101"),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), None);
    assert_eq!(walked[1].get_crosscode(), "14:0:ASK-1");
    assert_ne!(walked[1].get_crossuuid(), walked[0].get_crossuuid());

    // A statement tagging no side joins the one quote alive under the
    // name, and neither where both sides are.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            quote("BID-1", 10, Side::Buy, "99"),
            quote("ANY-1", 20, Side::Unknown, "98"),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_crosscode(), "14:0:BID-1");
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            quote("BID-1", 10, Side::Buy, "99"),
            quote("ASK-1", 20, Side::Sell, "101"),
            quote("ANY-1", 30, Side::Unknown, "98"),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[2].get_prevuuid(), None);

    // A tagged statement continues the untagged quote going by its name.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            quote("TWO-1", 10, Side::Unknown, "99"),
            quote("FILL-1", 20, Side::Buy, "99"),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_crosscode(), "14:0:TWO-1");
}

/// A quote quoting both legs and tagging no side states both sides
/// (`BOTH`), and its names are alive as an untagged quote's are: a tagged
/// statement going by its name continues it, a two-sided statement joins
/// the one quote alive under its name, and joins neither where a bid and an
/// offer both go by it.
#[test]
fn a_quote_holding_both_sides_goes_by_its_names_as_an_untagged_one() {
    use yggdryl::Side;
    use yggdryl::graph::QuoteEvent;

    let tagged = |code: &str, ms: i64, side: Side, px: &str| {
        let mut quote = QuoteEvent::at(at(ms));
        quote.set_crosscode(code.to_owned());
        quote.set_side(side, true);
        quote.set_price(Some(px.parse().unwrap()), true);
        quote
            .insert_identifier(identifier(&IdType::QuoteId, "Q1"))
            .unwrap();
        quote.finalize();
        quote
    };
    let two_sided = |code: &str, ms: i64| {
        let mut quote = QuoteEvent::at(at(ms));
        quote.set_crosscode(code.to_owned());
        quote.set_bidpx(Some("99".parse().unwrap()), true);
        quote.set_askpx(Some("101".parse().unwrap()), true);
        quote
            .insert_identifier(identifier(&IdType::QuoteId, "Q1"))
            .unwrap();
        quote.finalize();
        quote
    };
    assert_eq!(two_sided("TWO-1", 10).get_side(), Side::Both);

    // A tagged statement continues the quote holding both sides.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            two_sided("TWO-1", 10),
            tagged("FILL-1", 20, Side::Buy, "99"),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_crosscode(), "14:0:TWO-1");
    assert_eq!(walked[1].get_side(), Side::Buy, "its tag is its own");

    // A two-sided statement joins the one quote alive under its name.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![tagged("BID-1", 10, Side::Buy, "99"), two_sided("TWO-1", 20)],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[1].get_crosscode(), "14:0:BID-1");
    assert_eq!(walked[1].get_side(), Side::Both);

    // And neither where a bid and an offer go by it.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            tagged("BID-1", 10, Side::Buy, "99"),
            tagged("ASK-1", 20, Side::Sell, "101"),
            two_sided("TWO-1", 30),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[2].get_prevuuid(), None);
    assert_eq!(walked[2].get_crosscode(), "14:0:TWO-1");
}

/// Elements split off one message - two entries of one batch going by the
/// batch's own identifier - are two entries: neither joins the other's
/// chain by the name they share.
#[test]
fn elements_split_off_one_message_never_join_one_another_by_a_name() {
    use yggdryl::graph::QuoteEvent;

    let batch = Uuid::from_v8(7);
    let entry = |code: &str, ms: i64, sources: Vec<Uuid>| {
        let mut quote = QuoteEvent::at(at(ms));
        quote.set_crosscode(code.to_owned());
        quote.set_bidpx(Some("99".parse().unwrap()), true);
        quote
            .insert_identifier(identifier(&IdType::QuoteId, "MQ1"))
            .unwrap();
        quote.set_srcuuids(sources);
        quote.finalize();
        quote
    };
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![entry("E-1", 10, vec![batch]), entry("E-2", 10, vec![batch])],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), None, "a sibling is no step");
    assert_eq!(walked[1].get_crosscode(), "14:0:E-2");
    // An element of another message going by the name joins by it.
    let walked: Vec<QuoteEvent> = EventIterator::new(
        vec![
            entry("E-1", 10, vec![batch]),
            entry("ACK-1", 20, vec![Uuid::from_v8(8)]),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
}

/// Two sides of one code stay two chains for as long as both live: each
/// statement follows the live one of its own side, under the stored code
/// and the cross identity that side owns.
#[test]
fn two_sides_of_one_code_stay_two_chains_for_as_long_as_both_live() {
    use yggdryl::Side;

    let arrived = vec![
        sided("ORD-1", 10, Side::Buy, State::New, &[]),
        sided("ORD-1", 20, Side::Sell, State::New, &[]),
        sided("ORD-1", 30, Side::Buy, State::PartiallyFilled, &[]),
        sided("ORD-1", 40, Side::Sell, State::PartiallyFilled, &[]),
        sided("ORD-1", 50, Side::Buy, State::Filled, &[]),
    ];
    let mut walk = EventIterator::new(arrived, true);
    let walked: Vec<OrderEvent> = walk.by_ref().collect();
    assert_eq!(walked[2].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[3].get_prevuuid(), Some(walked[1].get_curruuid()));
    assert_eq!(walked[4].get_prevuuid(), Some(walked[2].get_curruuid()));
    assert_ne!(walked[0].get_crossuuid(), walked[1].get_crossuuid());
    assert_eq!(walked[0].get_crossuuid(), walked[4].get_crossuuid());
    assert_eq!(walked[1].get_crossuuid(), walked[3].get_crossuuid());
    assert_eq!(
        walked
            .iter()
            .map(|event| event.get_crosscode())
            .collect::<Vec<_>>(),
        [
            "10:1:ORD-1",
            "10:2:ORD-1",
            "10:1:ORD-1",
            "10:2:ORD-1",
            "10:1:ORD-1"
        ]
    );
    // The buy filled and ended its chain; the sell is the one left alive.
    assert_eq!(alive(&walk), [("10:2:ORD-1".to_owned(), 40)]);
}

/// The previous-value slot of an order identifier: `parentorderid`, the
/// first of `orderid`'s two parents.
fn parent_order_id() -> IdType {
    "parentorderid".parse().expect("a type")
}

/// The first value of an order identifier's chain: `origorderid`, the last
/// of `orderid`'s two parents.
fn orig_order_id() -> IdType {
    "origorderid".parse().expect("a type")
}

/// An element joins a live chain through the chain's first value a lineage
/// field names: one stating `origorderid` A names the chain whose `orderid`
/// was first A, whatever `orderid` it states itself, and a value the chain
/// moved past still names it. The previous-value slot `parentorderid` names
/// no chain - the walk writes it from a value the chain holds, and a bridge
/// spells a hierarchy parent by it - so an element stating it beside an
/// `orderid` of its own starts afresh, while one stating it alone is what it
/// came from: finalizing fills its `orderid` from it. A parent naming no
/// live chain joins nothing.
#[test]
fn an_element_joins_a_live_chain_through_a_parent_identifiers_value() {
    // An order replaced under a new `orderid` that says where it began.
    let first = named("O-100", 10, &ORDER_ID, "A");
    let replacement = anonymous(20, &[(ORDER_ID, "B"), (orig_order_id(), "A")]);
    assert_ne!(replacement.get_crossuuid(), first.get_crossuuid());
    let restated = anonymous(30, &[(ORDER_ID, "B")]);
    let replaced_again = anonymous(40, &[(ORDER_ID, "C"), (orig_order_id(), "A")]);
    let cited = anonymous(50, &[(ORDER_ID, "A")]);
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![first.clone(), replacement, restated, replaced_again, cited],
        true,
    )
    .collect();
    assert!(
        walked
            .windows(2)
            .all(|pair| pair[1].get_prevuuid() == Some(pair[0].get_curruuid())),
        "one chain"
    );
    for (at, event) in walked.iter().enumerate() {
        assert_eq!(event.get_crosscode(), "10:0:O-100", "statement {at}");
        assert_derived(event, &format!("statement {at}"));
        assert_eq!(event.get_crossuuid(), first.get_crossuuid());
    }
    let held = |event: &OrderEvent, kind: &str| {
        event
            .get_identifiers()
            .get_from(&IdKey::base(kind.parse().expect("a type")))
            .map(str::to_owned)
    };
    assert_eq!(held(&walked[1], "orderid").as_deref(), Some("B"));
    assert_eq!(held(&walked[1], "parentorderid").as_deref(), Some("A"));
    assert_eq!(held(&walked[1], "origorderid").as_deref(), Some("A"));
    // The chain now goes by B too, so a statement under B alone follows,
    // with the parents the chain gave B.
    assert_eq!(held(&walked[2], "parentorderid").as_deref(), Some("A"));
    assert_eq!(held(&walked[2], "origorderid").as_deref(), Some("A"));
    // C names the chain's first value and joins by it: the chain A, B, C
    // ends with `parentorderid` B and `origorderid` A.
    assert_eq!(held(&walked[3], "orderid").as_deref(), Some("C"));
    assert_eq!(held(&walked[3], "parentorderid").as_deref(), Some("B"));
    assert_eq!(held(&walked[3], "origorderid").as_deref(), Some("A"));
    // And A, the value the chain moved past, still names it until it ends.
    assert_eq!(held(&walked[4], "orderid").as_deref(), Some("A"));

    // The previous-value slot beside an `orderid` of its own names nothing:
    // a hierarchy parent, a chain of its own.
    let child = anonymous(20, &[(ORDER_ID, "B"), (parent_order_id(), "A")]);
    let walked: Vec<OrderEvent> = EventIterator::new(vec![first.clone(), child], true).collect();
    assert_eq!(walked[1].get_prevuuid(), None);
    assert_eq!(walked[1].get_crosscode(), "");

    // An element stating only the slot is what it came from: finalizing
    // fills its `orderid` from it, and it joins the chain going by A.
    let only = anonymous(20, &[(parent_order_id(), "A")]);
    assert_eq!(held(&only, "orderid").as_deref(), Some("A"));
    let walked: Vec<OrderEvent> = EventIterator::new(vec![first.clone(), only], true).collect();
    assert_eq!(walked[1].get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(walked[1].get_crosscode(), "10:0:O-100");

    // A first value no live chain goes by joins nothing: a chain of its own.
    let stranger = anonymous(20, &[(ORDER_ID, "Y"), (orig_order_id(), "Z")]);
    let walked: Vec<OrderEvent> = EventIterator::new(vec![first, stranger], true).collect();
    assert_eq!(walked[1].get_prevuuid(), None);
    assert_eq!(walked[1].get_crosscode(), "");
}

/// An order placed under `ClOrdID` C1 and replaced under C2 - the replace
/// stating C1 as its `OrigClOrdID(41)` and a parse code of its own - then
/// reported under C2 alone, under C1 alone, and under the lineage field
/// alone, which finalizing makes C1 as well, walked: what it yielded and the
/// walk after it.
fn replace_chain() -> (
    Vec<OrderEvent>,
    EventIterator<OrderEvent, std::vec::IntoIter<OrderEvent>>,
) {
    let mut replace = named("O-200", 20, &CL_ORD_ID, "C2");
    replace
        .insert_identifier(identifier(&IdType::OrigClOrdId, "C1"))
        .unwrap();
    replace.finalize();
    let lineage = anonymous(50, &[(IdType::OrigClOrdId, "C1")]);
    assert_eq!(
        lineage.get_identifiers().get(&CL_ORD_ID),
        Some("C1"),
        "filled from its parent"
    );
    let arrived = vec![
        named("O-100", 10, &CL_ORD_ID, "C1"),
        replace,
        anonymous(30, &[(CL_ORD_ID, "C2")]),
        anonymous(40, &[(CL_ORD_ID, "C1")]),
        lineage,
    ];
    let mut walk = EventIterator::new(arrived, true);
    let walked = walk.by_ref().collect();
    (walked, walk)
}

/// R1 and R3 over a replace: the replace and every report after it - one
/// citing the new value, one the old, one the lineage field - are one chain
/// under the first statement's stored cross code, each deriving its cross
/// hash and cross element from it and its identity once under them.
#[test]
fn a_replace_under_a_new_identifier_keeps_the_chains_cross_code_and_hashes() {
    let (walked, walk) = replace_chain();
    assert_eq!(walked.len(), 5);
    for (at, event) in walked.iter().enumerate() {
        assert_eq!(event.get_crosscode(), "10:0:O-100", "statement {at}");
        assert_derived(event, &format!("statement {at}"));
        assert_eq!(
            event.get_curruuid(),
            event.time_uuid().expect("an identity"),
            "statement {at}"
        );
        if at > 0 {
            assert_eq!(
                event.get_prevuuid(),
                Some(walked[at - 1].get_curruuid()),
                "statement {at} follows the one before"
            );
        }
    }
    assert_eq!(walk.alive().count(), 1);
}

/// Chain A placed under `ClOrdID` C1, chain B under `SecondaryClOrdID` S2,
/// and an element under no live code citing both, then `after`, walked.
fn conflicted(
    after: &[OrderEvent],
) -> (
    Vec<OrderEvent>,
    EventIterator<OrderEvent, std::vec::IntoIter<OrderEvent>>,
) {
    let mut arrived = vec![
        named("O-1", 10, &CL_ORD_ID, "C1"),
        named("O-2", 20, &IdType::SecondaryClOrdId, "S2"),
        anonymous(30, &[(CL_ORD_ID, "C1"), (IdType::SecondaryClOrdId, "S2")]),
    ];
    arrived.extend_from_slice(after);
    let mut walk = EventIterator::new(arrived, true);
    let walked = walk.by_ref().collect();
    (walked, walk)
}

/// R2's stated conflict: an element citing two live chains joins neither by
/// a pick. It stands under its own identity - its own live chain where that
/// is one of the cited, a chain of its own otherwise - and another
/// statement of it resolves the same way, as a restatement. A side-less
/// element whose base both sides hold names neither by the base, and the
/// one name it states decides.
#[test]
fn an_element_citing_two_live_chains_is_a_stated_conflict_and_stands_under_its_own_identity() {
    use yggdryl::Side;

    // Under no live code of its own: a chain of its own beside the two.
    let (walked, walk) = conflicted(&[]);
    assert_eq!(walked[2].get_prevuuid(), None);
    assert_eq!(walked[2].get_crosscode(), "");
    assert_eq!(walk.alive().count(), 3);

    // Under its own live code, citing the other chain too: its own chain,
    // and a statement of it logged again restates it.
    let own = named("O-2", 40, &CL_ORD_ID, "C1");
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![
            named("O-1", 10, &CL_ORD_ID, "C1"),
            named("O-2", 20, &IdType::SecondaryClOrdId, "S2"),
            own.clone(),
            own,
        ],
        true,
    )
    .collect();
    assert_eq!(walked[2].get_prevuuid(), Some(walked[1].get_curruuid()));
    assert_eq!(walked[2].get_crosscode(), "10:0:O-2");
    assert_eq!(walked[3], walked[2], "the twin restates it");

    // A side-less element whose base both sides hold: its name decides.
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![
            sided("C-1", 10, Side::Buy, State::New, &[(CL_ORD_ID, "CB")]),
            sided("C-1", 20, Side::Sell, State::New, &[(CL_ORD_ID, "CS")]),
            sided(
                "C-1",
                30,
                Side::Unknown,
                State::Canceled,
                &[(CL_ORD_ID, "CB")],
            ),
        ],
        true,
    )
    .collect();
    assert_eq!(walked[2].get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(walked[2].get_side(), Side::Buy);
    assert_eq!(walked[2].get_crosscode(), "10:1:C-1");
}

/// Parentage along a walk: each event of one chain takes the parents of the
/// identifiers it states from the live statement it follows, so a chain of
/// `orderid` A, B, C, D ends with `parentorderid` C and `origorderid` A, and
/// a replace chain of `clordid` A, B, C with `origclordid` B.
#[test]
fn a_walk_carries_the_parents_of_each_identifier_along_its_chain() {
    let held = |event: &OrderEvent, kind: &str| {
        event
            .get_identifiers()
            .get_from(&IdKey::base(kind.parse().expect("a type")))
            .map(str::to_owned)
    };

    let orders: Vec<OrderEvent> = ["A", "B", "C", "D"]
        .iter()
        .enumerate()
        .map(|(index, value)| named("O-100", 10 * (index as i64 + 1), &ORDER_ID, value))
        .collect();
    let walked: Vec<OrderEvent> = EventIterator::new(orders, true).collect();
    let chain: Vec<[Option<String>; 3]> = walked
        .iter()
        .map(|event| ["orderid", "parentorderid", "origorderid"].map(|kind| held(event, kind)))
        .collect();
    let some = |value: &str| Some(value.to_owned());
    assert_eq!(
        chain,
        [
            [some("A"), None, None],
            [some("B"), some("A"), some("A")],
            [some("C"), some("B"), some("A")],
            [some("D"), some("C"), some("A")],
        ]
    );
    assert!(
        walked
            .windows(2)
            .all(|pair| pair[1].get_prevuuid() == Some(pair[0].get_curruuid())),
        "one chain"
    );

    // A restatement of the live value moves neither parent.
    let restated: Vec<OrderEvent> = EventIterator::new(
        vec![
            named("O-100", 10, &ORDER_ID, "A"),
            named("O-100", 20, &ORDER_ID, "B"),
            named("O-100", 30, &ORDER_ID, "B"),
        ],
        true,
    )
    .collect();
    assert_eq!(held(&restated[2], "parentorderid").as_deref(), Some("A"));
    assert_eq!(held(&restated[2], "origorderid").as_deref(), Some("A"));

    // A client order identifier has one parent: the previous value.
    let replaced: Vec<OrderEvent> = EventIterator::new(
        ["A", "B", "C"]
            .iter()
            .enumerate()
            .map(|(index, value)| named("O-100", 10 * (index as i64 + 1), &CL_ORD_ID, value))
            .collect::<Vec<_>>(),
        true,
    )
    .collect();
    assert_eq!(
        replaced
            .iter()
            .map(|event| held(event, "origclordid"))
            .collect::<Vec<_>>(),
        [None, some("A"), some("B")]
    );
    // `parentclordid` is a word of its own - a bridge's hierarchy parent -
    // and no parent the walk writes.
    assert_eq!(
        replaced
            .iter()
            .map(|event| held(event, "parentclordid"))
            .collect::<Vec<_>>(),
        [None, None, None]
    );
}

/// The expiration a walk emits reports no fill: it takes the live
/// element's facts and its cumulative fill, never its last one - the last
/// price, its FX parts and the last quantity are the fill the live element
/// reported, which the expiry did not.
#[test]
fn an_expiration_reports_no_fill() {
    let decimal = |text: &str| yggdryl::Decimal::parse(text).expect("a decimal");
    let mut order = incarnation("O-100", 10);
    order.set_state(State::PartiallyFilled);
    order.set_ordqty(Some(decimal("100")), true);
    order.set_cumqty(Some(decimal("40")), true);
    order.set_lastqty(Some(decimal("40")), true);
    order.set_spotrate(Some(decimal("1.25")), true);
    order.set_forwardpoints(Some(decimal("0.25")), true);
    order.set_exprunix(Some(at(20)));
    order.finalize();
    assert_eq!(order.get_lastpx(), Some(decimal("1.5")));
    assert_eq!(order.get_avgpx(), Some(decimal("1.5")));
    let walked: Vec<_> = EventIterator::new([order], true).collect();
    let expired = &walked[1];
    assert_eq!(*expired.get_state(), State::Expired);
    assert_eq!(
        (expired.get_lastpx(), expired.get_lastqty()),
        (None, None),
        "an expiry reports no fill"
    );
    assert_eq!(
        (expired.get_spotrate(), expired.get_forwardpoints()),
        (None, None),
        "nor the parts of a fill's price"
    );
    assert_eq!(expired.get_cumqty(), Some(decimal("40")));
    assert_eq!(expired.get_avgpx(), Some(decimal("1.5")));
    assert_eq!(
        (expired.get_leavesqty(), expired.get_cxlqty()),
        (Some(decimal("0")), Some(decimal("60")))
    );
}
