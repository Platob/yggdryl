# Graph

`yggdryl::graph` is the vocabulary an element of a graph answers about itself, and one walk over elements: `Element` is a node that knows its own `Uuid`, the identity it has in another graph, the names it goes by, the UUIDs of the elements it descends from and the order it stands in; `TimeElement` is an element that also happened at one instant, stands in one lifecycle `State`, sits at one place in its chain and digests to one code, which is what an event is; `MarketElement` is one that happened in a market, with a price, a quantity, a side and the instrument's codes. The traits are signatures and two provided readings - following another element, merging with another statement of the same one - so a message, a chain entry and a lifecycle incarnation can each be an element without the graph owning any of them. `ElementIterator` is the walk: timed elements read in their order, each stated as the one after the live element it follows.

## Contract

| Key | Value |
| --- | --- |
| Owner | `graph::element` holds the three traits and `graph::iterator` the one walk; no storage, no second identity type - the identity is [`Uuid`](types/uuid.md), the crate's own, and a timed element's derived identity is a [`TxHash`](hashing.md) |
| Names | Every accessor is `get_` and every mutator `set_`, so the traits claim no bare name: an implementor keeps its own `uuid()`, `state()` or `px()` for whatever it means by them, and the trait's reading is always the prefixed one |
| `Element` | `get_current_uuid()` / `set_current_uuid(Uuid)`; `get_crossuuid() -> Option<Uuid>` / `set_crossuuid(Option<Uuid>)`, the cross element - what this element is in another graph - where it has one; `get_identifiers() -> &BTreeMap<String, String>` / `set_identifiers(BTreeMap<String, String>)`, the names it goes by elsewhere, each under the scheme that issued it, replaced whole; `get_parentuuids() -> &[Uuid]` / `set_parentuuids(Vec<Uuid>)`, the element's own ordered list, an empty one a root; `is_after(&self, &Self) -> bool`, the order the implementor states - a timed element's instant, a node's lineage - and `is_before`, provided as its mirror, the two one strict weak order; `finalize(&mut self)`, the implementor's: recompute the hashing and reset the identity where it derives from the content, which every provided reading calls once it changed an element; `with_previous(self, &Self) -> Option<Self>`, this element stated as the one after another or nothing where it cannot follow it or following changes nothing, which is the implementor's to say; `merge_with(self, &Self) -> Option<Self>`, provided: another statement of the same element folded in - the cross element where this one states none, the identifiers this one lacks, the parents' union in this element's order then the other's - and nothing for another element or for a fold that changes nothing, so a caller skips what it already holds |
| `TimeElement: Element` | `get_unix() -> i128` / `set_unix(i128)`, nanoseconds since the Unix epoch, UTC; `get_hashcode() -> u64` / `set_hashcode(u64)`, the XXH3-64 digest the element's content answers to, held beside it; `get_xhashcode() -> u64` / `set_xhashcode(u64)`, the cross element's; `get_state() -> &State` / `set_state(State)`, the ranked lifecycle [code](types/codes.md), never absent - `00UNKNOWN` where none was reached; `get_sequence_num() -> u64` / `set_sequence_num(u64)`, how many came before it in its chain; `creation_unix`, `expiration_unix`, `previous_unix`, `snapshot_unix` (`Option<i128>`) and `previous_uuid` (`Option<Uuid>`), each `get_` with its `set_`, stated only where known - the snapshot instant being the grid step a walk read the element as the snapshot of |
| `MarketElement: TimeElement` | `get_px() -> Decimal` / `set_px`, `get_currency() -> &Currency` / `set_currency`, `get_qty() -> Decimal` / `set_qty`, `get_unit() -> &str` / `set_unit(String)`, `get_side() -> &Side` / `set_side` - the price and the quantity exact, as [`Decimal`](types/numeric.md#decimal) holds a market's numbers; two quote lanes, each optional with its setter: `get_bidpx() -> Option<Decimal>`, `get_bidcurrency() -> Option<&Currency>`, `get_bidqty() -> Option<Decimal>`, `get_bidunit() -> Option<&str>` for what the element would pay and the `ask` four for what it would be paid, and `fill_lanes(&mut self)`, provided, filling the lane the side implies - `Side::is_bid`, `Side::is_ask` - from the price, the currency, the quantity and the unit where the lane states nothing; and the instrument as the market names it, each optional with its setter: `get_isincode() -> Option<&Isin>`, `get_cusipcode() -> Option<&Cusip>`, `get_sedolcode() -> Option<&Sedol>`, `get_bloombergcode() -> Option<&Bloomberg>`, `get_cficode() -> Option<&Cfi>`, and the market it traded on, `get_miccode() -> Option<&Mic>` - the crate's own validated [codes](types/codes.md), so a spelling that is no identifier never reaches the element |
| Following | `TimeElement::following(self, &Self) -> Option<Self>` is provided: it records the predecessor's identity and instant, puts the element one place after the predecessor's in the chain (saturating), and carries the lifecycle forward - the earliest creation the two know, the latest expiration, the furthest state, each where only one states it that one's, and the names the predecessor went by that this element does not state - while what the element itself says (its instant, its codes, its parents, its cross element) moves nowhere. Nothing for the element's own predecessor, for one that happened after it, or where following changes nothing, because the element already follows it; an equal instant follows. An element that moved is finalized. An implementor's `with_previous` delegates to it, or states its own reading |
| Merging | `TimeElement::merging(self, &Self) -> Option<Self>` is provided: the element-level merge, then the later statement's instant and codes (the last word on what an element says is the latest one), the further place in the chain, the lifecycle folded as following folds it, and the predecessor and the snapshot instant this element states else the other's. `fold_lifecycle(&mut self, &Self)` is the shared fold, and the state it folds is the better of the two as [`CodeValue::merge_with`](types/codes.md) reads a state. `MarketElement::merging_market(self, &Self) -> Option<Self>` is the market's: the timed merge, then the later statement's price, quantity, unit and lane facts, and the currency, the side and each instrument code the better of the two statements - the later leading, the earlier filling what it leaves unknown, a code only one names that one's. Each answers nothing where the fold changes nothing, and finalizes the element where it did. An implementor's `merge_with` delegates to the one for its kind |
| Identity | `TimeElement::txhash() -> Result<TxHash>` couples `unix` at nanosecond resolution with `hashcode` as its digest; `time_uuid() -> Result<Uuid>` is the UUIDv7 that `TxHash` answers, the instant in front so identities sort by instant first, to the microsecond, and by content second; `time_crossuuid() -> Result<Option<Uuid>>` is the same coupling of `creation_unix` with `xhashcode`, nothing where the creation is unknown. An instant past signed 64-bit nanoseconds is `ArithmeticOverflow`, and one before the epoch or past the 48-bit millisecond count a UUIDv7 holds is `InvalidRecord`, never a truncated identity |
| Walking | `ElementIterator::new(elements, sorted: bool)` over `TimeElement + Clone` values, an `Iterator` of the same type: the walk keeps the elements still alive - a live state, not past their expiration - under the identity every incarnation of one thing shares, the cross element or the element's own where it has none, and an element arriving under an identity a live element holds is stated as the one after it by its own `with_previous`, which is what the walk yields; the yielded element then stands as the live one where it is still alive and retires the identity where it is not, so a filled order ends its chain and a later element under its identity starts afresh. `sorted` takes the caller's word that the elements arrive in their own order and streams them; otherwise the walk collects and stably sorts them by `is_after` / `is_before` first. An element before the live one is yielded as it came and changes nothing; one its own reading refuses is yielded as it came and stands. `with_snapshot_ns(i128)` gives the walk a grid - a step in nanoseconds aligned on the epoch, zero or less no grid - and it then reads one snapshot per step per identity: the first element to reach a step its identity has not consumed is stamped with the step's opening instant as its `snapshot_unix`, every later one in that step with none, and the steps consumed go with the identity when its chain ends; without a grid the instant is left as it came. `alive()` reads the live elements, in no order, and `snapshot_ns()` the grid where there is one |
| Finalizing | `TimeElement::finalized(&mut self, hashcode: u64)` is provided: it records the code and resets the current identity to `time_uuid`, keeping it where the instant has no UUIDv7; an implementor's `finalize` digests its content and hands the code to it, or does nothing where its identity is assigned |
| Digest | `Element::digest(&self) -> Xxh3` starts the digest of what an element states - the cross element, the names it goes by, its parents - `TimeElement::digest_timed` continues with the cross code, the state, the place in the chain and the predecessor's identity, and `MarketElement::digest_market` with the price, the currency, the quantity, the unit, the side, the codes and the lanes; each fed through the typed accessors and none of the instants, so an implementor's `finalize` feeds its own content behind the one for its kind and reads `as_u64` |
| Parents | Named by identity, never by reference: an element can name a parent, a predecessor or a cross element it does not hold, and a caller resolves one through whatever holds the graph |
| Objects | Everything but `is_after`, `is_before`, `with_previous`, `merge_with`, `fill_lanes` and the provided readings is object-safe: a walk over `dyn MarketElement` reads the identities, the names, the parents, the instant, the state, the codes and the market's facts through one reference |
| Bindings | None: the traits state what a Rust value answers, and nothing crosses a boundary |

## Use

```rust
use std::collections::BTreeMap;

use yggdryl::graph::{Element, ElementIterator, TimeElement};
use yggdryl::types::{State, Uuid};

#[derive(Clone)]
struct Event {
    uuid: Uuid,
    crossuuid: Option<Uuid>,
    identifiers: BTreeMap<String, String>,
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
    snapshot_unix: Option<i128>,
}

impl Event {
    /// An event whose identity is when it happened and what it says.
    fn at(unix: i128, hashcode: u64) -> yggdryl::Result<Self> {
        let mut event = Self {
            uuid: Uuid::default(),
            crossuuid: None,
            identifiers: BTreeMap::new(),
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
            snapshot_unix: None,
        };
        event.uuid = event.time_uuid()?;
        event.crossuuid = event.time_crossuuid()?;
        Ok(event)
    }
}

impl Element for Event {
    fn get_current_uuid(&self) -> Uuid {
        self.uuid
    }
    fn set_current_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }
    fn get_crossuuid(&self) -> Option<Uuid> {
        self.crossuuid
    }
    fn set_crossuuid(&mut self, crossuuid: Option<Uuid>) {
        self.crossuuid = crossuuid;
    }
    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        &self.identifiers
    }
    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.identifiers = identifiers;
    }
    fn get_parentuuids(&self) -> &[Uuid] {
        &self.parents
    }
    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.parents = parents;
    }
    fn is_after(&self, other: &Self) -> bool {
        self.unix > other.unix
    }
    // The identity is the instant and the content, so a followed or merged
    // event is finalized to the identity it now has.
    fn finalize(&mut self) {
        let hashcode = self.hashcode;
        self.finalized(hashcode);
    }
    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following(previous)
    }
    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging(other)
    }
}

impl TimeElement for Event {
    fn get_unix(&self) -> i128 {
        self.unix
    }
    fn set_unix(&mut self, unix: i128) {
        self.unix = unix;
    }
    fn get_hashcode(&self) -> u64 {
        self.hashcode
    }
    fn set_hashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }
    fn get_xhashcode(&self) -> u64 {
        self.xhashcode
    }
    fn set_xhashcode(&mut self, xhashcode: u64) {
        self.xhashcode = xhashcode;
    }
    fn get_state(&self) -> &State {
        &self.state
    }
    fn set_state(&mut self, state: State) {
        self.state = state;
    }
    fn get_sequence_num(&self) -> u64 {
        self.sequence_num
    }
    fn set_sequence_num(&mut self, sequence_num: u64) {
        self.sequence_num = sequence_num;
    }
    fn get_creation_unix(&self) -> Option<i128> {
        self.creation_unix
    }
    fn set_creation_unix(&mut self, unix: Option<i128>) {
        self.creation_unix = unix;
    }
    fn get_expiration_unix(&self) -> Option<i128> {
        self.expiration_unix
    }
    fn set_expiration_unix(&mut self, unix: Option<i128>) {
        self.expiration_unix = unix;
    }
    fn get_previous_unix(&self) -> Option<i128> {
        self.previous_unix
    }
    fn set_previous_unix(&mut self, unix: Option<i128>) {
        self.previous_unix = unix;
    }
    fn get_previous_uuid(&self) -> Option<Uuid> {
        self.previous_uuid
    }
    fn set_previous_uuid(&mut self, uuid: Option<Uuid>) {
        self.previous_uuid = uuid;
    }
    fn get_snapshot_unix(&self) -> Option<i128> {
        self.snapshot_unix
    }
    fn set_snapshot_unix(&mut self, unix: Option<i128>) {
        self.snapshot_unix = unix;
    }
}

let first = Event::at(10_000, 0xA)?;
let second = Event::at(20_000, 0xB)?;
assert!(second.is_after(&first) && first.is_before(&second));
let second = second.with_previous(&first).expect("a later event follows");
assert_eq!(second.get_previous_uuid(), Some(first.get_current_uuid()));
assert_eq!(second.get_previous_unix(), Some(10_000));
assert_eq!(second.get_sequence_num(), 1);
// Following carries the lifecycle forward: the earliest creation known.
assert_eq!(second.get_creation_unix(), Some(10_000));
// Identities are UUIDv7s: they sort by instant first, to the microsecond.
assert!(first.get_current_uuid() < second.get_current_uuid());
// An event follows neither itself nor one that happened after it.
assert!(Event::at(10_000, 0xA)?.with_previous(&first).is_none());
assert!(Event::at(5_000, 0xC)?.with_previous(&second).is_none());

// A caller resolves a predecessor by identity through whatever holds the graph.
let graph: Vec<&dyn TimeElement> = vec![&first, &second];
let previous = graph
    .iter()
    .find(|element| Some(element.get_current_uuid()) == second.get_previous_uuid())
    .expect("the first event is in the graph");
assert_eq!(previous.get_unix(), 10_000);
assert!(previous.get_state().is_live());

// A second statement of the first event merges into it: the later instant
// and code have the last word, another event does not merge at all, and a
// restatement that adds nothing answers nothing. The merged event was
// finalized, so its identity is the one its new instant and code derive.
let mut restated = Event::at(15_000, 0xD)?;
restated.set_current_uuid(first.get_current_uuid());
let merged = first.clone().merge_with(&restated).expect("the same event");
assert_eq!((merged.get_unix(), merged.get_hashcode()), (15_000, 0xD));
assert_eq!(merged.get_current_uuid(), merged.time_uuid()?);
assert_ne!(merged.get_current_uuid(), first.get_current_uuid());
assert!(merged.clone().merge_with(&second).is_none());
let same = first.clone();
assert!(first.merge_with(&same).is_none());

// The walk does the chaining: events of one thing share its cross identity,
// and each is stated as the one after the live event before it. Unsorted
// input is sorted by the events' own order first.
let mut later = Event::at(30_000, 0xC)?;
later.set_crossuuid(Some(Uuid::from_v8(7)));
let mut earlier = Event::at(25_000, 0xD)?;
earlier.set_crossuuid(Some(Uuid::from_v8(7)));
let chained: Vec<Event> = ElementIterator::new(vec![later, earlier], false).collect();
assert_eq!(chained[0].get_unix(), 25_000);
assert_eq!((chained[1].get_sequence_num(), chained[1].get_previous_unix()), (1, Some(25_000)));
```

## Edges

- `set_parentuuids(Vec::new())` makes an element a root again; the list is replaced whole, never merged. `set_crossuuid(None)` unsays the cross element the same way, and `set_identifiers` replaces the names whole.
- `is_after` and `is_before` are one order: an element is never after itself, and two neither after nor before each other are equal in it - which a sort of elements, the walk's included, treats as a tie and keeps in arrival order.
- `following` refuses an element as its own predecessor and a predecessor that happened later; an equal instant follows, a later call replaces the predecessor recorded before, and a place past `u64::MAX` saturates. Following what the element already follows answers nothing.
- `merging` refuses another element outright, and folds the same element's later statement over the earlier: an equal instant keeps this element's codes. A restatement that adds nothing answers nothing, so the earlier statement merged into the later one is a no-op.
- A lane already stated is never overwritten by `fill_lanes`, and a side that takes no lane - a cross, `OPPOSITE`, `UNKNOWN` - fills none.
- `unix` is signed: an instant before the epoch is negative, and the type holds every nanosecond count an `i64` clock can state, times a thousand and more. `txhash` holds the 64-bit range a `TxHash` does and refuses the rest with `ArithmeticOverflow`; `time_uuid` and `time_crossuuid` also refuse an instant a UUIDv7 cannot hold, before the epoch or past its 48-bit millisecond count.
- A state is never absent: an element that reached none says `00UNKNOWN`, the code that means exactly that. An instrument code is absent where the market names none, and never a spelling the code refuses.
- The traits validate nothing else: a parent or a predecessor that no element answers to is the holder's to refuse, not the element's.
- The walk clones an element twice at most - once to hold it as the live one, once to keep one its predecessor refuses - and holds one live element per identity beside the highest grid step it consumed, so a stream of one thing's incarnations costs one element of memory whatever its length.
- The grid floors: an instant before the epoch falls in the step opening below it, so a step of ten holds `-1` as the snapshot at `-10`.
