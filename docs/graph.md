# Graph

`yggdryl::graph` is the vocabulary an element of a graph answers about itself, two holders that answer it as plain fields, and one walk over elements. `Element` is a node that knows its own `Uuid`, the identity it has in another graph and the code that names it there, the codes it digests to, the names it goes by, the UUIDs of the elements it descends from, the UUIDs of the elements it was read from and the order it stands in; `Event` is an element that also happened at one instant, stands in one lifecycle `State` and sits at one place in its chain; `MarketElement` is one that stands in a market, with a price, a quantity, a side, two quote lanes and the instrument's codes; `MarketEvent` is every type that is both. The traits are signatures and provided readings - following another element, restating one, merging with another statement of the same one - so a message, a chain entry and a lifecycle incarnation can each be an element without the graph owning any of them; `MarketElementData` and `MarketEventData` hold the facts for a holder that wants nothing more. `EventIterator` is the walk: events read in their order, each stated as the one after the live element it follows.

## Contract

| Key | Value |
| --- | --- |
| Owner | `graph::element` holds the four traits, `graph::event` the two holders and `graph::iterator` the one walk; no storage, no second identity type - the identity is [`Uuid`](types/uuid.md), the crate's own, and an event's derived identity is a [`TxHash`](hashing.md) |
| Names | Every accessor is `get_` and every mutator `set_`, so the traits claim no bare name: an implementor keeps its own `uuid()`, `state()` or `px()` for whatever it means by them, and the trait's reading is always the prefixed one |
| `Element` | `get_curruuid()` / `set_curruuid(Uuid)`; `get_crossuuid() -> Uuid` / `set_crossuuid(Uuid)`, the cross element - what this element is in another graph - never absent, its own identity where it states no cross code, so every element stands in exactly one chain; `get_crosscode() -> &str` / `set_crosscode(String)`, the text naming it there - an order's `OrderID`, a quote's `QuoteID` - empty where none is stated; `get_currhashcode() -> u64` / `set_currhashcode(u64)`, the XXH3-64 digest the element's content answers to; `get_crosshashcode() -> u64` / `set_crosshashcode(u64)`, the XXH3-64 of the cross code, zero where none; `get_identifiers() -> &BTreeMap<String, String>` / `set_identifiers(..)`, the names it goes by elsewhere, each under the scheme that issued it, replaced whole; `get_parentuuids() -> &[Uuid]` / `set_parentuuids(Vec<Uuid>)`, the element's own ordered list, an empty one a root; `get_srcuuids() -> &[Uuid]` / `set_srcuuids(Vec<Uuid>)`, the elements this one was read from - a message's [text line](media/text/lines.md#lines) - provenance and never lineage, so no provided reading carries it along a chain; `lineage(&self) -> Vec<Uuid>`, provided: its parents then itself, oldest first, each once, what an element following it descends from; `is_after(&self, &Self) -> bool`, the order the implementor states - an event's instant, a node's lineage - and `is_before`, provided as its mirror, the two one strict weak order; `finalize(&mut self)`, the implementor's: bring the cross codes in step, digest the content and record the code, resetting the identity where it derives from the content; `with_previous(self, &Self) -> Option<Self>`, this element stated as the one after another or nothing where it cannot follow it or following changes nothing, the implementor's to say; `merge_with(self, &Self) -> Option<Self>`, provided: another statement of the same element folded in - the cross code, the names, the parents and the sources it lacks, the two lists each the union in this element's order then the other's |
| Cross | `sync_cross(&mut self) -> bool` and `cross_uuid(&self) -> Uuid` are provided: the cross hash code is the cross code's digest, zero where none, and the cross element is RFC 9562 UUIDv8 over that code where it is not zero, else the element's own identity; every reading that takes a cross code from another element calls `sync_cross`, and an implementor's `finalize` calls it before digesting, so every element sharing a cross code shares the cross identity whichever holds them |
| `Event: Element` | `get_currunix() -> i64` / `set_currunix(i64)`, nanoseconds since the Unix epoch, UTC; `get_state() -> &State` / `set_state(State)`, the ranked lifecycle [code](types/codes/state.md), never absent - `00UNKNOWN` where none was reached; `get_seqnum() -> u64` / `set_seqnum(u64)`, how many came before it in its chain; `creaunix`, `exprtime`, `prevunix`, `snapunix` (`Option<i64>`) and `prevuuid` (`Option<Uuid>`), each `get_` with its `set_`, stated only where known - the snapshot instant being the grid step a walk read the element as the snapshot of |
| `MarketElement: Element` | `get_px() -> Decimal18` / `set_px`, `get_currency() -> &Currency` / `set_currency`, `get_qty() -> Decimal18` / `set_qty`, `get_unit() -> &str` / `set_unit(String)`, `get_side() -> &Side` / `set_side` - the price and the quantity exact, as [`Decimal18`](types/numeric/decimal.md#decimal18) holds a market's numbers; two quote lanes, each optional with its setter: `get_bidpx() -> Option<Decimal18>`, `get_bidcurrency() -> Option<&Currency>`, `get_bidqty() -> Option<Decimal18>`, `get_bidunit() -> Option<&str>` for what the element would pay and the `ask` four for what it would be paid, and `fill_lanes(&mut self)`, provided, filling the lane the side implies - `Side::is_bid`, `Side::is_ask` - from the price, the currency, the quantity and the unit where the lane states nothing and the fact is stated at all; and the instrument as the market names it, each optional with its setter: `get_isincode() -> Option<&IsinCode>`, `get_cusipcode() -> Option<&CusipCode>`, `get_sedolcode() -> Option<&SedolCode>`, `get_bloombergcode() -> Option<&BloombergCode>`, `get_figicode() -> Option<&FIGICode>`, `get_cficode() -> Option<&CfiCode>`, and the market it traded on, `get_miccode() -> Option<&MicCode>` - the crate's own validated [codes](types/codes/index.md), so a spelling that is no identifier never reaches the element |
| `MarketEvent: Event + MarketElement` | a blanket over every type that is both, with nothing to implement: `digest_market_event` and `merging_market_event`, the readings that need both |
| Following | `Event::following(self, &Self) -> Option<Self>` is provided: it records the predecessor's identity and instant, puts the element one place after the predecessor's in the chain (saturating), forces the predecessor's cross code onto it where its own differs - two events of one chain share it - with the cross codes brought in step, takes the names the predecessor went by that this element does not state, descends from the predecessor's whole `lineage` - its parents become the predecessor's parents then the predecessor, oldest first, each once - and carries the lifecycle forward: the earliest creation the two know, this event's explicit expiration where it states one else the predecessor's, and the furthest state. A newer event may therefore shorten a deadline. What the element itself says - its instant, its sources, the snapshot it is - moves nowhere. Nothing for the element's own predecessor, for one that happened after it, or where following changes nothing; an equal instant follows. An element that moved is finalized. An implementor's `with_previous` delegates to it, or states its own reading |
| Restating | `Event::restating(self, live: &Self) -> Self` is provided: this event as another statement of `live` - the same instant and content read a second time, as a capture logs one message at every hop it passes - taking the place `live` holds in its chain: its predecessor, position and snapshot, the chain's cross code, the names and parents it knows - never its sources, which say what each statement was read from - the lifecycle folded, then finalized, so the two statements finalize to one identity and the chain grows by nothing. The caller establishes the twin by the identity `live` arrived under, because once `live` has followed something its identity has moved and the event alone cannot tell a twin from a successor; the walk records that identity and does exactly this |
| Merging | `Event::merging(self, &Self) -> Option<Self>` is provided: the element-level merge, then the later statement's instant and code (the last word on what an element says is the latest one), the further place in the chain, the lifecycle folded, and the predecessor and the snapshot instant this element states else the other's. `fold_lifecycle(&mut self, &Self)` is the same-event fold: earliest creation, latest expiration and the better state as [`CodeValue::merge_with`](types/codes/index.md#the-code-family-value) reads it. Following preserves its newer event's explicit expiration separately; merging two statements of one event keeps the latest expiration either knows. `MarketElement::merging_market` is the market's, with no instant to say which is later: this element's price, quantity, unit and lane facts stand, and the currency, the side and each instrument code are the better of the two, this element leading. `MarketEvent::merging_market_event` is the timed merge, then the later statement's price, quantity, unit and lane facts, and each code the better of the two with the later statement leading and the earlier filling what it leaves unknown. Each answers nothing where the fold changes nothing, and finalizes the element where it did. An implementor's `merge_with` delegates to the one for its kind |
| Identity | `Event::txhash() -> Result<TxHash>` couples `currunix` at nanosecond resolution with `currhashcode` as its digest; `time_uuid() -> Result<Uuid>` is the UUIDv7 that `TxHash` answers, the instant in front so identities sort by instant first, to the microsecond, and by content second. An instant before the epoch or past the 48-bit millisecond count a UUIDv7 holds is `InvalidRecord`, never a truncated identity |
| Finalizing | `Event::finalized(&mut self, hashcode: u64)` is provided: it records the code, resets the current identity to `time_uuid` - kept where the instant has no UUIDv7 - and the cross element to `cross_uuid` over it; an implementor's `finalize` digests its content and hands the code to it, or keeps an assigned identity |
| Digest | `Element::digest(&self) -> Xxh3` starts the digest of what an element states - the cross code, the names it goes by, its parents - and never an identity, an instant or its sources, `Event::digest_event` continues with the state, the place in the chain and the predecessor's identity, `MarketElement::digest_market` with the price, the currency, the quantity, the unit, the side, the codes and the lanes, and `MarketEvent::digest_market_event` with both; each fed through the typed accessors and none of the instants, so an implementor's `finalize` feeds its own content behind the one for its kind and reads `as_u64` |
| Holders | `MarketElementData` holds every fact `Element` and `MarketElement` name, no instant; `Default` states nothing - a price and a quantity of nothing in no currency (`XXX`), no unit, a side of `UNKNOWN`, no instrument, no lane - `finalize` digests through `digest_market` and sets the identity UUIDv8 over the code, its order is lineage and it follows another by descending from it. `MarketEventData` holds every fact the three traits name; `at(unix)` dates one stating nothing else, `Default` is the epoch, `finalize` digests through `digest_market_event` and hands the code to `finalized`, its order is its instant, `with_previous` is `following` and `merge_with` is `merging_market_event`. `From<&E>` reads either out of any element of its kind through the shared signatures, and the two convert into each other: an event drops its instants, an element lands at the epoch for the caller to date |
| Walking | `EventIterator::new(elements, sorted: bool)` over `Event + Clone` values, an `Iterator` of the same type: the walk keeps the elements still alive - a live state, not past their expiration - under the identity every incarnation of one thing shares, the cross element, and an element arriving under a live identity - or, where its own is alive under nothing, under a name a live element goes by, so a report spelling only the `ClOrdID` a live order was placed under still finds it - is stated as the one after it by its own `with_previous`, which is what the walk yields; the yielded element then stands as the live one where it is still alive and retires the identity where it is not. An element arriving under the identity the live element *arrived* under is its twin, and is yielded `restating` it. `sorted` takes the caller's word that the elements arrive in their own order and streams them; otherwise the walk collects and stably sorts them by `is_after` / `is_before` first. A finite deadline emits one owned `95EXPIRED` event at that exact instant, following the live generation, then purges it; stale deadlines of replaced or terminal generations emit nothing. Deadlines win ties, then every source event at that instant is processed, then grid views are emitted. `with_snapshot_ns(i64)` gives the walk an epoch-aligned grid - zero or less means none - and yields one owned view of every living identity at each crossed tick; a view changes only `snapunix`, keeping the source identity, sequence and content, and no source event is backdated to a prior tick. At EOF the walk drains finite deadlines and views through the greatest deadline still reached, or through the last source instant where none remains; a nonexpiring identity never extends a finite source. `alive()` reads the live elements, in no order, and `snapshot_ns()` the grid where there is one |
| Parents | Named by identity, never by reference: an element can name a parent, a predecessor or a cross element it does not hold, and a caller resolves one through whatever holds the graph |
| Objects | Everything but `is_after`, `is_before`, `with_previous`, `merge_with`, `following`, `restating`, `merging`, `fold_lifecycle`, `fill_lanes` and the market merges is object-safe: a walk over `dyn MarketEvent` reads the identities, the codes, the names, the parents, the instant, the state and the market's facts through one reference |
| Bindings | None: the traits state what a Rust value answers, and nothing crosses a boundary; a [FIX message](fix/message.md) implements all four, and a [text line](media/text/lines.md#lines) is an `Event` - the one a message is read from, which the message states as its source |

## Columns

`EventColumn` is the sixteen columns every generated schema of an event opens with, one per fact `Element` and `Event` answer, in `EventColumn::ALL`'s order: when it happened - `currunix`, `creaunix`, `exprtime`, `prevunix`, `snapunix` - which event it is - `curruuid`, `crossuuid`, `crosscode`, `currhashcode`, `crosshashcode`, `prevuuid`, `seqnum`, `parentuuids`, `srcuuids`, `identifiers` - and last the `state` it reached. Each answers `name()`, `display()`, `description()`, `datatype()` - nanosecond UTC clocks, the crate's own [`uuid`](types/uuid.md), `uint64` codes, `list<uuid>` lists whose item is named by the fact (`parentuuid`, `srcuuid`), a sorted `map<utf8, utf8>` of names, a [`state`](types/codes/state.md#the-rank-leads) code - and `nullable()`: the instant, the two identities and the two codes are never null; every other column is where the event states nothing - an empty code, list or map, a place of zero, an absent instant - and the state, which an event always answers, `00UNKNOWN` where nothing states one, admits a null because a state has no neutral member for an empty cell to read as. `field()` and `fields()` are the same as fields; `fact(&event)` is what an event states under a column, nothing where it states no fact, and `record(&mut event, &cell)` states a cell back through the traits, a null clearing the fact; `of_name` finds a column by its name whatever the case.

A [text line](media/text/index.md#row-schema)'s batch, a [FIX row](fix/capture.md#the-crates-own-columns) and a [chained message](fix/lifecycle.md#a-chain-carries-its-creation-and-its-history) open with them under one name and one datatype each - the FIX row through the crate's own fields, each taking its column's datatype - so the three join without a mapping: a message's `srcuuids` are the `curruuid` of the lines it was parsed out of, and a chained message's `prevuuid` and `parentuuids` are the `curruuid` of the messages before it. A line read back out of its batch and a message read back out of its row restate every one of the sixteen, so the identity a later step named survives the round trip.

## Use

```rust
use std::collections::BTreeMap;

use yggdryl::graph::{Element, Event, EventIterator, MarketElement, MarketEvent, MarketEventData};
use yggdryl::{Currency, Decimal18, Side, State};

// An order's life as events: each states its instant, the identifier the
// venue gave the order - its cross code - and where it stands.
let event = |unix: i64, order: &str, state: &str| -> yggdryl::Result<MarketEventData> {
    let mut event = MarketEventData::at(unix);
    event.set_crosscode(order.to_owned());
    event.set_state(State::from_spelling(state).expect("a shipped state"));
    event.set_px("82.5".parse()?);
    event.set_qty(Decimal18::from_int(1_000));
    event.set_currency(Currency::new("USD")?);
    event.set_side(Side::read("1")?);
    event.set_identifiers(BTreeMap::from([("OrderID".to_owned(), order.to_owned())]));
    event.finalize();
    Ok(event)
};
let first = event(10_000, "O-100", "New")?;
// Finalized, the identity is what the event states and when: the UUIDv7 its
// instant and its code derive, and the cross identity the cross code's.
assert_eq!(first.get_curruuid(), first.time_uuid()?);
assert_ne!(first.get_currhashcode(), 0);
assert_eq!(first.get_crossuuid(), first.cross_uuid());
assert_ne!(first.get_crossuuid(), first.get_curruuid(), "a cross code names a chain of its own");
// A buy is a bid: the lane the side implies fills from the event's own facts.
let mut quoted = first.clone();
quoted.fill_lanes();
assert_eq!(quoted.get_bidpx(), Some("82.5".parse()?));
assert_eq!(quoted.get_askpx(), None);

// A later event of the order follows the first: it records its predecessor,
// takes the next place in the chain and carries the lifecycle forward.
let second = event(20_000, "O-100", "PartiallyFilled")?;
assert!(second.is_after(&first) && first.is_before(&second));
let second = second.with_previous(&first).expect("a later event follows");
assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
assert_eq!(second.get_prevunix(), Some(10_000));
assert_eq!(second.get_seqnum(), 1);
assert_eq!(second.get_crossuuid(), first.get_crossuuid(), "one chain");
// Identities are UUIDv7s: they sort by instant first, to the microsecond.
assert!(first.get_curruuid() < second.get_curruuid());
// An event follows neither itself nor one that happened after it.
assert!(event(10_000, "O-100", "New")?.with_previous(&first).is_none());
assert!(event(5_000, "O-100", "New")?.with_previous(&second).is_none());

// A caller resolves a predecessor by identity through whatever holds the graph.
let graph: Vec<&dyn MarketEvent> = vec![&first, &second];
let previous = graph
    .iter()
    .find(|element| Some(element.get_curruuid()) == second.get_prevuuid())
    .expect("the first event is in the graph");
assert_eq!(previous.get_currunix(), 10_000);
assert!(previous.get_state().is_live());
assert_eq!(previous.get_side().as_str(), "BUY");

// A later statement of the same event merges into it: the later instant,
// price and code have the last word, and the merged event is finalized to
// the identity its new instant and content derive.
let mut restated = event(15_000, "O-100", "New")?;
restated.set_curruuid(first.get_curruuid());
restated.set_px("83".parse()?);
let merged = first.clone().merge_with(&restated).expect("the same event");
assert_eq!((merged.get_currunix(), merged.get_px()), (15_000, Decimal18::from_int(83)));
assert_eq!(merged.get_curruuid(), merged.time_uuid()?);
assert_ne!(merged.get_curruuid(), first.get_curruuid());
assert!(merged.clone().merge_with(&second).is_none(), "another event does not merge");
assert!(first.clone().merge_with(&first).is_none(), "a restatement adding nothing answers nothing");

// The walk does the chaining: events of one thing share its cross identity,
// and each is stated as the one after the live event before it. Unsorted
// input is sorted by the events' own order first, a filled order ends its
// chain, and the same event read twice - one message logged at two hops -
// is one event, the second statement taking the first one's place.
let arrived = vec![
    event(30_000, "O-100", "Filled")?,
    event(10_000, "O-100", "New")?,
    event(15_000, "O-900", "New")?,
    event(20_000, "O-100", "PartiallyFilled")?,
    event(20_000, "O-100", "PartiallyFilled")?,
    event(40_000, "O-100", "New")?,
];
let mut walk = EventIterator::new(arrived, false);
let chained: Vec<MarketEventData> = walk.by_ref().collect();
let places: Vec<(i64, &str, u64)> = chained
    .iter()
    .map(|held| (held.get_currunix(), held.get_crosscode(), held.get_seqnum()))
    .collect();
assert_eq!(
    places,
    [
        (10_000, "O-100", 0),
        (15_000, "O-900", 0),
        (20_000, "O-100", 1),
        (20_000, "O-100", 1),
        (30_000, "O-100", 2),
        (40_000, "O-100", 0),
    ]
);
assert_eq!(chained[2].get_curruuid(), chained[3].get_curruuid(), "a twin, not a successor");
assert_eq!(chained[4].get_prevuuid(), Some(chained[2].get_curruuid()));
assert_eq!(chained[5].get_prevuuid(), None, "the fill ended the chain");
let mut alive: Vec<(String, i64)> = walk
    .alive()
    .map(|held| (held.get_crosscode().to_owned(), held.get_currunix()))
    .collect();
alive.sort();
assert_eq!(alive, [("O-100".to_owned(), 40_000), ("O-900".to_owned(), 15_000)]);

// A grid emits owned views; a deadline at a grid boundary emits first and
// retires its identity, so that tick has no view of the expired order.
let mut expiring = event(50_000, "O-EXP", "New")?;
expiring.set_exprtime(Some(70_000));
expiring.finalize();
let source_uuid = expiring.get_curruuid();
let timed: Vec<_> = EventIterator::new([expiring], true)
    .with_snapshot_ns(10_000)
    .collect();
let snapshot = timed
    .iter()
    .find(|held| held.get_snapunix() == Some(60_000))
    .expect("the living view at the crossed tick");
assert_eq!((snapshot.get_curruuid(), snapshot.get_seqnum()), (source_uuid, 0));
let expired = timed.last().expect("the deadline event");
assert_eq!((expired.get_currunix(), expired.get_state().as_str()), (70_000, "95EXPIRED"));
assert_eq!((expired.get_prevuuid(), expired.get_seqnum()), (Some(source_uuid), 1));

```

## Edges

- `set_parentuuids(Vec::new())` makes an element a root again; the list is replaced whole, never merged, and `lineage()` on a root is the root alone. `set_srcuuids(Vec::new())` unsays the sources: a source is stated where an element is read out of another - a FIX message out of a `TextLine` - and following, restating and merging never take one from a different element. `set_crosscode(String::new())` unsays the cross code, and `sync_cross` then puts the cross element back on the element's own identity; `set_identifiers` replaces the names whole.
- `is_after` and `is_before` are one order: an element is never after itself, and two neither after nor before each other are equal in it - which a sort of elements, the walk's included, treats as a tie and keeps in arrival order.
- `following` refuses an element as its own predecessor and a predecessor that happened later; an equal instant follows, a later call replaces the predecessor recorded before, and a place past `u64::MAX` saturates. Following what the element already follows answers nothing. A predecessor's cross code is forced onto the follower, so a report naming only the venue's `OrderID` joins the chain the order opened under its `ClOrdID` and takes that code.
- `restating` never refuses: it is the caller's word that the event is the live one's twin. The walk gives that word only for an arrival under the identity the live element arrived under - a finalized copy of the same instant and content - and never for a later statement, which chains.
- `merging` refuses another element outright, and folds the same element's later statement over the earlier: an equal instant keeps this element's codes. A restatement that adds nothing answers nothing, so the earlier statement merged into the later one is a no-op.
- A lane already stated is never overwritten by `fill_lanes`, only a stated fact fills one - a zero price or quantity, no currency and an empty unit fill nothing - and a side that takes no lane - a cross, `OPPOSITE`, `UNKNOWN` - fills none.
- `currunix` is signed: an instant before the epoch is negative, and the `i64` holds every nanosecond count a clock this crate reads states. `time_uuid` refuses an instant a UUIDv7 cannot hold, before the epoch or past its 48-bit millisecond count, and `finalized` then keeps the identity as it was.
- A state is never absent: an element that reached none says `00UNKNOWN`, the code that means exactly that. An instrument code is absent where the market names none, and never a spelling the code refuses.
- The traits validate nothing else: a parent or a predecessor that no element answers to is the holder's to refuse, not the element's.
- A source element is cloned twice at most - once to hold it as the live one, once to keep one its predecessor refuses. Each expiration and grid view is an additional owned clone; the walk holds one live element per identity and emits views lazily rather than queueing a whole grid.
- A grid starts at the first epoch-aligned tick at or after the first source instant. Its output count is the number of crossed ticks times the identities alive at each one; the caller chooses that width, and the iterator applies no implicit count or time-span cap.
