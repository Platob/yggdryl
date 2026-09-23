# Lifecycle

A message says what happened; it does not say which order's life it belongs to beyond the identifiers a venue chose. `FixCodec::lifecycle` is the one [walk](../graph.md) over messages: each is stated as the one after the live message of its chain - the chain named by the cross code every incarnation of one order shares - so a chained message carries its predecessor's identity and instant, its place in the chain and the lifecycle carried forward, and a monitor joins an order's whole life on `crossuuid` rather than rebuilding it from `ClOrdID`, `OrigClOrdID` and `OrderID` on its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::lifecycle`, `FixCodec::lifecycle_arrow_reader`; the walk itself is [`graph::EventIterator`](../graph.md), configured by the codec's snapshot width or opened directly by a Rust caller |
| Columns | the [crate's own](capture.md#the-crates-own-columns): `crosscode` (65048), `crosshashcode` (65018) and `crossuuid` (65040) name the chain; `prevuuid` (65022), `prevunix` (65021) and `seqnum` (65042) place a message in it; `creaunix` (65023), `exprtime` (65053) and `state` (65052) carry creation, the current generation's deadline and state; `execunix` (65062) is the precise execution instant, `recdunix` (65063) the earliest precise recording instant its statements know, and `refrecdunix` (65064) the persisted recording clock of the observation selected as merge reference; `snapunix` (65025) is a grid view's stamp. `currhashcode` (65017) and `curruuid` (65039) are settled after a source message moves; a view keeps them unchanged |
| Chain name | the cross code: an explicit nonempty value, else the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)`; `crosshashcode` is its XXH3-64 and `crossuuid` the UUIDv8 of that, so every message spelling one code shares one identity whatever else it says. A complete `msgtype`, capture session, capture context and message sequence separately build `identifiers["msgsesseventid"]`; it neither chooses the cross code nor changes the FIX content identity. A message naming no cross code is a chain of one: its `crossuuid` is its own `curruuid` |
| Joins | a message arriving under the identity a live message holds follows it; one arriving under no live identity, but going by a name a live message goes by - the same `(scheme, value)` in its [`identifiers`](capture.md#the-crates-own-columns), a report stating only the `ClOrdID` an order was placed under - follows that one, and takes the chain's cross code as its own |
| Follows | before following, equal complete `msgsesseventid` values force a full content-and-graph merge: the latest recorded observation is the reference, while `recdunix` and `execunix` become the earliest values either statement knows. Otherwise the predecessor's `curruuid` and `currunix` are recorded, `seqnum` is one past the predecessor's, the chain's cross code and prior names carry, and so do the earliest `creaunix`, the newer message's explicit `exprtime` where stated else the predecessor's, the furthest [state](../types/codes/state.md#the-rank-leads), and the later of the predecessor's latest execution clock and this event's precise `execunix`. `recdunix` and `refrecdunix` remain this observation's own. A missing `parentorderid` takes a different predecessor `OrderID(37)`, and a missing `ClOrdID(11)` takes the predecessor's; an explicitly stated link always wins. An absent `symbolticker` inherits the predecessor's ticker even when the timed link already exists; an explicitly stated ticker, including an empty string, remains. A full merge persists its selected latest recording as `refrecdunix`, so another merge selects the same reference after `recdunix` has become the earliest observation. A message that moved is settled again, so its `currhashcode` and `curruuid` are its own |
| Twins | a message arriving under the identity the live one *arrived* under - the same instant and content, one message a bridge logged at every hop it passed - is another statement of it, not the one after it: it takes the live one's place, predecessor, position and lifecycle, keeps the earliest execution and public recording instants and the latest merge-reference recording clock either observation states, and finalizes to the same `curruuid`, so the chain grows by nothing |
| Dating | a message whose `SendingTime(52)` the parse supplied rather than read - a carrier's, the codec's default, the intake's clock - is dated by the `TransactTime(60)` it states with a clock before the walk, and a resend's `OrigSendingTime(122)` is its creation where earlier; a stated sending clock stands, because the parse has already dated the message by [the official clock](capture.md#the-official-clock-dates-the-message) standing within `official_time_delay_ms` of it. No delay bounds this one: a clock nobody stated is no reference to measure a distance from. `FixMsg::dated_by_transaction` is the reading, so a capture whose frames state no sending clock still orders, expires and folds by when its transactions happened. Independently, a report accepted by `FixMsg::is_execution` uses its directly parsed execution clock where present and otherwise its final `currunix`; this includes an initial `35=AE` whose `TradeReportTransType(487)` is absent/New and whose `ExecType(150)` is absent or execution-like. That latest clock propagates to later lifecycle events and never moves backward; non-New/cancel/correct/reverse/status AE and `AD`, `AQ` or `AR` invent none. `recdunix` is only the precise carrier recording time or a directly stated value, never a sending, original-sending or hop clock |
| Order | the entire finite capture is collected; capture-identical observations are merged, then the retained messages are stably sorted by event time before the walk. Intake errors are retained and yielded first because sorting cannot preserve their position among messages |
| Deliveries | a complete nonempty `(msgtype, msgsessionid, msgctxid, msgseqnum)` is prebuilt as `msgsesseventid`: `<msgtype-byte-len>:<msgtype>|<session-byte-len>:<session>|<context-byte-len>:<context>|<msgseqnum>`, with UTF-8 byte lengths and the final sequence rendered as canonical `u64`. Equal values are one delivery before sorting, walking or ordinary with-previous following. The observation with greatest `recdunix` is the retained reference row; an already merged row carries that greatest original clock in `refrecdunix`. A full FIX-content and graph merge folds the others into it, so it takes no predecessor or extra chain place. Reference scalar conflicts win, older values fill absences, and repeating groups merge recursively at equal occurrence indexes with members sorted by FIX tag and no duplicate key. The merged `recdunix` and `execunix` facts are their earliest values, `refrecdunix` persists the latest reference clock, and provenance sources form a sorted unique union whose positions carry no reference meaning. Separately, exact republications and true retransmissions are removed across the entire finite capture, however many distinct deliveries intervene; session, sequence, original time and content distinguish normal deliveries, and headerless bridge rows use their event identity and capture context. An absent or empty text part or absent sequence produces no `msgsesseventid`; distinct message types, sessions, contexts and sequences survive |
| Instruments | after sorting, one lifecycle-local registry learns validated CFI, Bloomberg and FIGI associations by ISIN, plus CUSIP or SEDOL only when a normalized row or setter stated them, and fills only later missing facts. Coarse CFI is not learned; conflicting values make that association ambiguous and silent. Storage reserves 1 KiB for each first valid ISIN, at most 32 MiB or 32,768 ISINs; a known ISIN can keep learning at the cap |
| Ends | a terminal state retires the chain once yielded. A live finite deadline emits one owned `95EXPIRED` message at that exact instant, following the live generation with `prevuuid` and the next `seqnum`, then purges it; replaced or terminal generations leave no stale expiry |
| Grid | a positive codec `snapshot_ns` / `snapshotNs` enables an epoch-aligned grid; the default is off. Every crossed tick yields an owned view of every living chain, changing only `snapunix`. Deadlines win ties, all source messages at the instant follow, and views come last. Output is proportional to crossed ticks times living identities; there is no implicit output cap |
| Errors | intake errors are yielded before the sorted messages and never advance the walk; exhaustion is fused |
| Entries | delivery folding and missing order-link propagation rebuild the affected content canonically; stated values always win. The graph-only chain columns remain off the wire |
| Trade projection | lifecycle retains every FIX observation as one message; it neither filters trade reports nor manufactures child FIX rows. At the later market boundary only an initial TradeCaptureReport `35=AE` with `TradeReportTransType(487)` absent/New and `ExecType(150)` absent or execution-like is accepted; non-New/cancel/correct/reverse/status AE are refused there. Standalone market conversion also refuses `AD`, `AQ` and `AR`; the composed book reader ignores them and other records outside order/quote categories, actual executions and `W`/`X`. Accepted output passed to `FixMarketIterator` or `book_arrow_reader` is projected from its `NoSides(552)` occurrences into one composite graph `Trade`, preserving the enriched root and deriving one explicitly sided child execution per occurrence |
| Bindings | Rust; Python `FixCodec.lifecycle`, `lifecycle_arrow_reader`; JavaScript `lifecycle`, `lifecycleArrowReader` |

## Use

One order's life: the order under its client identifier, the acknowledgement under the venue's, that acknowledgement logged a second time on its way through a bridge, the fill naming the venue's alone, and a new order reusing the client identifier after the fill. The repeated delivery is omitted before the walk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, PREVUUID_TAG_NAME, SEQNUM_TAG_NAME};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 5] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-10:15:33.100|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|52=20260102-10:15:40.000|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    // Parsed, each message names the chain it spells: the order its ClOrdID,
    // the reports the venue's OrderID, and none follows anything yet.
    assert_eq!(parsed[0].get_crosscode(), "A1");
    assert_eq!(parsed[1].get_crosscode(), "O1");
    assert!(parsed.iter().all(|held| held.get_seqnum() == 0 && held.get_prevuuid().is_none()));

    let chained: Vec<FixMsg> = reader.lifecycle(parsed.clone()).collect::<yggdryl::Result<_>>()?;
    let [order, ack, fill, again] = chained.as_slice() else { panic!("four messages") };

    // The acknowledgement goes by the name the order was placed under, so it
    // follows the order and the chain keeps the order's code; the fill names
    // only the venue's identifier, which the acknowledgement went by.
    assert!(chained.iter().take(3).all(|held| held.get_crosscode() == "A1"));
    assert!(chained.iter().take(3).all(|held| held.get_crossuuid() == order.get_crossuuid()));
    assert_eq!((order.get_seqnum(), order.get_prevuuid()), (0, None));
    assert_eq!((ack.get_seqnum(), ack.get_prevuuid()), (1, Some(order.get_curruuid())));
    assert_eq!(ack.get_prevunix(), Some(order.get_currunix()));
    assert_eq!((fill.get_seqnum(), fill.get_prevuuid()), (2, Some(ack.get_curruuid())));
    // A message names only the one before it: the chain further back is read
    // by following `prevuuid` from one message to the next.
    let before_fill = chained.iter().find(|held| Some(held.get_curruuid()) == fill.get_prevuuid());
    assert_eq!(before_fill.and_then(|held| held.get_prevuuid()), Some(order.get_curruuid()));
    // The lifecycle travels: the chain's first creation, and the state.
    assert_eq!(fill.get_creaunix(), order.get_creaunix());
    assert_eq!(fill.get_state().as_str(), "80FILLED");
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
    assert!(wire.starts_with("8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|"), "{wire}");
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
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-10:15:33.100|10=0|",
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|52=20260102-10:15:40.000|10=0|",
    ]
    parsed = list(reader.parse_lines(lines))
    # Parsed, each message names the chain it spells: the order its ClOrdID, the
    # reports the venue's OrderID, and none follows anything yet.
    assert parsed[0].crosscode == "A1"
    assert parsed[1].crosscode == "O1"
    assert all(held.seqnum == 0 and held.prevuuid is None for held in parsed)

    order, ack, fill, again = reader.lifecycle(parsed)

    # The acknowledgement goes by the name the order was placed under, so it
    # follows the order and the chain keeps the order's code; the fill names only
    # the venue's identifier, which the acknowledgement went by.
    chained = [order, ack, fill]
    assert all(held.crosscode == "A1" for held in chained)
    assert all(held.crossuuid == order.crossuuid for held in chained)
    assert (order.seqnum, order.prevuuid) == (0, None)
    assert (ack.seqnum, ack.prevuuid) == (1, order.curruuid)
    assert ack.event().prevunix == order.currunix
    assert (fill.seqnum, fill.prevuuid) == (2, ack.curruuid)
    # A message names only the one before it: the chain further back is read by
    # following prevuuid from one message to the next.
    before_fill = next(held for held in chained if held.curruuid == fill.prevuuid)
    assert before_fill.prevuuid == order.curruuid
    # The lifecycle travels: the chain's first creation, and the state.
    assert fill.event().creaunix == order.event().creaunix
    assert fill.state.as_py() == "80FILLED"
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
    assert wire.startswith("8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|")
    assert "65042=" not in wire  # no stamp is a field

    # A chained stream replayed answers the same messages.
    assert list(reader.lifecycle([order, ack, fill, again])) == [order, ack, fill, again]
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
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-10:15:33.100|10=0|',
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=50|52=20260102-10:15:40.000|10=0|',
    ].map((line) => Buffer.from(line))
    const parsed = [...reader.parseLines(lines)]
    // Parsed, each message names the chain it spells: the order its ClOrdID, the
    // reports the venue's OrderID, and none follows anything yet.
    assert.equal(parsed[0].crosscode, 'A1')
    assert.equal(parsed[1].crosscode, 'O1')
    assert.ok(parsed.every((held) => held.seqnum === 0 && held.prevuuid === null))

    const [order, ack, fill, again] = [...reader.lifecycle(parsed)]

    // The acknowledgement goes by the name the order was placed under, so it
    // follows the order and the chain keeps the order's code; the fill names only
    // the venue's identifier, which the acknowledgement went by.
    const chained = [order, ack, fill]
    assert.ok(chained.every((held) => held.crosscode === 'A1'))
    assert.ok(chained.every((held) => held.crossuuid === order.crossuuid))
    assert.deepEqual([order.seqnum, order.prevuuid], [0, null])
    assert.deepEqual([ack.seqnum, ack.prevuuid], [1, order.curruuid])
    assert.equal(ack.event().prevunix, order.currunix)
    assert.deepEqual([fill.seqnum, fill.prevuuid], [2, ack.curruuid])
    // A message names only the one before it: the chain further back is read by
    // following prevuuid from one message to the next.
    const beforeFill = chained.find((held) => held.curruuid === fill.prevuuid)
    assert.equal(beforeFill.prevuuid, order.curruuid)
    // The lifecycle travels: the chain's first creation, and the state.
    assert.equal(fill.event().creaunix, order.event().creaunix)
    assert.equal(fill.state, '80FILLED')
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
    assert.ok(wire.startsWith('8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|'), wire)
    assert.ok(!wire.includes('65042='), 'no stamp is a field')

    // A chained stream replayed answers the same messages.
    const replayed = [...reader.lifecycle([order, ack, fill, again])]
    assert.ok(replayed.every((held, at) => held.equals([order, ack, fill, again][at])))
    ```

## A chain is named by its cross code

The cross code is what a message spells to name the thing it is about: an explicit nonempty value, else the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)`, read when the message is [built](message.md). `crossuuid` is the UUIDv8 of its XXH3-64, so two messages spelling one `OrderID` share one identity before any walk reads them. The separate `msgsesseventid` names one captured delivery only when its message type, session, context and sequence are all present; it never chooses the chain or changes the FIX content identity. A message spelling no cross code - a heartbeat, a logon - is a chain of one: its `crossuuid` is its own `curruuid`, and it opens nothing anyone else joins.

The walk keys the live chains on that identity, and where a message arrives under an identity nothing live holds, on the names a live message goes by: the message's [`identifiers`](capture.md#the-crates-own-columns) - the message component's own [`FIX:identifiers`](registry.md#component-identifiers), each under its canonical field name - are matched pair by pair, `(clordid, A1)`, against the names every live message went by, so a report stating only the `ClOrdID` an order was placed under joins the order, and a fill stating only the venue's `OrderID` joins through the acknowledgement that went by both. A message that follows takes the chain's cross code as its own, so `crosscode` and `crossuuid` name one chain whichever identifier each message chose, and a message's own spelling is still in its row and its wire. A replace's `OrigClOrdID(41)` is a different scheme from `ClOrdID(11)`, so it joins nothing by itself; the replace joins where it states the venue's `OrderID` or a `ClOrdID` a live message went by.

## A chain carries its creation and its history

With-previous first compares `msgsesseventid`. Equal complete values force a full merge and stop: no predecessor, lifecycle position or order-link inheritance is stamped. Otherwise following records the predecessor's `curruuid` and `currunix` as `prevuuid` and `prevunix` and puts the message one place after the predecessor's, `seqnum`. The predecessor is the one message a chained message names: nothing records the chain further back, so a caller reads a chain by following `prevuuid` from row to row. The `srcuuids` a message states - the text line it was parsed out of - are its own and travel nowhere: provenance names what a node was read from, never what it follows. `creaunix` is the earliest the two know and the [state](../types/codes/state.md#the-rank-leads) the furthest along. An `exprtime` the newer message explicitly states is the current generation's deadline even where it is earlier; only an absent deadline is inherited. The walk fills a missing execution instant from this message's `currunix` only when `FixMsg::is_execution` accepts the report, then carries the later of that clock and the predecessor's `execunix`; `recdunix` and `refrecdunix` remain this observation's own. It also fills an absent `parentorderid` from the exact predecessor's different `OrderID(37)` and an absent `ClOrdID(11)` from that predecessor's `ClOrdID`; neither overwrites a stated value. An absent `symbolticker` likewise carries from the predecessor, even when the timed link is already present, while an explicit ticker remains. The timed and market readings settle before one finalization, and only an inherited ticker is cloned. A full merge instead takes the statement with the latest recording clock as reference for conflicting content and identifiers while the provenance list becomes a sorted unique union, keeping the latest expiry, earliest duplicate-observation execution and public recording facts, and latest reference recording clock either statement knows. Persisting that clock separately means a third statement selects the same reference after either grouping even though `recdunix` exposes the earliest observation. The message is settled again once it moved, so its `currhashcode` and `curruuid` are those of the merged or chained message and never of what it was before the walk; a stamped stream replayed answers the same messages, because a message already following its predecessor with all inherited facts is one following changes nothing on.

Read across the three steps, provenance and the chain are joins on one column set: a message's `srcuuids` are the `curruuid` of the [line](../media/index.md#plain-text) it was parsed out of, a chained message's `prevuuid` is the `curruuid` of the message before it, and the line's batch, the parsed row and the chained row contain the same eighteen [event columns](../graph.md#columns) under one name and one datatype each, so the joins need no mapping.

## A twin is not a successor

A bridge logs one message at every hop it passes, so a capture routinely holds the same delivery twice. Each observation with all four delivery parts prebuilds `msgsesseventid` as `<msgtype-byte-len>:<msgtype>|<session-byte-len>:<session>|<context-byte-len>:<context>|<msgseqnum>`; the three text lengths count UTF-8 bytes and the final sequence is canonical `u64`, so delimiters inside capture text cannot collide. `lifecycle` fully merges equal values before sorting and walking instead of making one follow the other, and direct with-previous applies the same forced merge before ordinary following. Raw observations are ranked by `recdunix`; a merged observation persists the greatest original recording in `refrecdunix`, so the greatest recording remains the reference row across regrouping. Its stated scalar wins each conflict and older observations fill only what it leaves absent. Repeating-group occurrence `i` merges with occurrence `i` recursively; every merged level is sorted by FIX tag then folded name and contains one logical key, so marked and bare bridge spellings at the same indexes do not append duplicate occurrences or members. Both source UUIDs remain; the merged `execunix` and `recdunix` are their earliest values, while `refrecdunix` records the latest selected reference. What that fold costs is worth stating plainly, because nothing bounds it: an unbounded number of observations sharing one `msgsesseventid` fold into one event, however far apart their clocks and however much their content diverges, since the fold is deliberately blind to the content identities that would otherwise keep them apart - a bridge capture where one message is logged at a dozen hops routinely answers a chain several times shorter than its observation count. The bounds this walk states are all the other way round, naming what never folds: an incomplete key, a different message type, session, context or sequence, a distinct delivery carrying equal business content, and a lifecycle output or snapshot view read back in.

`srcuuids` is the sorted unique provenance union. Which observation becomes the merge reference is settled separately by the greatest `refrecdunix`/`recdunix` and then the later `currunix`, with an exact tie keeping the caller's leading statement; no list position identifies that reference or an arrival order. Feeding the same observations in another order therefore leaves the serialized provenance list unchanged. A missing or empty text part or missing sequence produces no `msgsesseventid`, and a different message type, context, session or sequence is a different delivery. Already placed lifecycle outputs and snapshot views are not raw capture observations and are never coalesced again on replay.

After that merge, `lifecycle` removes an exact republication or true retransmission across the whole finite capture, however many distinct deliveries it holds: a complete FIX header keys the session, sequence, original time and recorded canonical content identity, while a headerless bridge row must match the recorded event identity and content in the same capture session, context and direction. Reordering projected fields during Arrow reconstruction does not create another delivery. When `SendingTime(52)` was unstated, the retained event time supplies its clock; a replay's stated `OrigSendingTime(122)` takes precedence. The retained delivery set therefore grows with distinct deliveries in that finite capture; it is not a recent window.

After delivery deduplication, the walk still recognizes two source statements that arrive under the same event identity: the later statement takes the live one's predecessor, position and lifecycle and finalizes to the same `curruuid`, so the chain grows by nothing. Deduplication decides whether the finite capture delivered the same message twice; restating decides whether two retained graph events are statements of one event.

## Sided trades after the lifecycle

Lifecycle enrichment remains message-shaped: one retained FIX observation is one `FixMsg`, including any canonically merged `NoSides(552)` group. Decomposition belongs to the Rust [FIX-to-market boundary](message.md#market-operations), after duplicate folding, sorting and predecessor propagation have settled that root. `FixMarketIterator` accepts only an initial `35=AE` whose `TradeReportTransType(487)` is absent/New and whose `ExecType(150)` is absent or execution-like; it refuses non-New/cancel/correct/reverse/status AE and every `AD`, `AQ` or `AR` at `$.MsgType(35)`. An accepted report must have exactly one nonempty `NoSides` group and becomes one composite graph `Trade`. Every occurrence must state a bid or ask `Side(54)`; a missing side refuses at `$.NoSides(552)[index].Side(54)` rather than guessing from another occurrence or the root.

Each child execution starts from the lifecycle-enriched root, then applies the occurrence's side quantity, average price, currency and identifiers. Its cross code uses a tag-qualified stable side/order ID where stated, otherwise the occurrence's canonical 128-bit content digest; no `TradeSideIndex` identifier or source group index enters the identity. The composite constructor canonicalizes child order and derives a root identity from their canonical UUIDs. Passing `codec.lifecycle(messages)` to `book_arrow_reader` therefore keeps one trade operation at the market boundary and yields all of its sided executions in the book's nested `executions` list. Calling `book_arrow_reader` directly performs no lifecycle enrichment of its own. It ignores administration, requests, acknowledgements and non-executing reports, while admitting every `AE` to strict projection so correction, cancellation and status refusals remain visible. Source errors and malformed admitted records also stop the reader.

## Instrument associations are lifecycle state

Messages are sorted before one lifecycle-local instrument registry reads them. A valid ISIN may teach its precise CFI, Bloomberg code and FIGI to later messages that state the same ISIN and omit that code. CUSIP and SEDOL participate only when a semantic row or instrument setter stated their normalized facts; raw FIX source `1` and `2` identifiers remain under `SecurityID` or `secaltids` and are not lifted for learning. The registry never overwrites a stated fact. A coarse CFI such as `ESXXXX`, an invalid spelling and a null marker teach nothing; two valid conflicting values make that code family ambiguous and it stays silent.

The registry retains at most 32 MiB by reserving a conservative 1 KiB for each first valid ISIN, so at most 32,768 ISINs register. Reaching the cap refuses unseen ISINs without eviction; an already registered ISIN can still learn another code because its full payload was reserved on first insertion. The registry belongs to this ordered lifecycle, never to the codec, so a second lifecycle starts empty.

## Snapshots are a grid

The codec's positive `snapshot_ns` / `snapshotNs` gives its lifecycle an epoch-aligned grid; zero, a negative width, `None` / `null`, or omission disables it, which is the default. At every crossed tick the walk yields a separate owned view of every living message. The view keeps the live message's `curruuid`, `seqnum` and content and changes only `snapunix`; source messages keep the snapshot fact they stated and are never stamped backward to a previous tick.

At one instant, finite expirations come first, then every source message at that instant, then its views. Expiration emits one owned `95EXPIRED` message at the exact deadline with the live message as predecessor and the next sequence number, then purges the identity; a replaced or ended generation's stale deadline emits nothing. At EOF the grid reaches the greatest finite deadline still encountered, or the last source instant where none remains, and stops; a nonexpiring message does not make a finite capture infinite. The width is a caller's output choice: the number of rows is proportional to crossed ticks times identities alive at each tick, with no implicit count or span cap.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(registry).with_snapshot_ns(1_000_000_000);
    assert_eq!(codec.snapshot_ns(), Some(1_000_000_000));
    let source = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|",
    )?;
    let source_uuid = source.get_curruuid();
    let deadline = source.get_exprtime().expect("the stated expiry");
    let walked: Vec<_> = codec.lifecycle([source]).collect::<yggdryl::Result<_>>()?;

    let snapshots: Vec<_> = walked
        .iter()
        .filter(|message| message.get_snapunix().is_some())
        .collect();
    assert_eq!(snapshots.len(), 2, "the source tick and the crossed second");
    assert!(snapshots.iter().all(|view| {
        view.get_curruuid() == source_uuid
            && view.get_seqnum() == 0
            && view.get_snapunix().is_some_and(|tick| tick < deadline)
    }));
    let expired = walked.last().expect("the deadline event");
    assert_eq!((expired.get_currunix(), expired.get_state().as_str()), (deadline, "95EXPIRED"));
    assert_eq!((expired.get_prevuuid(), expired.get_seqnum()), (Some(source_uuid), 1));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry, snapshot_ns=1_000_000_000)
    assert codec.snapshot_ns == 1_000_000_000
    source = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|"
    )
    source_uuid = source.curruuid
    deadline = source.event().exprtime
    assert deadline is not None
    walked = list(codec.lifecycle([source]))

    snapshots = [message for message in walked if message.event().snapunix is not None]
    assert len(snapshots) == 2
    assert all(
        message.curruuid == source_uuid
        and message.seqnum == 0
        and message.event().snapunix < deadline
        for message in snapshots
    )
    expired = walked[-1]
    assert (expired.currunix, expired.state.as_py()) == (deadline, "95EXPIRED")
    assert (expired.prevuuid, expired.seqnum) == (source_uuid, 1)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry, { snapshotNs: 1_000_000_000n })
    assert.equal(codec.snapshotNs, 1_000_000_000n)
    const source = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|',
    ))
    const sourceUuid = source.curruuid
    const deadline = source.event().exprtime
    const walked = [...codec.lifecycle([source])]

    const snapshots = walked.filter((message) => message.event().snapunix !== null)
    assert.equal(snapshots.length, 2)
    assert.ok(snapshots.every((message) =>
      message.curruuid === sourceUuid
      && message.seqnum === 0
      && message.event().snapunix < deadline,
    ))
    const expired = walked.at(-1)
    assert.deepEqual([expired.currunix, expired.state], [deadline, '95EXPIRED'])
    assert.deepEqual([expired.prevuuid, expired.seqnum], [sourceUuid, 1])
    ```

## In a batch read

The walk is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call), and a stage is a call: `codec.lifecycle_arrow_reader(reader)` chains a whole read of FIX rows under the schema it read, and `codec.arrow_reader(schema, codec.lifecycle(codec.messages(reader)))` is the same composition spelled out - the same messages and the same rows, [the capture's own columns](message.md#a-row-is-a-message-again) included: `messages` reads each row's own cells into the message it makes and `into_row` states them again at their columns, so the spelled-out composition keeps them exactly as the door does. `lifecycle` takes owned messages or their `Result`s (Python and JavaScript accept any finite iterable). It consumes the input before yielding because stable event-time sorting, capture-wide delivery deduplication and ordered association learning need the complete capture; source errors are queued and yielded before the sorted messages. Nothing chains unasked: a parse answers messages that follow nothing, because a stamped value is indistinguishable from a stated one, and a batch that is walked twice is walked into the same rows.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
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
- A message whose expiry - `ExpireTime(126)`, else `ValidUntilTime(62)`, `ExpireDate(432)` or the instrument's `MaturityDate(541)` - is at or before its `currunix` is not alive and opens no chain. A later deadline is scheduled even where no later source arrives: one `95EXPIRED` message is emitted at the deadline and the identity is purged.
- The state is the furthest along the two know, as `CodeValue::merge_with` reads a state, so a chain never moves backward: a `New` after a `Filled` under a live chain would be yielded `Filled`; it is not, because the fill retired the chain first.
- A cleared or absent cross code keeps a message in a chain of one; nothing derives one from the identifiers alone, and a message in no chain joins one only by a name a live message goes by.
- `restating` is never refused: the walk gives the word only for an arrival under the identity the live message arrived under, which a later statement of the same instant and content has and a successor never does.
- The grid begins at the first aligned tick at or after the first source instant; it never stamps a source with a tick before that source existed.
- Errors: sorting consumes the finite source first, then `lifecycle` yields every intake error before its messages; no error advances the walk.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test graph
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix schema::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_fix.py -k lifecycle
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js node/tests/fix/catalog.test.js
    ```
