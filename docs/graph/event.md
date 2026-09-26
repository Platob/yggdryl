# Event

`Event: Element` is an element that happened at one instant, stands in one state and has a place in its chain; `EventIterator` is the walk that chains events.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Event` (`graph::element`); `EventIterator` walk (`graph::iterator`); Rust-only - dated [leaves](index.md#leaves) answer it in Python/JavaScript |
| `currunix` | `get_currunix`/`set_currunix`: nanoseconds since the Unix epoch, UTC, signed |
| `state` | `get_state`/`set_state`: the ranked lifecycle [code](../types/codes/state.md), never absent - `00UNKNOWN` where none reached |
| `is_execution` | provided via `State::is_execution` (`40PARTFILL`, `40TRADE`, `80FILLED`); overridable where lifecycle state and report kind differ |
| `seqnum` | `get_seqnum`/`set_seqnum`: how many came before it in its chain |
| Clocks | `creaunix`, `execunix`, `recdunix`, `exprtime`, `prevunix`, `snapunix` (`Option<i64>`) and `prevuuid` (`Option<Uuid>`), each `get_`/`set_`, stated only where known: `execunix` the execution instant, `recdunix` the earliest recording (what a merge ranks by), `exprtime` the deadline, `prevunix`/`prevuuid` the predecessor, `snapunix` the grid step |
| `digest_event` | provided: continues [`Element::digest`](element.md#contract) with the state, chain place and predecessor's identity; no instant fed |
| `fold_lifecycle` | provided, `(&mut self, &Self) -> bool`: earliest creation, latest expiration, better state ([`CodeValue::merge_with`](../types/codes/index.md#the-code-family-value)) |

## Identity

| Reading | Rule |
| --- | --- |
| `txhash() -> Result<TxHash>` | couples `currunix` (ns) with `currhashcode` as its digest ([TxHash](../hashing.md)) |
| `time_uuid() -> Result<Uuid>` | `TxHash::into_sequenced_uuid(seqnum, crosshashcode)`: UUIDv7 (RFC 9562) - ms-floored instant, `seqnum.min(4095)` in the 12-bit `rand_a`, `rand_b` = low 62 bits of XXH3(`currhashcode`+sequence) seeded by the cross hash; sorts by ms then sequence (terminal-lane sequences only probabilistically distinct); seed separates cross chains; pre-epoch = `InvalidRecord`, never truncated |
| `finalized(&mut self, hashcode)` | provided: records the code, resets `curruuid` to `time_uuid` (kept without one) and `crossuuid` to `cross_uuid()`; every `finalize` hands its digest here; a foreign implementor with an assigned identity may skip it |

## Following

`following(self, &Self) -> Option<Self>`, provided; what `with_previous` delegates to:

| Moves | Rule |
| --- | --- |
| The link | predecessor's identity/instant as `prevuuid`/`prevunix`, one place past its `seqnum` (saturating); chain history stops at `prevuuid` |
| The cross code | the predecessor's is forced on where this event's differs - one chain shares it |
| The lifecycle | earliest creation either knows; this event's explicit expiration else the predecessor's (can shorten a deadline); the furthest state |
| Execution | the later of this event's precise execution clock and the predecessor's |
| Nothing else | current/recording instants, sources and snapshot move nowhere |
| Refusals | nothing for its own predecessor, one that happened after it, or no change; an equal instant follows |

An event that moved is finalized; [`Market::following_market`](market.md#following-and-merging)/[`Operation::following_operation`](operation.md#following-and-merging) continue it before finalizing.

## Restating

`restating(self, live: &Self) -> Self`, provided: another statement of `live` (same instant/content read again), taking `live`'s chain place - predecessor, position, snapshot, cross code, never sources; lifecycle folded, earliest execution/recording kept, then finalized - one identity, chain unchanged. A market event also takes the market's place, an operation event the operation's ([Market](market.md#following-and-merging), [Operation](operation.md#following-and-merging)). The caller must give `live` under the identity it *arrived* under - following moved it; the [walk](#lifecycle-walk) does this.

## Merging

`merging(self, &Self) -> Option<Self>`, provided; what `merge_with` delegates to:

| Moves | Rule |
| --- | --- |
| The reference | the statement with the later `recdunix` (stated beats unstated; equal/absent falls back to the greater `currunix`; an exact tie keeps `self`) |
| From the reference | cross code, current instant/code, predecessor, snapshot; the other fills what it leaves unstated |
| Folded | sources: position-independent sorted-unique union; the further chain place; `fold_lifecycle`; execution/recording = earliest of either |
| Refusals | nothing for another element (different `curruuid`) or no change |

Merge order can change which of three statements leads (ranked by earliest recording kept). [`Market::merging_market_event`](market.md#following-and-merging)/[`Operation::merging_operation_event`](operation.md#following-and-merging) continue it.

## Examples

### A chain

An Apple order placed, then partly filled a second later.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal, Side, State};

    const T: i64 = 1_700_000_000_000_000_000;
    let event = |unix: i64, state: &str| -> yggdryl::Result<OrderEvent> {
        let mut event = OrderEvent::at(unix);
        event.set_crosscode("O-1001".to_owned());
        event.set_state(State::from_spelling(state).expect("a shipped state"));
        event.set_side(Side::Buy);
        event.set_price(Some("189.50".parse()?));
        event.set_quantity(Some(Decimal::from_int(100)));
        event.set_currency(Ccy::new("USD")?);
        event.insert_altid("ORDERID", "O-1001")?;
        event.finalize();
        Ok(event)
    };
    let placed = event(T, "New")?;
    // UUIDv7: the millisecond 1_700_000_000_000 leads the identity.
    assert_eq!(placed.get_curruuid(), placed.time_uuid()?);
    assert!(placed.get_curruuid().to_string().starts_with("018bcfe5-6800-7"));
    assert_eq!(placed.txhash()?.unix(), T);
    assert_eq!(placed.get_state().as_str(), "20NEW");

    let filled = event(T + 1_000_000_000, "PartiallyFilled")?;
    assert!(filled.is_after(&placed) && placed.is_before(&filled));
    let filled = filled.with_previous(&placed).expect("a later event follows");
    assert_eq!(filled.get_prevuuid(), Some(placed.get_curruuid()));
    assert_eq!(filled.get_prevunix(), Some(T));
    assert_eq!(filled.get_seqnum(), 1);
    assert_eq!(filled.get_crossuuid(), placed.get_crossuuid(), "one chain");
    assert_eq!(filled.get_prevpx(), Some("189.50".parse()?), "the step before");
    // UUIDv7s sort by their millisecond first.
    assert!(placed.get_curruuid() < filled.get_curruuid());

    // An event follows neither itself nor one that happened after it.
    assert!(event(T, "New")?.with_previous(&placed).is_none());
    assert!(placed.clone().with_previous(&filled).is_none());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000

    def event(unix: int, state: str) -> graph.OrderEvent:
        return graph.OrderEvent(
            unix,
            crosscode="O-1001",
            state=state,
            side="BUY",
            price=Decimal("189.50"),
            quantity=100,
            currency="USD",
            altids={"ORDERID": "O-1001"},
        )

    placed = event(T, "NEW")
    # UUIDv7: the millisecond 1_700_000_000_000 leads the identity.
    assert placed.curruuid.as_py().startswith("018bcfe5-6800-7")
    assert placed.state.as_py() == "20NEW"

    filled = event(T + 1_000_000_000, "PARTIALLY_FILLED")
    assert filled.is_after(placed) and placed.is_before(filled)
    followed = filled.with_previous(placed)
    assert followed is not None
    assert followed.prevuuid == placed.curruuid
    assert followed.prevunix == T
    assert followed.seqnum == 1
    assert followed.crossuuid == placed.crossuuid, "one chain"
    assert followed.prevpx is not None and followed.prevpx.as_py() == Decimal("189.50")

    # An event follows neither itself nor one that happened after it.
    assert event(T, "NEW").with_previous(placed) is None
    assert placed.with_previous(followed) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const event = (unix, state) => new graph.OrderEvent(unix, {
      crosscode: 'O-1001',
      state,
      side: 'BUY',
      price: '189.50',
      quantity: 100,
      currency: 'USD',
      altids: { ORDERID: 'O-1001' },
    })

    const placed = event(T, 'NEW')
    // UUIDv7: the millisecond 1_700_000_000_000 leads the identity.
    assert.ok(placed.curruuid.startsWith('018bcfe5-6800-7'))
    assert.equal(placed.state, '20NEW')

    const filled = event(T + 1_000_000_000n, 'PARTIALLY_FILLED')
    assert.ok(filled.isAfter(placed) && placed.isBefore(filled))
    const followed = filled.withPrevious(placed)
    assert.equal(followed.prevuuid, placed.curruuid)
    assert.equal(followed.prevunix, T)
    assert.equal(followed.seqnum, 1)
    assert.equal(followed.crossuuid, placed.crossuuid, 'one chain')
    assert.equal(followed.prevpx, '189.5')

    // An event follows neither itself nor one that happened after it.
    assert.equal(event(T, 'NEW').withPrevious(placed), null)
    assert.equal(placed.withPrevious(followed), null)
    ```

### Two hops of one message

The same fill report, recorded by a gateway at +2ms and an OMS at +5ms, each from its own log line.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, OrderEvent};
    use yggdryl::{State, Uuid};

    const T: i64 = 1_700_000_000_000_000_000;
    let report = |recorded: Option<i64>, line: u128| {
        let mut event = OrderEvent::at(T + 1_000_000_000);
        event.set_crosscode("O-1001".to_owned());
        event.set_state(State::from_spelling("PartiallyFilled").expect("a shipped state"));
        event.set_recdunix(recorded);
        event.set_srcuuids(vec![Uuid::from_v8(line)]);
        event.finalize();
        event
    };
    let gateway = report(Some(T + 1_002_000_000), 1);
    let oms = report(Some(T + 1_005_000_000), 2);
    // Recording clocks and sources are not content: one event.
    assert_eq!(gateway.get_curruuid(), oms.get_curruuid());

    // Merging: the later recording leads, the earliest recording stays,
    // and the sources are unioned.
    let merged = oms.clone().merge_with(&gateway).expect("another statement");
    assert_eq!(merged.get_recdunix(), Some(T + 1_002_000_000));
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(2)]);

    // Restating: once the gateway's statement followed the order, its
    // identity moved; the OMS statement takes its place and identity.
    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.finalize();
    let live = gateway.with_previous(&placed).expect("a later event follows");
    let twin = oms.restating(&live);
    assert_eq!((twin.get_curruuid(), twin.get_seqnum()), (live.get_curruuid(), 1));
    assert_eq!(twin.get_recdunix(), Some(T + 1_002_000_000));
    assert_eq!(twin.get_srcuuids(), [Uuid::from_v8(2)], "sources stay its own");
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    LINE_1 = "018bcfe5-6800-7000-8000-000000000001"
    LINE_2 = "018bcfe5-6800-7000-8000-000000000002"

    def report(recorded: int, line: str) -> graph.OrderEvent:
        return graph.OrderEvent(
            T + 1_000_000_000, crosscode="O-1001", state="PARTIALLY_FILLED", recdunix=recorded, srcuuids=[line]
        )

    gateway = report(T + 1_002_000_000, LINE_1)
    oms = report(T + 1_005_000_000, LINE_2)
    # Recording clocks and sources are not content: one event.
    assert gateway.curruuid == oms.curruuid

    # Merging: the later recording leads, the earliest recording stays,
    # and the sources are unioned.
    merged = oms.merge_with(gateway)
    assert merged is not None and merged.recdunix == T + 1_002_000_000
    assert [source.as_py() for source in merged.srcuuids] == [LINE_1, LINE_2]

    # Restating: once the gateway's statement followed the order, its
    # identity moved; the OMS statement takes its place and identity.
    live = gateway.with_previous(graph.OrderEvent(T, crosscode="O-1001"))
    assert live is not None
    twin = oms.restating(live)
    assert (twin.curruuid, twin.seqnum) == (live.curruuid, 1)
    assert twin.recdunix == T + 1_002_000_000
    assert [source.as_py() for source in twin.srcuuids] == [LINE_2], "sources stay its own"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const LINE_1 = '018bcfe5-6800-7000-8000-000000000001'
    const LINE_2 = '018bcfe5-6800-7000-8000-000000000002'
    const report = (recorded, line) => new graph.OrderEvent(T + 1_000_000_000n, {
      crosscode: 'O-1001', state: 'PARTIALLY_FILLED', recdunix: recorded, srcuuids: [line],
    })

    const gateway = report(T + 1_002_000_000n, LINE_1)
    const oms = report(T + 1_005_000_000n, LINE_2)
    // Recording clocks and sources are not content: one event.
    assert.equal(gateway.curruuid, oms.curruuid)

    // Merging: the later recording leads, the earliest recording stays,
    // and the sources are unioned.
    const merged = oms.mergeWith(gateway)
    assert.equal(merged.recdunix, T + 1_002_000_000n)
    assert.deepEqual(merged.srcuuids, [LINE_1, LINE_2])

    // Restating: once the gateway's statement followed the order, its
    // identity moved; the OMS statement takes its place and identity.
    const live = gateway.withPrevious(new graph.OrderEvent(T, { crosscode: 'O-1001' }))
    const twin = oms.restating(live)
    assert.equal(twin.curruuid, live.curruuid)
    assert.equal(twin.seqnum, 1)
    assert.equal(twin.recdunix, T + 1_002_000_000n)
    assert.deepEqual(twin.srcuuids, [LINE_2], 'sources stay its own')
    ```

## Lifecycle walk

`EventIterator::new(elements, sorted)` walks any `Event + Operation + Clone` (an operation leaf, a [FIX lifecycle message](../fix/lifecycle.md)) or `MarketData`, yielding the same type.

| Key | Rule |
| --- | --- |
| Live set, twins | elements still alive (a live state, not past expiration) share the cross identity; an arrival under a live identity - or (if dead) under a `(key, value)` of a live element's `get_altids()` - is yielded as `with_previous` of the live one, live until it isn't, then retiring; one arriving under the identity the live element *arrived* under is yielded [`restating`](#restating) it instead |
| Order | `sorted=true` trusts the caller and streams; else the walk collects and stably sorts by `is_after`/`is_before`. One before the live element, refused by it, or unchanged by following, is yielded as it came |
| Executions | `execunix` defaults to `currunix` pre-placement when `Event::is_execution` holds; following carries the latest execution clock through non-executions, never filling/carrying `recdunix`. FIX counts an initial `35=AE` only if `TradeReportTransType(487)` is absent/New and `ExecType(150)` is absent or execution-like (`F`, legacy `1`/`2`) - never non-New/cancel/correct/reverse/status `AE`, nor `AD`/`AQ`/`AR` |
| Deadlines, end | a finite `exprtime` emits one owned `95EXPIRED` at that instant, following the live generation, then purges it; a replaced/terminal generation's stale deadline emits nothing; ties: deadlines, then source events, then grid views; at EOF the walk drains finite deadlines/views to the greatest deadline reached, else the last source instant - a nonexpiring identity never extends a finite source |
| Grid | `with_snapshot_ns(i64)` (≤0=none): an epoch-aligned grid - one owned view per living identity per crossed tick, changing only `snapunix`, never backdating; `snapshot_ns()` reads it back |
| `MarketData` | `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, `TradeEvent` walk; every other variant yields unchanged and never stands live; unsorted, an undated value sorts first |
| `alive()` | the live elements, in no order |

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, EventIterator, OrderEvent};
    use yggdryl::State;

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
        .map(|held| ((held.get_currunix() - T) / SECOND, held.get_crosscode(), held.get_seqnum()))
        .collect();
    assert_eq!(
        places,
        [(0, "O-1001", 0), (1, "O-2002", 0), (2, "O-1001", 1), (2, "O-1001", 1), (3, "O-1001", 2), (4, "O-1001", 0)]
    );
    assert_eq!(chained[2].get_curruuid(), chained[3].get_curruuid(), "a twin, not a successor");
    assert_eq!(chained[4].get_prevuuid(), Some(chained[2].get_curruuid()));
    assert_eq!(chained[5].get_prevuuid(), None, "the fill ended the chain");
    let mut alive: Vec<&str> = walk.alive().map(Element::get_crosscode).collect();
    alive.sort_unstable();
    assert_eq!(alive, ["O-1001", "O-2002"]);

    // A 10 ms grid: a view of the living order at each tick, then its deadline.
    const MS: i64 = 1_000_000;
    let mut expiring = OrderEvent::at(T + 50 * MS);
    expiring.set_crosscode("O-3003".to_owned());
    expiring.set_exprtime(Some(T + 70 * MS));
    expiring.finalize();
    let source = expiring.get_curruuid();
    let timed: Vec<OrderEvent> = EventIterator::new([expiring], true).with_snapshot_ns(10 * MS).collect();
    let view = timed
        .iter()
        .find(|held| held.get_snapunix() == Some(T + 60 * MS))
        .expect("the living view at the crossed tick");
    assert_eq!((view.get_curruuid(), view.get_seqnum()), (source, 0));
    let expired = timed.last().expect("the deadline event");
    assert_eq!((expired.get_currunix(), expired.get_state().as_str()), (T + 70 * MS, "95EXPIRED"));
    assert_eq!((expired.get_prevuuid(), expired.get_seqnum()), (Some(source), 1));
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    SECOND = 1_000_000_000

    def event(second: int, order: str, state: str) -> graph.OrderEvent:
        return graph.OrderEvent(T + second * SECOND, crosscode=order, state=state)

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
    places = [((held.currunix - T) // SECOND, held.crosscode, held.seqnum) for held in chained]
    assert places == [(0, "O-1001", 0), (1, "O-2002", 0), (2, "O-1001", 1), (2, "O-1001", 1), (3, "O-1001", 2), (4, "O-1001", 0)]
    assert chained[2].curruuid == chained[3].curruuid, "a twin, not a successor"
    assert chained[4].prevuuid == chained[2].curruuid
    assert chained[5].prevuuid is None, "the fill ended the chain"
    assert sorted(value.crosscode for value in walk.alive()) == ["O-1001", "O-2002"]

    # A 10 ms grid: a view of the living order at each tick, then its deadline.
    MS = 1_000_000
    expiring = graph.OrderEvent(T + 50 * MS, crosscode="O-3003", exprtime=T + 70 * MS)
    timed = [value.as_order_event() for value in graph.EventIterator([expiring], snapshot_ns=10 * MS)]
    [view] = [held for held in timed if held.snapunix == T + 60 * MS]
    assert (view.curruuid, view.seqnum) == (expiring.curruuid, 0)
    expired = timed[-1]
    assert (expired.currunix, expired.state.as_py()) == (T + 70 * MS, "95EXPIRED")
    assert (expired.prevuuid, expired.seqnum) == (expiring.curruuid, 1)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const SECOND = 1_000_000_000n
    const event = (second, order, state) => new graph.OrderEvent(T + second * SECOND, { crosscode: order, state })

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
    const places = chained.map((held) => [(held.currunix - T) / SECOND, held.crosscode, held.seqnum])
    assert.deepEqual(places, [
      [0n, 'O-1001', 0], [1n, 'O-2002', 0], [2n, 'O-1001', 1], [2n, 'O-1001', 1], [3n, 'O-1001', 2], [4n, 'O-1001', 0],
    ])
    assert.equal(chained[2].curruuid, chained[3].curruuid, 'a twin, not a successor')
    assert.equal(chained[4].prevuuid, chained[2].curruuid)
    assert.equal(chained[5].prevuuid, null, 'the fill ended the chain')
    assert.deepEqual(walk.alive().map((value) => value.crosscode).sort(), ['O-1001', 'O-2002'])

    // A 10 ms grid: a view of the living order at each tick, then its deadline.
    const MS = 1_000_000n
    const expiring = new graph.OrderEvent(T + 50n * MS, { crosscode: 'O-3003', exprtime: T + 70n * MS })
    const timed = [...new graph.EventIterator([expiring], true, 10n * MS)].map((value) => value.asOrderEvent())
    const view = timed.find((held) => held.snapunix === T + 60n * MS)
    assert.equal(view.curruuid, expiring.curruuid)
    assert.equal(view.seqnum, 0)
    const expired = timed[timed.length - 1]
    assert.equal(expired.currunix, T + 70n * MS)
    assert.equal(expired.state, '95EXPIRED')
    assert.equal(expired.prevuuid, expiring.curruuid)
    assert.equal(expired.seqnum, 1)
    ```

## Edges

- A later `following` call replaces a prior predecessor; place saturates past `u64::MAX`. `following_market`/`following_operation` still fill missing facts even when the timed link is unchanged.
- `restating` never refuses (unlike `following`/`merging`) - it is the caller's unchecked assertion that the two are the same twin.
- The later `recdunix` picks the merge reference even if its event instant is earlier; merged `execunix`/`recdunix` stay earliest observed, so merge order can change the winner.
- The walk clones a source at most twice (as the live one, or one its predecessor refuses), plus one clone per expiration/grid view; it holds one live element per identity, emitting views lazily rather than queued.
- A grid starts at the first epoch-aligned tick at/after the first source instant; output is crossed ticks × identities alive, caller-chosen width, no implicit count/span cap.
