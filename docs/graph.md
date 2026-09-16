# Graph

`yggdryl::graph` is the vocabulary an element of a graph answers about itself: `Element` is a node that knows its own `Uuid` and the UUIDs of the elements it descends from, and `TimeElement` is an element that also happened at one instant and digests to one code, which is what an event is. The traits are signatures and nothing else, so a message, a chain entry and a lifecycle incarnation can each be an element without the graph owning any of them.

## Contract

| Key | Value |
| --- | --- |
| Owner | `graph::element` holds the two traits; no storage, no walk, no second identity type - the identity is [`Uuid`](types/uuid.md), the crate's own |
| `Element` | `uuid()` / `set_uuid(Uuid)`; `parentuuids() -> &[Uuid]` / `set_parentuuids(Vec<Uuid>)`; the parents are the element's own ordered list, and an empty one is a root |
| `TimeElement: Element` | `unix() -> i128` / `set_unix(i128)`, nanoseconds since the Unix epoch, UTC, wide enough for any clock the crate reads; `hashcode() -> u64` / `set_hashcode(u64)`, the digest the element's content answers to, held beside it rather than recomputed |
| Parents | Named by identity, never by reference: an element can name a parent it does not hold, and a caller resolves one through whatever holds the graph |
| Objects | Both traits are object-safe: a walk over `dyn TimeElement` reads the identity, the parents, the instant and the code through one reference |
| Bindings | None: the traits state what a Rust value answers, and nothing crosses a boundary |

## Use

```rust
use yggdryl::graph::{Element, TimeElement};
use yggdryl::types::Uuid;

struct Event {
    uuid: Uuid,
    parents: Vec<Uuid>,
    unix: i128,
    hashcode: u64,
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

let root = Event { uuid: Uuid::from_v8(1), parents: Vec::new(), unix: 10, hashcode: 0xA };
let mut child = Event { uuid: Uuid::from_v8(2), parents: Vec::new(), unix: 20, hashcode: 0xB };
assert!(child.parentuuids().is_empty(), "a node naming no parent is a root");
child.set_parentuuids(vec![root.uuid()]);

// A caller resolves a parent by identity through whatever holds the graph.
let graph: Vec<&dyn TimeElement> = vec![&root, &child];
let parent = graph
    .iter()
    .find(|element| element.uuid() == child.parentuuids()[0])
    .expect("the root is in the graph");
assert_eq!(parent.unix(), 10);
assert_eq!(parent.hashcode(), 0xA);
```

## Edges

- `set_parentuuids(Vec::new())` makes an element a root again; the list is replaced whole, never merged.
- `unix` is signed: an instant before the epoch is negative, and the type holds every nanosecond count an `i64` clock can state, times a thousand and more.
- The traits validate nothing: a parent that no element answers to is the holder's to refuse, not the element's.
