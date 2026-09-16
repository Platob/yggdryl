//! An element states its identity and its parents; a time element its
//! instant and its code. The traits are signatures, so what a caller can
//! rely on is that a value implementing them answers through them, including
//! as a trait object.

use yggdryl::graph::{Element, TimeElement};
use yggdryl::types::Uuid;

/// One event as a caller would hold it: every fact the two traits name, and
/// nothing the graph owns.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Event {
    uuid: Uuid,
    parents: Vec<Uuid>,
    unix: i128,
    hashcode: u64,
}

impl Event {
    fn new(uuid: u128) -> Self {
        Self {
            uuid: Uuid::from_v8(uuid),
            parents: Vec::new(),
            unix: 0,
            hashcode: 0,
        }
    }
}

impl Element for Event {
    fn uuid(&self) -> Uuid {
        self.uuid
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn parentuuids(&self) -> &[Uuid] {
        &self.parents
    }

    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.parents = parents;
    }
}

impl TimeElement for Event {
    fn unix(&self) -> i128 {
        self.unix
    }

    fn set_unix(&mut self, unix: i128) {
        self.unix = unix;
    }

    fn hashcode(&self) -> u64 {
        self.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }
}

#[test]
fn an_element_answers_the_identity_and_parents_it_was_given() {
    let mut event = Event::new(2);
    assert_eq!(event.uuid(), Uuid::from_v8(2));
    assert!(event.parentuuids().is_empty(), "a node naming no parent is a root");

    event.set_uuid(Uuid::from_v8(3));
    assert_eq!(event.uuid(), Uuid::from_v8(3));

    // The order is the element's own and comes back as stated.
    let parents = vec![Uuid::from_v8(9), Uuid::from_v8(1)];
    event.set_parentuuids(parents.clone());
    assert_eq!(event.parentuuids(), parents.as_slice());

    // An empty list makes it a root again.
    event.set_parentuuids(Vec::new());
    assert!(event.parentuuids().is_empty());
}

#[test]
fn a_time_element_answers_its_instant_and_code_and_is_still_an_element() {
    let mut event = Event::new(7);
    // Nanoseconds since the epoch, wider than any clock the crate reads.
    let unix = i128::from(i64::MAX) * 1_000;
    event.set_unix(unix);
    event.set_hashcode(0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(event.unix(), unix);
    assert_eq!(event.hashcode(), 0xDEAD_BEEF_CAFE_F00D);

    // Negative instants are before the epoch, and the type holds them.
    event.set_unix(-1);
    assert_eq!(event.unix(), -1);

    // One walk reads both traits through the subtrait object.
    event.set_parentuuids(vec![Uuid::from_v8(1)]);
    let held: &dyn TimeElement = &event;
    assert_eq!(held.uuid(), Uuid::from_v8(7));
    assert_eq!(held.parentuuids(), [Uuid::from_v8(1)]);
    assert_eq!(held.unix(), -1);
    assert_eq!(held.hashcode(), 0xDEAD_BEEF_CAFE_F00D);
}

#[test]
fn a_walk_over_elements_reaches_a_root_by_identity() {
    // A caller resolves parents by identity, so a graph is a map of them.
    let mut root = Event::new(1);
    root.set_unix(10);
    let mut child = Event::new(2);
    child.set_parentuuids(vec![root.uuid()]);
    child.set_unix(20);
    let mut leaf = Event::new(3);
    leaf.set_parentuuids(vec![child.uuid()]);
    leaf.set_unix(30);

    let held: Vec<Box<dyn TimeElement>> =
        vec![Box::new(root), Box::new(child), Box::new(leaf.clone())];
    let by_uuid = |uuid: Uuid| held.iter().find(|element| element.uuid() == uuid);

    let mut at: &dyn TimeElement = &leaf;
    let mut lineage = vec![at.unix()];
    while let Some(parent) = at.parentuuids().first().copied() {
        at = by_uuid(parent).expect("a parent is an element of the graph").as_ref();
        lineage.push(at.unix());
    }
    assert_eq!(lineage, [30, 20, 10]);
}
