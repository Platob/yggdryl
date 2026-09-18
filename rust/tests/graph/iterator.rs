//! The one walk over events: each event chained to the live one under its
//! cross identity - or, where that is alive under nothing, under a name a
//! live event goes by - the alive set kept as the lifecycle moves, and the
//! caller's word on the order taken or the order made.

use std::collections::BTreeMap;

use yggdryl::graph::{Element, Event, EventIterator, MarketEventData};
use yggdryl::types::Uuid;

use super::element::filled;

/// One nanosecond count per millisecond: the instants below are spaced so
/// two of them never share the microsecond a derived identity opens with.
const MS: i64 = 1_000_000;

/// An instant a derived identity holds: `ms` milliseconds after one
/// evening in November 2023, UTC.
fn at(ms: i64) -> i64 {
    1_700_000_000_000_000_000 + ms * MS
}

/// The milliseconds an instant stands after [`at`]'s origin.
fn ms(unix: i64) -> i64 {
    (unix - at(0)) / MS
}

fn identifiers<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(scheme, identifier)| (scheme.to_owned(), identifier.to_owned()))
        .collect()
}

/// One event of the thing `order` identifies across its life, at `ms`.
fn incarnation(order: &str, ms: i64) -> MarketEventData {
    let mut event = MarketEventData::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.finalize();
    event
}

/// An event of `order` at `ms` going by one name, so two events at one
/// instant are two events.
fn named(order: &str, ms: i64, scheme: &str, name: &str) -> MarketEventData {
    let mut event = MarketEventData::at(at(ms));
    event.set_crosscode(order.to_owned());
    event.set_identifiers(identifiers([(scheme, name)]));
    event.finalize();
    event
}

/// Where each event the walk yields stands: its instant, its place, and the
/// instant of the predecessor it names, in milliseconds.
fn places(walk: impl Iterator<Item = MarketEventData>) -> Vec<(i64, u64, Option<i64>)> {
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
fn alive(
    walk: &EventIterator<MarketEventData, std::vec::IntoIter<MarketEventData>>,
) -> Vec<(String, i64)> {
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
    first.set_creatunix(Some(at(5)));
    first.set_identifiers(identifiers([("ClOrdID", "C-1")]));
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
    // and the identity re-derived around it.
    assert_eq!(second.get_creatunix(), Some(at(5)));
    assert_eq!(second.get_identifiers()["ClOrdID"], "C-1");
    assert_eq!(
        second.get_curruuid(),
        second.time_uuid().expect("an identity")
    );
    let third = walk.next().expect("the third incarnation");
    assert_eq!(third.get_prevuuid(), Some(second.get_curruuid()));
    assert_eq!(third.get_seqnum(), 2);
    assert_eq!(third.get_creatunix(), Some(at(5)));
    assert!(walk.next().is_none());
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
        named("O-100", 40, "ExecID", "E-5"),
        named("O-100", 40, "ExecID", "E-4"),
    ];
    let walk = EventIterator::new(arrived, false);
    let walked: Vec<MarketEventData> = walk.collect();
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
    assert_eq!(walked[3].get_identifiers()["ExecID"], "E-5");
    assert_eq!(walked[4].get_identifiers()["ExecID"], "E-4");
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
    expired.set_expirunix(Some(at(20)));
    let mut open = incarnation("O-100", 30);
    open.set_expirunix(Some(at(31)));
    let arrived = vec![
        incarnation("O-100", 10),
        expired,
        open,
        incarnation("O-100", 40),
    ];
    let walk = EventIterator::new(arrived, true);
    assert_eq!(
        places(walk),
        [
            (10, 0, None),
            (20, 1, Some(10)),
            (30, 0, None),
            (40, 1, Some(30)),
        ]
    );
}

#[test]
fn an_element_with_no_cross_identity_stands_under_its_own() {
    // Two elements that are nothing elsewhere follow nothing: each is its
    // own identity, and nothing arrives under it but a restatement.
    let mut first = MarketEventData::at(at(10));
    first.finalize();
    assert_eq!(first.get_crossuuid(), first.get_curruuid());
    let mut second = MarketEventData::at(at(20));
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

    // A twin that knows more adds what it knows to the live one's place.
    let mut named = incarnation("O-100", 20);
    named.set_identifiers(identifiers([("ExecID", "E-2")]));
    let arrived = vec![incarnation("O-100", 10), incarnation("O-100", 20), named];
    let mut walk = EventIterator::new(arrived, true);
    walk.next().expect("the order");
    let second = walk.next().expect("the fill");
    let twin = walk.next().expect("the fill, named");
    assert_eq!(twin.get_prevuuid(), second.get_prevuuid());
    assert_eq!(twin.get_identifiers()["ExecID"], "E-2");
    assert_ne!(twin.get_curruuid(), second.get_curruuid(), "it says more");
    assert_eq!(
        walk.alive().next().map(Element::get_curruuid),
        Some(twin.get_curruuid())
    );

    // On a grid, the twin holds the step the live one consumed: it is the
    // snapshot the live one was, and the step stays consumed.
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
        [(0, Some(10)), (0, Some(10)), (1, None), (2, Some(20))]
    );
}

#[test]
fn an_element_under_no_live_identity_follows_the_live_one_it_shares_a_name_with() {
    // An order placed under a `ClOrdID`, and a report of it that spells no
    // `OrderID` of its own: the report's cross element is its own identity,
    // alive under nothing, so the name it shares with the live order is
    // the chain it belongs to.
    let order = named("O-100", 10, "ClOrdID", "C-1");
    let mut report = MarketEventData::at(at(20));
    report.set_identifiers(identifiers([("ClOrdID", "C-1"), ("ExecID", "E-1")]));
    report.finalize();
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
    assert_eq!(live.get_identifiers()["ExecID"], "E-1");

    // A name no live element goes by starts a chain of its own, and an
    // element whose own identity is alive stays under it whatever names
    // it shares.
    let mut stranger = MarketEventData::at(at(30));
    stranger.set_identifiers(identifiers([("ClOrdID", "C-9")]));
    stranger.finalize();
    let other = named("O-900", 40, "ClOrdID", "C-1");
    let mut walk = EventIterator::new(
        vec![
            named("O-100", 10, "ClOrdID", "C-1"),
            named("O-900", 15, "ClOrdID", "C-2"),
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
    let mut fill = named("O-100", 20, "ClOrdID", "C-1");
    fill.set_state(filled());
    fill.finalize();
    let mut late = MarketEventData::at(at(30));
    late.set_identifiers(identifiers([("ClOrdID", "C-1")]));
    late.finalize();
    let mut walk = EventIterator::new(vec![named("O-100", 10, "ClOrdID", "C-1"), fill, late], true);
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
    assert_eq!(streamed.size_hint(), (2, Some(2)));
    streamed.next();
    assert_eq!(streamed.size_hint(), (1, Some(1)));
    let mut sorted = EventIterator::new(arrived(), false);
    assert_eq!(sorted.size_hint(), (2, Some(2)));
    assert_eq!(sorted.by_ref().count(), 2);
    assert_eq!(sorted.size_hint(), (0, Some(0)));
    assert!(sorted.next().is_none());
    assert!(sorted.next().is_none());
    // A walk over an empty source is alive to nothing.
    let mut empty = EventIterator::new(Vec::<MarketEventData>::new(), false);
    assert!(empty.next().is_none());
    assert_eq!(empty.alive().count(), 0);
}

#[test]
fn a_grid_reads_one_snapshot_per_step_per_identity() {
    let mut late = incarnation("O-100", 25);
    late.set_snapunix(Some(at(99)));
    let arrived = vec![
        incarnation("O-100", 10),
        incarnation("O-100", 12),
        incarnation("O-900", 12),
        incarnation("O-100", 20),
        late,
    ];
    let walk = EventIterator::new(arrived, true).with_snapshot_ns(10 * MS);
    assert_eq!(walk.snapshot_ns(), Some(10 * MS));
    let snapshots: Vec<(String, Option<i64>)> = walk
        .map(|event| {
            (
                event.get_crosscode().to_owned(),
                event.get_snapunix().map(ms),
            )
        })
        .collect();
    assert_eq!(
        snapshots,
        [
            // The first element in a step is its snapshot; a later one in
            // the same step is stamped with none, whatever it arrived with.
            ("O-100".to_owned(), Some(10)),
            ("O-100".to_owned(), None),
            // Another identity reads its own steps.
            ("O-900".to_owned(), Some(10)),
            ("O-100".to_owned(), Some(20)),
            ("O-100".to_owned(), None),
        ]
    );

    // The grid is aligned on the epoch and floors, so an instant before it
    // falls in the step opening below it.
    let mut before = MarketEventData::at(-1);
    before.set_crosscode("O-100".to_owned());
    before.finalize();
    let walk = EventIterator::new(vec![before], true).with_snapshot_ns(10);
    assert_eq!(
        walk.map(|event| event.get_snapunix()).collect::<Vec<_>>(),
        [Some(-10)]
    );

    // A chain that ended reads its snapshots afresh: the step it consumed
    // went with it.
    let mut done = incarnation("O-100", 12);
    done.set_state(filled());
    done.finalize();
    let arrived = vec![incarnation("O-100", 10), done, incarnation("O-100", 15)];
    let walk = EventIterator::new(arrived, true).with_snapshot_ns(10 * MS);
    assert_eq!(
        walk.map(|event| event.get_snapunix().map(ms))
            .collect::<Vec<_>>(),
        [Some(10), None, Some(10)]
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
