# Lifecycle

A message says what happened; it does not say which order's life it belongs to beyond the identifiers a venue chose. `FixCodec::lifecycle` is the one [walk](../graph/event.md#lifecycle-walk) over messages: each is stated as the one after the live message of its chain - the chain named by the cross code every incarnation of one order shares - so a chained message carries its predecessor's identity and instant and the lifecycle carried forward, and a monitor joins an order's whole life on `crossuuid` rather than rebuilding it from `ClOrdID`, `OrigClOrdID` and `OrderID` on its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::lifecycle`, `FixCodec::lifecycle_arrow_reader`; the walk itself is [`graph::EventIterator`](../graph/event.md#lifecycle-walk), configured by the codec's snapshot width or opened directly by a Rust caller |
| Columns | the [crate's own](capture.md#the-crates-own-columns): `crosscode` (65010), `crosshashcode` (65012) and `crossuuid` (65009) name the chain; `prevuuid` (65013) and `prevunix` (65005) place a message in it; `seqnum` (65014) is its place among the messages of its instant; `creaunix` (65004), `exprunix` (65007) and `state` (65029) carry creation, the current generation's deadline and state; `execunix` (65002) is the precise execution instant and `recdunix` (65003) the earliest precise recording instant its statements know; `msgsesseventid` (65021) is the delivery a twin is matched by; `snapunix` (65006) is a grid view's stamp. `currhashcode` (65011) and `curruuid` (65008) are settled after a source message moves; a view keeps the live message's `currhashcode` and takes the `curruuid` its tick derives |
| Chain name | the cross code: an explicit nonempty value, else the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)`, stored under the side an order, a quote or an execution message takes - `BUYS:A1` - so a buy and a sell under one identifier are two chains, and as spelled for a message taking `UNKN` or filed under any other category; `crosshashcode` is its XXH3-64 and `crossuuid` the UUIDv8 of that, so every message spelling one code on one side shares one identity whatever else it says. A complete `msgtype`, capture session, capture context and message sequence separately build the capture's `msgsesseventid`; it neither chooses the cross code nor changes the FIX content identity. A message naming no cross code is a chain of one: its `crossuuid` is its own `curruuid`, which a message following it keeps as its own |
| Joins | a live chain is keyed by its cross code and its market data kind (`marketdatakind`: `ORDR`, `QUOT`, `EXEC`, `TRAD`, ...), and a message joins only a chain of its own kind; a message arriving under the identity a live message holds follows it; one arriving under no live identity, but going by a name a live message of its side goes by - the same `(key, value)` in its alternate identifiers, [`get_altids()`](message.md#the-identifier-maps), a report stating only the `ClOrdID` an order was placed under - follows that one, and takes the chain's cross code as its own. A message stating no side joins the one side alive under its base cross code, else under the first name it shares with a live message, where exactly one side is alive there, and takes that side; where both are, it starts a chain of its own. An execution joins nothing by a name or a base: it is a chain of its own, followed only under its own cross code |
| Follows | before following, equal complete `msgsesseventid` values force a full content-and-graph merge: the latest recorded observation is the reference, the later `currunix` closing a tie, while `recdunix` and `execunix` become the earliest values either statement knows. Otherwise the predecessor's `curruuid` and `currunix` are recorded, `seqnum` stays the message's own unless the predecessor happened at the same instant or later, where it takes the higher of its own and one past the predecessor's, the chain's cross code carries, and so do the earliest `creaunix`, the newer message's explicit `exprunix` where stated else the predecessor's, the furthest [state](../types/enum/state.md#the-code-is-the-rank) - a `NEW` stated over a live new-like predecessor [reads `UPDATED`](../types/enum/state.md#stated-anew-over-a-live-one) - and the later of the predecessor's latest execution clock and this event's precise `execunix`. `recdunix` remains this observation's own. A missing `parentorderid` takes a different predecessor `OrderID(37)`, and a missing `ClOrdID(11)` takes the predecessor's; an explicitly stated link always wins. An absent `ticker` inherits the predecessor's even when the timed link already exists; a stated one remains. The FIX reading is `Operation::following_operation`: the time in force and whether the instrument can trade carry where this message states neither, and so does every `metadata` key of the chain it does not state - a bridge's namespaced keys, its own values standing - and every alternate identifier whose `FIX:idmap` entry follows - the order's own and its parents', and only those; its accounts are its fields' and none follows. A merged statement ranks against a third by the earliest `recdunix` it keeps, so which of three statements leads depends on the order they merge in. A message that moved is settled again, so its `currhashcode` and `curruuid` are its own and digest what it took, moving where it took keys |
| Places | every message the walk reads takes its [place](#a-place-counts-one-instant) among the messages of its instant by content - after sorting, redating and dropping repeat deliveries, before it is keyed or chained: the expirations of an instant come first, each at the next place of its deadline, then each content read at that instant takes the next place, and a content repeated there takes the place it already took, so a message a bridge logged at two hops is one identity; the next instant starts again at 0. What a walk made - a grid view, an expiration at its deadline - keeps the place the walk gave it when the output is walked again. A place is where a message stands, never what it says, so it is outside `currhashcode` and reaches `curruuid` alone |
| Creation | every walked message leaves stating `creaunix`: one stating none takes its chain's - the earliest the fold keeps - or, starting a chain, its own instant; a stated one is never replaced. A parse already states one on every message, so this reads for an event a caller built without one |
| Unknowns | a statement that knows the instrument or the market leads one that does not: an ISIN numbered `ZZ` yields to a real one, an unclassified CFI (`XXXXXX`) to a classified one and the no-market MIC `XXXX` to a market, whether one statement follows another or two merge |
| Twins | a message arriving under the identity the live one *arrived* under - the same instant and content, one message a bridge logged at every hop it passed - is another statement of it, not the one after it: it takes the live one's place, predecessor, snapshot, cross code, lifecycle and every `metadata` key it does not state, keeps the earliest execution and recording instants either observation states, and finalizes to the same `curruuid`, so the chain grows by nothing - and the window below yields that identity once |
| Once | what the walk yields is [yielded once](#an-identity-is-yielded-once) within the codec's `dedup_window_ms` / `dedupWindowMs` of event time, one minute by default: a message whose `curruuid` the walk already yielded, no more than the window before the latest `currunix` it yielded, is dropped - a restated twin, a hop logged again after its chain moved on - while the walk still reads it. A grid view is exempt, a nil identity never repeats, and a nonpositive window, `None` or `null` yields everything the walk answers |
| Dating | a message whose `SendingTime(52)` the parse supplied rather than read - a carrier row's, its line's `currunix`, the codec's default, the intake's clock - is dated by the `TransactTime(60)` it states with a clock before the walk, and a resend's `OrigSendingTime(122)` is its creation where earlier; a stated sending clock stands, because the parse has already dated the message by [the official clock](capture.md#the-official-clock-dates-the-message) standing within `official_time_delay_ms` of it. No delay bounds this one: a clock nobody stated is no reference to measure a distance from. `FixMsg::dated_by_transaction` is the reading, so a capture whose frames state no sending clock still orders, expires and folds by when its transactions happened. Independently, a report accepted by `FixMsg::is_execution` uses its directly parsed execution clock where present and otherwise its own `currunix`, which the parse fills in rather than the walk. That latest clock propagates to later lifecycle events and never moves backward; non-New/cancel/correct/reverse/status AE and `AD`, `AQ` or `AR` invent none. `recdunix` is only the precise carrier recording time or a directly stated value, never a sending, original-sending or hop clock |
| Order | by default the entire finite capture is collected; capture-identical observations are merged, then the retained messages are stably sorted by event time before the walk. An item its source refused for what it states is left out with a [warning](capture.md#warnings), and a source failure ends the intake: the messages read before it are walked and yielded, then the failure, once. A codec whose [`sorted_lifecycle`](#a-sorted-source-is-walked-one-hour-at-a-time) pin is on reads a source already in instant order as it comes, one epoch hour at a time, and walks every hour still held before it yields a failure |
| Deliveries | a complete nonempty `(msgtype, msgsessionid, msgctxid, msgseqnum)` is prebuilt as the capture's `msgsesseventid`: `<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>`, the values joined by `:` as stated and the sequence rendered as canonical `u64`. Equal values are one delivery before sorting, walking or ordinary with-previous following - where the two also agree on the category, the side and an execution's chain, so the messages a parse split off one delivery never merge into each other. The observations are ranked latest `recdunix` first - a stated one ahead of an absent one, the later `currunix` closing a tie - and a full FIX-content and graph merge folds every other observation into the first, the reference chosen once over all of them, so the delivery takes no predecessor and no second place. Reference scalar conflicts win, older values fill absences, and repeating groups merge recursively at equal occurrence indexes with members sorted by FIX tag and no duplicate key. The merged `recdunix` and `execunix` facts are their earliest values, and provenance sources form a sorted unique union whose positions carry no reference meaning. Separately, exact republications and true retransmissions are removed across the entire finite capture, however many distinct deliveries intervene; session, sequence, original time and content distinguish normal deliveries, and headerless bridge rows use their event identity and capture context. An absent or empty text part or absent sequence produces no `msgsesseventid`; distinct message types, sessions, contexts and sequences survive |
| Instruments | after sorting, one lifecycle-local `SecurityIdRegistry` learns, under a valid ISIN, one association per security identifier key the message states - at most eight per instrument - and its detailed CFI, and fills only what a later message with the same ISIN leaves unstated, through `derive_securityid`, so a filled identifier never reaches a field or the wire. Coarse CFI is not learned, and neither is a bridge's `OMSINSTRUMENTID` or `ULLINKINSTRUMENTID`: each names a listing - `dbi;CH0012214059_XSWX_CHF` is the ISIN with its market and currency - and one ISIN has as many listings as markets. Conflicting values make an association ambiguous and silent. Storage reserves 2 KiB for each first valid ISIN, at most 32 MiB or 16,384 ISINs and never more than 65,536; a known ISIN can keep learning at the cap |
| Ends | a terminal state retires the chain once yielded. A live finite deadline emits one owned `EXPIRED` message at that exact instant, following the live generation with `prevuuid`, handed over before anything the walk reads at its deadline and at the next place there - the higher of that and one past the live message's where the deadline is not after it - then purges it; replaced or terminal generations leave no stale expiry |
| Grid | a positive codec `snapshot_ns` / `snapshotNs` enables an epoch-aligned grid; the default is off. Every crossed tick yields an owned view of every living chain: the live message as of that tick, dated at it - `currunix` and `snapunix` both the tick - so its `curruuid` is the identity that instant derives, while its content (`currhashcode`), `seqnum`, `prevuuid` and `crossuuid` are the live message's; a view does not advance the chain. Deadlines win ties, all source messages at the instant follow, and views come last. Output is proportional to crossed ticks times living identities; there is no implicit output cap |
| Errors | the walk never fails on what a message states: an item its source refused is left out, and a message whose content merge, order links or side the rebuild refuses is walked as it stated itself, each beside a [warning](capture.md#warnings); only a source failure is an error, yielded once after the messages read before it, and exhaustion is fused |
| Entries | delivery folding, missing order-link propagation and the side a follower stating no `Side(54)` takes from its chain rebuild the affected content canonically; stated values always win. The graph-only chain columns remain off the wire |
| Split messages | the walk reads what the parse answered: the execution a report or a trade [reports](message.md#a-parse-splits-what-a-message-reports) and each side of a two-sided quote are messages of their own, walked as chains of their own - an execution under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>`, a chain of the `EXEC` kind, so a fill never restates, follows or ends its order - while the report they were split from follows its order. `codec.market_data(codec.lifecycle(messages))` is the [sorted handoff](arrow.md#fix-market-books) to a book |
| Bindings | Rust; Python `FixCodec.lifecycle`, `lifecycle_arrow_reader`; JavaScript `lifecycle`, `lifecycleArrowReader` |

## Use

One order's life: the order under its client identifier, the acknowledgement under the venue's, that acknowledgement logged a second time on its way through a bridge, the fill naming the venue's alone, and a new order reusing the client identifier after the fill. The parse splits the fill's execution off its report, and the repeated delivery is omitted before the walk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, Market};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind, PREVUUID_TAG_NAME, SEQNUM_TAG_NAME};

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
    // Parsed, each message names the chain it spells - the order its ClOrdID
    // under the side it takes, the reports the venue's OrderID - and the fill
    // is followed by the execution it reports; none follows anything yet.
    assert_eq!(parsed.len(), 6);
    assert_eq!(parsed[0].get_crosscode(), "BUYS:A1");
    assert_eq!(parsed[1].get_crosscode(), "O1");
    assert_eq!(parsed[4].msgcat(), MarketDataKind::Execution);
    assert_eq!(parsed[4].get_srcuuids(), [parsed[3].get_curruuid()]);
    assert!(parsed.iter().all(|held| held.get_prevuuid().is_none()));
    // Each takes its place among the messages of its instant in the order
    // the lines hand them over: the acknowledgement logged twice stands
    // second, and the execution after the fill it was split off.
    let places: Vec<u64> = parsed.iter().map(Event::get_seqnum).collect();
    assert_eq!(places, [0, 0, 1, 0, 1, 0]);

    let chained: Vec<FixMsg> = reader.lifecycle(parsed.clone()).collect::<yggdryl::Result<_>>()?;
    let [order, ack, fill, execution, again] = chained.as_slice() else { panic!("five messages") };

    // The acknowledgement goes by the name the order was placed under, so it
    // follows the order and the chain keeps the order's code; the fill names
    // only the venue's identifier, which the acknowledgement went by.
    assert!([order, ack, fill].iter().all(|held| held.get_crosscode() == "BUYS:A1"));
    assert!([order, ack, fill].iter().all(|held| held.get_crossuuid() == order.get_crossuuid()));
    assert_eq!((order.get_seqnum(), order.get_prevuuid()), (0, None));
    // Each follows one of an earlier instant, so each keeps its own place.
    assert_eq!((ack.get_seqnum(), ack.get_prevuuid()), (0, Some(order.get_curruuid())));
    assert_eq!(ack.get_prevunix(), Some(order.get_currunix()));
    assert_eq!((fill.get_seqnum(), fill.get_prevuuid()), (0, Some(ack.get_curruuid())));
    // A message names only the one before it: the chain further back is read
    // by following `prevuuid` from one message to the next.
    let before_fill = chained.iter().find(|held| Some(held.get_curruuid()) == fill.get_prevuuid());
    assert_eq!(before_fill.and_then(|held| held.get_prevuuid()), Some(order.get_curruuid()));
    // The lifecycle travels: the chain's first creation, and the state.
    assert_eq!(fill.get_creaunix(), order.get_creaunix());
    assert_eq!(fill.get_state().as_str(), "FILLED");
    // The execution is a chain of its own: it follows nothing and joins nothing.
    assert_eq!((execution.get_seqnum(), execution.get_prevuuid()), (1, None));
    assert_ne!(execution.get_crossuuid(), order.get_crossuuid());
    // The fill ended the chain: the new order under the reused identifier
    // starts one afresh.
    assert_eq!((again.get_seqnum(), again.get_prevuuid()), (0, None));
    assert_eq!(again.get_crosscode(), "BUYS:A1");
    // The stamps are columns, reached like any typed fact. They never
    // reach the wire, and neither does a fact the walk folded: this
    // acknowledgement named no side, so the chain's buy is what the trait
    // answers and not a byte the message emits.
    assert_eq!(execution.by_tag(SEQNUM_TAG_NAME.0)?.as_u64(), Some(1));
    assert_eq!(ack.by_tag(PREVUUID_TAG_NAME.0)?, yggdryl::Scalar::Uuid(order.get_curruuid()));
    assert_eq!(parsed[1].get_side().as_str(), "UNKN");
    assert_eq!(ack.get_side().as_str(), "BUYS");
    let wire = ack.into_text('|')?;
    assert!(wire.starts_with("8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|"), "{wire}");
    assert!(!wire.contains("65014="), "no stamp is a field");

    // A chained stream replayed answers the same messages.
    let replayed: Vec<FixMsg> = reader.lifecycle(chained.clone()).collect::<yggdryl::Result<_>>()?;
    assert_eq!(replayed, chained);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import MarketDataKind, Side, State
    from yggdryl.fix import FixCodec, FixRegistry

    SEQNUM, PREVUUID = 65014, 65013
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
    # Parsed, each message names the chain it spells - the order its ClOrdID
    # under the side it takes, the reports the venue's OrderID - and the fill is
    # followed by the execution it reports; none follows anything yet.
    assert len(parsed) == 6
    assert parsed[0].crosscode == "BUYS:A1"
    assert parsed[1].crosscode == "O1"
    assert parsed[4].msgcat == MarketDataKind.EXEC
    assert parsed[4].srcuuids == [parsed[3].curruuid]
    assert all(held.prevuuid is None for held in parsed)
    # Each takes its place among the messages of its instant in the order the
    # lines hand them over: the acknowledgement logged twice stands second,
    # and the execution after the fill it was split off.
    assert [held.seqnum for held in parsed] == [0, 0, 1, 0, 1, 0]

    order, ack, fill, execution, again = reader.lifecycle(parsed)

    # The acknowledgement goes by the name the order was placed under, so it
    # follows the order and the chain keeps the order's code; the fill names only
    # the venue's identifier, which the acknowledgement went by.
    chained = [order, ack, fill]
    assert all(held.crosscode == "BUYS:A1" for held in chained)
    assert all(held.crossuuid == order.crossuuid for held in chained)
    assert (order.seqnum, order.prevuuid) == (0, None)
    # Each follows one of an earlier instant, so each keeps its own place.
    assert (ack.seqnum, ack.prevuuid) == (0, order.curruuid)
    assert ack.prevunix == order.currunix
    assert (fill.seqnum, fill.prevuuid) == (0, ack.curruuid)
    # A message names only the one before it: the chain further back is read by
    # following prevuuid from one message to the next.
    before_fill = next(held for held in chained if held.curruuid == fill.prevuuid)
    assert before_fill.prevuuid == order.curruuid
    # The lifecycle travels: the chain's first creation, and the state.
    assert fill.creaunix == order.creaunix
    assert fill.state == State.FILLED
    # The execution is a chain of its own: it follows nothing and joins nothing.
    assert (execution.seqnum, execution.prevuuid) == (1, None)
    assert execution.crossuuid != order.crossuuid
    # The fill ended the chain: the new order under the reused identifier starts
    # one afresh.
    assert (again.seqnum, again.prevuuid) == (0, None)
    assert again.crosscode == "BUYS:A1"
    # The stamps are columns, reached like any typed fact. They never
    # reach the wire, and neither does a fact the walk folded: this
    # acknowledgement named no side, so the chain's buy is what the trait
    # answers and not a byte the message emits.
    assert execution.by_tag(SEQNUM).as_py() == 1
    assert ack.by_tag(PREVUUID) == order.curruuid
    assert parsed[1].side == Side.UNKN
    assert ack.side == Side.BUYS
    wire = ack.into_text("|")
    assert wire.startswith("8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|")
    assert "65014=" not in wire  # no stamp is a field

    # A chained stream replayed answers the same messages.
    walked = [order, ack, fill, execution, again]
    assert list(reader.lifecycle(walked)) == walked
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const [SEQNUM, PREVUUID] = [65014, 65013]
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
    // Parsed, each message names the chain it spells - the order its ClOrdID
    // under the side it takes, the reports the venue's OrderID - and the fill is
    // followed by the execution it reports; none follows anything yet.
    assert.equal(parsed.length, 6)
    assert.equal(parsed[0].crosscode, 'BUYS:A1')
    assert.equal(parsed[1].crosscode, 'O1')
    assert.equal(parsed[4].msgcat, 'EXEC')
    assert.deepEqual(parsed[4].srcuuids, [parsed[3].curruuid])
    assert.ok(parsed.every((held) => held.prevuuid === null))
    // Each takes its place among the messages of its instant in the order the
    // lines hand them over: the acknowledgement logged twice stands second,
    // and the execution after the fill it was split off.
    assert.deepEqual(parsed.map((held) => held.seqnum), [0, 0, 1, 0, 1, 0])

    const [order, ack, fill, execution, again] = [...reader.lifecycle(parsed)]

    // The acknowledgement goes by the name the order was placed under, so it
    // follows the order and the chain keeps the order's code; the fill names only
    // the venue's identifier, which the acknowledgement went by.
    const chained = [order, ack, fill]
    assert.ok(chained.every((held) => held.crosscode === 'BUYS:A1'))
    assert.ok(chained.every((held) => held.crossuuid === order.crossuuid))
    assert.deepEqual([order.seqnum, order.prevuuid], [0, null])
    // Each follows one of an earlier instant, so each keeps its own place.
    assert.deepEqual([ack.seqnum, ack.prevuuid], [0, order.curruuid])
    assert.equal(ack.prevunix, order.currunix)
    assert.deepEqual([fill.seqnum, fill.prevuuid], [0, ack.curruuid])
    // A message names only the one before it: the chain further back is read by
    // following prevuuid from one message to the next.
    const beforeFill = chained.find((held) => held.curruuid === fill.prevuuid)
    assert.equal(beforeFill.prevuuid, order.curruuid)
    // The lifecycle travels: the chain's first creation, and the state.
    assert.equal(fill.creaunix, order.creaunix)
    assert.equal(fill.state, 'FILLED')
    // The execution is a chain of its own: it follows nothing and joins nothing.
    assert.deepEqual([execution.seqnum, execution.prevuuid], [1, null])
    assert.notEqual(execution.crossuuid, order.crossuuid)
    // The fill ended the chain: the new order under the reused identifier starts
    // one afresh.
    assert.deepEqual([again.seqnum, again.prevuuid], [0, null])
    assert.equal(again.crosscode, 'BUYS:A1')
    // The stamps are columns, reached like any typed fact. They never
    // reach the wire, and neither does a fact the walk folded: this
    // acknowledgement named no side, so the chain's buy is what the trait
    // answers and not a byte the message emits.
    assert.equal(execution.byTag(SEQNUM).asJs(), 1)
    assert.equal(ack.byTag(PREVUUID).asJs(), order.curruuid)
    assert.equal(parsed[1].side, 'UNKN')
    assert.equal(ack.side, 'BUYS')
    const wire = ack.intoText('|')
    assert.ok(wire.startsWith('8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|'), wire)
    assert.ok(!wire.includes('65014='), 'no stamp is a field')

    // A chained stream replayed answers the same messages.
    const walked = [order, ack, fill, execution, again]
    const replayed = [...reader.lifecycle(walked)]
    assert.ok(replayed.every((held, at) => held.equals(walked[at])))
    ```

## A chain is named by its cross code

The cross code is what a message spells to name the thing it is about: an explicit nonempty value, else the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)`, read when the message is [built](message.md) and, for a message filed under `ORDR`, `QUOT` or `EXEC` ([`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches)), stored under the side it takes - `BUYS:A1`, `SELL:A1` - through [`Market::sided_crosscode`](../graph/market.md#sides-and-cross-codes), the one speller of the prefix; a message of any other category keeps it as spelled, whatever side it states. `crossuuid` is the UUIDv8 of its XXH3-64, so two messages spelling one `OrderID` on one side share one identity before any walk reads them, and a buy and a sell under one `ClOrdID` are two chains. The separate `msgsesseventid` names one captured delivery only when its message type, session, context and sequence are all present; it never chooses the chain or changes the FIX content identity. A message spelling no cross code - a heartbeat, a logon - is a chain of one: its `crossuuid` is its own `curruuid`, and a message that follows it by a name keeps that `crossuuid`, so one chain is one cross identity.

The walk keys each live chain on that identity and on its market data kind (`marketdatakind`), so a message joins only a chain of its own kind - an order and an execution under one cross code are two chains, and a message of one kind never chains with a message of another - and where a message arrives under an identity nothing live holds, on the names a live message of its side goes by: the message's alternate identifiers - [`get_altids()`](message.md#the-identifier-maps), each under the upper-cased name of the field that stated it - are matched pair by pair, `(CLORDID, A1)`, against the names every live message of that side went by, so a report stating only the `ClOrdID` an order was placed under joins the order, and a fill stating only the venue's `OrderID` joins through the acknowledgement that went by both. A message stating no side - a cancel reject naming no `Side(54)`, an acknowledgement that states none - joins the one side alive under its cross code, else under the first name it shares with a live message, where exactly one side of it is alive, and takes that side and the chain's cross code; where both sides are alive it starts a chain of its own. A message that follows takes the chain's cross code as its own, so `crosscode` and `crossuuid` name one chain whichever identifier each message chose, and a message's own spelling is still in its row and its wire. A replace's `OrigClOrdID(41)` is a different scheme from `ClOrdID(11)`, so it joins nothing by itself; the replace joins where it states the venue's `OrderID` or a `ClOrdID` a live message went by. An execution - one a parse [split off](message.md#a-parse-splits-what-a-message-reports) its report, chained under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>` - joins nothing by a name or a base cross code, and is of the `EXEC` kind, so a fill never restates, follows or ends the order it filled.

## A chain carries its creation and its history

With-previous first compares `msgsesseventid`. Equal complete values force a full merge and stop: no predecessor or order-link inheritance is stamped, and the higher of the two places stands. Otherwise following records the predecessor's `curruuid` and `currunix` as `prevuuid` and `prevunix`, and the message keeps its own [place](#a-place-counts-one-instant), `seqnum`, unless the predecessor happened at the same instant or later, where it takes the higher of its own and one past the predecessor's. The predecessor is the one message a chained message names: nothing records the chain further back, so a caller reads a chain by following `prevuuid` from row to row. The `srcuuids` a message states - the text line it was parsed out of, the message a parse split it off - are its own and travel nowhere: provenance names what a node was read from, never what it follows. `creaunix` is the earliest the two know - and a walked message stating none takes its chain's, or starting a chain its own instant - and the [state](../types/enum/state.md#the-code-is-the-rank) the furthest along, as `State::merge_with` reads it; a `NEW` stated over a live predecessor that is itself new, working or restated [reads `UPDATED`](../types/enum/state.md#stated-anew-over-a-live-one), and later progress folds over it. An `exprunix` the newer message explicitly states is the current generation's deadline even where it is earlier; only an absent deadline is inherited. A report `FixMsg::is_execution` accepts that states no execution instant already took its own `currunix` as one when it was parsed, before any walk; following carries the later of that clock and the predecessor's `execunix`, and a lifecycle output read back with none is not refilled, because the state it carries may be one it inherited. `recdunix` remains this observation's own. It also fills an absent `parentorderid` from the exact predecessor's different `OrderID(37)` and an absent `ClOrdID(11)` from that predecessor's `ClOrdID`; neither overwrites a stated value. An absent `ticker` likewise carries from the predecessor, even when the timed link is already present, while a stated one remains; and what the chain is about - the currency, the side, the unit, each security identifier the message lacks, the classification and the market - carries where the message says nothing, a statement that knows leading one that does not: an ISIN numbered `ZZ` yields to a real one, an unclassified CFI to a classified one and the `XXXX` market to a real one. The metadata carries as well: a message takes every key of its chain's it does not state - the bridge's namespaced keys its row's `metadata` column holds, so a followed message's row carries the chain's keys - and its own values stand; of the alternate identifiers it takes only those whose `FIX:idmap` entry follows, written through its fields, and it takes no account, its accounts being its fields'. The timed and market readings settle before one finalization, and only an inherited ticker is cloned. A full merge instead takes the statement with the later `recdunix` as reference - a stated one leads an absent one, and the later `currunix` closes a tie - for conflicting content and identifier maps while the provenance list becomes a sorted unique union, keeping the latest expiry and the earliest execution and recording facts either statement knows. A merged statement therefore ranks against a third by the earliest recording it keeps, so which of three statements is the reference depends on the order they merge in. The message is settled again once it moved, so its `currhashcode` and `curruuid` are those of the merged or chained message and never of what it was before the walk; a stamped stream replayed answers the same messages, because a message already following its predecessor with all inherited facts is one following changes nothing on.

Read across the three steps, provenance and the chain are joins on one column set: a message's `srcuuids` are the `curruuid` of the [line](../media/index.md#plain-text) it was parsed out of, a chained message's `prevuuid` is the `curruuid` of the message before it, and the line's batch, the parsed row and the chained row contain the same fifteen [event columns](../graph/market-data.md#columns) under one name and one datatype each, so the joins need no mapping.

## A twin is not a successor

A bridge logs one message at every hop it passes, so a capture routinely holds the same delivery twice. Each observation with all four delivery parts prebuilds `msgsesseventid` on its capture - a column of the fixed row of its own - as `<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>`: the values joined by `:` exactly as stated, the sequence in canonical `u64`, so `8:e7256476:9effef3e6a:1094`. A session or context holding a `:` of its own could join two splits to one key; a bridge names none that way. `lifecycle` fully merges equal values before sorting and walking instead of making one follow the other, and direct with-previous applies the same forced merge before ordinary following. Observations are ranked latest `recdunix` first, and the others are folded into the first, which stays the reference row: the choice is made once over every observation, because a folded observation keeps the earliest recording its statements know and ranking by that against the next would hand the reference to whichever arrives later. Direct with-previous merges two at a time and has no such view - an observation already folded there ranks by its earliest recording against the next. Its stated scalar wins each conflict and older observations fill only what it leaves absent. Repeating-group occurrence `i` merges with occurrence `i` recursively; every merged level is sorted by FIX tag then folded name and contains one logical key, so marked and bare bridge spellings at the same indexes do not append duplicate occurrences or members. Both source UUIDs remain; the merged `execunix` and `recdunix` are their earliest values. What that fold costs is worth stating plainly, because nothing bounds it: an unbounded number of observations sharing one `msgsesseventid` fold into one event, however far apart their clocks and however much their content diverges, since the fold is deliberately blind to the content identities that would otherwise keep them apart - a bridge capture where one message is logged at a dozen hops routinely answers a chain several times shorter than its observation count. The bounds this walk states are all the other way round, naming what never folds: an incomplete key, a different message type, session, context or sequence, a distinct delivery carrying equal business content, and a lifecycle output or snapshot view read back in.

`srcuuids` is the sorted unique provenance union. Which observation becomes the merge reference is settled separately by the later `recdunix` and then the later `currunix`, with an exact tie keeping the caller's leading statement; no list position identifies that reference or an arrival order. Feeding the same observations in another order therefore leaves the serialized provenance list unchanged. A missing or empty text part or missing sequence produces no `msgsesseventid`, and a different message type, context, session or sequence is a different delivery. Already placed lifecycle outputs and snapshot views are not raw capture observations and are never coalesced again on replay.

After that merge, `lifecycle` removes an exact republication or true retransmission across the whole finite capture, however many distinct deliveries it holds: a complete FIX header keys the session, sequence, original time and recorded canonical content identity, while a headerless bridge row must match the recorded event identity and content in the same capture session, context and direction. Reordering projected fields during Arrow reconstruction does not create another delivery. When `SendingTime(52)` was unstated, the retained event time supplies its clock; a replay's stated `OrigSendingTime(122)` takes precedence. The retained delivery set therefore grows with distinct deliveries in that finite capture; it is not a recent window.

After delivery deduplication, the walk still recognizes two source statements that arrive under the same event identity: the later statement takes the live one's predecessor, place, snapshot, cross code, lifecycle and every `metadata` key it does not state, and finalizes to the same `curruuid`, so the chain grows by nothing. Deduplication decides whether the finite capture delivered the same message twice; restating decides whether two retained graph events are statements of one event; and the [window](#an-identity-is-yielded-once) decides that an event already yielded is not yielded again - so two distinct deliveries of equal content at one instant are two deliveries to the set and one identity in the walk's answer.

## An identity is yielded once

A twin restated is the live message again under its identity, and a hop a bridge logged after its chain had already moved on comes back under that identity too: a stream keyed by `curruuid` would hold the event twice, adjacent or far apart. `lifecycle` therefore yields each identity once within a window of event time, `dedup_window_ms` (Rust and Python) / `dedupWindowMs` (JavaScript), one minute - `FixCodec::DEFAULT_DEDUP_WINDOW_MS` - unless the codec states otherwise. An identity is remembered at the `currunix` it was yielded under and forgotten once the walk has yielded a message more than the window after it, so what the window holds is bounded by the event time it spans rather than by the capture's length; a later message under a remembered identity is dropped, whichever came between. The walk has already read it: the chain keeps what it said - its earliest recording and execution instants, the lifecycle it carries - and only its copy is not yielded again. A grid view is the live message as of a tick, one per tick and chain, so it is neither dropped nor remembered, and the view at the live message's own instant still answers that message's identity. A nil identity - an instant no UUIDv7 holds - is no identity and never repeats. A nonpositive window, `None` or `null` remembers nothing and yields every message the walk answers; `with_dedup_window_ms` (Rust and Python) / `withDedupWindowMs` (JavaScript) sets another on a codec in hand, keeping every other setting, and `dedup_window_ms` / `dedupWindowMs` reads it back - `None` / `null` where there is none.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Element;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(registry);
    assert_eq!(codec.dedup_window_ms(), Some(FixCodec::DEFAULT_DEDUP_WINDOW_MS));
    // The order delivered again under another sequence is another delivery
    // of one event: the walk restates the live order with it, after another
    // order rather than beside it.
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:30|11=B1|55=MSFT|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=D|49=S|56=T|34=3|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|10=0|",
    ];
    let parsed: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    let every: Vec<FixMsg> = codec
        .clone()
        .with_dedup_window_ms(0)
        .lifecycle(parsed.clone())
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(every.len(), 3);
    assert_eq!(every[2].get_curruuid(), every[0].get_curruuid());

    let once: Vec<FixMsg> = codec.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    assert_eq!(once, every[..2]);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)
    assert codec.dedup_window_ms == 60_000
    assert FixCodec(registry, dedup_window_ms=None).dedup_window_ms is None
    # The order delivered again under another sequence is another delivery of
    # one event: the walk restates the live order with it, after another order
    # rather than beside it.
    lines = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:30|11=B1|55=MSFT|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=D|49=S|56=T|34=3|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|10=0|",
    ]
    parsed = list(codec.parse_lines(lines))

    every = list(codec.with_dedup_window_ms(None).lifecycle(parsed))
    assert len(every) == 3
    assert every[2].curruuid == every[0].curruuid

    once = list(codec.lifecycle(parsed))
    assert [held.curruuid for held in once] == [held.curruuid for held in every[:2]]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)
    assert.equal(codec.dedupWindowMs, 60_000)
    assert.equal(new fix.FixCodec(registry, { dedupWindowMs: null }).dedupWindowMs, null)
    // The order delivered again under another sequence is another delivery of
    // one event: the walk restates the live order with it, after another order
    // rather than beside it.
    const lines = [
      '8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|10=0|',
      '8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:30|11=B1|55=MSFT|54=1|38=100|10=0|',
      '8=FIX.4.4|35=D|49=S|56=T|34=3|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|10=0|',
    ].map((line) => Buffer.from(line))
    const parsed = [...codec.parseLines(lines)]

    const every = [...codec.withDedupWindowMs(null).lifecycle(parsed)]
    assert.equal(every.length, 3)
    assert.equal(every[2].curruuid, every[0].curruuid)

    const once = [...codec.lifecycle(parsed)]
    assert.deepEqual(once.map((held) => held.curruuid), every.slice(0, 2).map((held) => held.curruuid))
    ```

## A place counts one instant

`seqnum` is where a message stands among the messages of its instant - the same `currunix`, to the nanosecond: 0 for the first a stream hands over there, one more for each next one at that instant, and 0 again at the next instant. It orders two identities of one millisecond: `curruuid` is RFC 9562 UUIDv7 over the millisecond, then the place in its 12-bit sequence lane, then an XXH3 payload seeded by the cross code, so identities sort by millisecond, then place. Two instants inside one millisecond each count from 0, so a message later in the millisecond can sort before one earlier in it; the lane holds up to 4,095, past which the payload still separates identities. A place is where a message stands, never what it says: it is outside `currhashcode`, and reaches the identity through `curruuid` alone.

The [parse](capture.md#every-message-is-dated) places by order: `parse_line` and `parse_text_line` within a row, and `parse_lines`, `parse_text_lines`, `parse_arrow_messages` and `parse_text_arrow_reader` across rows, hand every message the next place of its instant's run. A report and the execution the parse [split off](message.md#a-parse-splits-what-a-message-reports) it are places 0 and 1, and a split message's `srcuuids` name the identity its source was placed under. This walk places again by content, because the order it walks in is its own - sorted by instant, a stretch of messages dated by their transactions, repeat deliveries dropped: the expirations of an instant come first, each at the next place of its deadline, then each content read at that instant takes the next place, and a content repeated there takes the place it already took, so a message a bridge logged at two hops is one identity and the twin, delivery and window rules read one. What a walk made - a grid view, an expiration at its deadline - keeps the place the walk gave it when the output is walked again, so a walked stream answers itself. A place depends on nothing before the run of its instant, so regenerating any stretch of a capture that begins at a new instant places each message in it as the whole capture did - no count carried over from an earlier hour, file or chain.

Following keeps a message's own place unless its predecessor happened at the same instant or later, where it takes the higher of its own and one past the predecessor's; a merge keeps the higher of two places. The [market projection](arrow.md#fix-market-books) hands each leaf its message's place, so the entries of one book message share it; a book's place is the highest of its members' places at the book's own instant, and a [book](../graph/book.md#book-fold) fold trusts a chain step's place only where its predecessor stands at the same instant, folding in arrival order otherwise. A [text line](../media/index.md#plain-text)'s place is its row number instead, which orders the lines of one millisecond.

## Instrument associations are lifecycle state

Messages are sorted before one lifecycle-local `SecurityIdRegistry` reads them. A valid ISIN may teach its detailed CFI and every other security identifier the message states under it - a CUSIP, a SEDOL, a Bloomberg code, a FIGI, any source, at most eight keys per instrument - to later messages that state the same ISIN and omit that key; what it fills is derived, `derive_securityid`, so it reaches no field and no wire. The registry never overwrites a stated fact. A coarse CFI such as `ESXXXX`, an invalid spelling and a null marker teach nothing; two valid conflicting values make that code family ambiguous and it stays silent.

The registry retains at most 32 MiB by reserving a conservative 2 KiB for each first valid ISIN, so at most 16,384 ISINs register, and never more than 65,536. Reaching the cap refuses unseen ISINs without eviction; an already registered ISIN can still learn another code because its full payload was reserved on first insertion. The registry belongs to this ordered lifecycle, never to the codec, so a second lifecycle starts empty.

A currency pair is no association the walk learns. The parse [detects](capture.md#a-currency-pair-is-read-off-the-symbol) it off `Symbol(55)` into the message's derived identifiers, remembered per registry for 4,096 symbols, a symbol naming no pair remembered as one: a detected pair is derived, so a stated `FOREX` identifier replaces it, and a pair a row states in `forexcode` is stated and read back as it is.

## Snapshots are a grid

The codec's positive `snapshot_ns` / `snapshotNs` gives its lifecycle an epoch-aligned grid; zero, a negative width, `None` / `null`, or omission disables it, which is the default. A codec in hand takes another grid with `with_snapshot_ns` (Rust and Python) / `withSnapshotNs` (JavaScript), which keeps every other setting of it. At every crossed tick the walk yields a separate owned view of every living message: the live message as of that tick. The view is dated at the tick - its `currunix` and its `snapunix` are both the tick - so its `curruuid` is the identity that instant derives, and a view is a row of its own wherever rows are keyed by identity within a time; a view at the live message's own instant derives the live message's identity. Its content (`currhashcode`), `seqnum`, `prevuuid` and `crossuuid` are the live message's, and it does not advance the chain: the next message still follows the live one. Source messages keep the snapshot fact they stated and are never stamped backward to a previous tick.

At one instant, finite expirations come first, then every source message at that instant, then its views. Expiration emits one owned `EXPIRED` message at the exact deadline with the live message as predecessor, at the next place of that deadline - `seqnum` 0 for the first - then purges the identity; a replaced or ended generation's stale deadline emits nothing. At EOF the grid reaches the greatest finite deadline still encountered, or the last source instant where none remains, and stops; a nonexpiring message does not make a finite capture infinite. The width is a caller's output choice: the number of rows is proportional to crossed ticks times identities alive at each tick, with no implicit count or span cap.

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
    let deadline = source.get_exprunix().expect("the stated expiry");
    let walked: Vec<_> = codec.lifecycle([source]).collect::<yggdryl::Result<_>>()?;
    let live = &walked[0];
    assert!(live.get_snapunix().is_none(), "the source comes before its tick's view");

    let snapshots: Vec<_> = walked
        .iter()
        .filter(|message| message.get_snapunix().is_some())
        .collect();
    assert_eq!(snapshots.len(), 2, "the source tick and the crossed second");
    // A view is the live message as of its tick: dated at it, with the live
    // message's content, place and chain.
    assert!(snapshots.iter().all(|view| {
        view.get_snapunix() == Some(view.get_currunix())
            && view.get_currunix() < deadline
            && view.get_currhashcode() == live.get_currhashcode()
            && view.get_crossuuid() == live.get_crossuuid()
            && (view.get_seqnum(), view.get_prevuuid()) == (0, None)
    }));
    // Its identity is the one its tick derives: the source's at the source's
    // own instant, one of its own a second later.
    assert_eq!(snapshots[0].get_curruuid(), source_uuid);
    assert_ne!(snapshots[1].get_curruuid(), source_uuid);
    let expired = walked.last().expect("the deadline event");
    assert_eq!((expired.get_currunix(), expired.get_state().as_str()), (deadline, "EXPIRED"));
    assert_eq!((expired.get_prevuuid(), expired.get_seqnum()), (Some(source_uuid), 0));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import State
    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry, snapshot_ns=1_000_000_000)
    assert codec.snapshot_ns == 1_000_000_000
    source = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|"
    )
    source_uuid = source.curruuid
    deadline = source.exprunix
    assert deadline is not None
    walked = list(codec.lifecycle([source]))
    live = walked[0]
    assert live.snapunix is None  # the source comes before its tick's view

    snapshots = [message for message in walked if message.snapunix is not None]
    assert len(snapshots) == 2
    # A view is the live message as of its tick: dated at it, with the live
    # message's content, place and chain.
    assert all(
        view.snapunix == view.currunix
        and view.currunix < deadline
        and view.currhashcode == live.currhashcode
        and view.crossuuid == live.crossuuid
        and (view.seqnum, view.prevuuid) == (0, None)
        for view in snapshots
    )
    # Its identity is the one its tick derives: the source's at the source's
    # own instant, one of its own a second later.
    assert snapshots[0].curruuid == source_uuid
    assert snapshots[1].curruuid != source_uuid
    expired = walked[-1]
    assert (expired.currunix, expired.state) == (deadline, State.EXPIRED)
    assert (expired.prevuuid, expired.seqnum) == (source_uuid, 0)
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
    const deadline = source.exprunix
    const walked = [...codec.lifecycle([source])]
    const live = walked[0]
    assert.equal(live.snapunix, null, 'the source comes before its tick\'s view')

    const snapshots = walked.filter((message) => message.snapunix !== null)
    assert.equal(snapshots.length, 2)
    // A view is the live message as of its tick: dated at it, with the live
    // message's content, place and chain.
    assert.ok(snapshots.every((view) =>
      view.snapunix === view.currunix
      && view.currunix < deadline
      && view.currhashcode === live.currhashcode
      && view.crossuuid === live.crossuuid
      && view.seqnum === 0
      && view.prevuuid === null,
    ))
    // Its identity is the one its tick derives: the source's at the source's
    // own instant, one of its own a second later.
    assert.equal(snapshots[0].curruuid, sourceUuid)
    assert.notEqual(snapshots[1].curruuid, sourceUuid)
    const expired = walked.at(-1)
    assert.deepEqual([expired.currunix, expired.state], [deadline, 'EXPIRED'])
    assert.deepEqual([expired.prevuuid, expired.seqnum], [sourceUuid, 0])
    ```

## A sorted source is walked one hour at a time

A codec's `sorted_lifecycle` pin - `with_sorted_lifecycle(true)` (Rust), the `sorted_lifecycle=True` keyword or `with_sorted_lifecycle(True)` (Python), the `sortedLifecycle: true` option or `withSortedLifecycle(true)` (JavaScript), each read back by `sorted_lifecycle` / `sortedLifecycle` - states that the messages `lifecycle` is handed arrive in instant order: a table read hour partition by hour partition, sorted by `currunix`. The walk then reads them as they come instead of collecting the capture, and holds one epoch hour of them at a time. Each message waits in the bucket of its hour - an observation of a delivery already held joins the others of it, whatever its own hour - and a bucket is walked, its deliveries folded and its messages sorted exactly as a whole capture is, once the stream has read a message two hours past it. That hour of grace is what a message the walk dates by its transaction rather than by where it was stored needs to land in its own hour, and what the hops of one delivery logged across an hour's end need to meet. A message dated before an hour already walked is walked where it arrives, as any late message is. What is held is the messages of the hours not yet walked - two of them for a source in order - never the stream, and a source failure is yielded once every hour still held is walked.

The pin is off by default, because nothing says a capture is in order: a capture is collected and sorted whole. On a source that is in instant order both answer the same walk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let whole = FixCodec::new(registry);
    let hourly = whole.clone().with_sorted_lifecycle(true);
    assert!(!whole.sorted_lifecycle());
    assert!(hourly.sorted_lifecycle());
    // An order, its acknowledgement an hour later and its fill two hours
    // after that, in instant order.
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-11:15:30|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-13:15:30|10=0|",
    ];
    let messages: Vec<FixMsg> = whole.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    let chain = |codec: &FixCodec| {
        codec
            .lifecycle(messages.clone())
            .map(|held| held.map(|message| (message.get_curruuid(), message.get_seqnum(), message.get_prevuuid())))
            .collect::<yggdryl::Result<Vec<_>>>()
    };
    let walked = chain(&whole)?;
    // The order, the acknowledgement, the fill, and the execution split off it:
    // each first at its instant but the execution, which stands after its fill.
    let places: Vec<u64> = walked.iter().map(|(_, seqnum, _)| *seqnum).collect();
    assert_eq!(places, [0, 0, 0, 1]);
    assert_eq!(chain(&hourly)?, walked);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    whole = FixCodec(registry)
    hourly = FixCodec(registry, sorted_lifecycle=True)
    assert (whole.sorted_lifecycle, hourly.sorted_lifecycle) == (False, True)
    assert whole.with_sorted_lifecycle(True).sorted_lifecycle
    # An order, its acknowledgement an hour later and its fill two hours after
    # that, in instant order.
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-11:15:30|10=0|",
        b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-13:15:30|10=0|",
    ]
    messages = list(whole.parse_lines(lines))


    def chain(codec: FixCodec) -> list[tuple[object, int, object]]:
        return [(held.curruuid, held.seqnum, held.prevuuid) for held in codec.lifecycle(messages)]


    walked = chain(whole)
    # The order, the acknowledgement, the fill, and the execution split off it:
    # each first at its instant but the execution, which stands after its fill.
    assert [seqnum for _, seqnum, _ in walked] == [0, 0, 0, 1]
    assert chain(hourly) == walked
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const whole = new fix.FixCodec(registry)
    const hourly = new fix.FixCodec(registry, { sortedLifecycle: true })
    assert.deepEqual([whole.sortedLifecycle, hourly.sortedLifecycle], [false, true])
    assert.equal(whole.withSortedLifecycle(true).sortedLifecycle, true)
    // An order, its acknowledgement an hour later and its fill two hours after
    // that, in instant order.
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-11:15:30|10=0|',
      '8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-13:15:30|10=0|',
    ].map((line) => Buffer.from(line))
    const messages = [...whole.parseLines(lines)]
    const chain = (codec) => [...codec.lifecycle(messages)]
      .map((held) => [held.curruuid, held.seqnum, held.prevuuid])

    const walked = chain(whole)
    // The order, the acknowledgement, the fill, and the execution split off it:
    // each first at its instant but the execution, which stands after its fill.
    assert.deepEqual(walked.map(([, seqnum]) => seqnum), [0, 0, 0, 1])
    assert.deepEqual(chain(hourly), walked)
    ```

## In a batch read

The walk is a [stage](arrow.md#a-pin-is-on-the-codec-a-stage-is-a-call), and a stage is a call: `codec.lifecycle_arrow_reader(reader)` chains a whole read of FIX rows under the schema it read, and `codec.arrow_reader(schema, codec.lifecycle(codec.messages(reader)))` is the same composition spelled out - the same messages and the same rows, [the capture's own columns](message.md#a-row-is-a-message-again) included: `messages` reads each row's own cells into the message it makes and `into_row` states them again at their columns, so the spelled-out composition keeps them exactly as the door does. `lifecycle` takes owned messages or their `Result`s (Python and JavaScript accept any finite iterable). By default it consumes the input before yielding because stable event-time sorting, capture-wide delivery deduplication and ordered association learning need the complete capture; a source failure ends the intake, and the messages read before it are walked and yielded ahead of it. A [sorted lifecycle](#a-sorted-source-is-walked-one-hour-at-a-time) reads a source already in instant order as it comes, one hour at a time. Nothing chains unasked: a parse answers messages that follow nothing, because a stamped value is indistinguishable from a stated one, and a batch that is walked twice is walked into the same rows.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::LocalFolder;
    use yggdryl::{ArrowCastOptions, FixCodec, FixMsg, FixRegistry, Scalar, Serie, fix_schema};

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
    // The order, the fill's report and the execution the parse split off it.
    assert_eq!(batch.num_rows(), 3);
    let rows = Serie::from_arrow_batch(Some(&schema), &batch, ArrowCastOptions::new())?;
    let column = |name: &str| rows.child(name).expect("a column");
    // One order, one chain: the report names the order before it.
    assert_eq!(column("crossuuid").scalar(0)?, column("crossuuid").scalar(1)?);
    assert!(column("prevuuid").is_null(0)?);
    assert!(!column("prevuuid").is_null(1)?);
    // The execution is a chain of its own.
    assert_ne!(column("crossuuid").scalar(2)?, column("crossuuid").scalar(0)?);
    assert!(column("prevuuid").is_null(2)?);
    // Each is first at its instant but the execution, which stands after
    // the report it was split off: a first place states none.
    assert_eq!(column("seqnum").scalar(0)?, Scalar::Null, "a first message states no place");
    assert_eq!(column("seqnum").scalar(1)?, Scalar::Null);
    assert_eq!(column("seqnum").scalar(2)?, Scalar::from(1_u64));
    ```

## Edges

- Two messages of one chain at the same instant are neither after nor before one another: the walk keeps them in arrival order, and the second follows the first at the higher of its own place and one past the first's.
- A message that happened before the live one it would follow - out of order on a walk a caller opened as sorted - is yielded as it came and changes nothing; `lifecycle` sorts first, so nothing arrives out of order there. A sorted lifecycle sorts within each hour it holds; a message dated before an hour already walked is walked where it arrives.
- A message the reading refuses - its own predecessor, one following changes nothing on - is yielded as it came, beside a [warning](capture.md#warnings), and still stands as the live one.
- A terminal state ends the chain once the message is yielded; a message arriving under the retired identity starts a chain afresh, sharing the `crossuuid` and nothing else.
- A message whose expiry - `ExpireTime(126)`, else `ValidUntilTime(62)`, `ExpireDate(432)` or the instrument's `MaturityDate(541)` - is at or before its `currunix` is not alive and opens no chain. A later deadline is scheduled even where no later source arrives: one `EXPIRED` message is emitted at the deadline and the identity is purged.
- The state is the furthest along the two know, as `State::merge_with` reads it, so a chain never moves backward: a `New` after a `Filled` under a live chain would be yielded `Filled`; it is not, because the fill retired the chain first. A `NEW` after a live `NEW` reads `UPDATED`; after a live `PENDING_NEW` it is the first acknowledgement and stays `NEW`.
- A cleared or absent cross code keeps a message in a chain of one; nothing derives one from the identifiers alone, and a message in no chain joins one only by a name a live message goes by.
- A buy and a sell under one `ClOrdID` are two chains, `BUYS:A1` and `SELL:A1`; a cancel reject stating no side under that `ClOrdID` joins neither while both are alive, and joins the one that is where only one is.
- `restating` is never refused: the walk gives the word only for an arrival under the identity the live message arrived under, which a later statement of the same instant and content has and a successor never does.
- The grid begins at the first aligned tick at or after the first source instant; it never stamps a source with a tick before that source existed.
- Errors: sorting consumes the finite source first; a source failure ends that intake, the messages read before it are walked and yielded, then the failure, once, and a sorted lifecycle walks every hour still held before it yields the failure. An item the source refused for what it states is left out with a [warning](capture.md#warnings). No error advances the walk.
- A view at a later tick is a row of its own: dated at the tick, its `curruuid` differs from the live message's, so a monitor keyed by `curruuid` within a time holds one row per tick and chain.
- The deduplication window is inclusive: a repeat yielded exactly the window after the latest instant the walk yielded is still a repeat. It is measured on event time, so a sorted lifecycle that walks a late message where it arrives - after the hours it yielded since - yields it again once the walk has moved further than the window past it.
- A repeat the window drops still moved the walk: a twin restated the live message with its earliest instants, and a message following the live one follows the restated one.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test graph
    cargo test -p yggdryl --test fix batch::
    cargo test -p yggdryl --test fix market::
    cargo test -p yggdryl --test fix schema::
    cargo test -p yggdryl --test fix enrich::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_fix.py -k lifecycle
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js node/tests/fix/catalog.test.js
    ```
