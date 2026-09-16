# Graph

`yggdryl::graph` is the vocabulary an element of a graph answers about itself: `Element` is a node that knows its own `Uuid`, the identity it has in another graph, and the UUIDs of the elements it descends from; `TimeElement` is an element that also happened at one instant, stands in one lifecycle `State`, sits at one place in its chain and digests to one code, which is what an event is; `MarketElement` is one that happened in a market, with a price, a quantity, a side and the instrument's codes. The traits are signatures and two provided readings - following another element, merging with another statement of the same one - so a message, a chain entry and a lifecycle incarnation can each be an element without the graph owning any of them.

## Contract

| Key | Value |
| --- | --- |
| Owner | `graph::element` holds the three traits; no storage, no walk, no second identity type - the identity is [`Uuid`](types/uuid.md), the crate's own, and a timed element's derived identity is a [`TxHash`](hashing.md) |
| `Element` | `uuid()` / `set_uuid(Uuid)`; `xuuid() -> Option<Uuid>` / `set_xuuid(Option<Uuid>)`, the cross element - what this element is in another graph - where it has one; `parentuuids() -> &[Uuid]` / `set_parentuuids(Vec<Uuid>)`, the element's own ordered list, an empty one a root; `with_previous(self, &Self) -> Option<Self>`, this element stated as the one after another or nothing where it cannot follow it, which is the implementor's to say; `merge_with(self, &Self) -> Option<Self>`, provided: another statement of the same element folded in - the cross element where this one states none, the parents' union in this element's order then the other's - and nothing for another element |
| `TimeElement: Element` | `unix() -> i128` / `set_unix(i128)`, nanoseconds since the Unix epoch, UTC; `hashcode() -> u64` / `set_hashcode(u64)`, the XXH3-64 digest the element's content answers to, held beside it; `xhashcode() -> u64` / `set_xhashcode(u64)`, the cross element's; `state() -> &State` / `set_state(State)`, the ranked lifecycle [code](types/codes.md), never absent - `00UNKNOWN` where none was reached; `sequence_num() -> u64` / `set_sequence_num(u64)`, how many came before it in its chain; `creation_unix`, `expiration_unix`, `previous_unix` (`Option<i128>`) and `previous_uuid` (`Option<Uuid>`), each with its setter, stated only where known |
| `MarketElement: TimeElement` | `px() -> f64` / `set_px`, `currency() -> &Currency` / `set_currency`, `qty() -> f64` / `set_qty`, `unit() -> &str` / `set_unit(String)`, `side() -> &Side` / `set_side`; and the instrument as the market names it, each optional with its setter: `isincode() -> Option<&Isin>`, `cusipcode() -> Option<&Cusip>`, `sedolcode() -> Option<&Sedol>`, `bloombergcode() -> Option<&Bloomberg>`, `cficode() -> Option<&Cfi>` - the crate's own validated [codes](types/codes.md), so a spelling that is no identifier never reaches the element |
| Following | `TimeElement::following(self, &Self) -> Option<Self>` is provided: it records the predecessor's identity and instant, puts the element one place after the predecessor's in the chain (saturating), and carries the lifecycle forward - the earliest creation the two know, the latest expiration, the furthest state, each where only one states it that one's - while what the element itself says (its instant, its codes, its parents, its cross element) moves nowhere. Nothing for the element's own predecessor or for one that happened after it; an equal instant follows. An implementor's `with_previous` delegates to it, or states its own reading |
| Merging | `TimeElement::merging(self, &Self) -> Option<Self>` is provided: the element-level merge, then the later statement's instant and codes (the last word on what an element says is the latest one), the further place in the chain, the lifecycle folded as following folds it, and the predecessor this element names else the other's. `fold_lifecycle(&mut self, &Self)` is the shared fold. An implementor's `merge_with` delegates to it |
| Identity | `TimeElement::txhash() -> Result<TxHash>` couples `unix` at nanosecond resolution with `hashcode` as its digest; `time_uuid() -> Result<Uuid>` is the UUIDv8 that `TxHash` answers, the instant in front so identities sort by instant first as UUIDv7's do and by content second; `time_xuuid() -> Result<Option<Uuid>>` is the same coupling of `creation_unix` with `xhashcode`, nothing where the creation is unknown. An instant past signed 64-bit nanoseconds is `ArithmeticOverflow`, never a truncated identity |
| Parents | Named by identity, never by reference: an element can name a parent, a predecessor or a cross element it does not hold, and a caller resolves one through whatever holds the graph |
| Objects | Everything but `with_previous`, `merge_with` and the provided readings is object-safe: a walk over `dyn MarketElement` reads the identities, the parents, the instant, the state, the codes and the market's facts through one reference |
| Bindings | None: the traits state what a Rust value answers, and nothing crosses a boundary |

## Use

```rust
use yggdryl::graph::{Element, TimeElement};
use yggdryl::types::{State, Uuid};

struct Event {
    uuid: Uuid,
    xuuid: Option<Uuid>,
    parents: Vec<Uuid>,
    unix: i128,
    hashcode: u64,
    xhashcode: u64,
    state: State,
    sequence_num: u64,
    creation_unix: Option<i128>,
    expiration_unix: Option<i128>,
    previous_unix: Option<i128>,
    previous_uuid: Option<Uuid>,
}

impl Event {
    /// An event whose identity is when it happened and what it says.
    fn at(unix: i128, hashcode: u64) -> yggdryl::Result<Self> {
        let mut event = Self {
            uuid: Uuid::default(),
            xuuid: None,
            parents: Vec::new(),
            unix,
            hashcode,
            xhashcode: 0,
            state: State::from_spelling("New").expect("a shipped state"),
            sequence_num: 0,
            creation_unix: Some(unix),
            expiration_unix: None,
            previous_unix: None,
            previous_uuid: None,
        };
        event.uuid = event.time_uuid()?;
        event.xuuid = event.time_xuuid()?;
        Ok(event)
    }
}

impl Element for Event {
    fn uuid(&self) -> Uuid {
        self.uuid
    }
    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }
    fn xuuid(&self) -> Option<Uuid> {
        self.xuuid
    }
    fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
        self.xuuid = xuuid;
    }
    fn parentuuids(&self) -> &[Uuid] {
        &self.parents
    }
    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.parents = parents;
    }
    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following(previous)
    }
    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging(other)
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
    fn xhashcode(&self) -> u64 {
        self.xhashcode
    }
    fn set_xhashcode(&mut self, xhashcode: u64) {
        self.xhashcode = xhashcode;
    }
    fn state(&self) -> &State {
        &self.state
    }
    fn set_state(&mut self, state: State) {
        self.state = state;
    }
    fn sequence_num(&self) -> u64 {
        self.sequence_num
    }
    fn set_sequence_num(&mut self, sequence_num: u64) {
        self.sequence_num = sequence_num;
    }
    fn creation_unix(&self) -> Option<i128> {
        self.creation_unix
    }
    fn set_creation_unix(&mut self, unix: Option<i128>) {
        self.creation_unix = unix;
    }
    fn expiration_unix(&self) -> Option<i128> {
        self.expiration_unix
    }
    fn set_expiration_unix(&mut self, unix: Option<i128>) {
        self.expiration_unix = unix;
    }
    fn previous_unix(&self) -> Option<i128> {
        self.previous_unix
    }
    fn set_previous_unix(&mut self, unix: Option<i128>) {
        self.previous_unix = unix;
    }
    fn previous_uuid(&self) -> Option<Uuid> {
        self.previous_uuid
    }
    fn set_previous_uuid(&mut self, uuid: Option<Uuid>) {
        self.previous_uuid = uuid;
    }
}

let first = Event::at(10, 0xA)?;
let second = Event::at(20, 0xB)?.with_previous(&first).expect("a later event follows");
assert_eq!(second.previous_uuid(), Some(first.uuid()));
assert_eq!(second.previous_unix(), Some(10));
assert_eq!(second.sequence_num(), 1);
// Following carries the lifecycle forward: the earliest creation known.
assert_eq!(second.creation_unix(), Some(10));
// Identities sort by instant first, as a UUIDv7's do.
assert!(first.uuid() < second.uuid());
// An event follows neither itself nor one that happened after it.
assert!(Event::at(10, 0xA)?.with_previous(&first).is_none());
assert!(Event::at(5, 0xC)?.with_previous(&second).is_none());

// A second statement of the first event merges into it: the later instant
// and code have the last word, and another event does not merge at all.
let mut restated = Event::at(15, 0xD)?;
restated.set_uuid(first.uuid());
let merged = first.merge_with(&restated).expect("the same event");
assert_eq!((merged.unix(), merged.hashcode()), (15, 0xD));
assert!(merged.merge_with(&second).is_none());

// A caller resolves a predecessor by identity through whatever holds the graph.
let graph: Vec<&dyn TimeElement> = vec![&merged, &second];
let previous = graph
    .iter()
    .find(|element| Some(element.uuid()) == second.previous_uuid())
    .expect("the first event is in the graph");
assert_eq!(previous.unix(), 15);
assert!(previous.state().is_live());
```

## Edges

- `set_parentuuids(Vec::new())` makes an element a root again; the list is replaced whole, never merged. `set_xuuid(None)` unsays the cross element the same way.
- `following` refuses an element as its own predecessor and a predecessor that happened later; an equal instant follows, a later call replaces the predecessor recorded before, and a place past `u64::MAX` saturates.
- `merging` refuses another element outright, and folds the same element's later statement over the earlier: an equal instant keeps this element's codes.
- `unix` is signed: an instant before the epoch is negative, and the type holds every nanosecond count an `i64` clock can state, times a thousand and more. `txhash`, `time_uuid` and `time_xuuid` hold the 64-bit range a `TxHash` does and refuse the rest with `ArithmeticOverflow`.
- A state is never absent: an element that reached none says `00UNKNOWN`, the code that means exactly that. An instrument code is absent where the market names none, and never a spelling the code refuses.
- The traits validate nothing else: a parent or a predecessor that no element answers to is the holder's to refuse, not the element's.
