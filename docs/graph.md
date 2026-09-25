# Graph

`yggdryl::graph` is three layers. The **traits** say what an element of a graph answers about itself - `Element`, `Event`, `Market`, `Operation` - as signatures and provided readings, with no storage. The **leaves** are the typed values that answer them: an order, a quote and an execution, undated (`Order`, `Quote`, `Execution`) or dated (`OrderEvent`, `QuoteEvent`, `ExecutionEvent`), the composite `TradeEvent`, the `BookSide`, the `BookEvent` and the `SnapshotEvent`, gathered under the one generic `MarketData` enum that any of them crosses a boundary as. The **implementations** are what runs over them: the column vocabulary, the lifted Arrow schema and its two doors, the lifecycle walk and the book fold.

## Traits

| Key | Value |
| --- | --- |
| Owner | `graph::element` holds `Element` and `Event`; `graph::market` holds `Market`, `Operation`, `Lane`, `Metadata` and `FOLLOWED_ALTIDS`; `graph::operation` the operation leaves `OperationElement<K>` and `OperationEvent<K>` with their six aliases, the sealed `OperationKind` and its three markers, `BookRef` and `MdUpdateAction`; `graph::trade` the `TradeEvent`; `graph::book` `BookSide`, `BookEvent`, `SnapshotEvent`, `SnapshotPartition`, `BookIterator` and the keys `GLOBAL_SYMBOL`, `ENTRY_ID`, `ENTRY_REF_ID`; `graph::market_data` the `MarketData` enum and `graph::kind` its `MarketKind`; `graph::iterator` the lifecycle walk; `graph::column`, `graph::market_column` and `graph::operation_column` the three column enums; `graph::arrow` the lifted Arrow row; no second identity type - the identity is [`Uuid`](types/uuid.md), the crate's own, while [`TxHash`](hashing.md) remains the independently exposed time-and-digest coupling |
| Names | Every accessor is `get_` and every mutator `set_`, so the traits claim no bare name: an implementor keeps its own `uuid()`, `state()` or `price()` for whatever it means by them, and the trait's reading is always the prefixed one. The security identifiers and the three identifier maps are the exception: a holder answers each as a set and changes it through fallible verbs - `insert_`, `remove_`, `derive_` - because a holder that is a view of another store, a [FIX message](fix/message.md) over its own fields, writes the store through and may refuse; a plain holder always answers `Ok` |
| Digest | `Element::digest(&self) -> Xxh3` starts the digest of what an element states - the cross code - and never an identity, an instant or its sources; `Event::digest_event` continues with the state, the place in the chain and the predecessor's identity; `Market::digest_market` with the price, the currency, the quantity, the unit, the side, each security identifier as its key and code, the classification, the market, each of the last-trade, average, progress and FX numbers where stated, the ticker and the metadata in key order - the step before, `prevpx` and `prevqty`, left out as the predecessor's instant and identity are; `Operation::digest_operation` with the category, the time in force, whether it trades, the three maps in key order and each lane; `Market::digest_market_event` and `Operation::digest_operation_event`, provided where `Self: Event`, with the event's and the market's or the operation's; each fed through the typed accessors and none of the instants, so an implementor's `finalize` feeds its own content behind the one for its kind and reads `as_u64`. A dated [operation leaf](#operations) feeds its kind's word - `order`, `quote` or `execution` - under `operationkind` and its book control - `mdupdateaction`, `bookscope`, `mdentrypositionno`, `mdentrypx`, `mdentrysize` - behind `digest_operation_event`, and an undated one its kind's word behind `digest_operation`, so the same entry as an order and as a quote are two operations. At every parent-defined structural occurrence, a composite feeds only the nested event or element's canonical `curruuid` bytes, never that child's `currhashcode` or content again; the parent still frames the sequence with the structural kind and count its layout needs |
| Links | Named by identity, never by reference: an element can name a predecessor, a source or a cross element it does not hold, and a caller resolves one through whatever holds the graph |
| Objects | Everything but `is_after`, `is_before`, `with_previous`, `merge_with`, `following`, `restating`, `merging`, `fold_lifecycle`, the fills and the market and operation digests and merges is object-safe: a walk over `dyn Event` or `dyn Operation` reads the identities, the codes, the sources, the instant, the state, the market's facts, the maps and the lanes through one reference |
| Bindings | The traits are Rust-only; the leaves, `MarketData`, `Lane`, `BookRef`, `SnapshotPartition` and the two walks are one class each in Python's `yggdryl.graph` and JavaScript's `graph` namespace, built from named facts keyed by column name - `...` or `undefined` skipped, `None` or `null` clearing - and redirecting every verb to the native one. Python `FixCodec.book_arrow_reader` and JavaScript `FixCodec.bookArrowReader` redirect into the Rust FIX-to-book pipeline and answer their native Arrow reader over [`MarketData::field()`](#arrow) rows. A FIX message implements `Element`, `Event`, `Market` and `Operation`, and a [text line](media/index.md#plain-text) is an `Event` - the one a message is read from, which the message states as its source |

### Element

`get_curruuid()` / `set_curruuid(Uuid)`; `get_crossuuid() -> Uuid` / `set_crossuuid(Uuid)`, the cross element - what this element is in another graph - never absent, its own identity where it states no cross code, so every element stands in exactly one chain; `get_crosscode() -> &str` / `set_crosscode(String)`, the text naming it there - an order's `OrderID`, a quote's `QuoteID` - empty where none is stated; `get_currhashcode() -> u64` / `set_currhashcode(u64)`, the XXH3-64 digest the element's content answers to; `get_crosshashcode() -> u64` / `set_crosshashcode(u64)`, the XXH3-64 of the cross code, zero where none; `get_srcuuids() -> &[Uuid]` / `set_srcuuids(Vec<Uuid>)`, the sorted unique identities of elements this one was read from - a message's [text line](media/index.md#plain-text) - provenance and never its chain, so no provided reading carries it along one; `is_after(&self, &Self) -> bool`, the order the implementor states - an event's instant, a node's predecessor - and `is_before`, provided as its mirror, the two one strict weak order; `finalize(&mut self)`, the implementor's: bring the cross codes in step, digest the content and record the code, resetting the identity where it derives from the content; `with_previous(self, &Self) -> Option<Self>`, this element stated as the one after another or nothing where it cannot follow it or following changes nothing, the implementor's to say; `merge_with(self, &Self) -> Option<Self>`, provided: another statement of the same element folded in - the cross code it lacks, the sources kept as a sorted unique union. An element carries no identifier map: the names an operation goes by are [`Operation::get_altids`](#operation), and what a book reads to place an entry is [`OperationEvent::book`](#operations).

`sync_cross(&mut self) -> bool` and `cross_uuid(&self) -> Uuid` are provided: the cross hash code is the cross code's digest, zero where none, and the cross element is RFC 9562 UUIDv8 over that code where it is not zero, else the element's own identity; every reading that takes a cross code from another element calls `sync_cross`, and an implementor's `finalize` calls it before digesting, so every element sharing a cross code shares the cross identity whichever holds them.

### Event

`get_currunix() -> i64` / `set_currunix(i64)`, nanoseconds since the Unix epoch, UTC; `get_state() -> &State` / `set_state(State)`, the ranked lifecycle [code](types/codes/index.md), never absent - `00UNKNOWN` where none was reached; `is_execution() -> bool`, provided from `State::is_execution`, says whether this observation itself reports an execution and can be overridden where a protocol's lifecycle state and report kind differ; `get_seqnum() -> u64` / `set_seqnum(u64)`, how many came before it in its chain; `creaunix`, `execunix`, `recdunix`, `exprtime`, `prevunix`, `snapunix` (`Option<i64>`) and `prevuuid` (`Option<Uuid>`), each `get_` with its `set_`, stated only where known. `execunix` is the precise execution instant and `recdunix` the earliest precise recording instant the statements know, which is also what a merge ranks two statements by; the snapshot instant is the grid step a walk read the element as the snapshot of.

`Event::txhash() -> Result<TxHash>` couples `currunix` at nanosecond resolution with `currhashcode` as its digest. `time_uuid() -> Result<Uuid>` calls that value's `into_sequenced_uuid(seqnum, crosshashcode)`: the instant is floored to Unix milliseconds, `seqnum.min(4095)` occupies UUIDv7's ordered 12-bit `rand_a` lane, and `rand_b` carries the low 62 bits of XXH3 over `currhashcode` plus the whole `u64` sequence with the cross hash as seed. Identities therefore sort by millisecond and then every sequence the lane represents; a larger sequence stays in the terminal lane but remains probabilistically distinct through the payload. The seed separates cross chains and the payload separates content and sequence overflow. An instant before the epoch is `InvalidRecord`, never a truncated identity.

`Event::finalized(&mut self, hashcode: u64)` is provided: it records the code, resets the current identity to `time_uuid` - kept where the instant has no UUIDv7 - and the cross element to `cross_uuid` over it. Every dated leaf's own `Element::finalize` hands its digest to this derivation; a foreign implementor that owns an assigned identity may instead preserve it by not calling `finalized`.

### Market

nineteen facts and no supertrait. Five every market answers: `get_price() -> Option<Decimal18>` / `set_price(Option<Decimal18>)`, the price it states and `None` where it states none - never a last executed price, which `lastpx` answers, and never a default - `get_currency() -> &Ccy` / `set_currency(Ccy)` (`Ccy::none()` where unstated), `get_quantity() -> Option<Decimal18>` / `set_quantity(Option<Decimal18>)`, the quantity it states on the same rule, `get_unit() -> &Unit` / `set_unit(Unit)` ([`Unit::none()`](types/codes/unit.md) where unstated), `get_side() -> Side` / `set_side(Side)` - the [side](types/codes/side.md) by value, one byte, `Side::Unknown` where none - the price and the quantity exact, as [`Decimal18`](types/numeric/decimal.md#decimal18) holds a market's numbers. The instrument: `get_securityids() -> &SecurityIds`, the identifiers it is stated under, one validated code per source key, sorted - `ids.get("ISIN")`, also `"4"` or `"isin"`, answers `Option<&str>` - changed through `set_securityids(SecurityIds) -> Result<()>` (replaced whole), `insert_securityid(SecurityId) -> Result<bool>` (fill only), `remove_securityid(&SecType) -> Result<bool>` and `derive_securityid(SecurityId) -> bool` (fill only, and never the store a holder is a view of: what the element implies rather than states); `get_cficode() -> Option<&CfiCode>`, `get_miccode() -> Option<&MicCode>`, each with its setter; `get_ticker() -> Option<&str>` / `set_ticker(Option<SmolStr>)`, the name a person knows it by, beside the codes rather than among them. The last executed price and quantity, `lastpx` and `lastqty`, then `avgpx`, `cumqty`, `leavesqty`, `prevpx`, `prevqty`, `spotrate`, `forwardpoints`, each `get_` answering `Option<Decimal18>` with its `set_`. And `get_metadata() -> &Metadata` / `set_metadata(Option<Metadata>)`, a `BTreeMap<SmolStr, SmolStr>` of free-form facts - never an identifier, which has a typed home - a shared empty map where the element carries none. Provided: `fill_market(&mut self)` never invents a price or a quantity - a last executed price or quantity is `lastpx` or `lastqty`, an average is `avgpx`, and how much is done and how much is left stay beside the quantity ordered, because together they *are* it - and derives the national identifier a canonical ISIN embeds ([`securityid::embedded`](types/codes/isin.md)) into a key the element does not state; idempotent, and what an implementor's `finalize` runs before it digests; `digest_market(&self) -> Xxh3` and `merging_market(self, &Self) -> Option<Self>` where `Self: Element`. Provided where `Self: Event`, for an event that is also a market: `digest_market_event`, `following_market` and `merging_market_event`, the readings that need both - [below](#following-restating-merging).

### Operation

eight more facts: `get_marketoperationid() -> Option<i32>` / `set_marketoperationid`, the stable numeric market-operation category; `get_tif() -> Option<&TimeInForce>` / `set_tif(Option<TimeInForce>)`, how long it stands ([`TimeInForce::from_spelling("day")`](types/codes/timeinforce.md) stores the code `0`); `get_tradable() -> Option<bool>` / `set_tradable`, `None` a market that said nothing either way; three [`IdMap`](fix/message.md#the-identifier-maps)s - `get_accountids`, `get_userids`, `get_altids`, each `-> &IdMap`, upper-cased ASCII keys to ASCII values in key order - the accounts the operation is for, the users it is by, and the names it goes by (`ORDERID`, `CLORDID`, `MDENTRYID`, ...), each changed through `set_<map>(IdMap) -> Result<()>`, `insert_<key>(key, value) -> Result<bool>` (fill only; `insert_accountid`, `insert_userid`, `insert_altid`) and `remove_<key>(key) -> Result<bool>`; and two lanes, `get_bid() -> Option<&Lane>` / `set_bid(Option<Lane>)`, `get_ask` / `set_ask`, what a party would pay and what it would be paid - a `Lane { price, spotrate, forwardpoints, currency: Option<Ccy>, quantity, unit: Option<Unit> }`, every slot optional, `Lane::is_stated` whether it states any and `stated()` the lane as `Some` only where it does; a lane stating nothing sets as `None`. Provided: `fill_operation(&mut self)` continues `fill_market` - a quote stating one lane and no side, no price, no last trade and no average of its own is that lane's side, `BUY` on the bid and `SELL` on the ask, two lanes or none naming nothing; then the price, the quantity, the currency, the unit and the two FX parts are the element's own, else what its own side's lane quotes; then `fill_lanes(&mut self)`, provided, filling the lane the side implies - `Side::is_bid`, `Side::is_ask` - from those facts where the lane states nothing of its own, a stated fact only, and nothing for a side taking neither lane; `digest_operation(&self) -> Xxh3` and `merging_operation(self, &Self) -> Option<Self>` where `Self: Element`. Provided where `Self: Event`, one trait further: `digest_operation_event`, `following_operation` and `merging_operation_event`; the [walk](#lifecycle-walk) is over an `Event` that is an `Operation`.

### Following, restating, merging

`Event::following(self, &Self) -> Option<Self>` is provided: it records the predecessor's identity and instant, puts the element one place after the predecessor's in the chain (saturating), forces the predecessor's cross code onto it where its own differs - two events of one chain share it - with the cross codes brought in step, and keeps the later of its own precise execution clock and the predecessor's. The predecessor is the one element an event names in its chain: nothing records the chain further back than `prevuuid`. It carries the lifecycle forward: the earliest creation the two know, this event's explicit expiration where it states one else the predecessor's, and the furthest state. A newer event may therefore shorten a deadline. What the element itself says - its current and recording instants, its sources, the snapshot it is - moves nowhere. Nothing for the element's own predecessor, for one that happened after it, or where following changes nothing; an equal instant follows. An element that moved is finalized. An implementor's `with_previous` delegates to it, or states its own reading. `Market::following_market` additionally takes the price and the quantity the predecessor settled on as `prevpx` and `prevqty` where this event states none of its own, and what the chain is about where this event says nothing of it - the currency, the unit, the side (a side stated as none takes the predecessor's), an absent ticker, each security identifier under a key this event does not state, the classification and the market - even when the timed link is already present; this statement always leads. `Operation::following_operation` continues with the operation: the time in force and whether it trades where unstated, every account and user under a key this event lacks, and of the alternate identifiers only the order's own - `FOLLOWED_ALTIDS`: `ORDERID`, `SECONDARYORDERID`, `PARENTORDERID`, `PARENTCLORDID`, `OMSDEALERPARENTORDERID`, `EXCHANGECLIENTORDERID`, `TRANSVERSALKEY`, `MARKETORDERID`, `OMSDEALERORDERID` - never an execution's or a quote's, and never a lane; and for an operation the side is the chain's to give only where it quotes no lane of its own - one lane names its side itself and two name none - in following and in restating alike. The timed and market readings settle before one finalization.

`Event::restating(self, live: &Self) -> Self` is provided: this event as another statement of `live` - the same instant and content read a second time, as a capture logs one message at every hop it passes - taking the place `live` holds in its chain: its predecessor, position and snapshot, the chain's cross code - never its sources, which say what each statement was read from - the lifecycle folded, the earliest execution and recording instants kept, then finalized, so the two statements finalize to one identity and the chain grows by nothing. Every market event overrides it to take the market's place too - the step before it, `prevpx` and `prevqty`, and what the chain is about where this reading said nothing of it - and every market operation event the operation's: the time in force, whether it trades, the accounts, the users and the followed alternate identifiers. The caller establishes the twin by the identity `live` arrived under, because once `live` has followed something its identity has moved and the event alone cannot tell a twin from a successor; the walk records that identity and does exactly this.

`Event::merging(self, &Self) -> Option<Self>` is provided. The reference is the statement with the later `recdunix`: a stated recording clock leads an unstated one, equal or absent clocks fall back to the greater `currunix`, and an exact tie keeps `self`. Its cross code, current instant and code, predecessor and snapshot lead; source identities are a position-independent sorted unique union, and the other statement fills what the reference leaves unstated. The place in the chain is the further one, and `fold_lifecycle(&mut self, &Self)` keeps the earliest creation, latest expiration and better state as [`CodeValue::merge_with`](types/codes/index.md#the-code-family-value) reads it. Execution and recording facts independently remain the earliest either observation states, so a merged statement ranks against a third by the earliest recording it keeps, and which of three statements is the reference depends on the order they merge in. `Market::merging_market` and `Operation::merging_operation` have no clocks and keep `self` leading. `Market::merging_market_event` uses the event reference for the market too: the reference's price, quantity and unit stand; each optional number, the ticker and the metadata are the reference's where it states them, else the other's, the metadata a union with the reference's values leading; the currency, the side, the classification and the market the better of the two; a security identifier the reference states under a key replaces the other's, and one only the other states fills the key. `Operation::merging_operation_event` continues with the category, the time in force, whether it trades, the three maps - each a union with the reference's values leading - and the lanes, slot by slot. Each answers nothing where the fold changes nothing, and finalizes the element where it did. An implementor's `merge_with` delegates to the one for its kind.

Rust only: the traits have no binding - Python and JavaScript read the facts on the leaves and on `MarketData`.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, EventIterator, Market, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal18, Side, State};

    // An order's life as order events: each states its instant, the
    // identifier the venue gave the order - its cross code and the ORDERID it
    // goes by - and where it stands.
    let event = |unix: i64, order: &str, state: &str| -> yggdryl::Result<OrderEvent> {
        let mut event = OrderEvent::at(unix);
        event.set_crosscode(order.to_owned());
        event.set_state(State::from_spelling(state).expect("a shipped state"));
        event.set_price(Some("82.5".parse()?));
        event.set_quantity(Some(Decimal18::from_int(1_000)));
        event.set_currency(Ccy::new("USD")?);
        event.set_side(Side::read("1")?);
        event.insert_altid("ORDERID", order)?;
        event.finalize();
        Ok(event)
    };
    let first = event(10_000, "O-100", "New")?;
    // Finalized, the identity is what the event states and when: UUIDv7 ordered by
    // millisecond and sequence with a content payload seeded by the cross hash.
    assert_eq!(first.get_curruuid(), first.time_uuid()?);
    assert_ne!(first.get_currhashcode(), 0);
    assert_eq!(first.get_crossuuid(), first.cross_uuid());
    assert_ne!(first.get_crossuuid(), first.get_curruuid(), "a cross code names a chain of its own");
    assert_eq!(first.get_altids().get("orderid"), Some("O-100"), "keys fold to upper case");
    // A buy is a bid: finalizing filled the lane the side implies from the
    // event's own facts, and the other lane stays unstated.
    assert_eq!(first.get_bid().and_then(|lane| lane.price), Some("82.5".parse()?));
    assert_eq!(first.get_ask(), None);

    // A later event of the order follows the first: it records its predecessor,
    // takes the next place in the chain and carries the lifecycle forward.
    let second = event(20_000, "O-100", "PartiallyFilled")?;
    assert!(second.is_after(&first) && first.is_before(&second));
    let second = second.with_previous(&first).expect("a later event follows");
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevunix(), Some(10_000));
    assert_eq!(second.get_seqnum(), 1);
    assert_eq!(second.get_crossuuid(), first.get_crossuuid(), "one chain");
    // The step before it is what the predecessor settled on.
    assert_eq!(second.get_prevpx(), Some("82.5".parse()?));
    // Identities are UUIDv7s: their millisecond leads and their chain sequence is
    // the ordered lane. These two share one millisecond and sequence 0 precedes 1.
    assert!(first.get_curruuid() < second.get_curruuid());
    // An event follows neither itself nor one that happened after it.
    assert!(event(10_000, "O-100", "New")?.with_previous(&first).is_none());
    assert!(event(5_000, "O-100", "New")?.with_previous(&second).is_none());

    // A caller resolves a predecessor by identity through whatever holds the graph.
    let graph: Vec<&OrderEvent> = vec![&first, &second];
    let previous = graph
        .iter()
        .find(|element| Some(element.get_curruuid()) == second.get_prevuuid())
        .expect("the first event is in the graph");
    assert_eq!(previous.get_currunix(), 10_000);
    assert!(previous.get_state().is_live());
    assert_eq!(previous.get_side(), Side::Buy);

    // Another statement of the same event merges into it. With no recording
    // clocks, the later event instant is the reference, so its instant, price and
    // code lead; a stated later recdunix would choose the reference instead.
    let mut restated = event(15_000, "O-100", "New")?;
    restated.set_curruuid(first.get_curruuid());
    restated.set_price(Some("83".parse()?));
    let merged = first.clone().merge_with(&restated).expect("the same event");
    assert_eq!((merged.get_currunix(), merged.get_price()), (15_000, Some(Decimal18::from_int(83))));
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
    let chained: Vec<OrderEvent> = walk.by_ref().collect();
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

## Leaves

### Operations

An operation is an order, a quote or an execution, and the kind is the type: `OperationElement<K>` is one with no instant and `OperationEvent<K>` one at an instant, `K` the sealed `OperationKind` - `OrderKind`, `QuoteKind` or `ExecutionKind`, whose `KIND` is the [`MarketKind`](#marketdata) it digests and stores under. The six aliases `Order`, `Quote`, `Execution`, `OrderEvent`, `QuoteEvent` and `ExecutionEvent` are the leaves this crate ships, and no other kind can be named. Each leaf holds its role's facts in a crate-private holder and answers them through the traits: an element `Element`, `Market` and `Operation`, an event `Event` too. `Order::new()` states nothing; `OrderEvent::at(unix)` states an instant and nothing else; `Default` is the same at zero. `at(unix)` on an element answers its event, finalized, and `into_element()` on an event drops the clocks and the book control - both moves. `impl<E: Event + Operation + ?Sized> From<&E> for OperationEvent<K>` copies every fact another event states - the identities as stated, no book control, not refinalized - which is how a [FIX message](fix/message.md#market-operations) becomes an operation of the kind it is.

`kind()` answers `K::KIND`, and `is_execution` is `K::KIND == Execution` whatever the state says. `finalize` fills the market and the operation, brings the cross codes in step and digests the kind: an undated element continues `digest_operation` with it and takes RFC 9562 UUIDv8 over the code, so it states no order and `is_after` is always false; an event continues `digest_operation_event` with the kind and the book control and settles the event identity. `with_previous` is `following_operation`, then finalized; `merge_with` is `merging_operation_event`; `restating` is the operation restatement.

The book control rides beside a dated operation's facts, boxed, so an operation that is no market-data entry pays one null pointer: `book() -> Option<&BookRef>`, `set_book(Option<BookRef>)` - a control stating nothing lands as `None`, and the operation is not refinalized, so a caller finalizes once its facts are in - and `with_book(BookRef)`; `action()` and `scope()` (empty where none) read through it, and `is_full_snapshot()` is `action() == Some(MdUpdateAction::Snapshot)`, the marker every FIX `W` occurrence carries. `BookRef { action: Option<MdUpdateAction>, scope: Option<SmolStr>, position: Option<u32>, entry_px: Option<Decimal18>, entry_size: Option<Decimal18> }` is the typed book control a market-data entry carries - the update action, the book scope, `MDEntryPositionNo(290)`, and the price and size the entry stated for itself, so a partial update still knows what it stated - never an identifier: the entry's own and referenced identifiers are the operation's `MDENTRYID` and `MDENTRYREFID` alternate identifiers. `MdUpdateAction { New = 0, Change, Delete, DeleteThru, DeleteFrom, Overlay, Snapshot = 6 }` is FIX's `MDUpdateAction(279)` plus the full snapshot a `W` replaces a scope with: `as_str` the wire code or `snapshot`, `read` the code, the code set's name folded or the legacy `SNAPSHOT`, `is_range_delete` and `is_partial`.

=== "Rust"

    ```rust
    use yggdryl::graph::{
        BookRef, Element, Event, Market, MarketKind, MdUpdateAction, Order, OrderEvent, QuoteEvent,
    };
    use yggdryl::{Side, Uuid};

    // An undated order: its identity is its content, UUIDv8 over its code.
    let mut order = Order::new();
    order.set_crosscode("O-100".to_owned());
    order.set_side(Side::Buy);
    order.set_price(Some("82.5".parse()?));
    order.finalize();
    assert_eq!(order.kind(), MarketKind::Order);
    assert_eq!(order.get_curruuid(), Uuid::from_v8(u128::from(order.get_currhashcode())));

    // Dated at an instant, it is an order event, finalized: UUIDv7.
    let event: OrderEvent = order.clone().at(1_700_000_000_000_000_000);
    assert_eq!(event.get_curruuid(), event.time_uuid()?);
    assert!(!event.is_execution(), "the kind decides, whatever the state");

    // The same facts as a quote are another operation.
    let mut quote = QuoteEvent::from(&event);
    quote.finalize();
    assert_ne!(quote.get_curruuid(), event.get_curruuid());

    // A market-data entry carries its book control, which digests into it.
    let mut entry = quote.clone().with_book(BookRef {
        action: Some(MdUpdateAction::New),
        ..BookRef::default()
    });
    entry.finalize();
    assert_eq!(entry.action(), Some(MdUpdateAction::New));
    assert_ne!(entry.get_curruuid(), quote.get_curruuid());

    // Undated again, the clocks and the control stay behind.
    let mut element = entry.into_element();
    element.finalize();
    let mut plain = quote.into_element();
    plain.finalize();
    assert_eq!(element, plain);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    # An undated order: its identity is its content.
    order = graph.Order(crosscode="O-100", side="BUY", price=Decimal("82.5"))
    assert order.kind == "order"

    # Dated at an instant, it is an order event.
    event = order.at(1_700_000_000_000_000_000)
    assert isinstance(event, graph.OrderEvent)
    assert event.currunix == 1_700_000_000_000_000_000
    assert not event.is_execution, "the kind decides, whatever the state"

    # The same facts as a quote are another operation.
    quote = graph.QuoteEvent(event.currunix, crosscode="O-100", side="BUY", price=Decimal("82.5"))
    assert quote.curruuid != event.curruuid

    # A market-data entry carries its book control, which digests into it.
    entry = quote.with_book(graph.BookRef(action="0"))
    assert entry.action == "0"
    assert entry.curruuid != quote.curruuid

    # Undated again, the clocks and the control stay behind.
    assert entry.into_element() == quote.into_element()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    // An undated order: its identity is its content.
    const order = new graph.Order({ crosscode: 'O-100', side: 'BUY', price: '82.5' })
    assert.equal(order.kind, 'order')

    // Dated at an instant, it is an order event.
    const event = order.at(1_700_000_000_000_000_000n)
    assert.ok(event instanceof graph.OrderEvent)
    assert.equal(event.currunix, 1_700_000_000_000_000_000n)
    assert.equal(event.isExecution, false, 'the kind decides, whatever the state')

    // The same facts as a quote are another operation.
    const quote = new graph.QuoteEvent(event.currunix, { crosscode: 'O-100', side: 'BUY', price: '82.5' })
    assert.notEqual(quote.curruuid, event.curruuid)

    // A market-data entry carries its book control, which digests into it.
    const entry = quote.withBook(new graph.BookRef({ action: '0' }))
    assert.equal(entry.action, '0')
    assert.notEqual(entry.curruuid, quote.curruuid)

    // Undated again, the clocks and the control stay behind.
    assert.ok(entry.intoElement().equals(quote.intoElement()))
    ```

### Trades

`TradeEvent::from_parts(&root, executions)` is the single construction boundary for a composite trade: the facts of any `Event + Operation` root and a `Vec<ExecutionEvent>`. It requires at least one execution; every child must be an execution on a bid or ask side at the root instant, carry either no ticker or the root's, and have a cross code unique within the trade. Construction finalizes the children and sorts them by side, cross code and identity, so input order cannot change the result. The root then takes the maximum child sequence, earliest creation and recording instants and latest execution instant; its final digest feeds the execution count and each child's canonical `curruuid` bytes exactly once, never a child `currhashcode` or content replay. `executions() -> &[ExecutionEvent]` exposes that canonical order. `TradeEvent` is `Element`, `Event`, `Market` and `Operation` over the root: `is_execution` is always true, and `set_currunix` atomically rebases the root and every child to the new observation instant and re-finalizes them while retaining each child's precise `execunix`. Following and merging trades require the same root cross code, combine children by execution cross code and perform that same rebase to the resulting root instant before deriving the composite again.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, ExecutionEvent, Market, OrderEvent, TradeEvent};
    use yggdryl::Side;

    let fill = |code: &str, side: Side| {
        let mut execution = ExecutionEvent::at(10_000);
        execution.set_crosscode(code.to_owned());
        execution.set_side(side);
        execution
    };
    let mut root = OrderEvent::at(10_000);
    root.set_crosscode("T-1".to_owned());

    let trade = TradeEvent::from_parts(&root, vec![fill("E-SELL", Side::Sell), fill("E-BUY", Side::Buy)])?;
    let codes: Vec<&str> = trade.executions().iter().map(Element::get_crosscode).collect();
    assert_eq!(codes, ["E-BUY", "E-SELL"]);
    assert!(trade.is_execution());
    // Input order cannot change the trade.
    let again = TradeEvent::from_parts(&root, vec![fill("E-BUY", Side::Buy), fill("E-SELL", Side::Sell)])?;
    assert_eq!(again.get_curruuid(), trade.get_curruuid());
    // An execution is required.
    assert!(TradeEvent::from_parts(&root, Vec::new()).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import graph

    def fill(code: str, side: str) -> graph.ExecutionEvent:
        return graph.ExecutionEvent(10_000, crosscode=code, side=side)

    root = graph.OrderEvent(10_000, crosscode="T-1")
    trade = graph.TradeEvent.from_parts(root, [fill("E-SELL", "SELL"), fill("E-BUY", "BUY")])
    assert [execution.crosscode for execution in trade.executions] == ["E-BUY", "E-SELL"]
    assert trade.is_execution
    # Input order cannot change the trade.
    again = graph.TradeEvent.from_parts(root, [fill("E-BUY", "BUY"), fill("E-SELL", "SELL")])
    assert again.curruuid == trade.curruuid
    # An execution is required.
    try:
        graph.TradeEvent.from_parts(root, [])
    except ValueError as error:
        assert "at least one execution" in str(error)
    else:
        raise AssertionError("a trade of no execution was built")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const fill = (code, side) => new graph.ExecutionEvent(10_000n, { crosscode: code, side })
    const root = new graph.OrderEvent(10_000n, { crosscode: 'T-1' })

    const trade = graph.TradeEvent.fromParts(root, [fill('E-SELL', 'SELL'), fill('E-BUY', 'BUY')])
    assert.deepEqual(trade.executions.map((execution) => execution.crosscode), ['E-BUY', 'E-SELL'])
    assert.equal(trade.isExecution, true)
    // Input order cannot change the trade.
    const again = graph.TradeEvent.fromParts(root, [fill('E-BUY', 'BUY'), fill('E-SELL', 'SELL')])
    assert.equal(again.curruuid, trade.curruuid)
    // An execution is required.
    assert.throws(() => graph.TradeEvent.fromParts(root, []), /at least one execution/)
    ```

### Book sides and books

`BookSide` owns persistent live order and quote depth as `Arc<MarketData>`s plus the deltas since the last emitted book; `BookEvent` owns bid, ask, the executions at its effective timestamp and the scopes its last snapshot replaced. A composite `TradeEvent` contributes its child executions and does not enter depth as a root operation; the complete book execution list is sorted and deduplicated by `curruuid`. `BookIterator` consumes already sorted `MarketData` values and emits one `Result<BookEvent>` per touched symbol and effective timestamp, or one consolidated `GLOBAL` book.

`BookSide::new(side)` accepts only a bid or ask. Its `live()` iterator of `&MarketData`, each an `OrderEvent` or a `QuoteEvent`, is price ordered - highest bid or lowest ask first - and entries at the same price are ordered by `BookRef::position` when any states one; `best_quantity()` is the aggregate at the exact best price, and an update whose aggregate cannot fit `Decimal18` is refused atomically. `add_operation(MarketData)` accepts only an `OrderEvent` or `QuoteEvent` on that side; any other variant is `InvalidRecord` naming its kind. A live key is `(symbol, scope, cross identity)`, so identical venue IDs in two feeds or two symbols remain distinct. A stated same-symbol, same-scope `MDENTRYREFID` alternate identifier is resolved first, then the incoming identity or its `MDENTRYID` - the keys `graph::book::ENTRY_REF_ID` and `ENTRY_ID` - and a distinct live destination beside the referenced entry is ambiguous and refused. A live new, change or overlay replaces and chains the prior generation, while a terminal delete removes it. A change or overlay - `MdUpdateAction::is_partial` - inherits price or size from that matched generation only where the occurrence left `entry_px` or `entry_size` unstated; an explicit zero remains zero, and an omitted value with no predecessor is refused instead of fabricating zero-valued depth. Actions `1` and `5` retain a matched `OrderEvent` when the `ORDERID` alternate identifier is omitted, or promote an unidentified `QuoteEvent` to an `OrderEvent` when one is stated; a contradictory known `ORDERID` is a located `InvalidRecord`. Action `2` retains the matched order kind and its link to the matched predecessor in its delete delta. An anonymous action `0` cannot insert into an occupied position because shifting would change the identity of every following anonymous entry; it must state `MDENTRYID` or arrive in a snapshot. Actions `3` and `4` - `is_range_delete` - delete through and from a required positive, in-range `BookRef::position`, considering only entries of the same symbol and scope; malformed or absent positions refuse atomically. Every accepted operation remains in `deltas()` even when it leaves no live depth. The side is a `Market`: its summary is the best level - the price, the aggregate quantity, the first entry's currency and unit - and nothing where the side is empty. The side's identity digests each list count, then each operation kind and canonical child `curruuid` in order, never the child's `currhashcode` or content again; clearing deltas therefore immediately restates that identity.

`BookEvent::new(unix, symbol)` creates empty bid and ask sides at that nanosecond, the symbol its ticker and its cross code. `add_operations` takes any `IntoIterator` of `MarketData` or `Result<MarketData>` and is a no-op for an empty input; an `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, `TradeEvent` or `SnapshotEvent` folds, and any other variant is `InvalidRecord` at `$.operations[index].kind` before anything is applied; otherwise it atomically requires every input's `currunix` to agree, refuses timestamp regression and applies them in source order. Advancing the timestamp clears the prior timestamp's deltas, executions and snapshot stamp while retaining live depth. A full-snapshot order, quote or `SnapshotEvent` first clears only its declared `(symbol, scope)` partition on both sides; an empty FIX `W` reaches this as one `SnapshotEvent` and therefore clears stale depth without inventing an order or quote, while an execution or composite trade never controls resting membership. The partitions the snapshot replaced are recorded and read back by `snapshot_partitions() -> &BTreeSet<SnapshotPartition>`, empty once an ordinary update follows. The same live identity moving sides first continues its matched predecessor, retaining its place in the chain, inherited market facts and explicitly stated lane values, then retires its old side before the new generation is inserted. The book folds its bounds from the composite trade root and then flattens that trade's children into `executions() -> &[ExecutionEvent]`; neither root nor children enter the live sides or deltas. Before the book identity is derived, the complete execution list is sorted by `curruuid` and duplicate identities are removed, so source order and repeated delivery cannot change the serialized book. The book keeps the maximum sequence, earliest creation and recording instants, and latest execution instant from the finalized applied generation, after any predecessor chain has advanced it. Rehydration verifies those four propagation bounds and every temporal bound against every nested operation rather than accepting a contradictory root. Executions append for that timestamp and never add, remove or decrement resting depth. A book identity feeds the bid and ask canonical UUIDs in fixed positions followed by the ordered execution UUIDs and the scopes its last snapshot replaced; the fixed-width tail carries the execution count structurally, and no nested hash or content is replayed.

`bid()` and `ask()` expose the persistent sides. The book is an `Event` and a `Market` and no operation - it has no lanes of its own; the sides are its lanes. `is_crossed()` means best bid strictly above best ask - a locked book is coherent - and `bbo_midpoint()` is the overflow-safe arithmetic midpoint of a coherent two-sided BBO, `None` only for a one-sided or crossed book. This is the usual midpoint of the national best bid and offer, `(bid + offer) / 2`, used in the [SEC's midpoint-price methodology](https://www.sec.gov/files/rules/sro/btnl/2026/34-106421-ex4.pdf). The book's `price` is that midpoint, otherwise the first available best price while not crossed, and zero where empty or crossed. `quantity` is `median_quantity()`: for two best-level quantities the [NIST median rule](https://www.itl.nist.gov/div898/handbook/eda/section3/eda351.htm) is their arithmetic mean; a one-sided book uses its one quantity and an empty book zero. Two sides state a book currency or unit only when they agree; one side supplies its own; disagreement states none. A crossed book therefore has zero `price` but still exposes both sides and their quantity median.

As an `Event`, a `BookEvent` follows only the same cross code at a nondecreasing instant. An incremental book starts from the previous live sides, clears only the `(symbol, scope)` partitions it authoritatively replaced, then reapplies its deltas, so an empty or nonempty FIX snapshot preserves unrelated partitions. A grid or supplied membership snapshot is a complete authoritative view and is never refilled. Two books merge only for the same cross code and instant, with the later `recdunix`, then `currunix`, selecting the reference. Its live entries lead, the other observation fills identities it lacks except in partitions the reference replaced; a complete membership reference admits no missing depth. Deltas and executions are unioned once, so a self-merge changes nothing.

`SnapshotEvent` is the full-snapshot control an empty `W` is - an event, the scope it replaces under `MdUpdateAction::Snapshot`, and no entry of its own - built by `SnapshotEvent::snapshot(&event, scope)`, the one owner of the control role: it copies the event and market facts of any `Event + Market`, never an operation's, and finalizes through `digest_market_event`, so the control it answers is canonical however `event` arrived; `book()` reads the control. `SnapshotPartition { symbol: Option<SmolStr>, scope: SmolStr }` names one scope a snapshot replaced.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, Order, OrderEvent};
    use yggdryl::{Decimal18, Side, State};

    let order = |code: &str, side: Side, price: i64| {
        let mut order = OrderEvent::at(1_000);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("IBM".into()));
        order.set_side(side);
        order.set_price(Some(Decimal18::from_int(price)));
        order.set_quantity(Some(Decimal18::from_int(2)));
        order.set_state(State::from_spelling("New").expect("a shipped state"));
        order.finalize();
        MarketData::from(order)
    };
    let mut book = BookEvent::new(1_000, "IBM");
    book.add_operations([order("B-1", Side::Buy, 100), order("A-1", Side::Sell, 102)])?;
    assert_eq!(book.bid().best_price(), Some(Decimal18::from_int(100)));
    assert_eq!(book.ask().best_price(), Some(Decimal18::from_int(102)));
    assert!(!book.is_crossed());
    assert_eq!(book.bbo_midpoint(), Some(Decimal18::from_int(101)));
    let best = book.bid().live().next().expect("the bid");
    assert_eq!(best.as_order_event().map(Element::get_crosscode), Some("B-1"));

    // Only a dated operation, a trade or a snapshot control folds.
    let error = book.add_operations([MarketData::from(Order::new())]).unwrap_err();
    assert!(error.to_string().contains("$.operations[0].kind"), "{error}");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    def order(code: str, side: str, price: int) -> graph.OrderEvent:
        return graph.OrderEvent(
            1_000, crosscode=code, ticker="IBM", side=side, price=Decimal(price), quantity=2, state="NEW"
        )

    book = graph.BookEvent(1_000, "IBM").with_operations([order("B-1", "BUY", 100), order("A-1", "SELL", 102)])
    assert book.bid.best_price is not None and book.bid.best_price.as_py() == Decimal(100)
    assert book.ask.best_price is not None and book.ask.best_price.as_py() == Decimal(102)
    assert not book.is_crossed
    assert book.bbo_midpoint is not None and book.bbo_midpoint.as_py() == Decimal(101)
    best = book.bid.live[0].as_order_event()
    assert best is not None and best.crosscode == "B-1"

    # Only a dated operation, a trade or a snapshot control folds.
    try:
        book.with_operations([graph.Order()])
    except ValueError as error:
        assert "$.operations[0].kind" in str(error)
    else:
        raise AssertionError("an undated order was folded")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = (code, side, price) => new graph.OrderEvent(1_000n, {
      crosscode: code, ticker: 'IBM', side, price: String(price), quantity: 2, state: 'NEW',
    })

    const book = new graph.BookEvent(1_000n, 'IBM')
      .withOperations([order('B-1', 'BUY', 100), order('A-1', 'SELL', 102)])
    assert.equal(book.bid.bestPrice, '100')
    assert.equal(book.ask.bestPrice, '102')
    assert.equal(book.isCrossed, false)
    assert.equal(book.bboMidpoint, '101')
    assert.equal(book.bid.live[0].asOrderEvent().crosscode, 'B-1')

    // Only a dated operation, a trade or a snapshot control folds.
    assert.throws(() => book.withOperations([new graph.Order()]), /\$\.operations\[0\]\.kind/)
    ```

### MarketData

`MarketData` is one value over every leaf: `Order`, `Quote`, `Execution`, `BookSide`, `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, `TradeEvent`, `BookEvent` (boxed, so the enum is not a book wide) and `SnapshotEvent`, one variant each. `kind()` answers its `MarketKind` - `as_str` the stored spelling (`order`, `quote`, `execution`, `book_side`, `order_event`, `quote_event`, `execution_event`, `trade_event`, `book_event`, `snapshot_event`), `read` the same ignoring ASCII case, `is_event` whether it is one of the six dated leaves - and `is_event()` the same of the value. `From<leaf>` builds one from every leaf and `TryFrom<MarketData>` takes it back, another variant `InvalidRecord` at `$.kind` naming the kind expected and the kind found; `as_order()`, `as_quote()`, ... `as_snapshot_event()` borrow a variant, and `book()` is the control of an operation event or a snapshot. `MarketData` answers `Element` and `Market` by delegating to the leaf it holds, so a boundary that resolved a value reads it generically past that point. `is_after` orders two dated values by instant and states no order where either is undated; `with_previous` and `merge_with` are the leaf's own for two values of one variant; an operation event also follows an operation event of another kind through the facts both hold, keeping its own kind - an execution follows the order it fills - and a merge never crosses variants.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, MarketData, MarketKind, OrderEvent, QuoteEvent};

    let mut order = OrderEvent::at(1_000_000);
    order.set_crosscode("O-1".to_owned());
    order.finalize();

    let value = MarketData::from(order.clone());
    assert_eq!(value.kind(), MarketKind::OrderEvent);
    assert_eq!(value.kind().as_str(), "order_event");
    assert!(value.is_event());
    assert_eq!(value.as_order_event(), Some(&order));
    assert_eq!(value.get_curruuid(), order.get_curruuid());
    assert!(QuoteEvent::try_from(value.clone()).is_err(), "another kind is refused at $.kind");
    assert_eq!(OrderEvent::try_from(value)?, order);
    ```

=== "Python"

    ```python
    from yggdryl import graph

    order = graph.OrderEvent(1_000_000, crosscode="O-1")
    value = graph.MarketData(order)
    assert value.kind == "order_event"
    assert "order_event" in graph.MarketData.kinds
    assert value.is_event
    assert value.as_order_event() == order
    assert value.curruuid == order.curruuid
    assert value.as_quote_event() is None, "another kind is none of this value"
    assert value.into_leaf() == order
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_000_000n, { crosscode: 'O-1' })
    const value = new graph.MarketData(order)
    assert.equal(value.kind, 'order_event')
    assert.ok(graph.MarketData.kinds().includes('order_event'))
    assert.equal(value.isEvent, true)
    assert.ok(value.asOrderEvent().equals(order))
    assert.equal(value.curruuid, order.curruuid)
    assert.equal(value.asQuoteEvent(), null, 'another kind is none of this value')
    assert.ok(value.intoLeaf() instanceof graph.OrderEvent)
    ```

## Implementations

### Columns

`EventColumn` is the sixteen columns every generated schema of an event states, one per fact `Element` and `Event` answer. `EventColumn::ALL` gives the canonical order used by `fields()` and event-native schemas: when it happened or was observed - `currunix`, `creaunix`, `execunix`, `recdunix`, `exprtime`, `prevunix`, `snapunix` - which event it is - `curruuid`, `crossuuid`, `crosscode`, `currhashcode`, `crosshashcode`, `prevuuid`, `seqnum`, `srcuuids` - and last the `state` it reached. A FIX row contains the same columns under its protocol-oriented time and identity bands rather than reordering the fixed schema around `ALL`. Each answers `name()`, `display()`, `description()`, `datatype()` - nanosecond UTC clocks, the crate's own [`uuid`](types/uuid.md), `uint64` codes, the `srcuuids` `serie<uuid>` whose item is named `srcuuid`, a [`state`](types/codes/state.md#the-rank-leads) code - and `nullable()`: the current instant, the two identities and the two codes are never null; every other column is where the event states nothing - an empty code, an empty serie, a place of zero, an absent instant - and the state, which an event always answers, `00UNKNOWN` where nothing states one, admits a null because a state has no neutral member for an empty cell to read as. `field()` and `fields()` are the same as fields; `fact(&event)` is what an event states under a column, nothing where it states no fact, and `record(&mut event, &cell)` states a cell back through the traits, a null clearing the fact; `of_name` finds a column by its name whatever the case.

A [text line](media/index.md#plain-text)'s batch, a [FIX row](fix/capture.md#the-crates-own-columns) and a [chained message](fix/lifecycle.md#a-chain-carries-its-creation-and-its-history) contain them under one name and one datatype each - the FIX row through the crate's own fields, each taking its column's datatype - so the three join without a mapping: a message's `srcuuids` are the `curruuid` of the lines it was parsed out of, and a chained message's `prevuuid` is the `curruuid` of the message before it. A line read back out of its batch and a message read back out of its row restate every one of the sixteen, so the identity and observation clocks a later step named survive the round trip.

`MarketColumn` is the matching canonical order for the nineteen facts `Market` answers: required `currency`, `unit`, `side`; then nullable `price`, `quantity`, `securityids`, `cficode`, `miccode`, `lastpx`, `lastqty`, `avgpx`, `cumqty`, `leavesqty`, `prevpx`, `prevqty`, `spotrate`, `forwardpoints`, `ticker`, `metadata`. The datatypes are the crate's decimal for every price and quantity, each [code](types/codes/index.md)'s own leaf - `ccy`, `unit`, `side`, `cfi`, `mic` - a sorted `map<utf8, utf8>` for the identifiers and the metadata, and `utf8` for the ticker. `OperationColumn` is the eight facts `Operation` adds, every one nullable: `marketoperationid` (`int32`), `tif` (`timeinforce`), `tradable` (`boolean`), `accountids`, `userids`, `altids` (each a sorted `map<utf8, utf8>`, `IdMap::dtype()`), `bid` and `ask` (each a nullable `struct<price, spotrate, forwardpoints, currency, quantity, unit>`, the children `OperationColumn::LANE_FIELDS`, built by `lane_datatype()`, written by `lane_fact` and read by `lane_of`). Both enums mirror `EventColumn` - `ALL`, `name`, `display`, `datatype`, `nullable`, `field`, `fields`, `of_name`, `fact` and `record` - a null clearing an optional fact and a cell that cannot be read as its declared fact leaving the element unchanged; `MarketColumn::fact` and `record` take any `Market`, `OperationColumn`'s any `Operation`.

The five book-control columns are `BookRef` as a row: `mdupdateaction` (`utf8`, the action's `as_str`), `bookscope` (`utf8`), `mdentrypositionno` (`uint32`), `mdentrypx` and `mdentrysize` (the crate's decimal), all nullable and all null on an operation that is no market-data entry. The [lifted row](#arrow) built from these is, in column order, `kind` + 16 event + 19 market + 8 operation + 5 book-control columns + `executions`, `bidside`, `askside`, `snapshotpartitions`, `live`, `deltas`; an operation nested in it is `kind` + 16 + 19 + 8 + 5; a side is 6 element columns + 19 + `live` + `deltas`.

### Arrow

`MarketData::field()` is the required `marketdata` struct: required `kind`, a `MarketKind` spelling, then `EventColumn::ALL` - every one nullable here, because an undated leaf states no clock - `MarketColumn::ALL`, `OperationColumn::ALL`, the five book-control columns, then the nullable nested columns: `executions`, a serie of operation rows a trade or a book fills; `bidside` and `askside`, a book's sides, each the six element facts - current and cross identities, cross code, current and cross hashes, sources - then `MarketColumn::ALL` and `live` and `deltas` series of operation rows; `snapshotpartitions`, a serie of `snapshotpartition` structs - nullable `symbol`, required `scope` - naming the scopes a book's last snapshot replaced, absent where the last change was an ordinary update; and `live` and `deltas`, a book side's own. Every fact is its own typed column and a leaf leaves null what it does not state; a composite trade has already been flattened into a book's `executions` before encoding.

`MarketData::arrow_reader(values, batch_row_size, batch_byte_size)` streams any `IntoIterator` of `MarketData` or `Result<MarketData>` into bounded `BatchReader` batches of that schema. It is lazy - the source is not pulled until the reader is - and lays every batch out column by column from the typed leaves, with no per-row `Scalar`. A missing row bound uses the shared default, zero normalizes to one row, and a byte bound closes a nonempty batch once the rows already held reach it, so one oversized row still travels alone. A value is written only as its canonical self: a stale derived identity or fact is refused at its row rather than written into a stream the inverse would reject, and a source or refusal error follows the completed prefix and fuses the reader.

`MarketData::from_arrow_reader(batches)` reads one value per row and is tolerant of the shape it is given: the batch's root columns are resolved by name once per stream, whatever their case, in any subset and any order; a column the row does not name is ignored, and a column of another castable type is cast through one plan compiled before the first batch, so a FIX lifecycle batch - its event and operation columns, its own foreign columns, a `kind` column added - reads as order and execution events. Two columns naming one fact are refused before a row is read. A row's `kind` names its leaf; a missing or unknown kind, or a fact the kind requires - `currunix` for a dated leaf, a trade's `executions`, a book's two sides - is refused at the first row that needs it. Each batch lands once as one record [`Serie`](types/serie.md) under the resolved root and every cell is read off it by row, so no cell is decoded twice; a value the schema refuses is named at the landing by its row and path - `$[0].bidside.live[0].miccode` - before any leaf is rebuilt. A trade is rebuilt only through `TradeEvent::from_parts`, and a book's live depth and deltas directly rather than by replaying the deltas as new mutations; every identity a row states must be the one the rebuilt leaf derives, and an identity column that stands must state one - a null `curruuid`, `crossuuid`, `currhashcode` or `crosshashcode` is refused, an absent column states nothing - while every other fact a row states must be the one that leaf settles on, a null cell of one stating nothing. A reader failure or a refused row is returned once and fuses the iterator.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{BookEvent, Element, Event, MarketData, Order, OrderEvent};

    let mut order = OrderEvent::at(2_000_000);
    order.set_crosscode("O-1".to_owned());
    order.finalize();
    // A value is written only as its canonical self: finalized.
    let mut undated = Order::new();
    undated.finalize();
    let values = vec![
        MarketData::from(undated),
        MarketData::from(order.clone()),
        MarketData::from(BookEvent::new(3_000_000, "IBM")),
    ];

    // Written column by column, read back value for value.
    let batches: Vec<RecordBatch> =
        MarketData::arrow_reader(values.clone(), None, None)?.collect::<Result<_, _>>()?;
    let read: Vec<MarketData> =
        MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
            .collect::<yggdryl::Result<_>>()?;
    assert_eq!(read, values);

    // A foreign shape: a few columns in another order, one it does not name.
    let written = MarketData::arrow_reader([MarketData::from(order.clone())], None, None)?
        .next()
        .expect("one batch")?;
    let mut fields = Vec::new();
    let mut columns: Vec<ArrayRef> = Vec::new();
    for name in ["crosscode", "currunix", "kind"] {
        let index = written.schema().index_of(name)?;
        fields.push(written.schema().field(index).clone());
        columns.push(written.column(index).clone());
    }
    fields.push(Field::new("msgtype", DataType::Utf8, true));
    columns.push(Arc::new(StringArray::from(vec!["D"])));
    let foreign = RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)?;
    let lifted: Vec<MarketData> =
        MarketData::from_arrow_reader(batch_reader(foreign.schema(), [foreign]))?
            .collect::<yggdryl::Result<_>>()?;
    let event = lifted[0].as_order_event().expect("an order event");
    assert_eq!((event.get_crosscode(), event.get_currunix()), ("O-1", 2_000_000));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import graph

    order = graph.OrderEvent(2_000_000, crosscode="O-1")
    values = [graph.Order(), order, graph.BookEvent(3_000_000, "IBM")]

    # Written column by column, read back value for value.
    reader = graph.MarketData.arrow_reader(values)
    assert isinstance(reader, pa.RecordBatchReader)
    read = list(graph.MarketData.from_arrow_reader(reader))
    assert read == [graph.MarketData(value) for value in values]

    # A foreign shape: a few columns in another order, one it does not name.
    foreign = pa.table(
        {
            "crosscode": ["O-1"],
            "currunix": pa.array([2_000_000], pa.int64()),
            "kind": ["order_event"],
            "msgtype": ["D"],
        }
    )
    [lifted] = graph.MarketData.from_arrow_reader(foreign)
    event = lifted.as_order_event()
    assert event is not None and (event.crosscode, event.currunix) == ("O-1", 2_000_000)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, graph } = require('yggdryl')

    const order = new graph.OrderEvent(2_000_000n, { crosscode: 'O-1' })
    const values = [new graph.Order(), order, new graph.BookEvent(3_000_000n, 'IBM')]

    // Written column by column, read back value for value.
    const read = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(values))]
    assert.equal(read.length, 3)
    read.forEach((value, at) => assert.ok(value.intoLeaf().equals(values[at])))

    // A foreign shape: a few columns in another order, one it does not name.
    const foreign = new arrow.Table({
      crosscode: arrow.vectorFromArray(['O-1'], new arrow.Utf8()),
      currunix: arrow.vectorFromArray([2_000_000n], new arrow.Int64()),
      kind: arrow.vectorFromArray(['order_event'], new arrow.Utf8()),
      msgtype: arrow.vectorFromArray(['D'], new arrow.Utf8()),
    })
    const [lifted] = graph.MarketData.fromArrowReader(BatchReader.from(foreign))
    const event = lifted.asOrderEvent()
    assert.equal(event.crosscode, 'O-1')
    assert.equal(event.currunix, 2_000_000n)
    ```

### Lifecycle walk

`EventIterator::new(elements, sorted: bool)` over any `Event + Operation + Clone` values - a [FIX lifecycle message](fix/lifecycle.md), an operation leaf - or over `MarketData`, an `Iterator` of the same type. Of a `MarketData`, the `OrderEvent`, `QuoteEvent`, `ExecutionEvent` and `TradeEvent` variants walk and every other variant is yielded unchanged, in place, never entering the live set; the walk: before placement it fills an absent `execunix` with `currunix` only where `Event::is_execution` says that incoming observation reports an execution - a parsed FIX message arrives with it already filled; the default recognizes `40PARTFILL`, `40TRADE` and `80FILLED`. FIX additionally recognizes an initial `35=AE` only when `TradeReportTransType(487)` is absent or New and `ExecType(150)` is absent or execution-like (`F`, with legacy `1` and `2`); it rejects a non-New or cancel/correct/reverse/status AE and every `AD`, `AQ` or `AR`. Following then carries the latest execution clock through non-execution events and keeps the later clock when another execution arrives; it never fills or carries `recdunix`. The walk keeps the elements still alive - a live state, not past their expiration - under the identity every incarnation of one thing shares, the cross element, and an element arriving under a live identity - or, where its own is alive under nothing, under a name a live element goes by, the `(key, value)` pairs of its `get_altids()`, so a report spelling only the `CLORDID` a live order was placed under still finds it - is stated as the one after it by its own `with_previous`, which is what the walk yields; the yielded element then stands as the live one where it is still alive and retires the identity where it is not. An element arriving under the identity the live element *arrived* under is its twin, and is yielded `restating` it. `sorted` takes the caller's word that the elements arrive in their own order and streams them; otherwise the walk collects and stably sorts them by `is_after` / `is_before` first. A finite deadline emits one owned `95EXPIRED` event at that exact instant, following the live generation, then purges it; stale deadlines of replaced or terminal generations emit nothing. Deadlines win ties, then every source event at that instant is processed, then grid views are emitted. `with_snapshot_ns(i64)` gives the walk an epoch-aligned grid - zero or less means none - and yields one owned view of every living identity at each crossed tick; a view changes only `snapunix`, keeping the source identity, sequence and content, and no source event is backdated to a prior tick. At EOF the walk drains finite deadlines and views through the greatest deadline still reached, or through the last source instant where none remains; a nonexpiring identity never extends a finite source. `alive()` reads the live elements, in no order, and `snapshot_ns()` the grid where there is one.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, EventIterator, MarketData, Order, OrderEvent};

    let placed = |unix: i64| {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1".to_owned());
        order.finalize();
        MarketData::from(order)
    };
    let arrived = vec![placed(1_000_000), MarketData::from(Order::new()), placed(2_000_000)];
    let walked: Vec<MarketData> = EventIterator::new(arrived, true).collect();
    assert_eq!(walked[1], MarketData::from(Order::new()), "an undated value passes through");
    let second = walked[2].as_order_event().expect("an order event");
    assert_eq!(second.get_prevuuid(), Some(walked[0].get_curruuid()));
    assert_eq!(second.get_seqnum(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import graph

    def placed(unix: int) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode="O-1")

    walked = list(graph.EventIterator([placed(1_000_000), graph.Order(), placed(2_000_000)]))
    assert walked[1] == graph.MarketData(graph.Order()), "an undated value passes through"
    second = walked[2].as_order_event()
    assert second is not None
    assert second.prevuuid == walked[0].curruuid
    assert second.seqnum == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const placed = (unix) => new graph.OrderEvent(unix, { crosscode: 'O-1' })
    const walked = [...new graph.EventIterator([placed(1_000_000n), new graph.Order(), placed(2_000_000n)])]
    assert.ok(walked[1].equals(new graph.MarketData(new graph.Order())), 'an undated value passes through')
    const second = walked[2].asOrderEvent()
    assert.equal(second.prevuuid, walked[0].curruuid)
    assert.equal(second.seqnum, 1)
    ```

### Book fold

`BookIterator::new(values, snapshot_millis, global)` takes `MarketData` or `Result<MarketData>` items - any `I::Item: Into<Result<MarketData>>` - already sorted by event order, folds what `BookEvent::add_operations` folds and refuses any other variant by its kind at `$.operation.kind`, and refuses a timestamp regression; `global()` answers the mode. An input error follows every completed timestamp prefix exactly once and then fuses the iterator; an infallible stream uses the same path through `Into<Result<_>>`. It groups by the effective instant - an explicitly supplied `snapunix`, otherwise `currunix` - and emits one cloned book per touched symbol in symbol order. Each timestamp and symbol commits atomically. Ordinary retained groups use `BookEvent::add_operations` directly without an extra full-depth candidate clone; groups with supplied snapshot membership stage that replacement and their deltas and expirations together. Each symbol's lifecycle is isolated before global consolidation, and the scope-aware live key keeps repeated IDs distinct there too. A failed symbol group yields its error and no stale or empty `Ok` book. A live order or quote with `exprtime` produces a terminal delete for that generation at the exact instant before equal-time source operations, then leaves depth without clearing its snapshot partition; executions and trades never enter retained lifecycle state. Outside global mode every input must state a `ticker`; global mode combines them under `GLOBAL_SYMBOL`, whose value is `GLOBAL`. `snapshot_millis == 0` disables the epoch-aligned grid; any positive value is checked before conversion to nanoseconds.

The iterator produces its grid directly from retained books: every crossed epoch-aligned tick before a source group emits the complete live book with `snapunix` set, empty deltas and no historical executions. A snapshot is the exact same `BookEvent` struct with every living order kept and everything else purged - a dead order is a delta of the book it died in and no part of any later one - and with `snapshot_millis == 0` there is no grid, so every emitted book keeps all living orders beside its own deltas. At a tick equal to a source or expiration instant, those operations are applied first and one complete tick is emitted for touched and untouched books alike. An explicitly supplied `snapunix` stream is a membership view rather than a second delta: its order and quote rows replace only the represented `(symbol, scope)` partitions and can therefore create or update depth even when the stream begins with that view; keys absent from each represented partition are purged, while other scopes and other symbols in a global book remain intact. A `SnapshotEvent` represents an empty partition. A snapshotted execution is reported at the view's effective instant, retains its precise `execunix`, and does not replace depth. A snapshotted composite trade rebases the root and every child `currunix` to that effective instant before finalization while retaining each child's precise `execunix`; its children are then reported the same way. Every supplied component must have happened no later than its snapshot instant; a future row is refused rather than silently backdated. This supplied membership view is distinct from a FIX `W` partition replacement. A failed same-time source group leaves its book and pending expirations intact, so the expiry is retried independently after the located error. After each emitted book, side deltas and executions are cleared in the retained internal book, so the next result carries only the changes and executions at its own effective timestamp while resting depth persists.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookIterator, BookSide, Element, Event, Market, MarketData, OrderEvent};
    use yggdryl::{Decimal18, Side, State};

    let bid = |code: &str, unix: i64, price: i64| {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("IBM".into()));
        order.set_side(Side::Buy);
        order.set_price(Some(Decimal18::from_int(price)));
        order.set_quantity(Some(Decimal18::from_int(1)));
        order.set_state(State::from_spelling("New").expect("a shipped state"));
        order.finalize();
        MarketData::from(order)
    };
    let books = BookIterator::new([bid("B-1", 1_000, 100), bid("B-2", 2_000, 101)].into_iter(), 0, false)?
        .collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(books.len(), 2, "one book per touched instant");
    assert_eq!(books[1].bid().len(), 2, "depth persists");
    assert_eq!(books[1].bid().best_price(), Some(Decimal18::from_int(101)));
    assert_eq!(books[1].bid().deltas().len(), 1, "a book carries its own instant's changes");

    // A value a book does not fold is refused by its kind.
    let side = MarketData::from(BookSide::new(Side::Buy)?);
    let mut refused = BookIterator::new([side].into_iter(), 0, false)?;
    assert!(refused.next().expect("one result").is_err());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    def bid(code: str, unix: int, price: int) -> graph.OrderEvent:
        return graph.OrderEvent(
            unix, crosscode=code, ticker="IBM", side="BUY", price=Decimal(price), quantity=1, state="NEW"
        )

    books = list(graph.BookIterator([bid("B-1", 1_000, 100), bid("B-2", 2_000, 101)]))
    assert len(books) == 2, "one book per touched instant"
    assert len(books[1].bid) == 2, "depth persists"
    assert books[1].bid.best_price is not None and books[1].bid.best_price.as_py() == Decimal(101)
    assert len(books[1].bid.deltas) == 1, "a book carries its own instant's changes"

    # A value a book does not fold is refused by its kind.
    try:
        list(graph.BookIterator([graph.BookSide("BUY")]))
    except ValueError as error:
        assert "book_side" in str(error)
    else:
        raise AssertionError("a book side was folded")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const bid = (code, unix, price) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'IBM', side: 'BUY', price: String(price), quantity: 1, state: 'NEW',
    })
    const books = [...new graph.BookIterator([bid('B-1', 1_000n, 100), bid('B-2', 2_000n, 101)])]
    assert.equal(books.length, 2, 'one book per touched instant')
    assert.equal(books[1].bid.length, 2, 'depth persists')
    assert.equal(books[1].bid.bestPrice, '101')
    assert.equal(books[1].bid.deltas.length, 1, "a book carries its own instant's changes")

    // A value a book does not fold is refused by its kind.
    assert.throws(() => [...new graph.BookIterator([new graph.BookSide('BUY')])], /book_side/)
    ```

## Edges

- `set_srcuuids(Vec::new())` unsays the sources: a source is stated where an element is read out of another - a FIX message out of a `TextLine`. Every nonempty source list is sorted and deduplicated at the setter, so merge order cannot change serialized output. Following and restating preserve the observation's own sources; merging statements of one element forms their sorted unique union. `set_crosscode(String::new())` unsays the cross code and puts the cross element back on the element's own identity. A dated leaf's own setters eagerly reproject its current UUID whenever the cross code or cross hash changes, because the cross code is inside the content digest the identity is packed from; a book side reaches a crate-private plain store instead, so its identity stays content-derived rather than dated, and a foreign event that owns an assigned identity may define its setters to preserve that assignment instead. `set_altids(IdMap::new())` unsays the names an operation goes by, and `set_securityids(SecurityIds::default())` its identifiers; `insert_*` fills only an absent key and answers `false` for a held one, and a [FIX message](fix/message.md#the-identifier-maps) refuses a key no field of its dictionary states.
- `is_after` and `is_before` are one order - an undated element and a book side state none, and a `MarketData` states one only between two dated values: an element is never after itself, and two neither after nor before each other are equal in it - which a sort of elements, the walk's included, treats as a tie and keeps in arrival order.
- `following` refuses an element as its own predecessor and a predecessor that happened later; an equal instant follows, a later call replaces the predecessor recorded before, and a place past `u64::MAX` saturates. Following what the element already follows answers nothing once its inherited facts are present; `following_market` and `following_operation` still fill missing facts when the timed link is unchanged. A predecessor's cross code is forced onto the follower, so a report naming only the venue's `OrderID` joins the chain the order opened under its `ClOrdID` and takes that code.
- `restating` never refuses: it is the caller's word that the event is the live one's twin. The walk gives that word only for an arrival under the identity the live element arrived under - a finalized copy of the same instant and content - and never for a later statement, which chains.
- `merging` refuses another element outright. The later `recdunix` selects the reference even when its event instant is earlier; equal or absent recording clocks fall back to event time, and an exact tie keeps this element leading. The merged `execunix` and `recdunix` are still the earliest facts observed, so a merged statement ranks by its earliest recording against a third: merging `a` with `b` and then `c` can choose another reference than merging `a` with the result of `b` and `c`. A restatement that adds nothing answers nothing.
- A lane slot already stated is never overwritten by `fill_lanes`, only a stated fact fills one - no price or quantity, no currency and a unit of none fill nothing - and a side that takes no lane - a cross, `OPPOSITE`, `UNKNOWN` - fills none. `set_bid(Some(Lane::default()))` states nothing and lands as `None`.
- A book folds only a dated operation, a trade or a snapshot control: `BookEvent::add_operations` refuses an undated leaf, a `BookSide` or a `BookEvent` at `$.operations[index].kind` and `BookIterator` at `$.operation.kind`, naming the kind it got. A `TradeEvent` is built through `TradeEvent::from_parts`, and a row whose `kind` is `trade_event` decodes only through it.
- `currunix` is signed: an instant before the epoch is negative, and the `i64` holds every nanosecond count a clock this crate reads states. `time_uuid` refuses a pre-epoch instant; every nonnegative `i64` nanosecond instant fits UUIDv7's 48-bit millisecond timestamp. `finalized` keeps the prior identity on refusal.
- A state is never absent: an element that reached none says `00UNKNOWN`, the code that means exactly that. A security identifier is absent where the market names none, and never a spelling its source refuses: `SecurityId::new` validates the code by its key, and `SecType::read` refuses `TICKER`, which belongs on `set_ticker`.
- The traits validate nothing else: a predecessor or a source that no element answers to is the holder's to refuse, not the element's.
- A source element is cloned twice at most - once to hold it as the live one, once to keep one its predecessor refuses. Each expiration and grid view is an additional owned clone; the walk holds one live element per identity and emits views lazily rather than queueing a whole grid.
- A grid starts at the first epoch-aligned tick at or after the first source instant. Its output count is the number of crossed ticks times the identities alive at each one; the caller chooses that width, and the iterator applies no implicit count or time-span cap.
