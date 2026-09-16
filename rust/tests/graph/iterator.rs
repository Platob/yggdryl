//! The one walk over timed elements: each element chained to the live one
//! under its identity, the alive set kept as the lifecycle moves, and the
//! caller's word on the order taken or the order made.

use std::collections::BTreeMap;

use yggdryl::graph::{Element, ElementIterator, TimeElement};
use yggdryl::types::Uuid;

use super::element::{Event, filled};

/// One event of the thing `order` identifies across its life.
fn incarnation(uuid: u128, order: u128, unix: i128) -> Event {
    let mut event = Event::at(uuid, unix);
    event.set_crossuuid(Some(Uuid::from_v8(order)));
    event
}

/// Where each element the walk yields stands: its identity, its place, and
/// the predecessor it names.
fn places(walk: impl Iterator<Item = Event>) -> Vec<(Uuid, u64, Option<Uuid>)> {
    walk.map(|event| {
        (
            event.get_current_uuid(),
            event.get_sequence_num(),
            event.get_previous_uuid(),
        )
    })
    .collect()
}

/// The identities alive after a walk, in one order.
fn alive(walk: &ElementIterator<Event, std::vec::IntoIter<Event>>) -> Vec<Uuid> {
    let mut alive = walk
        .alive()
        .map(Element::get_current_uuid)
        .collect::<Vec<_>>();
    alive.sort_unstable();
    alive
}

#[test]
fn a_sorted_walk_chains_each_element_to_the_live_one_under_its_identity() {
    let mut first = incarnation(1, 100, 10);
    first.set_creation_unix(Some(5));
    first.set_identifiers(BTreeMap::from([("ClOrdID".to_owned(), "C-1".to_owned())]));
    let arrived = vec![
        first,
        incarnation(9, 900, 15),
        incarnation(2, 100, 20),
        incarnation(3, 100, 30),
    ];
    let mut walk = ElementIterator::new(arrived, true);
    let first = walk.next().expect("the first");
    assert_eq!(
        (first.get_sequence_num(), first.get_previous_uuid()),
        (0, None)
    );
    let other = walk
        .next()
        .expect("another order's, under its own identity");
    assert_eq!(
        (other.get_sequence_num(), other.get_previous_uuid()),
        (0, None)
    );
    let second = walk.next().expect("the second incarnation");
    assert_eq!(second.get_previous_uuid(), Some(Uuid::from_v8(1)));
    assert_eq!(second.get_previous_unix(), Some(10));
    assert_eq!(second.get_sequence_num(), 1);
    // Enriched by the element's own reading: the lifecycle carried forward.
    assert_eq!(second.get_creation_unix(), Some(5));
    assert_eq!(second.get_identifiers()["ClOrdID"], "C-1");
    let third = walk.next().expect("the third incarnation");
    assert_eq!(third.get_previous_uuid(), Some(Uuid::from_v8(2)));
    assert_eq!(third.get_sequence_num(), 2);
    assert_eq!(third.get_creation_unix(), Some(5));
    assert!(walk.next().is_none());
    // Both orders are still alive, the latest incarnation of each.
    assert_eq!(alive(&walk), [Uuid::from_v8(3), Uuid::from_v8(9)]);
}

#[test]
fn an_unsorted_walk_sorts_by_the_elements_own_order_first_and_stably() {
    let arrived = vec![
        incarnation(3, 100, 30),
        incarnation(1, 100, 10),
        incarnation(2, 100, 20),
        // Two at one instant are neither after nor before each other, and
        // keep the order they arrived in.
        incarnation(5, 100, 40),
        incarnation(4, 100, 40),
    ];
    let walk = ElementIterator::new(arrived, false);
    assert_eq!(
        places(walk),
        [
            (Uuid::from_v8(1), 0, None),
            (Uuid::from_v8(2), 1, Some(Uuid::from_v8(1))),
            (Uuid::from_v8(3), 2, Some(Uuid::from_v8(2))),
            (Uuid::from_v8(5), 3, Some(Uuid::from_v8(3))),
            (Uuid::from_v8(4), 4, Some(Uuid::from_v8(5))),
        ]
    );
}

#[test]
fn an_element_that_ended_retires_its_identity_and_a_later_one_starts_afresh() {
    let mut done = incarnation(2, 100, 20);
    done.set_state(filled());
    let arrived = vec![incarnation(1, 100, 10), done, incarnation(3, 100, 30)];
    let mut walk = ElementIterator::new(arrived, true);
    walk.next().expect("the first");
    let done = walk.next().expect("the fill");
    assert_eq!(
        done.get_sequence_num(),
        1,
        "the ending element still follows"
    );
    assert_eq!(walk.alive().count(), 0, "and nothing is alive after it");
    let fresh = walk.next().expect("the late one");
    assert_eq!(
        (fresh.get_sequence_num(), fresh.get_previous_uuid()),
        (0, None)
    );
    assert_eq!(walk.alive().count(), 1);

    // An element past its expiration ended the same way; one that expires
    // later than it happened is still alive.
    let mut expired = incarnation(2, 100, 20);
    expired.set_expiration_unix(Some(20));
    let mut open = incarnation(3, 100, 30);
    open.set_expiration_unix(Some(31));
    let arrived = vec![
        incarnation(1, 100, 10),
        expired,
        open,
        incarnation(4, 100, 40),
    ];
    let walk = ElementIterator::new(arrived, true);
    assert_eq!(
        places(walk),
        [
            (Uuid::from_v8(1), 0, None),
            (Uuid::from_v8(2), 1, Some(Uuid::from_v8(1))),
            (Uuid::from_v8(3), 0, None),
            (Uuid::from_v8(4), 1, Some(Uuid::from_v8(3))),
        ]
    );
}

#[test]
fn an_element_with_no_cross_identity_stands_under_its_own() {
    // Two elements that are nothing elsewhere follow nothing: each is its
    // own identity, and nothing arrives under it but a restatement.
    let arrived = vec![Event::at(1, 10), Event::at(2, 20), Event::at(1, 30)];
    let mut walk = ElementIterator::new(arrived, true);
    walk.next().expect("the first");
    let second = walk.next().expect("the second");
    assert_eq!(
        (second.get_sequence_num(), second.get_previous_uuid()),
        (0, None)
    );
    // A restatement of the first arrives under its identity and is its own
    // predecessor, which the element's reading refuses: yielded as it came,
    // and standing as the live one.
    let restated = walk.next().expect("the restatement");
    assert_eq!(
        (restated.get_unix(), restated.get_previous_uuid()),
        (30, None)
    );
    assert_eq!(alive(&walk), [Uuid::from_v8(1), Uuid::from_v8(2)]);
    let live = walk
        .alive()
        .find(|event| event.get_current_uuid() == Uuid::from_v8(1))
        .expect("the restatement stands");
    assert_eq!(live.get_unix(), 30);
}

#[test]
fn an_element_before_the_live_one_is_yielded_as_it_came_and_changes_nothing() {
    // The caller called the walk sorted, and one element arrives late.
    let arrived = vec![
        incarnation(1, 100, 10),
        incarnation(3, 100, 30),
        incarnation(2, 100, 20),
        incarnation(4, 100, 40),
    ];
    let mut walk = ElementIterator::new(arrived, true);
    walk.next().expect("the first");
    let third = walk.next().expect("the third");
    assert_eq!(third.get_sequence_num(), 1);
    let stray = walk.next().expect("the late one");
    assert_eq!(
        (stray.get_sequence_num(), stray.get_previous_uuid()),
        (0, None)
    );
    // The live element is still the third, and the fourth follows it.
    let fourth = walk.next().expect("the fourth");
    assert_eq!(fourth.get_previous_uuid(), Some(Uuid::from_v8(3)));
    assert_eq!(fourth.get_sequence_num(), 2);
}

#[test]
fn the_walk_states_its_size_and_is_fused() {
    let arrived = || vec![incarnation(1, 100, 10), incarnation(2, 100, 20)];
    let mut streamed = ElementIterator::new(arrived(), true);
    assert_eq!(streamed.size_hint(), (2, Some(2)));
    streamed.next();
    assert_eq!(streamed.size_hint(), (1, Some(1)));
    let mut sorted = ElementIterator::new(arrived(), false);
    assert_eq!(sorted.size_hint(), (2, Some(2)));
    assert_eq!(sorted.by_ref().count(), 2);
    assert_eq!(sorted.size_hint(), (0, Some(0)));
    assert!(sorted.next().is_none());
    assert!(sorted.next().is_none());
    // A walk over an empty source is alive to nothing.
    let mut empty = ElementIterator::new(Vec::<Event>::new(), false);
    assert!(empty.next().is_none());
    assert_eq!(empty.alive().count(), 0);
}

#[test]
fn a_grid_reads_one_snapshot_per_step_per_identity() {
    let mut late = incarnation(4, 100, 25);
    late.set_snapshot_unix(Some(99));
    let arrived = vec![
        incarnation(1, 100, 10),
        incarnation(2, 100, 12),
        incarnation(9, 900, 12),
        incarnation(3, 100, 20),
        late,
    ];
    let walk = ElementIterator::new(arrived, true).with_snapshot_ns(10);
    assert_eq!(walk.snapshot_ns(), Some(10));
    let snapshots: Vec<(Uuid, Option<i128>)> = walk
        .map(|event| (event.get_current_uuid(), event.get_snapshot_unix()))
        .collect();
    assert_eq!(
        snapshots,
        [
            // The first element in a step is its snapshot; a later one in
            // the same step is stamped with none, whatever it arrived with.
            (Uuid::from_v8(1), Some(10)),
            (Uuid::from_v8(2), None),
            // Another identity reads its own steps.
            (Uuid::from_v8(9), Some(10)),
            (Uuid::from_v8(3), Some(20)),
            (Uuid::from_v8(4), None),
        ]
    );

    // The grid is aligned on the epoch and floors, so an instant before it
    // falls in the step opening below it.
    let walk = ElementIterator::new(vec![incarnation(1, 100, -1)], true).with_snapshot_ns(10);
    assert_eq!(
        walk.map(|event| event.get_snapshot_unix())
            .collect::<Vec<_>>(),
        [Some(-10)]
    );

    // A chain that ended reads its snapshots afresh: the step it consumed
    // went with it.
    let mut done = incarnation(2, 100, 12);
    done.set_state(filled());
    let arrived = vec![incarnation(1, 100, 10), done, incarnation(3, 100, 15)];
    let walk = ElementIterator::new(arrived, true).with_snapshot_ns(10);
    assert_eq!(
        walk.map(|event| event.get_snapshot_unix())
            .collect::<Vec<_>>(),
        [Some(10), None, Some(10)]
    );

    // Without a grid the snapshot instant is left as it came, and a step
    // of no width is no grid.
    let mut stamped = incarnation(1, 100, 10);
    stamped.set_snapshot_unix(Some(5));
    let walk = ElementIterator::new(vec![stamped], true).with_snapshot_ns(0);
    assert_eq!(walk.snapshot_ns(), None);
    assert_eq!(
        walk.map(|event| event.get_snapshot_unix())
            .collect::<Vec<_>>(),
        [Some(5)]
    );
}
