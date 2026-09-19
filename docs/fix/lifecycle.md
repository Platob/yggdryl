# Lifecycle

A message says what happened; it does not say which order's life it belongs to beyond the identifiers a venue chose. `FixCodec::lifecycle` is the one [walk](../graph.md) over messages: each is stated as the one after the live message of its chain - the chain named by the cross code every incarnation of one order shares - so a chained message carries its predecessor's identity and instant, its place in the chain, the predecessor as a parent and the lifecycle carried forward, and a monitor joins an order's whole life on `crossuuid` rather than rebuilding it from `ClOrdID`, `OrigClOrdID` and `OrderID` on its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::lifecycle`, `FixCodec::lifecycle_arrow_reader`; the walk itself is [`graph::EventIterator`](../graph.md), which a caller opens over messages directly for a grid |
| Columns | the [crate's own](capture.md#the-crates-own-columns): `crosscode` (65048), `crosshashcode` (65018) and `crossuuid` (65040) name the chain; `prevuuid` (65022), `prevunix` (65021), `seqnum` (65042) and `parentuuids` (65041) place a message in it; `creatunix` (65023) is the lifecycle carried forward, and the expiry and the state the walk folds are the traits' to answer off FIX's own fields rather than columns of their own; `snapunix` (65025) is a grid's stamp. `currhashcode` (65017) and `curruuid` (65039) are settled again after every stamp |
| Chain name | the cross code: the first the message states of `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` and `MDReqID(262)`; `crosshashcode` is its XXH3-64 and `crossuuid` the UUIDv8 of that, so every message spelling one code shares one identity whatever else it says. A message naming none is a chain of one: its `crossuuid` is its own `curruuid` |
| Joins | a message arriving under the identity a live message holds follows it; one arriving under no live identity, but going by a name a live message goes by - the same `(scheme, value)` in its [`identifiers`](capture.md#the-crates-own-columns), a report stating only the `ClOrdID` an order was placed under - follows that one, and takes the chain's cross code as its own |
| Follows | the predecessor's `curruuid` and `currunix` recorded, `seqnum` one past the predecessor's, the predecessor adopted as a parent, the chain's cross code forced, the names the predecessor went by taken, and the lifecycle folded: the earliest `creatunix` the two know, the latest expiry, the furthest [state](../types/codes.md#a-state-sorts-by-its-lifecycle) - the last two answered by the traits, because a fold is not a statement and reaches no column; a message that moved is settled again, so its `currhashcode` and `curruuid` are its own |
| Twins | a message arriving under the identity the live one *arrived* under - the same instant and content, one message a bridge logged at every hop it passed - is another statement of it, not the one after it: it takes the live one's place, predecessor, position and lifecycle, and finalizes to the same `curruuid`, so the chain grows by nothing |
| Order | collected and stably sorted by instant before the walk, because a capture's lines are in the order they were written and two messages of one chain routinely arrive out of their own order |
| Ends | a terminal state - filled, done for day, cancelled, rejected, expired - or a message past the expiry its fields state retires the chain once yielded, so a venue reusing a `ClOrdID` tomorrow starts a chain afresh under the same `crossuuid` |
| Grid | `EventIterator::with_snapshot_ns(step)` reads one snapshot per step per chain: the first message to reach a step its chain has not consumed is stamped with the step's opening instant as `snapunix`, every later one in that step with none; `lifecycle` reads no grid |
| Errors | move through in source order and never advance the walk; exhaustion is fused |
| Entries | untouched: what the message stated stays what it stated, and the wire re-emits it with the chain's columns nowhere in it |
| Bindings | Rust; Python `FixCodec.lifecycle`, `lifecycle_arrow_reader`; JavaScript `lifecycle`, `lifecycleArrowReader` |

## Use

One order's life: the order under its client identifier, the acknowledgement under the venue's, the fill naming the venue's alone, the acknowledgement logged a second time on its way through a bridge, and a new order reusing the client identifier after the fill.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, PREVUUID_TAG_NAME, SEQNUM_TAG_NAME};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 5] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|60=20260102-10:15:33.100|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|60=20260102-10:15:40.000|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    // Parsed, each message names the chain it spells: the order its ClOrdID,
    // the reports the venue's OrderID, and none follows anything yet.
    assert_eq!(parsed[0].get_crosscode(), "A1");
    assert_eq!(parsed[1].get_crosscode(), "O1");
    assert!(parsed.iter().all(|held| held.get_seqnum() == 0 && held.get_prevuuid().is_none()));

    let chained: Vec<FixMsg> = reader.lifecycle(parsed.clone()).collect::<yggdryl::Result<_>>()?;
    let [order, ack, twin, fill, again] = chained.as_slice() else { panic!("five messages") };

    // The acknowledgement goes by the name the order was placed under, so it
    // follows the order and the chain keeps the order's code; the fill names
    // only the venue's identifier, which the acknowledgement went by.
    assert!(chained.iter().take(4).all(|held| held.get_crosscode() == "A1"));
    assert!(chained.iter().take(4).all(|held| held.get_crossuuid() == order.get_crossuuid()));
    assert_eq!((order.get_seqnum(), order.get_prevuuid()), (0, None));
    assert_eq!((ack.get_seqnum(), ack.get_prevuuid()), (1, Some(order.get_curruuid())));
    assert_eq!(ack.get_prevunix(), Some(order.get_currunix()));
    assert_eq!(ack.get_parentuuids(), [order.get_curruuid()]);
    assert_eq!((fill.get_seqnum(), fill.get_prevuuid()), (2, Some(ack.get_curruuid())));
    // The lifecycle travels: the chain's first creation, and the state.
    assert_eq!(fill.get_creatunix(), order.get_creatunix());
    assert_eq!(fill.get_state().as_str(), "80FILLED");
    // The acknowledgement logged twice is one message: the second statement
    // takes the first one's place and identity, and the chain grows by nothing.
    assert_eq!(twin.get_curruuid(), ack.get_curruuid());
    assert_eq!((twin.get_seqnum(), twin.get_prevuuid()), (1, Some(order.get_curruuid())));
    // The fill ended the chain: the new order under the reused identifier
    // starts one afresh.
    assert_eq!((again.get_seqnum(), again.get_prevuuid()), (0, None));
    assert_eq!(again.get_crosscode(), "A1");
    // The stamps are columns, reached like any typed fact. They never
    // reach the wire, and neither does a fact the walk folded: this
    // acknowledgement named no side, so the chain's buy is what the trait
    // answers and not a byte the message emits.
    assert_eq!(ack.by_tag(SEQNUM_TAG_NAME.0)?.as_u64(), Some(1));
    assert_eq!(ack.by_tag(PREVUUID_TAG_NAME.0)?, yggdryl::Scalar::Uuid(order.get_curruuid()));
    assert_eq!(parsed[1].get_side().as_str(), "UNKNOWN");
    assert_eq!(ack.get_side().as_str(), "BUY");
    let wire = ack.into_text('|')?;
    assert!(wire.starts_with("8=FIX.4.4|35=8|11=A1|37=O1|150=0|"), "{wire}");
    assert!(!wire.contains("65042="), "no stamp is a field");

    // A chained stream replayed answers the same messages.
    let replayed: Vec<FixMsg> = reader.lifecycle(chained.clone()).collect::<yggdryl::Result<_>>()?;
    assert_eq!(replayed, chained);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    SEQNUM, PREVUUID = 65042, 65022
    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|60=20260102-10:15:33.100|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|60=20260102-10:15:40.000|10=0|",
    ]
    parsed = list(reader.parse_lines(lines))
    # Parsed, each message names the chain it spells: the order its ClOrdID, the
    # reports the venue's OrderID, and none follows anything yet.
    assert parsed[0].crosscode == "A1"
    assert parsed[1].crosscode == "O1"
    assert all(held.seqnum == 0 and held.prevuuid is None for held in parsed)

    order, ack, twin, fill, again = reader.lifecycle(parsed)

    # The acknowledgement goes by the name the order was placed under, so it
    # follows the order and the chain keeps the order's code; the fill names only
    # the venue's identifier, which the acknowledgement went by.
    chained = [order, ack, twin, fill]
    assert all(held.crosscode == "A1" for held in chained)
    assert all(held.crossuuid == order.crossuuid for held in chained)
    assert (order.seqnum, order.prevuuid) == (0, None)
    assert (ack.seqnum, ack.prevuuid) == (1, order.curruuid)
    assert ack.event().prevunix == order.currunix
    assert ack.parentuuids == [order.curruuid]
    assert (fill.seqnum, fill.prevuuid) == (2, ack.curruuid)
    # The lifecycle travels: the chain's first creation, and the state.
    assert fill.event().creatunix == order.event().creatunix
    assert fill.state.as_py() == "80FILLED"
    # The acknowledgement logged twice is one message: the second statement takes
    # the first one's place and identity, and the chain grows by nothing.
    assert twin.curruuid == ack.curruuid
    assert (twin.seqnum, twin.prevuuid) == (1, order.curruuid)
    # The fill ended the chain: the new order under the reused identifier starts
    # one afresh.
    assert (again.seqnum, again.prevuuid) == (0, None)
    assert again.crosscode == "A1"
    # The stamps are columns, reached like any typed fact. They never
    # reach the wire, and neither does a fact the walk folded: this
    # acknowledgement named no side, so the chain's buy is what the trait
    # answers and not a byte the message emits.
    assert ack.by_tag(SEQNUM).as_py() == 1
    assert ack.by_tag(PREVUUID) == order.curruuid
    assert parsed[1].side.as_py() == "UNKNOWN"
    assert ack.side.as_py() == "BUY"
    wire = ack.into_text("|")
    assert wire.startswith("8=FIX.4.4|35=8|11=A1|37=O1|150=0|")
    assert "65042=" not in wire  # no stamp is a field

    # A chained stream replayed answers the same messages.
    assert list(reader.lifecycle([order, ack, twin, fill, again])) == [order, ack, twin, fill, again]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const [SEQNUM, PREVUUID] = [65042, 65022]
    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|60=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|60=20260102-10:15:33.100|10=0|',
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|60=20260102-10:15:40.000|10=0|',
    ].map((line) => Buffer.from(line))
    const parsed = [...reader.parseLines(lines)]
    // Parsed, each message names the chain it spells: the order its ClOrdID, the
    // reports the venue's OrderID, and none follows anything yet.
    assert.equal(parsed[0].crosscode, 'A1')
    assert.equal(parsed[1].crosscode, 'O1')
    assert.ok(parsed.every((held) => held.seqnum === 0 && held.prevuuid === null))

    const [order, ack, twin, fill, again] = [...reader.lifecycle(parsed)]

    // The acknowledgement goes by the name the order was placed under, so it
    // follows the order and the chain keeps the order's code; the fill names only
    // the venue's identifier, which the acknowledgement went by.
    const chained = [order, ack, twin, fill]
    assert.ok(chained.every((held) => held.crosscode === 'A1'))
    assert.ok(chained.every((held) => held.crossuuid === order.crossuuid))
    assert.deepEqual([order.seqnum, order.prevuuid], [0, null])
    assert.deepEqual([ack.seqnum, ack.prevuuid], [1, order.curruuid])
    assert.equal(ack.event().prevunix, order.currunix)
    assert.deepEqual(ack.parentuuids, [order.curruuid])
    assert.deepEqual([fill.seqnum, fill.prevuuid], [2, ack.curruuid])
    // The lifecycle travels: the chain's first creation, and the state.
    assert.equal(fill.event().creatunix, order.event().creatunix)
    assert.equal(fill.state, '80FILLED')
    // The acknowledgement logged twice is one message: the second statement takes
    // the first one's place and identity, and the chain grows by nothing.
    assert.equal(twin.curruuid, ack.curruuid)
    assert.deepEqual([twin.seqnum, twin.prevuuid], [1, order.curruuid])
    // The fill ended the chain: the new order under the reused identifier starts
    // one afresh.
    assert.deepEqual([again.seqnum, again.prevuuid], [0, null])
    assert.equal(again.crosscode, 'A1')
    // The stamps are columns, reached like any typed fact. They never
    // reach the wire, and neither does a fact the walk folded: this
    // acknowledgement named no side, so the chain's buy is what the trait
    // answers and not a byte the message emits.
    assert.equal(ack.byTag(SEQNUM).asJs(), 1)
    assert.equal(ack.byTag(PREVUUID).asJs(), order.curruuid)
    assert.equal(parsed[1].side, 'UNKNOWN')
    assert.equal(ack.side, 'BUY')
    const wire = ack.intoText('|')
    assert.ok(wire.startsWith('8=FIX.4.4|35=8|11=A1|37=O1|150=0|'), wire)
    assert.ok(!wire.includes('65042='), 'no stamp is a field')

    // A chained stream replayed answers the same messages.
    const replayed = [...reader.lifecycle([order, ack, twin, fill, again])]
    assert.ok(replayed.every((held, at) => held.equals([order, ack, twin, fill, again][at])))
    ```

## A chain is named by its cross code

The cross code is what a message spells to name the thing it is about, read off the first stated of `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` and `MDReqID(262)` when the message is [built](message.md), and `crossuuid` is derived from it - the UUIDv8 of its XXH3-64 - so two messages spelling one `OrderID` share one identity before any walk reads them. A message spelling none - a heartbeat, a logon - is a chain of one: its `crossuuid` is its own `curruuid`, and it opens nothing anyone else joins.

The walk keys the live chains on that identity, and where a message arrives under an identity nothing live holds, on the names a live message goes by: the message's [`identifiers`](capture.md#the-crates-own-columns) - the message component's own [`fix:identifiers`](registry.md#component-identifiers), each under its canonical field name - are matched pair by pair, `(clordid, A1)`, against the names every live message went by, so a report stating only the `ClOrdID` an order was placed under joins the order, and a fill stating only the venue's `OrderID` joins through the acknowledgement that went by both. A message that follows takes the chain's cross code as its own, so `crosscode` and `crossuuid` name one chain whichever identifier each message chose, and a message's own spelling is still in its row and its wire. A replace's `OrigClOrdID(41)` is a different scheme from `ClOrdID(11)`, so it joins nothing by itself; the replace joins where it states the venue's `OrderID` or a `ClOrdID` a live message went by.

## A chain carries its creation and its history

Following records the predecessor's `curruuid` and `currunix` as `prevuuid` and `prevunix`, adopts the predecessor as a parent - `parentuuids` is a message's own list, so a chain is also a lineage a caller walks backward - and puts the message one place after the predecessor's, `seqnum`. The lifecycle folds with it: `creatunix` is the earliest the two know, so the chain's first creation instant travels to every message of it; the expiry is the latest and the [state](../types/codes.md#a-state-sorts-by-its-lifecycle) the furthest along, each answered by the trait rather than stamped on a column. What the message itself said - its instant, its content - stays its own, and the message is settled again once it moved, so its `currhashcode` and `curruuid` are those of the chained message and never of what it was before the walk; a stamped stream replayed answers the same messages, because a message already following its predecessor is one following changes nothing on.

## A twin is not a successor

A bridge logs one message at every hop it passes, so a capture routinely holds the same message twice: the same instant, the same content, a second line. The walk records the identity each live message *arrived* under, and a message arriving under that identity is yielded restating the live one - it takes the live one's predecessor, position and lifecycle and finalizes to the same `curruuid` - rather than chained behind it, so the chain grows by nothing and a monitor counting places in it counts messages the venue sent. Over the 94 messages of `rust/tests/fix/ulbridge.log`, a bridge's own second of capture, [`FixDedup`](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call) drops 38 adjacent republished copies before the walk, and the walk chains 35 messages to a predecessor and folds the remaining twins into their first statement.

## Snapshots are a grid

`EventIterator::with_snapshot_ns(step)` gives the walk a grid, a step in nanoseconds aligned on the epoch, and it then reads one snapshot per step per chain: the first message to reach a step its chain has not consumed is stamped with the step's opening instant as `snapunix`, and every later message in a step already consumed with none, so "is this row a snapshot" is answerable from the row and a monitor reads one row per second per order. The steps consumed go with the chain, so a chain that ended and started afresh reads its snapshots afresh. `lifecycle` reads no grid: a caller wanting one opens the walk over the messages itself.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Event, EventIterator};
    use yggdryl::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, TimeUnit};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|60=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:33.100|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|60=20260102-10:15:40.000|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    // The lines are in their messages' order, so the walk streams them; one
    // snapshot per second per chain.
    let mut walk = EventIterator::new(parsed, true).with_snapshot_ns(1_000_000_000);
    let held: Vec<FixMsg> = walk.by_ref().collect();
    let snapshots: Vec<Option<i64>> = held
        .iter()
        .map(|message| message.get_snapunix().map(|unix| unix / 1_000_000_000))
        .collect();
    // 30.250 opens the step at 30, 30.500 is in a step already read, 33.100
    // opens 33, and the new order opens 40 afresh.
    assert_eq!(snapshots, [Some(1_767_348_930), None, Some(1_767_348_933), Some(1_767_348_940)]);
    assert_eq!(held[1].get_seqnum(), 1);
    assert_eq!(held[2].get_seqnum(), 2);
    // The fill retired the chain: only the new order is alive.
    assert_eq!(walk.alive().count(), 1);
    assert_eq!(walk.snapshot_ns(), Some(1_000_000_000));
    // The stamp is the crate's own column, a nanosecond UTC clock.
    assert_eq!(
        held[0].by_tag(yggdryl::SNAPUNIX_TAG_NAME.0)?.temporal_count_at(TimeUnit::Second),
        Some(1_767_348_930),
    );
    ```

## In a batch read

The walk is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call), and a stage is a call: `codec.lifecycle_arrow_reader(reader)` chains a whole read of FIX rows under the schema it read, and `codec.arrow_reader(schema, codec.lifecycle(codec.messages(reader)))` is the same composition spelled out - the same messages and the same rows, except for [the capture's own columns](message.md#a-row-is-a-message-again), which no message holds: the door keeps each row's own cells beside its message, and the spelled-out composition has nowhere to put them, so it answers them null. `lifecycle` takes owned messages or their `Result`s (Python and JavaScript accept any iterable of messages); a source error is yielded as an item without advancing the walk, and only exhaustion fuses. Nothing chains unasked: a parse answers messages that follow nothing, because a stamped value is indistinguishable from a stated one, and a batch that is walked twice is walked into the same rows.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::Folder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let codec = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix")?;
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|60=20260102-10:15:30.000|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=F|39=2|14=100|151=0|55=AAPL|60=20260102-10:15:31.000|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    let read = codec.lifecycle_arrow_reader(codec.arrow_reader(schema.clone(), parsed)?)?;
    let batch = read.into_iter().next().expect("one batch")?;
    assert_eq!(batch.num_rows(), 2);
    let rows = yggdryl::arrow::batch_to_value(&batch)?;
    let rows = rows.as_sequence().expect("rows");
    let (seqnum, prevuuid, crossuuid) = (
        schema.index_of("seqnum").expect("a column"),
        schema.index_of("prevuuid").expect("a column"),
        schema.index_of("crossuuid").expect("a column"),
    );
    // One order, one chain: the fill is the second in it and names the
    // order before it.
    assert_eq!(rows[0].get(crossuuid), rows[1].get(crossuuid));
    assert_eq!(rows[0].get(seqnum), Some(&Scalar::Null), "a first message states no place");
    assert_eq!(rows[1].get(seqnum), Some(&Scalar::from(1_u64)));
    assert!(rows[0].get(prevuuid).is_some_and(Scalar::is_null));
    assert!(rows[1].get(prevuuid).is_some_and(|held| !held.is_null()));
    ```

## Edges

- Two messages of one chain at the same instant are neither after nor before one another: the walk keeps them in arrival order, and the second follows the first.
- A message that happened before the live one it would follow - out of order on a walk a caller opened as sorted - is yielded as it came and changes nothing; `lifecycle` sorts first, so nothing arrives out of order there.
- A message the reading refuses - its own predecessor, one following changes nothing on - is yielded as it came and still stands as the live one.
- A terminal state ends the chain once the message is yielded; a message arriving under the retired identity starts a chain afresh at `seqnum` 0, sharing the `crossuuid` and nothing else.
- A message whose expiry - `ExpireTime(126)`, else `ValidUntilTime(62)`, `ExpireDate(432)` or the instrument's `MaturityDate(541)` - is at or before its `currunix` is not alive and opens no chain; one whose expiry is later stays alive until a message past it arrives.
- The state is the furthest along the two know, as `CodeValue::merge_with` reads a state, so a chain never moves backward: a `New` after a `Filled` under a live chain would be yielded `Filled`; it is not, because the fill retired the chain first.
- A cleared or absent cross code keeps a message in a chain of one; nothing derives one from the identifiers alone, and a message in no chain joins one only by a name a live message goes by.
- `restating` is never refused: the walk gives the word only for an arrival under the identity the live message arrived under, which a later statement of the same instant and content has and a successor never does.
- The grid floors: an instant before the epoch falls in the step opening below it.
- Errors: `lifecycle` yields a source error where the source had it, then the message it pulled while the error was met; the walk's state is unchanged by an error.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test graph
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix schema::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/fix -k lifecycle
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/fix/*.test.js"
    ```
