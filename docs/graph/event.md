# Event

`Event: Element` is an element that happened at one instant, stands in one state and has a place among the events of that instant; `EventIterator` is the walk that chains events.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Event` (`graph::element`); `EventIterator` walk (`yggdryl_market::graph::iterator`); Rust-only - dated [leaves](index.md#leaves) answer it in Python/JavaScript |
| `transunix` | `get_transunix`/`set_transunix`: the transaction instant - when the operation really happened, the identity and order axis - nanoseconds since the Unix epoch, UTC, signed |
| `state` | `get_state`/`set_state`: the lifecycle-sorted [`State`](../types/enum/state.md) member, never absent - `UNKNOWN` where none reached |
| `is_execution` | provided via `State::is_execution` (`PARTIALLY_FILLED`, `TRADE`, `FILLED`); overridable where lifecycle state and report kind differ - an operation leaf's kind decides |
| `seqnum` | `get_seqnum`/`set_seqnum`: its place among the events of its instant (same `transunix`) - zero for the first of a run, the events a stream hands over at that instant one after another, one more for each next, the next run starting again at zero, a stream coming back to an instant it left included; outside the content code. The FIX parse places by order; the [FIX lifecycle](../fix/lifecycle.md#a-place-counts-one-instant) places by content, after the expirations it hands over at that instant - a content repeated at an instant takes the place it already took there; market leaves take their message's place; `EventIterator` keeps each source element's own place and places only its expirations; a text line's place is its row number |
| Clocks | `creaunix`, `sendunix`, `exprunix`, `prevunix`, `snapunix` (`Option<i64>`) and `prevuuid` (`Option<Uuid>`), each `get_`/`set_`, stated only where known: `creaunix` when its lifecycle was created, `sendunix` the technical wire clock - when the message crossed the wire, the earliest its statements know (what a merge ranks by), `exprunix` the deadline, `prevunix`/`prevuuid` the predecessor - a [text line](../media/text.md) states `prevunix` alone - `snapunix` the instant a grid view's content was stated at - the original the view copies, never the tick it is dated at nor a predecessor's - and no digest reads it. `execunix`, when an element last executed, is no event's clock: it is a [market fact](market.md#contract), so a text line states none |
| `digest_event` | provided: continues [`Element::digest`](element.md#contract) with the state and predecessor's identity; no place, no instant fed |
| `fold_lifecycle` | provided, `(&mut self, &Self) -> bool`: earliest creation, latest expiration, the further state ([`State::merge_with`](../types/enum/state.md#the-further-along-stands)) |

## Identity

| Reading | Rule |
| --- | --- |
| `txhash() -> Result<TxHash>` | couples `transunix` (ns) with `hashcode` as its digest ([TxHash](../hashing.md)) |
| `time_uuid() -> Result<Uuid>` | `TxHash::into_sequenced_uuid(seqnum, crosshashcode)`: UUIDv7 (RFC 9562) - ms-floored instant, `seqnum.min(4095)` in the 12-bit `rand_a`, `rand_b` = low 62 bits of XXH3(`hashcode`+sequence) seeded by the cross hash; sorts by ms then place - two instants inside one ms each count from 0, so the later may sort first; places past 4095 only probabilistically distinct; seed separates cross chains; pre-epoch = `InvalidRecord`, never truncated |
| `finalized(&mut self, hashcode)` | provided: records the code, resets `uuid` to `time_uuid` (kept without one) and `crossuuid` to `cross_uuid()`; every `finalize` hands its digest here; a foreign implementor with an assigned identity may skip it |

## Following

`following(self, &Self) -> Option<Self>`, provided; what `with_previous` delegates to:

| Moves | Rule |
| --- | --- |
| The link | predecessor's identity/instant as `prevuuid`/`prevunix`; `seqnum` stays this event's own unless the predecessor happened at the same instant or later, where it is the higher of its own and one past the predecessor's (saturating); chain history stops at `prevuuid` |
| The cross code | the predecessor's is forced on where this event's differs, stored under this event's own kind and side (`{kind}:{side}:{base}`) - one chain shares one base |
| The lifecycle | earliest creation either knows; this event's explicit expiration else the predecessor's (can shorten a deadline); the furthest state |
| Nothing else | the transaction instant, the wire clock, sources and snapshot move nowhere |
| Refusals | nothing for its own predecessor, one that happened after it, or no change; an equal instant follows |

An event that moved is finalized; [`Market::following_market`](market.md#following-and-merging)/[`Operation::following_operation`](operation.md#following-and-merging) continue it before finalizing - the side, the chain's metadata and, for an operation, its identifiers and accounts among what they take.

## Restating

`restating(self, live: &Self) -> Self`, provided: another statement of `live` (same instant/content read again), taking `live`'s predecessor, place, snapshot and cross code, never sources; lifecycle folded, earliest `sendunix` kept, then finalized - one identity, chain unchanged. A market event also takes the market's place, an operation event the operation's ([Market](market.md#following-and-merging), [Operation](operation.md#following-and-merging)). The caller must give `live` under the identity it *arrived* under - following moved it; the [walk](#lifecycle-walk) does this.

## Merging

`merging(self, &Self) -> Option<Self>`, provided; what `merge_with` delegates to:

| Moves | Rule |
| --- | --- |
| The reference | the statement sent last, the later `sendunix` (stated beats unstated; equal/absent falls back to the greater `transunix`; an exact tie keeps `self`) |
| From the reference | cross code, transaction instant `transunix` and `hashcode`, predecessor, snapshot; the other fills what it leaves unstated |
| Folded | sources: position-independent sorted-unique union; the higher place; `fold_lifecycle`; `sendunix` = earliest of either |
| Refusals | nothing for another element (different `uuid`) or no change |

Merge order can change which of three statements leads (ranked by the earliest `sendunix` kept). [`Market::merging_market_event`](market.md#following-and-merging)/[`Operation::merging_operation_event`](operation.md#following-and-merging) continue it.

## Examples

### A chain

An Apple order placed, then partly filled a second later.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::{Market, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal, State};
    use yggdryl_market::{IdKey, IdType, Identifier, Side};
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let event = |unix: i64, state: &str| -> yggdryl::Result<OrderEvent> {
        let mut event = OrderEvent::at(unix);
        event.set_crosscode("O-1001".to_owned());
        event.set_state(State::from_spelling(state).expect("a shipped state"));
        event.set_side(Side::Buy, true);
        event.set_price(Some("189.50".parse()?), true);
        event.set_quantity(Some(Decimal::from_int(100)), true);
        event.set_currency(Ccy::new("USD")?, true);
        event.insert_identifier(Identifier::new(IdKey::base(IdType::OrderId), "O-1001")?)?;
        event.finalize();
        Ok(event)
    };
    let placed = event(T, "New")?;
    // UUIDv7: the millisecond 1_700_000_000_000 leads the identity.
    assert_eq!(placed.get_uuid(), placed.time_uuid()?);
    assert!(placed.get_uuid().to_string().starts_with("018bcfe5-6800-7"));
    assert_eq!(placed.txhash()?.unix(), T);
    assert_eq!(placed.get_state().as_str(), "NEW");

    let filled = event(T + 1_000_000_000, "PartiallyFilled")?;
    assert!(filled.is_after(&placed) && placed.is_before(&filled));
    let filled = filled.with_previous(&placed).expect("a later event follows");
    assert_eq!(filled.get_prevuuid(), Some(placed.get_uuid()));
    assert_eq!(filled.get_prevunix(), Some(T));
    // A later instant is a place of its own: the first there.
    assert_eq!(filled.get_seqnum(), 0);
    assert_eq!(filled.get_crossuuid(), placed.get_crossuuid(), "one chain");
    assert_eq!(filled.get_prevpx(), Some("189.50".parse()?), "the step before");
    // UUIDv7s sort by their millisecond first.
    assert!(placed.get_uuid() < filled.get_uuid());

    // An event follows neither itself nor one that happened after it.
    assert!(event(T, "New")?.with_previous(&placed).is_none());
    assert!(placed.clone().with_previous(&filled).is_none());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Identifier, State, graph

    T = 1_700_000_000_000_000_000

    def event(unix: int, state: str) -> graph.OrderEvent:
        return graph.OrderEvent(
            unix,
            crosscode="O-1001",
            state=state,
            side="BUYS",
            price=Decimal("189.50"),
            quantity=100,
            currency="USD",
            identifiers=[Identifier("orderid", "O-1001")],
        )

    placed = event(T, "NEW")
    # UUIDv7: the millisecond 1_700_000_000_000 leads the identity.
    assert placed.uuid.as_py().startswith("018bcfe5-6800-7")
    assert placed.state is State.NEW

    filled = event(T + 1_000_000_000, "PARTIALLY_FILLED")
    assert filled.is_after(placed) and placed.is_before(filled)
    followed = filled.with_previous(placed)
    assert followed is not None
    assert followed.prevuuid == placed.uuid
    assert followed.prevunix == T
    # A later instant is a place of its own: the first there.
    assert followed.seqnum == 0
    assert followed.crossuuid == placed.crossuuid, "one chain"
    assert followed.prevpx is not None and followed.prevpx.as_py() == Decimal("189.50")

    # An event follows neither itself nor one that happened after it.
    assert event(T, "NEW").with_previous(placed) is None
    assert placed.with_previous(followed) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const event = (unix, state) => new graph.OrderEvent(unix, {
      crosscode: 'O-1001',
      state,
      side: 'BUYS',
      price: '189.50',
      quantity: 100,
      currency: 'USD',
      identifiers: [new Identifier('orderid', 'O-1001')],
    })

    const placed = event(T, 'NEW')
    // UUIDv7: the millisecond 1_700_000_000_000 leads the identity.
    assert.ok(placed.uuid.startsWith('018bcfe5-6800-7'))
    assert.equal(placed.state, 'NEW')

    const filled = event(T + 1_000_000_000n, 'PARTIALLY_FILLED')
    assert.ok(filled.isAfter(placed) && placed.isBefore(filled))
    const followed = filled.withPrevious(placed)
    assert.equal(followed.prevuuid, placed.uuid)
    assert.equal(followed.prevunix, T)
    // A later instant is a place of its own: the first there.
    assert.equal(followed.seqnum, 0)
    assert.equal(followed.crossuuid, placed.crossuuid, 'one chain')
    assert.equal(followed.prevpx, '189.5')

    // An event follows neither itself nor one that happened after it.
    assert.equal(event(T, 'NEW').withPrevious(placed), null)
    assert.equal(placed.withPrevious(followed), null)
    ```

### Two hops of one message

The same fill report, sent through a gateway at +2ms and an OMS at +5ms, each from its own log line.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::OrderEvent;
    use yggdryl::{State, Uuid};
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    let report = |sent: Option<i64>, line: u128| {
        let mut event = OrderEvent::at(T + 1_000_000_000);
        event.set_crosscode("O-1001".to_owned());
        event.set_state(State::from_spelling("PartiallyFilled").expect("a shipped state"));
        event.set_sendunix(sent);
        event.set_srcuuids(vec![Uuid::from_v8(line)]);
        event.finalize();
        event
    };
    let gateway = report(Some(T + 1_002_000_000), 1);
    let oms = report(Some(T + 1_005_000_000), 2);
    // Wire clocks and sources are not content: one event.
    assert_eq!(gateway.get_uuid(), oms.get_uuid());

    // Merging: the later `sendunix` leads, the earliest one stays,
    // and the sources are unioned.
    let merged = oms.clone().merge_with(&gateway).expect("another statement");
    assert_eq!(merged.get_sendunix(), Some(T + 1_002_000_000));
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(2)]);

    // Restating: once the gateway's statement followed the order, its
    // identity moved; the OMS statement takes its place and identity.
    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.finalize();
    let live = gateway.with_previous(&placed).expect("a later event follows");
    let twin = oms.restating(&live);
    assert_eq!((twin.get_uuid(), twin.get_seqnum()), (live.get_uuid(), live.get_seqnum()));
    assert_eq!(twin.get_sendunix(), Some(T + 1_002_000_000));
    assert_eq!(twin.get_srcuuids(), [Uuid::from_v8(2)], "sources stay its own");
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    LINE_1 = "018bcfe5-6800-7000-8000-000000000001"
    LINE_2 = "018bcfe5-6800-7000-8000-000000000002"

    def report(sent: int, line: str) -> graph.OrderEvent:
        return graph.OrderEvent(
            T + 1_000_000_000, crosscode="O-1001", state="PARTIALLY_FILLED", sendunix=sent, srcuuids=[line]
        )

    gateway = report(T + 1_002_000_000, LINE_1)
    oms = report(T + 1_005_000_000, LINE_2)
    # Wire clocks and sources are not content: one event.
    assert gateway.uuid == oms.uuid

    # Merging: the later `sendunix` leads, the earliest one stays,
    # and the sources are unioned.
    merged = oms.merge_with(gateway)
    assert merged is not None and merged.sendunix == T + 1_002_000_000
    assert [source.as_py() for source in merged.srcuuids] == [LINE_1, LINE_2]

    # Restating: once the gateway's statement followed the order, its
    # identity moved; the OMS statement takes its place and identity.
    live = gateway.with_previous(graph.OrderEvent(T, crosscode="O-1001"))
    assert live is not None
    twin = oms.restating(live)
    assert (twin.uuid, twin.seqnum) == (live.uuid, live.seqnum)
    assert twin.sendunix == T + 1_002_000_000
    assert [source.as_py() for source in twin.srcuuids] == [LINE_2], "sources stay its own"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const LINE_1 = '018bcfe5-6800-7000-8000-000000000001'
    const LINE_2 = '018bcfe5-6800-7000-8000-000000000002'
    const report = (sent, line) => new graph.OrderEvent(T + 1_000_000_000n, {
      crosscode: 'O-1001', state: 'PARTIALLY_FILLED', sendunix: sent, srcuuids: [line],
    })

    const gateway = report(T + 1_002_000_000n, LINE_1)
    const oms = report(T + 1_005_000_000n, LINE_2)
    // Wire clocks and sources are not content: one event.
    assert.equal(gateway.uuid, oms.uuid)

    // Merging: the later `sendunix` leads, the earliest one stays,
    // and the sources are unioned.
    const merged = oms.mergeWith(gateway)
    assert.equal(merged.sendunix, T + 1_002_000_000n)
    assert.deepEqual(merged.srcuuids, [LINE_1, LINE_2])

    // Restating: once the gateway's statement followed the order, its
    // identity moved; the OMS statement takes its place and identity.
    const live = gateway.withPrevious(new graph.OrderEvent(T, { crosscode: 'O-1001' }))
    const twin = oms.restating(live)
    assert.equal(twin.uuid, live.uuid)
    assert.equal(twin.seqnum, live.seqnum)
    assert.equal(twin.sendunix, T + 1_002_000_000n)
    assert.deepEqual(twin.srcuuids, [LINE_2], 'sources stay its own')
    ```

## Lifecycle walk

`EventIterator::new(elements, sorted)` walks any `Event + Operation + Clone` (an operation leaf, a [FIX lifecycle message](../fix/lifecycle.md)) or `MarketData`, yielding the same type.

| Key | Rule |
| --- | --- |
| Live set, twins | elements still alive (a live state, not past expiration) share the cross identity, and a chain is keyed by that identity and its [`marketdatakind`](../types/enum/marketdatakind.md) - an element joins only a chain of its own kind, by its cross code, a base code or a shared name, so an order and an execution under one cross code are two chains and a fill never restates, follows or ends its order; an arrival citing one live chain - under its live identity, by the one side alive under its base code, or by a [name](#names-re-keying-and-conflicts) a live element goes by - is yielded as `with_previous` of the live one, live until it isn't, then retiring; one arriving under the identity the live element *arrived* under is yielded [`restating`](#restating) it instead, a twin taking the chain's metadata and identifiers as a follower does; and one arriving under the identity a statement the chain moved past *at the live element's own instant* arrived under is yielded restating that statement - the same identity again - while the chain stays where it moved: the walk keeps those statements until the chain moves to a later instant, the only place a sorted walk reads another statement of them; a chain a step ends keeps that step and the statements it moved past at the step's instant the same way, each restated where it arrives again and the chain staying ended, until the walk reads a later instant |
| Sides | an order's or an execution's stored cross code states its [side](market.md#sides-and-cross-codes) (`10:1:O-7` to buy, `10:2:O-7` to sell), so a buy and a sell under one identifier are two chains, and a name is alive on each side apart: an arrival joins a live element of its own side it shares a name with. A quote's chain is keyed by no side - its code states `0` - and a name it goes by is alive on the leg its latest statement tags, so a bid and an offer going by one name are two chains. A name is matched on its side whenever both the arrival and the holder state one, and on the instrument ([`instcode`](instrument.md)) whenever both state one, so entries of two instruments going by one `mdentryid` stay apart and an arrival stating neither cites every holder |
| No side | an element stating `UKNW` cites the one side alive under its base cross code (a live order's or execution's code without its `{kind}:{side}:` prefix) where exactly one is - two cite nothing by the base - and every side a name of it is alive on: citing one chain it joins it, taking that chain's side and code; a name alive on both sides is a [conflict](#names-re-keying-and-conflicts) |
| Names | an arrival matches the live chain of its kind, side and instrument sharing one identifier of the same type and value ([`IdType::is_chain_name`](identifier.md#parentage)), whatever its source: a chain goes by every value of a chain identity type ([`IdType::is_chain_identity`](identifier.md#parentage)) its statements stated - the old value beside the new after a change - by the chain's first value a lineage identifier names (`origclordid`, `origorderid`, `origtradeid`, `tradereportrefid`), filed under its base too, and, for any other type - a report's own `execid`, a `tvtic`, a bridge's own word - by the value its live statement holds alone, released by the next statement holding another or none; each name is held by the first live chain of its kind, side and instrument that stated it until that chain ends, when the next to state it takes it. An identifier many elements share names no chain - a match's `trdmatchid`, which both orders it filled state, a request many chains answer (`quotereqid`, `mdreqid`), a parent order's slot (`parentorderid`, the previous value the walk writes, a bridge's `parentclordid`) - and neither does a security or a party, so the index holds one entry per chain identity a live chain stated and one per other type its live statement holds, never one per report, and a name's slot one chain per kind, side and instrument |
| Conflict | an element citing two live chains - its own and another, or two by its names - is joined to neither: it stands under its own identity, its own live chain where that is one of those cited, else a chain of its own, and the conflict is told twice - on the element by [`Operation::note_conflict(cited)`](operation.md#following-and-merging), before anything restates or finalizes it (a FIX message records a `FixAnomaly` under `crosscode`, a typed leaf nothing), `cited` naming its own stored code and each chain's with the name that cited it, and once per kind as a [warning](../fix/capture.md#warnings) on `yggdryl.graph.iterator`; another statement of it resolves the same way and restates it. Nothing merges two chains |
| Re-key | every element the walk states as a chain's - a follower, a twin, one before the live element, an expiration - takes the live element's side where it is sided and states none, then the live element's stored cross code where that states one ([`Operation::follow_identity`](operation.md#following-and-merging)), finalized once where that moved anything, so its `crosshashcode` and `crossuuid` derive from the chain's code and its `uuid`, seeded by that hash, is derived once under them: an identifier change moves no element onto another code and no order onto another [book](book.md#book-fold) entry |
| `UPDATED` | a `NEW` stated over a live element that is new-like - [`State::is_new_like`](../types/enum/state.md): acknowledged or working (rank 20 or 30), or `UPDATED`, `REPLACED`, `RESTATED`, `AMENDED` - is yielded `UPDATED` (`3004`, rank 30), read before following folds the state, so later progress folds over it; a `PENDING_NEW` followed by `NEW` stays `NEW` |
| Fills | an order's chain (`ORDR`) counts the fills its statements report once each, by [`Operation::fill_of`](operation.md#following-and-merging) - `Fill::New { execid, qty }`, `Bust { refid }`, `Correct { refid, qty }`, `Unidentified { qty }`, `NotAFill` - read off each statement's own words before it follows: a fill whose `execid` or `secondaryexecid` the chain already counted adds nothing, moves no quantity and promotes nothing; a new one adds its quantity, a bust takes the fill it names back, a correction replaces it, an unidentified fill counts nowhere, warned. The chain's first stated `cumqty` anchors the count - what traded before the walk saw the chain - and from there `cumqty` is the anchor plus the count and `leavesqty` the accepted `ordqty` (stated by a statement awaiting no answer) less it, floored at nothing: a stated total that disagrees is warned once per kind and adopted nowhere, an overfill warned and promoting nothing. A partial-fill-like state - `IN_PROGRESS`, `PARTIALLY_FILLED`, `TRADE`, or the `UPDATED`, `REPLACED`, `RESTATED`, `AMENDED` the fold carries a fill in - whose count reaches `ordqty` after a fill, a bust or a correction reads `FILLED` and ends the chain, never while a replace or a cancel awaits its answer; a stated `FILLED` stays, warned where the count falls short, while one that is no word of its holder's own ([`Operation::states_end`](operation.md#following-and-merging) false) reads, over a chain the walk counted, as the partial fill the count says. A repeated fill leaves the hidden part where it was. A quote, an execution and a trade count no fill: an execution's chain remembers its own identifier alone, so a second statement of it restates the first ([FIX](../fix/lifecycle.md#an-orders-fills-are-counted-once)) |
| Ended fills | a chain that ends leaves its counted fills behind for `with_window_ns(i64)` of event time past the instant it ended - `DEFAULT_WINDOW_NS`, one minute, the FIX codec's `dedup_window_ms`; ≤0 remembers none - so a copy of one logged at another hop starts no chain from a count: an order's statement is yielded as it came, an execution's restating the one that counted it. A second statement of a live execution's fill restates the first only where it says what the first said - the same fill in the same state; a correction stays its own. Held per ended chain under its base cross code, its kind and its side - a fill of the other side of one match under one execution identifier is its own - and swept, past a size bound, of what the walk has passed. Rust only: the bindings' `EventIterator` remembers for the default minute, and a FIX codec's lifecycle for its window |
| Creation | every element leaves stating `creaunix`: one stating none takes its chain's - the earliest the fold kept - or, starting a chain, its own instant; a stated one is never replaced, and no identity moves, since no instant is digested |
| One cross element | a chain whose first element states no cross code stands under that element's identity, and every element of it after the first carries that identity as its `crossuuid` - one joining by a name, an `UPDATED`, a twin, the `EXPIRED` the walk emits and a follower stating a code of its own alike - stated after its last finalize, since a finalize derives an element's own; the first code a follower states is the code the elements after it are re-keyed onto, and since no follower's own content derives the cross element, a consumer finalizing one again derives the follower's own |
| Order | `sorted=true` trusts the caller and streams; else the walk collects and stably sorts by `is_after`/`is_before`. One before the live element follows nothing and moves the live one not at all, and one refused by it or unchanged by following still stands as the live one: each is yielded [re-keyed](#names-re-keying-and-conflicts) onto its chain |
| Executions | an execution is a chain of its own kind: it joins a live execution of its side sharing an identifier as any element does, and never the order it filled, which is of another kind; a market event stating no `execunix` is dated from `transunix` pre-placement when `Event::is_execution` holds; a market event carries the latest execution clock through non-executions ([`execunix`](market.md#following-and-merging)), and no event carries `sendunix`; a FIX message's fills are split into execution messages at the [parse](../fix/message.md#market-data) |
| Deadlines, end | a finite `exprunix` emits one owned `EXPIRED` at that instant, following the live generation, then purges it; the expirations of one deadline take its next places in the order of the identities they retire, and a source element keeps its own place; a replaced/terminal generation's stale deadline emits nothing; ties: deadlines, then source events, then grid views; at EOF the walk drains finite deadlines/views to the greatest deadline reached, else the last source instant - a nonexpiring identity never extends a finite source |
| Grid | `with_snapshot_ns(i64)` (≤0=none): an epoch-aligned grid - one owned view per living identity per crossed tick, never backdating; `snapshot_ns()` reads it back. A view is the live element as of its tick: dated at it (`transunix` = the tick), its `snapunix` the instant the element it copies was stated at - that element's own `snapunix` where it is itself a view - so its `uuid` is the identity the tick derives - a row of its own wherever rows are keyed by identity within a time - while its content (`hashcode`), `seqnum`, `prevuuid` and `crossuuid` are the live element's; it does not advance the chain |
| `MarketData` | `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, `TradeEvent` and `Fix` walk, each following and restating - a twin, a statement logged again - through its own reading; every other variant yields unchanged and never stands live; unsorted, an undated value sorts first |
| `alive()` | the live elements, in no order |
| Bindings | Python `graph.EventIterator(items, sorted=True, snapshot_ns=None)`, JavaScript `new graph.EventIterator(items, sorted, snapshotNs)`: each yields `MarketData`, and `alive()` answers it |

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::{EventIterator, Market, OrderEvent};
    use yggdryl_market::Side;
    use yggdryl::State;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    const SECOND: i64 = 1_000_000_000;
    let event = |second: i64, order: &str, state: &str| {
        let mut event = OrderEvent::at(T + second * SECOND);
        event.set_crosscode(order.to_owned());
        event.set_state(State::from_spelling(state).expect("a shipped state"));
        event.finalize();
        event
    };
    // Unsorted: one report logged twice, and O-1001 reopened after its fill.
    let arrived = vec![
        event(3, "O-1001", "Filled"),
        event(0, "O-1001", "New"),
        event(1, "O-2002", "New"),
        event(2, "O-1001", "PartiallyFilled"),
        event(2, "O-1001", "PartiallyFilled"),
        event(4, "O-1001", "New"),
    ];
    let mut walk = EventIterator::new(arrived, false);
    let chained: Vec<OrderEvent> = walk.by_ref().collect();
    let places: Vec<(i64, &str, u64)> = chained
        .iter()
        .map(|held| ((held.get_transunix() - T) / SECOND, held.get_crosscode(), held.get_seqnum()))
        .collect();
    assert_eq!(
        places,
        [
            (0, "10:0:O-1001", 0),
            (1, "10:0:O-2002", 0),
            (2, "10:0:O-1001", 0),
            (2, "10:0:O-1001", 0),
            (3, "10:0:O-1001", 0),
            (4, "10:0:O-1001", 0),
        ]
    );
    // Each step is at an instant of its own, so each keeps the first place
    // there; the chain is in `prevuuid`.
    assert_eq!(chained[2].get_uuid(), chained[3].get_uuid(), "a twin, not a successor");
    assert_eq!(chained[4].get_prevuuid(), Some(chained[2].get_uuid()));
    assert_eq!(chained[5].get_prevuuid(), None, "the fill ended the chain");
    // Each states when its lifecycle was created: its chain's first instant.
    assert_eq!(chained[4].get_creaunix(), Some(T));
    assert_eq!(chained[5].get_creaunix(), Some(T + 4 * SECOND));
    let mut alive: Vec<&str> = walk.alive().map(Element::get_crosscode).collect();
    alive.sort_unstable();
    assert_eq!(alive, ["10:0:O-1001", "10:0:O-2002"]);

    // One identifier on each side: a NEW restated, a cancel stating no side.
    let sided = |second: i64, side: Side, state: &str| {
        let mut sided = event(second, "O-7", state);
        sided.set_side(side, true);
        sided.finalize();
        sided
    };
    let walked: Vec<OrderEvent> = EventIterator::new(
        vec![
            sided(0, Side::Buy, "New"),
            sided(1, Side::Buy, "New"),
            sided(2, Side::Unknown, "PendingCancel"),
            sided(3, Side::Sell, "New"),
        ],
        true,
    )
    .collect();
    let chains: Vec<(&str, &str, u64)> = walked
        .iter()
        .map(|held| (held.get_crosscode(), held.get_state().as_str(), held.get_seqnum()))
        .collect();
    assert_eq!(
        chains,
        [
            ("10:1:O-7", "NEW", 0),
            ("10:1:O-7", "UPDATED", 0),
            ("10:1:O-7", "PENDING_CANCEL", 0),
            ("10:2:O-7", "NEW", 0),
        ]
    );
    assert_eq!(walked[2].get_side(), Side::Buy, "the one side alive under O-7");

    // A 10 ms grid: a view of the living order at each tick, then its deadline.
    const MS: i64 = 1_000_000;
    let mut expiring = OrderEvent::at(T + 50 * MS);
    expiring.set_crosscode("O-3003".to_owned());
    expiring.set_exprunix(Some(T + 70 * MS));
    expiring.finalize();
    let (source, content, cross) = (expiring.get_uuid(), expiring.get_hashcode(), expiring.get_crossuuid());
    let timed: Vec<OrderEvent> = EventIterator::new([expiring], true).with_snapshot_ns(10 * MS).collect();
    let view = timed
        .iter()
        .find(|held| held.get_snapunix().is_some() && held.get_transunix() == T + 60 * MS)
        .expect("the living view at the crossed tick");
    // The live order as of the tick: dated at it, so the identity is the
    // tick's own, while content, place and chain are the live order's, and
    // its snapshot instant the one the order was stated at.
    assert_eq!(view.get_snapunix(), Some(T + 50 * MS));
    assert_ne!(view.get_uuid(), source);
    assert_eq!((view.get_hashcode(), view.get_crossuuid(), view.get_seqnum()), (content, cross, 0));
    let expired = timed.last().expect("the deadline event");
    assert_eq!((expired.get_transunix(), expired.get_state().as_str()), (T + 70 * MS, "EXPIRED"));
    assert_eq!((expired.get_prevuuid(), expired.get_seqnum()), (Some(source), 0));
    ```

=== "Python"

    ```python
    from yggdryl import State, graph

    T = 1_700_000_000_000_000_000
    SECOND = 1_000_000_000

    def event(second: int, order: str, state: str, side: str = "UKNW") -> graph.OrderEvent:
        return graph.OrderEvent(T + second * SECOND, crosscode=order, state=state, side=side)

    # Unsorted: one report logged twice, and O-1001 reopened after its fill.
    arrived = [
        event(3, "O-1001", "FILLED"),
        event(0, "O-1001", "NEW"),
        event(1, "O-2002", "NEW"),
        event(2, "O-1001", "PARTIALLY_FILLED"),
        event(2, "O-1001", "PARTIALLY_FILLED"),
        event(4, "O-1001", "NEW"),
    ]
    walk = graph.EventIterator(arrived, sorted=False)
    chained = [value.as_order_event() for value in walk]
    places = [((held.transunix - T) // SECOND, held.crosscode, held.seqnum) for held in chained]
    # Each step is at an instant of its own, so each keeps the first place
    # there; the chain is in prevuuid.
    assert places == [
        (0, "10:0:O-1001", 0),
        (1, "10:0:O-2002", 0),
        (2, "10:0:O-1001", 0),
        (2, "10:0:O-1001", 0),
        (3, "10:0:O-1001", 0),
        (4, "10:0:O-1001", 0),
    ]
    assert chained[2].uuid == chained[3].uuid, "a twin, not a successor"
    assert chained[4].prevuuid == chained[2].uuid
    assert chained[5].prevuuid is None, "the fill ended the chain"
    # Each states when its lifecycle was created: its chain's first instant.
    assert (chained[4].creaunix, chained[5].creaunix) == (T, T + 4 * SECOND)
    assert sorted(value.crosscode for value in walk.alive()) == ["10:0:O-1001", "10:0:O-2002"]

    # One identifier on each side: a NEW restated, a cancel stating no side.
    walked = [
        value.as_order_event()
        for value in graph.EventIterator(
            [
                event(0, "O-7", "NEW", "BUYS"),
                event(1, "O-7", "NEW", "BUYS"),
                event(2, "O-7", "PENDING_CANCEL"),
                event(3, "O-7", "NEW", "SELL"),
            ]
        )
    ]
    assert [(held.crosscode, held.state, held.seqnum) for held in walked] == [
        ("10:1:O-7", State.NEW, 0),
        ("10:1:O-7", State.UPDATED, 0),
        ("10:1:O-7", State.PENDING_CANCEL, 0),
        ("10:2:O-7", State.NEW, 0),
    ]

    # A 10 ms grid: a view of the living order at each tick, then its deadline.
    MS = 1_000_000
    expiring = graph.OrderEvent(T + 50 * MS, crosscode="O-3003", exprunix=T + 70 * MS)
    timed = [value.as_order_event() for value in graph.EventIterator([expiring], snapshot_ns=10 * MS)]
    [view] = [held for held in timed if held.snapunix is not None and held.transunix == T + 60 * MS]
    # The live order as of the tick: dated at it, so the identity is the
    # tick's own, while content, place and chain are the live order's, and
    # its snapshot instant the one the order was stated at.
    assert view.snapunix == T + 50 * MS
    assert view.uuid != expiring.uuid
    assert (view.hashcode, view.crossuuid, view.seqnum) == (expiring.hashcode, expiring.crossuuid, 0)
    expired = timed[-1]
    assert (expired.transunix, expired.state) == (T + 70 * MS, State.EXPIRED)
    assert (expired.prevuuid, expired.seqnum) == (expiring.uuid, 0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const SECOND = 1_000_000_000n
    const event = (second, order, state, side = 'UKNW') => new graph.OrderEvent(T + second * SECOND, {
      crosscode: order, state, side,
    })

    // Unsorted: one report logged twice, and O-1001 reopened after its fill.
    const arrived = [
      event(3n, 'O-1001', 'FILLED'),
      event(0n, 'O-1001', 'NEW'),
      event(1n, 'O-2002', 'NEW'),
      event(2n, 'O-1001', 'PARTIALLY_FILLED'),
      event(2n, 'O-1001', 'PARTIALLY_FILLED'),
      event(4n, 'O-1001', 'NEW'),
    ]
    const walk = new graph.EventIterator(arrived, false)
    const chained = [...walk].map((value) => value.asOrderEvent())
    const places = chained.map((held) => [(held.transunix - T) / SECOND, held.crosscode, held.seqnum])
    assert.deepEqual(places, [
      [0n, '10:0:O-1001', 0], [1n, '10:0:O-2002', 0], [2n, '10:0:O-1001', 0],
      [2n, '10:0:O-1001', 0], [3n, '10:0:O-1001', 0], [4n, '10:0:O-1001', 0],
    ])
    assert.equal(chained[2].uuid, chained[3].uuid, 'a twin, not a successor')
    assert.equal(chained[4].prevuuid, chained[2].uuid)
    assert.equal(chained[5].prevuuid, null, 'the fill ended the chain')
    // Each states when its lifecycle was created: its chain's first instant.
    assert.deepEqual([chained[4].creaunix, chained[5].creaunix], [T, T + 4n * SECOND])
    assert.deepEqual(walk.alive().map((value) => value.crosscode).sort(), ['10:0:O-1001', '10:0:O-2002'])

    // One identifier on each side: a NEW restated, a cancel stating no side.
    const walked = [...new graph.EventIterator([
      event(0n, 'O-7', 'NEW', 'BUYS'),
      event(1n, 'O-7', 'NEW', 'BUYS'),
      event(2n, 'O-7', 'PENDING_CANCEL'),
      event(3n, 'O-7', 'NEW', 'SELL'),
    ])].map((value) => value.asOrderEvent())
    assert.deepEqual(walked.map((held) => [held.crosscode, held.state, held.seqnum]), [
      ['10:1:O-7', 'NEW', 0],
      ['10:1:O-7', 'UPDATED', 0],
      ['10:1:O-7', 'PENDING_CANCEL', 0],
      ['10:2:O-7', 'NEW', 0],
    ])

    // A 10 ms grid: a view of the living order at each tick, then its deadline.
    const MS = 1_000_000n
    const expiring = new graph.OrderEvent(T + 50n * MS, { crosscode: 'O-3003', exprunix: T + 70n * MS })
    const timed = [...new graph.EventIterator([expiring], true, 10n * MS)].map((value) => value.asOrderEvent())
    const view = timed.find((held) => held.snapunix !== null && held.transunix === T + 60n * MS)
    // The live order as of the tick: dated at it, so the identity is the
    // tick's own, while content, place and chain are the live order's, and
    // its snapshot instant the one the order was stated at.
    assert.equal(view.snapunix, T + 50n * MS)
    assert.notEqual(view.uuid, expiring.uuid)
    assert.deepEqual([view.hashcode, view.crossuuid, view.seqnum], [expiring.hashcode, expiring.crossuuid, 0])
    const expired = timed[timed.length - 1]
    assert.equal(expired.transunix, T + 70n * MS)
    assert.equal(expired.state, 'EXPIRED')
    assert.equal(expired.prevuuid, expiring.uuid)
    assert.equal(expired.seqnum, 0)
    ```

### Names, re-keying and conflicts

A replace under another cross code and a new `ClOrdID`, naming the one it replaced, then a report citing the new value and one citing the old: one chain under the first statement's code, every hash derived from it. Then a report citing two live orders by the names it shares with each: a conflict, never a pick.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::{EventIterator, Operation, OrderEvent};
    use yggdryl_market::{IdKey, IdType, Identifier};
    use yggdryl::Uuid;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;
    const MS: i64 = 1_000_000;
    // An order at `ms` going by `names`, under the cross code `code` where one is given.
    let order = |ms: i64, code: Option<&str>, names: &[(IdType, &str)]| -> yggdryl::Result<OrderEvent> {
        let mut order = OrderEvent::at(T + ms * MS);
        if let Some(code) = code {
            order.set_crosscode(code.to_owned());
        }
        for (kind, value) in names {
            order.insert_identifier(Identifier::new(IdKey::base(kind.clone()), *value)?)?;
        }
        order.finalize();
        Ok(order)
    };

    let replace = order(10, Some("O-200"), &[(IdType::ClOrdId, "C2"), (IdType::OrigClOrdId, "C1")])?;
    assert_eq!(replace.get_crosscode(), "10:0:O-200", "its own spelling");
    let mut walk = EventIterator::new(
        vec![
            order(0, Some("O-100"), &[(IdType::ClOrdId, "C1")])?,
            replace,
            order(20, None, &[(IdType::ClOrdId, "C2")])?,
            order(30, None, &[(IdType::ClOrdId, "C1")])?,
        ],
        true,
    );
    let walked: Vec<OrderEvent> = walk.by_ref().collect();
    // One chain under its first statement's code, each statement after the one before.
    for (at, event) in walked.iter().enumerate() {
        assert_eq!(event.get_crosscode(), "10:0:O-100");
        assert_eq!(event.get_crosshashcode(), yggdryl::xxhash::xxh3(event.get_crosscode().as_bytes()));
        assert_eq!(event.get_crossuuid(), Uuid::from_v8(u128::from(event.get_crosshashcode())));
        if at > 0 {
            assert_eq!(event.get_prevuuid(), Some(walked[at - 1].get_uuid()));
        }
    }
    assert_eq!(walk.alive().count(), 1);

    // Two live orders, and a report citing both: it stands under its own
    // identity - a chain of its own here - and the walk warns once.
    let mut walk = EventIterator::new(
        vec![
            order(0, Some("O-1"), &[(IdType::ClOrdId, "C1")])?,
            order(10, Some("O-2"), &[(IdType::SecondaryClOrdId, "S2")])?,
            order(20, None, &[(IdType::ClOrdId, "C1"), (IdType::SecondaryClOrdId, "S2")])?,
        ],
        true,
    );
    let conflicted: Vec<OrderEvent> = walk.by_ref().collect();
    assert_eq!(conflicted[2].get_prevuuid(), None);
    assert_eq!(walk.alive().count(), 3);
    ```

=== "Python"

    ```python
    import logging

    from yggdryl import Identifier, graph

    T = 1_700_000_000_000_000_000
    MS = 1_000_000
    said: list[logging.LogRecord] = []


    class Collect(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            said.append(record)


    logging.getLogger("yggdryl").addHandler(Collect())


    def order(ms: int, *names: tuple[str, str], **facts) -> graph.OrderEvent:
        ids = [Identifier(kind, value) for kind, value in names]
        return graph.OrderEvent(T + ms * MS, identifiers=ids, **facts)


    def walk(items):
        iterator = graph.EventIterator(items)
        return [value.as_order_event() for value in iterator], iterator


    replace = order(10, ("clordid", "C2"), ("origclordid", "C1"), crosscode="O-200")
    assert replace.crosscode == "10:0:O-200", "its own spelling"
    walked, iterator = walk([
        order(0, ("clordid", "C1"), crosscode="O-100"),
        replace,
        order(20, ("clordid", "C2")),
        order(30, ("clordid", "C1")),
    ])
    # One chain under its first statement's code, each statement after the one before.
    assert all(held.crosscode == "10:0:O-100" for held in walked)
    assert all(held.crossuuid == walked[0].crossuuid for held in walked)
    assert [held.prevuuid for held in walked[1:]] == [held.uuid for held in walked[:-1]]
    assert len(iterator.alive()) == 1

    # Two live orders, and a report citing both: it stands under its own
    # identity - a chain of its own here - and the walk warns once.
    conflicted, iterator = walk([
        order(0, ("clordid", "C1"), crosscode="O-1"),
        order(10, ("secondaryclordid", "S2"), crosscode="O-2"),
        order(20, ("clordid", "C1"), ("secondaryclordid", "S2")),
    ])
    assert conflicted[2].prevuuid is None
    assert len(iterator.alive()) == 3
    assert any(
        record.name.startswith("yggdryl.graph") and "cites two live chains" in record.getMessage()
        for record in said
    )
    ```

`IdType` and the walk's index are Rust's; a binding states the identifiers as `Identifier`s and reads what the walk answers.

## Edges

- A later `following` call replaces a prior predecessor; place saturates past `u64::MAX`. `following_market`/`following_operation` still fill missing facts even when the timed link is unchanged.
- `restating` never refuses (unlike `following`/`merging`) - it is the caller's unchecked assertion that the two are the same twin.
- The later `sendunix` picks the merge reference even if its event instant is earlier; the merged `sendunix` - and, for a market event, `execunix` - stay earliest observed, so merge order can change the winner.
- The walk clones a source at most twice (as the live one, or one its predecessor refuses), plus one clone per expiration/grid view; it holds one live element per identity, emitting views lazily rather than queued.
- A grid starts at the first epoch-aligned tick at/after the first source instant; output is crossed ticks × identities alive, caller-chosen width, no implicit count/span cap.
- An element stating no side whose base code is alive on both sides cites neither by it, and its names decide; a name alive on both sides is a [conflict](#names-re-keying-and-conflicts) - never refused and never a pick: it stands under its own identity, a chain of its own under side `0` where nothing else holds it.
