# Lifecycle

A message says what happened; it does not say which order's life it belongs to beyond the identifiers a venue chose. `FixCodec::lifecycle` is the one [walk](../graph/event.md#lifecycle-walk) over messages: each is stated as the one after the live message of its chain - the chain named by the cross code every incarnation of one order shares - so a chained message carries its predecessor's identity and instant and the lifecycle carried forward, and a monitor joins an order's whole life on `crossuuid` rather than rebuilding it from `ClOrdID`, `OrigClOrdID` and `OrderID` on its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `FixCodec::lifecycle`, `FixCodec::lifecycle_arrow_reader`; the walk itself is [`graph::EventIterator`](../graph/event.md#lifecycle-walk), configured by the codec's snapshot width or opened directly by a Rust caller |
| Columns | the [crate's own](capture.md#the-crates-own-columns): `crosscode` (65003), `crosshashcode` (65005) and `crossuuid` (65002) name the chain; `prevuuid` (65013) and `prevunix` (65011) place a message in it; `seqnum` (65014) is its place among the messages of its instant; `creaunix` (65008), `exprunix` (65010) and `state` (65015) carry creation, the current generation's deadline and state; `execunix` (65024) is the precise execution instant and `sendunix` (65009) the earliest wire clock its statements know - when the message crossed the wire; `msgsesseventid` (65047) is the delivery a twin is matched by; `snapunix` (65012) is a grid view's stamp. `hashcode` (65004) and `uuid` (65001) are settled after a source message moves; a view keeps the live message's `hashcode` and takes the `uuid` its tick derives |
| Chain name | the cross code: an explicit nonempty value, else the first nonempty of the message's cross tags - `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)`, `MDReqID(262)`, led for a `TRAD` message by its trade's own `TradeID(1003)` and `TradeReportID(571)`, so a trade capture is keyed by the trade it reports and two reports of one order are two trades - stored as `{kind}:{side}:{base}` - the category's code, then the side an order or an execution message takes, `0` for a message taking `UKNW` or filed under any other category, a quote's among them, whose side is a tag - `10:1:A1`, `14:0:Q1`, `21:0:T1` - so a buy and a sell order under one identifier are two chains; `crosshashcode` is its XXH3-64 and `crossuuid` the UUIDv8 of that, so every message spelling one code on one side shares one identity whatever else it says. A chain's code is its first message's where that states one, and every message the walk states as the chain's - a follower, a twin, one stated before the live one - is [re-keyed](#a-chain-is-named-by-its-cross-code) onto it, the chain's side lent first to an order's or an execution's message stating none, so an identifier change moves no message onto another code; the message's own spelling stays in its fields and its wire. A complete `msgtype`, capture session, capture context and message sequence separately build the capture's `msgsesseventid`; it neither chooses the cross code nor changes the FIX content identity. A message naming no cross code is a chain of one: its `crossuuid` is its own `uuid`, which every message of its chain after it keeps as its own - the `EXPIRED` the walk emits, a twin and a follower naming a code of its own included, the first code a follower names becoming the one the messages after it are re-keyed onto |
| Joins | a live chain is keyed by its cross identity and its market data kind (`marketdatakind`: `ORDR`, `QUOT`, `EXEC`, `TRAD`, ...), and a message joins only a chain of its own kind. A message cites the chain its own identity is alive under; an order's or an execution's message stating no side, the one side alive under its base cross code, where exactly one is; and every chain a name of it is held by: a current message matches the previous alive message of its kind, of its instrument (`instcode`) where both state one, of its side whenever both state one - an order's or an execution's stored side, the leg a quote or a book entry tags, so a bid and an offer going by one `MDEntryID(278)` are two entries - sharing one identifier of the same type and value, [`IdType::is_chain_name`](../graph/identifier.md#parentage) - every type among its [`get_identifiers()`](message.md#the-identifier-maps), whatever its source: a [chain identity](../graph/identifier.md#parentage) (`orderid`, `clordid`, `secondaryorderid`, `secondaryclordid`, `quoteid`, `secondaryquoteid`, `tradeid`, `secondarytradeid`, `secondaryfirmtradeid`, `tradereportid`), every value of which the chain went by, the old beside the new after a replace; the chain's first value a lineage field names - `OrigClOrdID(41)`, `OrigTradeID(1126)`, `TradeReportRefID(572)`, `origorderid` - under the base it names too; and any other type - a report's own reference (`ExecID(17)`, a `TVTIC`), a bridge's own word - for the value the chain's live message holds alone, released by the next that holds another or none. A name is held by the first live chain of its kind, side and instrument that stated it until that chain ends; a message stating no side, or no instrument, cites a name on every one it is alive on. An identifier many messages share names no chain: a match's `TrdMatchID(880)`, which both orders it filled state, a request many chains answer (`QuoteReqID(131)`, `MDReqID(262)`) and a parent order's slot - `parentorderid`, the value before the last change, which the walk writes and a bridge spells a hierarchy parent by, a bridge's `parentclordid`; neither does a security or a party. Citing one chain, a message follows it - a report stating only the `ClOrdID` an order was placed under follows that order - and citing none it starts one; citing two is a [conflict](#a-message-citing-two-chains-is-a-conflict), never a pick: it stands under its own identity - its own live chain where that is one of the two, else a chain of its own - with a [`FixAnomaly`](message.md#anomalies) under `crosscode` naming the chains and what cited each, and a [warning](capture.md#warnings) once per kind. A quote's chain is one whatever side its statements tag, and lends no side; a name it goes by is alive on the leg its statement tags, so a bid and an offer going by one `QuoteID(117)` are two chains, and a statement tagging neither cites both. An execution joins an execution of its side the same way, and never the order it filled, which is of another kind |
| Follows | before following, equal complete `msgsesseventid` values force a full content-and-graph merge: the observation sent last - the latest `sendunix` - is the reference, the later `transunix` closing a tie, while `sendunix` and `execunix` become the earliest values either statement knows. Otherwise the predecessor's `uuid` and `transunix` are recorded, `seqnum` stays the message's own unless the predecessor happened at the same instant or later, where it takes the higher of its own and one past the predecessor's, the chain's cross code carries, and so do the earliest `creaunix`, the newer message's explicit `exprunix` where stated else the predecessor's, the furthest [state](../types/enum/state.md#the-code-is-the-rank) - a `NEW` stated over a live new-like predecessor [reads `UPDATED`](../types/enum/state.md#stated-anew-over-a-live-one) - and the later of the predecessor's latest execution clock and this event's precise `execunix`. `sendunix` remains this observation's own. Each base identifier the message states takes the parents its chain gave it - an `OrderID(37)` that changed names the value before it `parentorderid` and its chain's first `origorderid`, a `ClOrdID(11)` the previous one `origclordid`, a `TradeID(1003)` `origtradeid` and a `TradeReportID(571)` `tradereportrefid` ([Parentage](../graph/identifier.md#parentage)), read through the registry's [`FIX:parents`](registry.md#parents-of-an-identifier); only a chain identity has parents, so an `ExecID(17)` or a `TrdMatchID(880)` stated anew carries no lineage - and a message stating a parent but not its base is the base of its nearest parent; an explicitly stated link always wins. An absent `ticker` inherits the predecessor's even when the timed link already exists; a stated one remains. The FIX reading is `Operation::following_operation`: the time in force and whether the instrument can trade carry where this message states neither, and so does every `metadata` key of the chain it does not state - a bridge's namespaced keys, its own values standing - and every identifier whose `FIX:idmap` entry follows - the order's own, and only those - with the [parents](../graph/identifier.md#parentage) the chain gave each, so a message naming no `OrderID(37)` keeps its order's lineage for the next that names it, and every party id of the chain it names none for, each held as the message's word and written to no field, each base identifier it states taking the chain's [parents](../graph/identifier.md#parentage). A merged statement ranks against a third by the earliest `sendunix` it keeps, so which of three statements leads depends on the order they merge in. A message that moved is settled again, so its `hashcode` and `uuid` are its own and digest what it took, moving where it took keys. An order's chain then [counts its fills once](#an-orders-fills-are-counted-once) by `ExecID(17)` over its first stated total: `cumqty` and `leavesqty` are the count's, a repeated fill counts nothing, and a partial fill whose count reaches the order quantity reads `FILLED` |
| Places | every message the walk reads takes its [place](#a-place-counts-one-instant) among the messages of its instant by content - after sorting, redating and dropping repeat deliveries, before it is keyed or chained: the expirations of an instant come first, each at the next place of its deadline, then each content read at that instant takes the next place, and a content repeated there takes the place it already took, so a message a bridge logged at two hops is one identity; the next instant starts again at 0. What a walk made - a grid view, an expiration at its deadline - keeps the place the walk gave it when the output is walked again. A place is where a message stands, never what it says, so it is outside `hashcode` and reaches `uuid` alone |
| Creation | every walked message leaves stating `creaunix`: one stating none takes its chain's - the earliest the fold keeps - or, starting a chain, its own instant; a stated one is never replaced. A parse already states one on every message, so this reads for an event a caller built without one |
| Unknowns | a statement that knows the instrument or the market leads one that does not: an ISIN of a lower [rank](../types/codes/isin.md#the-check-digit) - a `ZZ` number, a masked `XX0000000001`, a typo - yields to a real one whatever the order, an unclassified CFI (`XXXXXX`) to a classified one and the no-market MIC `XXXX` to a market, whether one statement follows another or two merge |
| Twins | a message arriving under the identity the live one *arrived* under - the same instant and content, one message a bridge logged at every hop it passed - is another statement of it, not the one after it: it takes the live one's place, predecessor, snapshot, cross code, lifecycle and every `metadata` key it does not state, and the `ClOrdID(11)` and `Side(54)` its chain wrote into the live one's content where it states neither, keeps the earliest execution instant and wire clock either observation states, and finalizes to the same `uuid`, so the chain grows by nothing - and the window below yields that identity once. A message arriving under the identity of one its chain moved past at the live one's own instant - an acknowledgement delivered again after the fill of its instant, that fill ending the order or not, or the fill that ended it delivered again - is that one's twin the same way, and the chain stays where it moved |
| Once | what the walk yields is [yielded once](#an-identity-is-yielded-once) within the codec's `dedup_window_ms` / `dedupWindowMs` of event time, one minute by default: a message whose `uuid` the walk already yielded, no more than the window before the latest `transunix` it yielded, is dropped - a restated twin, a hop logged again after its chain moved on - while the walk still reads it. A grid view is exempt, a nil identity never repeats, and a nonpositive window, `None` or `null` yields everything the walk answers |
| Dating | a message whose `SendingTime(52)` the parse supplied rather than read - a carrier row's, its line's `transunix`, the codec's default, the intake's clock - is dated by the `TransactTime(60)` it states with a clock before the walk, and a resend's `OrigSendingTime(122)` is its creation where earlier; a stated sending clock stands, because the parse has already dated the message by [the official clock](capture.md#the-official-clock-dates-the-message) standing less than `official_time_delay_ms` from it. No delay bounds this one: a clock nobody stated is no reference to measure a distance from. `FixMsg::dated_by_transaction` is the reading, so a capture whose frames state no sending clock still orders, expires and folds by when its transactions happened. Independently, a report accepted by `FixMsg::is_execution` uses its directly parsed execution clock where present and otherwise its own `transunix`, which the parse fills in rather than the walk. That latest clock propagates to later lifecycle events and never moves backward; non-New/cancel/correct/reverse/status AE and `AD`, `AQ` or `AR` invent none. `sendunix` is only the precise carrier recording time or a directly stated value, never a sending, original-sending or hop clock |
| Order | by default the entire finite capture is collected; capture-identical observations are merged, then the retained messages are stably sorted by event time before the walk. An item its source refused for what it states is left out with a [warning](capture.md#warnings), and a source failure ends the intake: the messages read before it are walked and yielded, then the failure, once. A codec whose [`sorted_lifecycle`](#a-sorted-source-is-walked-one-hour-at-a-time) pin is on reads a source already in instant order as it comes, one epoch hour at a time, and walks every hour still held before it yields a failure |
| Deliveries | a complete nonempty `(msgtype, msgsessionid, msgctxid, msgseqnum)` is prebuilt as the capture's `msgsesseventid`: `<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>`, the values joined by `:` as stated and the sequence rendered as canonical `u64`. Equal values are one delivery before sorting, walking or ordinary with-previous following - where the two also agree on the category, the side and an execution's chain, so the messages a parse split off one delivery never merge into each other. The observations are ranked latest `sendunix` first - a stated one ahead of an absent one, the later `transunix` closing a tie - and a full FIX-content and graph merge folds every other observation into the first, the reference chosen once over all of them, so the delivery takes no predecessor and no second place. Reference scalar conflicts win, older values fill absences, and repeating groups merge recursively at equal occurrence indexes with members sorted by FIX tag and no duplicate key. The merged `sendunix` and `execunix` facts are their earliest values, and provenance sources form a sorted unique union whose positions carry no reference meaning. Separately, exact republications and true retransmissions are removed across the entire finite capture, however many distinct deliveries intervene; session, sequence, original time and content distinguish normal deliveries, and headerless bridge rows use their event identity and capture context. An absent or empty text part or absent sequence produces no `msgsesseventid`; distinct message types, sessions, contexts and sequences survive |
| Instruments | after sorting, every message is [learned and filled](#instruments-are-learned-in-instant-order) through an [`Instruments`](../graph/instrument.md): keyed by the code its facts spell - its real ISIN, or the class and body of an FX pair or a derivative - its CFI code, country of issue, underlying, legs, characteristics, product category, descriptive metadata, market, ticker, currency, pair and security codes are learned, a valid stated value filling and replacing whatever the time, and what a later message of that instrument leaves unsaid is filled - the identifiers from `derived`, the ticker, the CFI code and the currency as market facts, the instrument's cross code as `instcode` - never reaching a field, the wire or the message's identity. The collection is each walk's own, starting empty, unless the codec shares one (`with_instruments`), which its parse doors fill from too |
| Ends | a terminal state retires the chain once yielded, its counted fills remembered for the window below, so a copy of one starts no chain from a count. A live finite deadline emits one owned `EXPIRED` message at that exact instant, following the live generation with `prevuuid`, handed over before anything the walk reads at its deadline and at the next place there - the higher of that and one past the live message's where the deadline is not after it - then purges it; replaced or terminal generations leave no stale expiry |
| Grid | a positive codec `snapshot_ns` / `snapshotNs` enables an epoch-aligned grid; the default is off. Every crossed tick yields an owned view of every living chain: the live message as of that tick, dated at it - `transunix` the tick, `snapunix` the instant the live message it copies was stated at - so its `uuid` is the identity that instant derives, while its content (`hashcode`), `seqnum`, `prevuuid` and `crossuuid` are the live message's; a view does not advance the chain. Deadlines win ties, all source messages at the instant follow, and views come last. Output is proportional to crossed ticks times living identities; there is no implicit output cap |
| Errors | the walk never fails on what a message states: an item its source refused is left out, and a message whose content merge, order links or side the rebuild refuses is walked as it stated itself, each beside a [warning](capture.md#warnings); only a source failure is an error, yielded once after the messages read before it, and exhaustion is fused |
| Entries | delivery folding, missing order-link propagation and the side a follower stating no `Side(54)` takes from its chain rebuild the affected content canonically; stated values always win. The graph-only chain columns remain off the wire |
| Split messages | the walk reads what the parse answered: the execution a report or a trade [reports](message.md#a-parse-splits-what-a-message-reports) and each entry of a batch are messages of their own, walked as chains of their own - a quote is one message holding both its legs - an execution under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>`, a chain of the `EXEC` kind, so a fill never restates, follows or ends its order - while the report they were split from follows its order. An execution under an `ExecID` whose chain ended within the window - the same fill split off a report logged at another hop - restates the execution that counted it `codec.market_data(codec.lifecycle(messages))` is the [sorted handoff](arrow.md#fix-market-books) to a book |
| Bindings | Rust; Python `FixCodec.lifecycle`, `lifecycle_arrow_reader`; JavaScript `lifecycle`, `lifecycleArrowReader` |

## Use

One order's life: the order under its client identifier, the acknowledgement under the venue's, that acknowledgement logged a second time on its way through a bridge, the fill naming the venue's alone, and a new order reusing the client identifier after the fill. The parse splits the fill's execution off its report, and the repeated delivery is omitted before the walk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::Market;
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry, PREVUUID_TAG_NAME, SEQNUM_TAG_NAME};
    use yggdryl_market::MarketDataKind;
    yggdryl_fix::install()?;

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
    assert_eq!(parsed[0].get_crosscode(), "10:1:A1");
    assert_eq!(parsed[1].get_crosscode(), "10:0:O1");
    assert_eq!(parsed[4].marketdatakind(), MarketDataKind::Execution);
    assert_eq!(parsed[4].get_srcuuids(), [parsed[3].get_uuid()]);
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
    assert!([order, ack, fill].iter().all(|held| held.get_crosscode() == "10:1:A1"));
    assert!([order, ack, fill].iter().all(|held| held.get_crossuuid() == order.get_crossuuid()));
    assert_eq!((order.get_seqnum(), order.get_prevuuid()), (0, None));
    // Each follows one of an earlier instant, so each keeps its own place.
    assert_eq!((ack.get_seqnum(), ack.get_prevuuid()), (0, Some(order.get_uuid())));
    assert_eq!(ack.get_prevunix(), Some(order.get_transunix()));
    assert_eq!((fill.get_seqnum(), fill.get_prevuuid()), (0, Some(ack.get_uuid())));
    // A message names only the one before it: the chain further back is read
    // by following `prevuuid` from one message to the next.
    let before_fill = chained.iter().find(|held| Some(held.get_uuid()) == fill.get_prevuuid());
    assert_eq!(before_fill.and_then(|held| held.get_prevuuid()), Some(order.get_uuid()));
    // The lifecycle travels: the chain's first creation, and the state.
    assert_eq!(fill.get_creaunix(), order.get_creaunix());
    assert_eq!(fill.get_state().as_str(), "FILLED");
    // The execution is a chain of its own: it follows nothing and joins nothing.
    assert_eq!((execution.get_seqnum(), execution.get_prevuuid()), (1, None));
    assert_ne!(execution.get_crossuuid(), order.get_crossuuid());
    // The fill ended the chain: the new order under the reused identifier
    // starts one afresh.
    assert_eq!((again.get_seqnum(), again.get_prevuuid()), (0, None));
    assert_eq!(again.get_crosscode(), "10:1:A1");
    // The stamps are columns, reached like any typed fact, and never
    // reach the wire. The side is content: this acknowledgement named none,
    // so the walk wrote the chain's buy into it - what the trait answers and
    // what the wire states. A side a message states always stands.
    assert_eq!(execution.by_tag(SEQNUM_TAG_NAME.0)?.as_u64(), Some(1));
    assert_eq!(ack.by_tag(PREVUUID_TAG_NAME.0)?, yggdryl::Scalar::Uuid(order.get_uuid()));
    assert_eq!(parsed[1].get_side().as_str(), "UKNW");
    assert_eq!(ack.get_side().as_str(), "BUYS");
    let wire = ack.into_text('|')?;
    assert!(wire.starts_with("8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|"), "{wire}");
    assert!(wire.contains("|54=1|"), "{wire}");
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
    assert parsed[0].crosscode == "10:1:A1"
    assert parsed[1].crosscode == "10:0:O1"
    assert parsed[4].marketdatakind == MarketDataKind.EXEC
    assert parsed[4].srcuuids == [parsed[3].uuid]
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
    assert all(held.crosscode == "10:1:A1" for held in chained)
    assert all(held.crossuuid == order.crossuuid for held in chained)
    assert (order.seqnum, order.prevuuid) == (0, None)
    # Each follows one of an earlier instant, so each keeps its own place.
    assert (ack.seqnum, ack.prevuuid) == (0, order.uuid)
    assert ack.prevunix == order.transunix
    assert (fill.seqnum, fill.prevuuid) == (0, ack.uuid)
    # A message names only the one before it: the chain further back is read by
    # following prevuuid from one message to the next.
    before_fill = next(held for held in chained if held.uuid == fill.prevuuid)
    assert before_fill.prevuuid == order.uuid
    # The lifecycle travels: the chain's first creation, and the state.
    assert fill.creaunix == order.creaunix
    assert fill.state == State.FILLED
    # The execution is a chain of its own: it follows nothing and joins nothing.
    assert (execution.seqnum, execution.prevuuid) == (1, None)
    assert execution.crossuuid != order.crossuuid
    # The fill ended the chain: the new order under the reused identifier starts
    # one afresh.
    assert (again.seqnum, again.prevuuid) == (0, None)
    assert again.crosscode == "10:1:A1"
    # The stamps are columns, reached like any typed fact, and never
    # reach the wire. The side is content: this acknowledgement named none,
    # so the walk wrote the chain's buy into it - what the trait answers and
    # what the wire states. A side a message states always stands.
    assert execution.by_tag(SEQNUM).as_py() == 1
    assert ack.by_tag(PREVUUID) == order.uuid
    assert parsed[1].side == Side.UKNW
    assert ack.side == Side.BUYS
    wire = ack.into_text("|")
    assert wire.startswith("8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|")
    assert "|54=1|" in wire
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
    assert.equal(parsed[0].crosscode, '10:1:A1')
    assert.equal(parsed[1].crosscode, '10:0:O1')
    assert.equal(parsed[4].marketdatakind, 'EXEC')
    assert.deepEqual(parsed[4].srcuuids, [parsed[3].uuid])
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
    assert.ok(chained.every((held) => held.crosscode === '10:1:A1'))
    assert.ok(chained.every((held) => held.crossuuid === order.crossuuid))
    assert.deepEqual([order.seqnum, order.prevuuid], [0, null])
    // Each follows one of an earlier instant, so each keeps its own place.
    assert.deepEqual([ack.seqnum, ack.prevuuid], [0, order.uuid])
    assert.equal(ack.prevunix, order.transunix)
    assert.deepEqual([fill.seqnum, fill.prevuuid], [0, ack.uuid])
    // A message names only the one before it: the chain further back is read by
    // following prevuuid from one message to the next.
    const beforeFill = chained.find((held) => held.uuid === fill.prevuuid)
    assert.equal(beforeFill.prevuuid, order.uuid)
    // The lifecycle travels: the chain's first creation, and the state.
    assert.equal(fill.creaunix, order.creaunix)
    assert.equal(fill.state, 'FILLED')
    // The execution is a chain of its own: it follows nothing and joins nothing.
    assert.deepEqual([execution.seqnum, execution.prevuuid], [1, null])
    assert.notEqual(execution.crossuuid, order.crossuuid)
    // The fill ended the chain: the new order under the reused identifier starts
    // one afresh.
    assert.deepEqual([again.seqnum, again.prevuuid], [0, null])
    assert.equal(again.crosscode, '10:1:A1')
    // The stamps are columns, reached like any typed fact, and never
    // reach the wire. The side is content: this acknowledgement named none,
    // so the walk wrote the chain's buy into it - what the trait answers and
    // what the wire states. A side a message states always stands.
    assert.equal(execution.byTag(SEQNUM).asJs(), 1)
    assert.equal(ack.byTag(PREVUUID).asJs(), order.uuid)
    assert.equal(parsed[1].side, 'UKNW')
    assert.equal(ack.side, 'BUYS')
    const wire = ack.intoText('|')
    assert.ok(wire.startsWith('8=FIX.4.4|35=8|52=20260102-10:15:30.500|11=A1|37=O1|150=0|'), wire)
    assert.ok(wire.includes('|54=1|'), wire)
    assert.ok(!wire.includes('65014='), 'no stamp is a field')

    // A chained stream replayed answers the same messages.
    const walked = [order, ack, fill, execution, again]
    const replayed = [...reader.lifecycle(walked)]
    assert.ok(replayed.every((held, at) => held.equals(walked[at])))
    ```

## A chain is named by its cross code

The cross code is what a message spells to name the thing it is about: an explicit nonempty value, else the first nonempty of its category's cross tags - `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)`, a `TRAD` message's own `TradeID(1003)` and `TradeReportID(571)` before them, so a trade capture naming the order it filled is still keyed by its trade (`21:0:T1`) - read when the message is [built](message.md) and stored as `{kind}:{side}:{base}` - `10:1:A1`, `10:2:A1` - through [`Market::stored_crosscode`](../graph/market.md#sides-and-cross-codes), the one speller of the prefix: the category's code, then the side a message filed under `ORDR` or `EXEC` ([`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches)) takes, and `0` for a message of any other category, whatever side it states - a quote, which holds its bid and its offer and tags a side, is `14:0:Q1` whichever side it tags. `crossuuid` is the UUIDv8 of its XXH3-64, so two messages spelling one `OrderID` on one side share one identity before any walk reads them, and a buy and a sell under one `ClOrdID` are two chains. The separate `msgsesseventid` names one captured delivery only when its message type, session, context and sequence are all present; it never chooses the chain or changes the FIX content identity. A message spelling no cross code - a heartbeat, a logon - is a chain of one: its `crossuuid` is its own `uuid`, and a message that follows it by a name keeps that `crossuuid`, so one chain is one cross identity; the first code a follower spells becomes the one the messages after it are re-keyed onto. No follower's own content derives such a cross identity, so a consumer settling a follower of it again derives the follower's own: a chain that must survive that states a code.

The walk keys each live chain on that identity and on its market data kind (`marketdatakind`), so a message joins only a chain of its own kind - an order and an execution under one cross code are two chains, and a message of one kind never chains with a message of another. A message cites the chains it names: the one its own identity is alive under; for an order's or an execution's message stating no side - a cancel reject naming no `Side(54)`, an acknowledgement that states none - the one side alive under its base cross code, where exactly one is; and each chain a name it goes by is held by. A current message matches the previous alive message of its kind, of its instrument ([`instcode`](../graph/instrument.md)) where both state one, of its side whenever both state one - an order's or an execution's stored side, the leg a quote or a book entry tags - sharing one identifier of the same type and value: its names are every identifier among its [`get_identifiers()`](message.md#the-identifier-maps) whose type names a chain, [`IdType::is_chain_name`](../graph/identifier.md#parentage), each a type named for the field that stated it, matched by type and value, `(clordid, A1)`, whatever their source - a venue's `OrderID(37)` and a bridge's `OMS_OrderID` are both `orderid`, and `execid=1705` never matches `orderid=1705`. A chain identity, [`IdType::is_chain_identity`](../graph/identifier.md#parentage) - the order's, the client order's, the quote's, the trade's and the trade report's identifiers and their secondary ones - names the chain by every value it went by: a value stays the chain's after a replace, so a late report citing the old `ClOrdID` still finds the order. The chain's first value a lineage field names - a replace's `OrigClOrdID(41)`, a trade's `OrigTradeID(1126)`, a trade report's `TradeReportRefID(572)` - is matched under the base it names too, so a replace joins the live order that went by that `ClOrdID` even where it states no `OrderID`. Any other type - a reference a message states about itself, `ExecID(17)`, a `TVTIC`, a bridge's own word - names the chain for the value its live message holds alone: the next message holding another value of the type, or none, releases it, so a chain of a thousand reports is indexed by its identities and its live report's references, never by its reports. An identifier many messages share names none - two orders one match filled share its `TrdMatchID(880)`, two dealers answering one request a `QuoteReqID(131)`, every entry of one subscription an `MDReqID(262)`, and each stays its own, with no conflict told - and neither does a parent order's slot - `parentorderid`, the value before the last change, which the walk itself writes and a bridge spells a hierarchy parent by, and a bridge's `PARENTCLORDID`, the word `parentclordid` of its own - so two child orders of one parent stay two. Each name is held by the first live chain of its kind, side and instrument that stated it, until that chain ends, and a name whose chain ended is the next stater's; a message stating no side, or no instrument, cites a name on every one it is alive on - a change to one instrument's entry never joins another instrument's going by the same `MDEntryID(278)`. Citing one chain, a message follows it - a report stating only the `ClOrdID` an order was placed under joins the order, a fill stating only the venue's `OrderID` joins through the acknowledgement that went by both, and the predecessor's values propagate onto it ([A chain carries its creation and its history](#a-chain-carries-its-creation-and-its-history)); citing none, it starts a chain of its own; citing two, it is a [conflict](#a-message-citing-two-chains-is-a-conflict). An execution - one a parse [split off](message.md#a-parse-splits-what-a-message-reports) its report, chained under its `ExecID(17)` as given, else `TradeID=<TradeID(1003)>` - is of the `EXEC` kind, so it matches a live execution of its side sharing an identifier and never the order it filled: a fill never restates, follows or ends its order. A quote's chain is keyed by no side - its side is a tag, the statement's own - but a name it goes by is alive on the leg its statement tags, as FIX scopes an `MDEntryID(278)` by its `MDEntryType(269)`, so a bid and an offer going by one `QuoteID(117)` or one `MDEntryID(278)` are two chains, a tagged statement names the one of its leg, else the untagged quote holding both, and one tagging neither cites both and so, by the conflict, neither.

Every message the walk states as a chain's is re-keyed onto it: an order's or an execution's message stating no side takes the chain's side first, written as `Side(54)` so its row, its wire and its `hashcode` state it, then the chain's stored cross code where the chain's first message stated one - so `crosscode` and `crossuuid` name one chain whichever identifier each message chose, and a replace under a new `ClOrdID` or `OrderID` moves no message onto another code and no order onto another book entry. `crosshashcode` is the XXH3-64 of the chain's code and `crossuuid` its UUIDv8 on every message of it, and `uuid`, which that hash seeds, is settled once under them. A follower, a twin and a message stated before the live one - which follows nothing - are re-keyed alike; a message's own spelling of its identifiers stays in its fields and its wire.

## A message citing two chains is a conflict

A message citing two live chains - the `ClOrdID` one chain holds and the `OrderID` another's code is alive under - is never read as one of them: a wrong join would re-key it onto a stranger and fold its facts into that chain. It stands under its own identity - its own live chain where that is one of those cited, else a chain of its own beside them - and the conflict is told twice: on the message, as a [`FixAnomaly`](message.md#anomalies) under `crosscode` recorded before anything restates or settles it, so another statement of it carries the same and digests alike - its own stored code, then each cited chain's with the name that cited it, `10:1:O1 cites 10:1:O1 by its cross code and 10:1:C1 by clordid=C1` - and once per kind in the log, a [warning](capture.md#warnings) on `yggdryl.graph.iterator` counted at each tenfold after. An order standing apart is a book entry of its own too: the split is visible, and nothing is folded into a stranger. A message stating no side that cites a name alive on both sides is a conflict the same way. Nothing merges two chains.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|",
        // The venue's report of an order the walk knows by no name it states.
        b"8=FIX.4.4|35=8|37=O1|17=E1|150=0|39=0|54=1|55=AAPL|52=20260921-10:00:01|10=0|",
        // Both names at once: the ClOrdID the first chain holds, the OrderID
        // the second chain's code is alive under.
        b"8=FIX.4.4|35=8|11=C1|37=O1|17=E2|150=0|39=0|54=1|55=AAPL|52=20260921-10:00:02|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    let walked: Vec<FixMsg> = reader.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    let [order, venue, report] = walked.as_slice() else { panic!("three messages") };
    assert_eq!((order.get_crosscode(), venue.get_crosscode()), ("10:1:C1", "10:1:O1"));
    assert_eq!(venue.get_prevuuid(), None, "two chains");
    // Two chains cited: it stands under its own identity, the venue's chain,
    // and says what it cited.
    assert_eq!(report.get_prevuuid(), Some(venue.get_uuid()));
    assert_eq!(report.get_crosscode(), "10:1:O1");
    let cited: Vec<&str> = report
        .anomalies()
        .iter()
        .filter(|anomaly| anomaly.field() == "crosscode")
        .map(|anomaly| anomaly.reason())
        .collect();
    let [cited] = cited.as_slice() else { panic!("one conflict") };
    for named in ["10:1:O1", "10:1:C1", "clordid=C1"] {
        assert!(cited.contains(named), "{named} in {cited}");
    }
    // A message citing one chain states none.
    assert!(order.anomalies().iter().chain(venue.anomalies()).all(|anomaly| anomaly.field() != "crosscode"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    lines = [
        b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|",
        # The venue's report of an order the walk knows by no name it states.
        b"8=FIX.4.4|35=8|37=O1|17=E1|150=0|39=0|54=1|55=AAPL|52=20260921-10:00:01|10=0|",
        # Both names at once: the ClOrdID the first chain holds, the OrderID the
        # second chain's code is alive under.
        b"8=FIX.4.4|35=8|11=C1|37=O1|17=E2|150=0|39=0|54=1|55=AAPL|52=20260921-10:00:02|10=0|",
    ]
    order, venue, report = reader.lifecycle(reader.parse_lines(lines))
    assert (order.crosscode, venue.crosscode) == ("10:1:C1", "10:1:O1")
    assert venue.prevuuid is None, "two chains"
    # Two chains cited: it stands under its own identity, the venue's chain, and
    # says what it cited.
    assert report.prevuuid == venue.uuid
    assert report.crosscode == "10:1:O1"
    [cited] = [reason for field, reason in report.anomalies if field == "crosscode"]
    assert all(named in cited for named in ("10:1:O1", "10:1:C1", "clordid=C1")), cited
    # A message citing one chain states none.
    assert all(field != "crosscode" for field, _ in order.anomalies + venue.anomalies)
    ```

## A chain carries its creation and its history

With-previous first compares `msgsesseventid`. Equal complete values force a full merge and stop: no predecessor or order-link inheritance is stamped, and the higher of the two places stands. Otherwise following records the predecessor's `uuid` and `transunix` as `prevuuid` and `prevunix`, and the message keeps its own [place](#a-place-counts-one-instant), `seqnum`, unless the predecessor happened at the same instant or later, where it takes the higher of its own and one past the predecessor's. The predecessor is the one message a chained message names: nothing records the chain further back, so a caller reads a chain by following `prevuuid` from row to row. The `srcuuids` a message states - the text line it was parsed out of, the message a parse split it off - are its own and travel nowhere: provenance names what a node was read from, never what it follows. `creaunix` is the earliest the two know - and a walked message stating none takes its chain's, or starting a chain its own instant - and the [state](../types/enum/state.md#the-code-is-the-rank) the furthest along, as `State::merge_with` reads it; a `NEW` stated over a live predecessor that is itself new, working or restated [reads `UPDATED`](../types/enum/state.md#stated-anew-over-a-live-one), and later progress folds over it. An `exprunix` the newer message explicitly states is the current generation's deadline even where it is earlier; only an absent deadline is inherited. A report `FixMsg::is_execution` accepts that states no execution instant already took its own `transunix` as one when it was parsed, before any walk; following carries the later of that clock and the predecessor's `execunix`, and a lifecycle output read back with none is not refilled, because the state it carries may be one it inherited. `sendunix` remains this observation's own. Each base identifier it states takes the parents the chain gave it - [`follow_parents`](../graph/identifier.md#parentage) - and a base missing beside a stated parent is filled from the nearest one; neither overwrites a stated value. An absent `ticker` likewise carries from the predecessor, even when the timed link is already present, while a stated one remains; and what the chain is about - the currency, an order's or an execution's side, the unit, each security identifier the message lacks, the classification and the market - carries where the message says nothing, a statement that knows leading one that does not: an ISIN of a lower rank - a `ZZ` number, a masked one, a typo - yields to a real one, an unclassified CFI to a classified one and the `XXXX` market to a real one. A quote's side is its own tag and carries nowhere; a quote stating nothing of one leg takes that leg from the statement it follows, its price, quantity and currency whole, unless both tag a side and the one it follows quotes one leg alone - a book level, which restates itself whole. The metadata carries as well: a message takes every key of its chain's it does not state - the bridge's namespaced keys its row's `metadata` column holds, so a followed message's row carries the chain's keys - and its own values stand; of the identifiers it takes only those whose `FIX:idmap` entry follows, each with the parents the chain gave it, and of the party ids every one it names none for, each held as its word rather than written through its fields, and each base identifier it states takes the chain's [parents](../graph/identifier.md#parentage). The timed and market readings settle before one finalization, and only an inherited ticker is cloned. A full merge instead takes the statement with the later `sendunix` as reference - a stated one leads an absent one, and the later `transunix` closes a tie - for conflicting content and identifier maps while the provenance list becomes a sorted unique union, keeping the latest expiry and the earliest execution and recording facts either statement knows. A merged statement therefore ranks against a third by the earliest recording it keeps, so which of three statements is the reference depends on the order they merge in. The message is settled again once it moved, so its `hashcode` and `uuid` are those of the merged or chained message and never of what it was before the walk; a stamped stream replayed answers the same messages, because a message already following its predecessor with all inherited facts is one following changes nothing on.

Read across the three steps, provenance and the chain are joins on one column set: a message's `srcuuids` are the `uuid` of the [line](../media/text.md) it was parsed out of, a chained message's `prevuuid` is the `uuid` of the message before it, and the line's batch, the parsed row and the chained row contain the same fifteen [event columns](../graph/market-data.md#columns) under one name and one datatype each, so the joins need no mapping.

## An order's fills are counted once

An order's chain counts the fills its reports state, once each, by the execution identifier a report states of itself - `ExecID(17)`, or `SecondaryExecID(527)`, the exchange's identifier of one fill on another route - so a fill delivered again at another hop, a status reply repeating its last quantity or a copy logged after the order ended counts nothing. The count is the order's: `cumqty` is the chain's first stated total plus every fill counted since, `leavesqty` the accepted order quantity less it, and a partial fill whose count reaches the order quantity reads `FILLED`. The [walk](../graph/event.md#lifecycle-walk) keeps the ledger per live order chain; the FIX crate says what a report is, `FixMsg`'s [`Operation::fill_of`](../graph/operation.md#following-and-merging), read off the message's own tags and never its lifecycle state.

| A report | Reads as (`Fill`) | Moves |
| --- | --- | --- |
| `ExecType(150)` `F`, `1` or `2` stating `LastQty(32)` and an `ExecID(17)` other than `0` | `New`: a fill of `LastQty`, of the rise in `CumQty(14)` over the chain's count where it states none | the count, once: a fill either of whose identifiers the chain already counted adds nothing, moves no quantity of the chain and promotes nothing - the report keeps what it states and takes the chain's count where it states none |
| `150=H` - or FIX 4.2's `ExecTransType(20)` `1`, which states the original's `150` beside it - stating `ExecRefID(19)` | `Bust` | takes the fill `19` names back; one naming no counted fill moves nothing, warned |
| `150=G` (or `20=2`) stating `19` and `32` | `Correct` | the fill `19` names becomes `32`, the count moving by the difference; one naming no counted fill is counted as that fill, warned |
| a fill naming no `ExecID(17)`, or `17=0` - FIX's own value for a status reply | `Unidentified` | nothing: counted nowhere, warned, since the walk cannot tell it from a resend |
| `MultiLegReportingType(442)` `2` - a leg's report, in leg units - a status reply (`150=I`, or `20=3`), an acknowledgement, a cancel, a replace, a restatement, a done-for-day, a pending report and anything else | `NotAFill` | nothing, whatever `LastQty` it repeats |

| Rule | Reading |
| --- | --- |
| Anchor | the chain's first stated total is adopted once, while the chain counted no fill, as what traded before the walk saw the chain - a capture opened mid-life: a report stating 340 traded beside a fill of 21 anchors 319 - and after it the count is the order's: a stated `CumQty(14)` that disagrees is warned once per kind and moves nothing, a cumulative quantity only rising but for a bust or a correction, which the count does itself |
| Order quantity | the accepted one: stated by a report that awaits no answer - none of the pending band, no cancel or replace request - so a `35=G`'s `OrderQty(38)` moves it only once the replace is accepted |
| What is left | `leavesqty` is the order quantity less the count, floored at nothing; a count past the order quantity is an overfill, warned, its state standing. A report that counted nothing keeps a `LeavesQty(151)` it states, warned where it disagrees, so a venue's `151=0` beside `14=40 38=100` - an IOC's remainder expiring - stays as stated |
| `FILLED` | a partial-fill-like state - `IN_PROGRESS`, `PARTIALLY_FILLED`, `TRADE`, and the carrying-on `UPDATED`, `REPLACED`, `RESTATED`, `AMENDED` the fold leaves a fill in - whose count reaches the order quantity after a fill, a bust or a correction reads `FILLED`, and the chain ends as a stated one ends it; never from a repeated fill or an overfill, never while a replace or a cancel awaits its answer. A `FILLED` a report states stays, warned where the count falls short; a `FILLED` it reads off a remainder - an `ExecType(150)` `F` or `G` report's `LeavesQty(151)` of nothing, the parse's derivation or a `39=2` beside it ([`Operation::states_end`](../graph/operation.md#following-and-merging) false) - reads, over a chain the walk counted before it, as the partial fill the count says it is. A chain's first statement stands as read |
| An ended chain | its fills are remembered for the codec's `dedup_window_ms` of event time - `EventIterator::with_window_ns`, one minute by default, none for a nonpositive window - so a copy of one logged at another hop and dated after the chain ended starts no chain from a count: an order's report is yielded as it came, its own chain's first, an execution's restates the execution that counted it and is [yielded once](#an-identity-is-yielded-once) |
| Orders alone | a quote's two legs share one unsided chain, so no quote, execution or trade chain counts a fill; an execution's quantity is the fill it reports ([one quantity per kind](../graph/market.md#setting-fill-or-overwrite)) |

On the bridge capture `rust/tests/support/ulbridge.log`, order `00079132557GLXC0` anchors on its first report's 340 and counts fills 467 and 468 once each - the exchange-side frames' `LeavesQty(151)` of nothing is the child order's - so it stands `PARTIALLY_FILLED` at 472 of 600, 128 left, until its `ExpireTime(126)` expires it; the frame hops' executions of fills 461 and 467 restate the executions walked before them.

JavaScript's `lifecycle` counts the same way.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::State;
    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::MarketDataKind;
    use yggdryl_market::graph::Market;
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
    let lines: [&[u8]; 8] = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|38=100|14=0|151=100|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:31.000|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        // The fill delivered again at a later hop.
        b"8=FIX.4.4|35=8|49=T|56=S|34=3|52=20260102-10:15:31.200|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        // A status reply repeating the last quantity.
        b"8=FIX.4.4|35=8|49=T|56=S|34=4|52=20260102-10:15:32.000|11=A1|37=O1|17=0|150=I|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        // A bust of the fill.
        b"8=FIX.4.4|35=8|49=T|56=S|34=5|52=20260102-10:15:33.000|11=A1|37=O1|17=E3|19=E1|150=H|39=1|38=100|32=40|31=10|14=0|151=100|54=1|55=AAPL|10=0|",
        // A fill of the whole order, then its copy five seconds later.
        b"8=FIX.4.4|35=8|49=T|56=S|34=6|52=20260102-10:15:34.000|11=A1|37=O1|17=E4|150=F|39=2|38=100|32=100|31=10|14=100|151=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=7|52=20260102-10:15:39.000|11=A1|37=O1|17=E4|150=F|38=100|32=100|31=10|151=0|54=1|55=AAPL|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    let orders: Vec<FixMsg> = reader
        .lifecycle(parsed)
        .collect::<yggdryl::Result<Vec<_>>>()?
        .into_iter()
        .filter(|held| held.marketdatakind() == MarketDataKind::Order)
        .collect();
    let [_, _, fill, again, status, bust, filled, copy] = orders.as_slice() else { panic!("eight reports") };
    let read = |held: &FixMsg| (held.get_cumqty(), held.get_leavesqty(), *held.get_state());
    let dec = |text: &str| -> yggdryl::Decimal { text.parse().unwrap() };
    assert_eq!(read(fill), (Some(dec("40")), Some(dec("60")), State::PartiallyFilled));
    // Fill E1 is counted once, and the status reply counts nothing.
    assert_eq!(read(again), read(fill));
    assert_eq!(read(status), read(fill));
    // The bust takes E1 back; the order is still working.
    assert_eq!((bust.get_cumqty(), bust.get_leavesqty()), (Some(dec("0")), Some(dec("100"))));
    assert!(bust.get_state().is_live());
    // E4 leaves nothing: filled, and the chain ends.
    assert_eq!(read(filled), (Some(dec("100")), Some(dec("0")), State::Filled));
    assert_eq!(filled.get_prevuuid(), Some(bust.get_uuid()));
    // Its copy after the chain ended starts no chain from a count.
    assert_eq!((copy.get_prevuuid(), *copy.get_state()), (None, State::Filled));
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    from yggdryl import MarketDataKind, State
    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    lines = [
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30.000|11=A1|55=AAPL|54=1|38=100|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=1|52=20260102-10:15:30.500|11=A1|37=O1|150=0|39=0|38=100|14=0|151=100|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=2|52=20260102-10:15:31.000|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        # The fill delivered again at a later hop.
        b"8=FIX.4.4|35=8|49=T|56=S|34=3|52=20260102-10:15:31.200|11=A1|37=O1|17=E1|150=F|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        # A status reply repeating the last quantity.
        b"8=FIX.4.4|35=8|49=T|56=S|34=4|52=20260102-10:15:32.000|11=A1|37=O1|17=0|150=I|39=1|38=100|32=40|31=10|14=40|151=60|54=1|55=AAPL|10=0|",
        # A bust of the fill.
        b"8=FIX.4.4|35=8|49=T|56=S|34=5|52=20260102-10:15:33.000|11=A1|37=O1|17=E3|19=E1|150=H|39=1|38=100|32=40|31=10|14=0|151=100|54=1|55=AAPL|10=0|",
        # A fill of the whole order, then its copy five seconds later.
        b"8=FIX.4.4|35=8|49=T|56=S|34=6|52=20260102-10:15:34.000|11=A1|37=O1|17=E4|150=F|39=2|38=100|32=100|31=10|14=100|151=0|54=1|55=AAPL|10=0|",
        b"8=FIX.4.4|35=8|49=T|56=S|34=7|52=20260102-10:15:39.000|11=A1|37=O1|17=E4|150=F|38=100|32=100|31=10|151=0|54=1|55=AAPL|10=0|",
    ]
    walked = reader.lifecycle(reader.parse_lines(lines))
    orders = [held for held in walked if held.marketdatakind is MarketDataKind.ORDR]
    _, _, fill, again, status, bust, filled, copy = orders


    def read(held):
        return (held.cumqty.as_py(), held.leavesqty.as_py(), held.state)


    assert read(fill) == (Decimal(40), Decimal(60), State.PARTIALLY_FILLED)
    # Fill E1 is counted once, and the status reply counts nothing.
    assert read(again) == read(fill) and read(status) == read(fill)
    # The bust takes E1 back; the order is still working.
    assert read(bust)[:2] == (Decimal(0), Decimal(100)) and bust.state.is_live()
    # E4 leaves nothing: filled, and the chain ends.
    assert read(filled) == (Decimal(100), Decimal(0), State.FILLED)
    assert filled.prevuuid == bust.uuid
    # Its copy after the chain ended starts no chain from a count.
    assert copy.prevuuid is None and copy.state is State.FILLED
    ```

## A replace chain names its parents

A message that states a chain identity again under another value is a step in that identifier's chain, and the values the chain held before are kept under the types the identifier's [parents](../graph/identifier.md#parentage) are, read through the registry's [`FIX:parents`](registry.md#parents-of-an-identifier): an `OrderID(37)` that changed names the value before it `parentorderid` and its chain's first value `origorderid`, a `ClOrdID(11)` names the previous one `origclordid`, FIX's own `OrigClOrdID(41)`, a `TradeID(1003)` `origtradeid`, FIX's `OrigTradeID(1126)`, and a `TradeReportID(571)` `tradereportrefid`, FIX's `TradeReportRefID(572)`. Only a chain identity has parents: the `ExecID(17)`, `TrdMatchID(880)` or `TVTIC` a report states of itself is no step of anything and carries no lineage. The walk joins a replace to its chain by the first value its lineage field states, under the base - never by `parentorderid` itself, which it writes - and every settle fills a base from the nearest parent a message states.

| Base | Parents, nearest first | A chain `O1`, `O2`, `O3` ends with |
| --- | --- | --- |
| `orderid` | `parentorderid`, `origorderid` | `parentorderid` `O2`, `origorderid` `O1` |
| `clordid` | `origclordid` | `origclordid` the one before it |
| `tradeid` | `origtradeid` | `origtradeid` the one before it |
| `tradereportid` | `tradereportrefid` | `tradereportrefid` the one before it |

A replace chain of one order - its identifiers under `fix`, `-` where a message holds none:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Element;
    use yggdryl_market::graph::Operation;
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::IdType;
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|",
        b"8=FIX.4.4|35=8|11=C1|37=O1|150=0|39=0|55=AAPL|52=20260921-10:00:01|10=0|",
        b"8=FIX.4.4|35=G|11=C2|41=C1|37=O2|55=AAPL|54=1|38=10|44=101|52=20260921-10:00:02|10=0|",
        b"8=FIX.4.4|35=G|11=C3|41=C2|37=O3|55=AAPL|54=1|38=10|44=102|52=20260921-10:00:03|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    let chained: Vec<FixMsg> = reader.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;

    // What each message holds under each type, `-` where it holds none.
    let held = |message: &FixMsg| -> [String; 5] {
        ["orderid", "parentorderid", "origorderid", "clordid", "origclordid"].map(|kind| {
            let kind: IdType = kind.parse().expect("a type");
            message.get_identifiers().get(&kind).unwrap_or("-").to_owned()
        })
    };
    assert_eq!(held(&chained[0]), ["-", "-", "-", "C1", "-"]);
    assert_eq!(held(&chained[1]), ["O1", "-", "-", "C1", "-"]);
    // Each replace names the value before it and the chain's first.
    assert_eq!(held(&chained[2]), ["O2", "O1", "O1", "C2", "C1"]);
    assert_eq!(held(&chained[3]), ["O3", "O2", "O1", "C3", "C2"]);
    // One chain: the replaces joined the order by the parent they state.
    assert!(chained.iter().all(|message| message.get_crossuuid() == chained[0].get_crossuuid()));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    KINDS = ("orderid", "parentorderid", "origorderid", "clordid", "origclordid")
    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    lines = [
        b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|",
        b"8=FIX.4.4|35=8|11=C1|37=O1|150=0|39=0|55=AAPL|52=20260921-10:00:01|10=0|",
        b"8=FIX.4.4|35=G|11=C2|41=C1|37=O2|55=AAPL|54=1|38=10|44=101|52=20260921-10:00:02|10=0|",
        b"8=FIX.4.4|35=G|11=C3|41=C2|37=O3|55=AAPL|54=1|38=10|44=102|52=20260921-10:00:03|10=0|",
    ]
    chained = list(reader.lifecycle(reader.parse_lines(lines)))


    def held(message):
        # What a message holds under each type, "-" where it holds none.
        return tuple(message.identifiers.get(kind) or "-" for kind in KINDS)


    assert held(chained[0]) == ("-", "-", "-", "C1", "-")
    assert held(chained[1]) == ("O1", "-", "-", "C1", "-")
    # Each replace names the value before it and the chain's first.
    assert held(chained[2]) == ("O2", "O1", "O1", "C2", "C1")
    assert held(chained[3]) == ("O3", "O2", "O1", "C3", "C2")
    # One chain: the replaces joined the order by the parent they state.
    assert all(message.crossuuid == chained[0].crossuuid for message in chained)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const KINDS = ['orderid', 'parentorderid', 'origorderid', 'clordid', 'origclordid']
    const reader = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config/fix')))
    const lines = [
      '8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|',
      '8=FIX.4.4|35=8|11=C1|37=O1|150=0|39=0|55=AAPL|52=20260921-10:00:01|10=0|',
      '8=FIX.4.4|35=G|11=C2|41=C1|37=O2|55=AAPL|54=1|38=10|44=101|52=20260921-10:00:02|10=0|',
      '8=FIX.4.4|35=G|11=C3|41=C2|37=O3|55=AAPL|54=1|38=10|44=102|52=20260921-10:00:03|10=0|',
    ].map((line) => Buffer.from(line))
    const chained = [...reader.lifecycle([...reader.parseLines(lines)])]

    // What a message holds under each type, '-' where it holds none.
    const held = (message) => KINDS.map((kind) => message.identifiers.get(kind) ?? '-')
    assert.deepEqual(held(chained[0]), ['-', '-', '-', 'C1', '-'])
    assert.deepEqual(held(chained[1]), ['O1', '-', '-', 'C1', '-'])
    // Each replace names the value before it and the chain's first.
    assert.deepEqual(held(chained[2]), ['O2', 'O1', 'O1', 'C2', 'C1'])
    assert.deepEqual(held(chained[3]), ['O3', 'O2', 'O1', 'C3', 'C2'])
    // One chain: the replaces joined the order by the parent they state.
    assert.ok(chained.every((message) => message.crossuuid === chained[0].crossuuid))
    ```

A trade replaced under a new `TradeID(1003)` and `TradeReportID(571)` - `TradeReportTransType(487)` `2` - names the trade it replaces by `OrigTradeID(1126)` and `TradeReportRefID(572)`, so the walk keeps it in the first trade's chain, keyed by that trade:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl_market::graph::Operation;
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::IdType;
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let reader = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));
    let lines: [&[u8]; 2] = [
        b"8=FIX.4.4|35=AE|571=TR1|1003=T1|55=ABB|32=10|31=100|52=20260814-12:00:00|10=0|",
        b"8=FIX.4.4|35=AE|571=TR2|572=TR1|487=2|1003=T2|1126=T1|55=ABB|32=10|31=100|52=20260814-12:00:01|10=0|",
    ];
    let parsed: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;
    let walked: Vec<FixMsg> = reader.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    let [first, replaced] = walked.as_slice() else { panic!("two trade captures") };
    // Keyed by the trade, never by an order it names.
    assert_eq!(first.get_crosscode(), "21:0:T1");
    // The replacement joined by the first values its lineage fields name, and
    // stands under the chain's code though it states T2.
    assert_eq!(replaced.get_prevuuid(), Some(first.get_uuid()));
    assert_eq!(replaced.get_crosscode(), "21:0:T1");
    assert_eq!(replaced.get_crossuuid(), first.get_crossuuid());
    let held = |kind: &str| -> Option<String> {
        let kind: IdType = kind.parse().expect("a type");
        replaced.get_identifiers().get(&kind).map(str::to_owned)
    };
    assert_eq!(held("tradeid").as_deref(), Some("T2"));
    assert_eq!(held("origtradeid").as_deref(), Some("T1"));
    assert_eq!(held("tradereportrefid").as_deref(), Some("TR1"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
    lines = [
        b"8=FIX.4.4|35=AE|571=TR1|1003=T1|55=ABB|32=10|31=100|52=20260814-12:00:00|10=0|",
        b"8=FIX.4.4|35=AE|571=TR2|572=TR1|487=2|1003=T2|1126=T1|55=ABB|32=10|31=100|52=20260814-12:00:01|10=0|",
    ]
    first, replaced = reader.lifecycle(reader.parse_lines(lines))
    # Keyed by the trade, never by an order it names.
    assert first.crosscode == "21:0:T1"
    # The replacement joined by the first values its lineage fields name, and
    # stands under the chain's code though it states T2.
    assert replaced.prevuuid == first.uuid
    assert (replaced.crosscode, replaced.crossuuid) == ("21:0:T1", first.crossuuid)
    kinds = ("tradeid", "origtradeid", "tradereportrefid")
    assert [replaced.identifiers.get(kind) for kind in kinds] == ["T2", "T1", "TR1"]
    ```

## A twin is not a successor

A bridge logs one message at every hop it passes, so a capture routinely holds the same delivery twice. Each observation with all four delivery parts prebuilds `msgsesseventid` on its capture - a column of the fixed row of its own - as `<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>`: the values joined by `:` exactly as stated, the sequence in canonical `u64`, so `8:e7256476:9effef3e6a:1094`. A session or context holding a `:` of its own could join two splits to one key; a bridge names none that way. `lifecycle` fully merges equal values before sorting and walking instead of making one follow the other, and direct with-previous applies the same forced merge before ordinary following. Observations are ranked latest `sendunix` first, and the others are folded into the first, which stays the reference row: the choice is made once over every observation, because a folded observation keeps the earliest recording its statements know and ranking by that against the next would hand the reference to whichever arrives later. Direct with-previous merges two at a time and has no such view - an observation already folded there ranks by its earliest recording against the next. Its stated scalar wins each conflict and older observations fill only what it leaves absent. Repeating-group occurrence `i` merges with occurrence `i` recursively; every merged level is sorted by FIX tag then folded name and contains one logical key, so marked and bare bridge spellings at the same indexes do not append duplicate occurrences or members. Both source UUIDs remain; the merged `execunix` and `sendunix` are their earliest values. What that fold costs is worth stating plainly, because nothing bounds it: an unbounded number of observations sharing one `msgsesseventid` fold into one event, however far apart their clocks and however much their content diverges, since the fold is deliberately blind to the content identities that would otherwise keep them apart - a bridge capture where one message is logged at a dozen hops routinely answers a chain several times shorter than its observation count. The bounds this walk states are all the other way round, naming what never folds: an incomplete key, a different message type, session, context or sequence, a distinct delivery carrying equal business content, and a lifecycle output or snapshot view read back in.

`srcuuids` is the sorted unique provenance union. Which observation becomes the merge reference is settled separately by the later `sendunix` and then the later `transunix`, with an exact tie keeping the caller's leading statement; no list position identifies that reference or an arrival order. Feeding the same observations in another order therefore leaves the serialized provenance list unchanged. A missing or empty text part or missing sequence produces no `msgsesseventid`, and a different message type, context, session or sequence is a different delivery. Already placed lifecycle outputs and snapshot views are not raw capture observations and are never coalesced again on replay.

After that merge, `lifecycle` removes an exact republication or true retransmission across the whole finite capture, however many distinct deliveries it holds: a complete FIX header keys the session, sequence, original time and recorded canonical content identity, while a headerless bridge row must match the recorded event identity and content in the same capture session, context and direction. Reordering projected fields during Arrow reconstruction does not create another delivery, and neither does a group read back empty where the parse stated none: a table column holds a group as null or as at least one occurrence, so a list read back as `[]` where the row held null - PyIceberg reads a null list of structs back as `[]` - states nothing, the group absent, while a stated zero (`NoPartySubIDs(802)=0`) rides the residual `fixentries` record. The observation merge writes no count, since a merged group's length is its count, so a table that stores an absent list as an empty one hands the walk the content the parse recorded, and an observation settled again from it keeps its `hashcode` and folds with the ones that kept their recorded code. When `SendingTime(52)` was unstated, the retained event time supplies its clock; a replay's stated `OrigSendingTime(122)` takes precedence. The retained delivery set therefore grows with distinct deliveries in that finite capture; it is not a recent window.

After delivery deduplication, the walk still recognizes two source statements that arrive under the same event identity: the later statement takes the live one's predecessor, place, snapshot, cross code, lifecycle and every `metadata` key it does not state, and finalizes to the same `uuid`, so the chain grows by nothing. Deduplication decides whether the finite capture delivered the same message twice; restating decides whether two retained graph events are statements of one event; and the [window](#an-identity-is-yielded-once) decides that an event already yielded is not yielded again - so two distinct deliveries of equal content at one instant are two deliveries to the set and one identity in the walk's answer.

## An identity is yielded once

A twin restated is the live message again under its identity, and a hop a bridge logged after its chain had already moved on comes back under that identity too: a stream keyed by `uuid` would hold the event twice, adjacent or far apart. `lifecycle` therefore yields each identity once within a window of event time, `dedup_window_ms` (Rust and Python) / `dedupWindowMs` (JavaScript), one minute - `FixCodec::DEFAULT_DEDUP_WINDOW_MS` - unless the codec states otherwise. An identity is remembered at the `transunix` it was yielded under and forgotten once the walk has yielded a message more than the window after it, so what the window holds is bounded by the event time it spans rather than by the capture's length; a later message under a remembered identity is dropped, whichever came between. The walk has already read it: the chain keeps what it said - its earliest recording and execution instants, the lifecycle it carries - and only its copy is not yielded again. A grid view is the live message as of a tick, one per tick and chain, so it is neither dropped nor remembered, and the view at the live message's own instant still answers that message's identity. A nil identity - an instant no UUIDv7 holds - is no identity and never repeats. A nonpositive window, `None` or `null` remembers nothing and yields every message the walk answers; `with_dedup_window_ms` (Rust and Python) / `withDedupWindowMs` (JavaScript) sets another on a codec in hand, keeping every other setting, and `dedup_window_ms` / `dedupWindowMs` reads it back - `None` / `null` where there is none.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Element;
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    yggdryl_fix::install()?;

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
    assert_eq!(every[2].get_uuid(), every[0].get_uuid());

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
    assert every[2].uuid == every[0].uuid

    once = list(codec.lifecycle(parsed))
    assert [held.uuid for held in once] == [held.uuid for held in every[:2]]
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
    assert.equal(every[2].uuid, every[0].uuid)

    const once = [...codec.lifecycle(parsed)]
    assert.deepEqual(once.map((held) => held.uuid), every.slice(0, 2).map((held) => held.uuid))
    ```

## A place counts one instant

`seqnum` is where a message stands among the messages of its instant - the same `transunix`, to the nanosecond: 0 for the first of a run - the messages a stream hands over at that instant one after another, with no other instant between - one more for each next one in the run, and 0 again when the next run starts, a stream that comes back to an instant it left starting a run there anew. It orders two identities of one millisecond: `uuid` is RFC 9562 UUIDv7 over the millisecond, then the place in its 12-bit sequence lane, then an XXH3 payload seeded by the cross code, so identities sort by millisecond, then place. Two instants inside one millisecond each count from 0, so a message later in the millisecond can sort before one earlier in it; the lane holds up to 4,095, past which the payload still separates identities. A place is where a message stands, never what it says: it is outside `hashcode`, and reaches the identity through `uuid` alone.

The [parse](capture.md#every-message-is-dated) places by order: `parse_line` and `parse_text_line` within a row, and `parse_lines`, `parse_text_lines`, `parse_arrow_messages` and `parse_text_arrow_reader` across rows, hand every message the next place of its instant's run. A report and the execution the parse [split off](message.md#a-parse-splits-what-a-message-reports) it are places 0 and 1, and a split message's `srcuuids` name the identity its source was placed under. This walk places again by content, because the order it walks in is its own - sorted by instant, a stretch of messages dated by their transactions, repeat deliveries dropped: the expirations of an instant come first, each at the next place of its deadline, then each content read at that instant takes the next place, and a content repeated there takes the place it already took, so a message a bridge logged at two hops is one identity and the twin, delivery and window rules read one. What a walk made - a grid view, an expiration at its deadline - keeps the place the walk gave it when the output is walked again, so a walked stream answers itself. A place depends on nothing before the run of its instant, so regenerating any stretch of a capture that begins at a new instant places each message in it as the whole capture did - no count carried over from an earlier hour, file or chain.

Following keeps a message's own place unless its predecessor happened at the same instant or later, where it takes the higher of its own and one past the predecessor's; a merge keeps the higher of two places. The [market projection](arrow.md#fix-market-books) hands each leaf its message's place, so the entries of one book message share it; a book's place is the highest of its members' places at the book's own instant, and a [book](../graph/book.md#book-fold) fold trusts a chain step's place only where its predecessor stands at the same instant, folding in arrival order otherwise. A [text line](../media/text.md)'s place is its row number instead, which orders the lines of one millisecond.

## Instruments are learned in instant order

Messages are sorted before the walk learns and fills them, one at a time, through an [`Instruments`](../graph/instrument.md): `learn` reads what a message states about its instrument, keyed by the [cross code](../graph/instrument.md#the-cross-code) its facts spell - its stated real ISIN, closing under a listed prefix; an FX pair's class and body, its number [minted](../graph/instrument.md#the-minted-number); a derivative's class and body, its underlying or its legs resolved to codes the collection keys ([Derivatives and strategies](../graph/instrument.md#derivatives-and-strategies)), else a placeholder under its real ISIN - and by nothing else: its detailed CFI code, the country of issue it states that its ISIN's prefix does not already say (`CountryOfIssue(470)`), the instrument it is written on (`UnderlyingSecurityID(309)` under an ISIN source, an `UnderlyingSymbol(311)` shaped as an ISIN, a related `Underlier`, a bridge's `UnderlyingISIN`; a basket names none), which stays out of `securityids`, the expiry, the strike, the settles, the multiplier and the exercise style, a strategy's `NoLegs(555)`, the [product category](../graph/instrument.md#the-product-category) a bridge's key states (`EUSIPACode`, `OMS_SSPACategory`; two different ones name none), which stays out of every map, the instrument component's descriptive fields (`Issuer(106)`, `SecurityDesc(107)`, `SecurityType(167)` and eight more) as its metadata, its market but `XXXX`, its ticker, its currency but `XXX` - unless it holds a currency pair, where `Currency(15)` is the currency dealt and no listing's - the pair it states as a `forex` identifier and every security code of a type the crate names that it states with a real value rather than derived, under its own source, never a masked number or a typo, at its `transunix`, which every learn states as the instrument's `lastunix` - so a message stating its ISIN alone is learned and a known instrument met later moves its instant; then `fill` gives it what it leaves unsaid from the instrument `resolve` names - by its real ISIN, stated or derived, else its cross code, else a minted number it holds, else a lookup code, else its ticker on its market (`Instruments::get_by_ticker`), whose ISIN it derives first - over none, or over a number of a lower rank: the equivalents and the pair from `derived`, the ticker on the same market, the CFI code where the instrument's refines it, the currency on the same stated market under the listing's ticker, and the instrument's cross code as `instcode` where the message holds none. A filled code is derived, `derive_securityid`, from `derived`, so it reaches no field and no wire - a row without the `securityids` column carries it only through a [view](message.md#typed-tags), reading back stated; a refined CFI code shows in the market `cficode` and in the fixed row only where `CFICode(461)` is unstated, a filled currency in the market `currency` and never in `Currency(15)`, and a filled ISIN or ticker moves the [book](../graph/book.md#books-by-key) the message stands in - its ISIN, else its ticker. The fill settles no more than the market facts it implies: the message's identity is the parse's, and `instcode` is fed to no digest. A private source's code (`100`) and a venue's own word are never learned, a coarse CFI such as `ESXXXX` is no statement, and a RIC or a Bloomberg symbol is a listing code, held on the listing of the market the message names, never a key.

A valid stated value fills a fact the instrument lacks and replaces one that differs, whatever the time; an invalid value moves nothing; a compatible CFI code refines the held one and a contradicting one replaces it. An instrument holds one [listing](../graph/instrument.md#listings) per market: the instrument's facts are every listing's, a market it is not listed on makes a listing there, and a listing fact of a message stating no market lands on its single listing, or on none of several, with one warning ([the update rule](../graph/instrument.md#the-update-rule)). The collection holds at most `max_instruments` instruments - 16,384 by default, each at most 32 KiB - and a new instrument past the bound is passed over with one warning while a known one keeps learning; nothing is evicted.

The parse reads the same table: a door opened on a codec sharing a collection fixes its table once, as it opens, under one lock on the calling thread, and every message it reads takes from that table the security identifiers it leaves unsaid - the ISIN its ticker names on its market, every equivalent, the pair - as `derived` identifiers, before its identity is stamped, so a ticker-only message stands in its instrument's book from the first message; it writes `instcode` only where the message alone spells the code - a stated real ISIN, a detected FX pair - and never from the table, which the lifecycle fills it from. No worker reaches the lock, a learn while a door reads reaches no message of it, and a parse learns nothing and moves nothing. Without a collection of its own the walk learns into one it makes, starting empty, so a second walk starts empty too, and a parse fills nothing. `FixCodec::with_instruments(Arc<Mutex<Instruments>>)` - Python `FixCodec(..., instruments=held)`, JavaScript `{ instruments }` - shares one collection with every walk the codec runs and every door it opens, one uncontended lock per message walked and one per door opened, so a walk run after another starts from what the first learned, and a collection loaded from a store (`Instruments::from_url`) fills from the first message; `FixCodec::from_env` shares the [process's own](../graph/instrument.md#the-process-instruments). Walks run at once on one shared collection interleave their learning: share it across walks run one after another. The walk reads its input as already cleaned: every message it is handed is walked, whatever its type - the parse that read it refused what the codec refuses - and nothing is derived or detected again on the way in.

=== "Rust"

    ```rust
    use std::sync::{Arc, Mutex};

    use yggdryl_market::graph::Market;
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    use yggdryl_market::Instruments;
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let instruments = Arc::new(Mutex::new(Instruments::new()));
    let codec = FixCodec::new(Arc::clone(&registry)).with_instruments(Arc::clone(&instruments));
    let walk = |line: &str| -> yggdryl::Result<Vec<FixMsg>> {
        let parsed: Vec<FixMsg> = codec.parse_lines([line]).collect::<yggdryl::Result<_>>()?;
        codec.lifecycle(parsed).collect()
    };
    walk("8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|")?;
    // A later walk stating only the ticker on the market starts from what
    // the first learned: the ISIN at the parse, the currency in the walk.
    let later = walk("8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|")?;
    assert_eq!(later[0].get_isincode(), Some("CH0012214059"));
    assert_eq!(later[0].get_instcode(), Some("CH0012214059"), "the instrument's cross code");
    assert_eq!(later[0].get_securityids().get(&yggdryl_market::IdType::Ric), Some("HOLN.S"));
    assert_eq!(later[0].get_currency().as_str(), "CHF");
    // A codec without one learns into each walk's own.
    let alone = FixCodec::new(registry);
    let parsed: Vec<FixMsg> = alone.parse_lines(["8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|"]).collect::<yggdryl::Result<_>>()?;
    let walked: Vec<FixMsg> = alone.lifecycle(parsed).collect::<yggdryl::Result<_>>()?;
    assert_eq!(walked[0].get_isincode(), None);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import Instruments
    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix"))
    instruments = Instruments()
    codec = FixCodec(registry, instruments=instruments)
    assert codec.instruments == instruments


    def walk(line: bytes) -> list:
        return list(codec.lifecycle(codec.parse_lines([line])))


    walk(b"8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|")
    # A later walk stating only the ticker on the market starts from what the
    # first learned: the ISIN at the parse, the currency in the walk.
    [later] = walk(b"8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|")
    assert later.isincode == later.instcode == "CH0012214059" and later.securityids.get("ric") == "HOLN.S"
    assert later.currency.as_py() == "CHF"
    # A codec without one learns into each walk's own.
    alone = FixCodec(registry)
    [unfilled] = alone.lifecycle(alone.parse_lines([b"8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|"]))
    assert unfilled.isincode is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Instruments, fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const instruments = new Instruments()
    const codec = new fix.FixCodec(registry, { instruments: instruments })
    assert.ok(codec.instruments.equals(instruments))
    const walk = (reader, line) => [...reader.lifecycle([...reader.parseLines([Buffer.from(line)])])]

    walk(codec, '8=FIX.4.4|35=D|11=A|22=4|48=CH0012214059|454=1|455=HOLN.S|456=5|461=ESVUFR|55=HOLN|207=XSWX|15=CHF|10=0|')
    // A later walk stating only the ticker on the market starts from what the
    // first learned: the ISIN at the parse, the currency in the walk.
    const [later] = walk(codec, '8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|')
    assert.equal(later.isincode, 'CH0012214059')
    assert.equal(later.securityids.get('ric'), 'HOLN.S')
    assert.equal(later.currency, 'CHF')
    // A codec without one learns into each walk's own.
    const [unfilled] = walk(new fix.FixCodec(registry), '8=FIX.4.4|35=D|11=B|55=HOLN|207=XSWX|10=0|')
    assert.equal(unfilled.isincode, null)
    ```

A currency pair is detected by the parse, never by the walk: the parse [detects](capture.md#a-currency-pair-is-read-off-the-symbol) it off `Symbol(55)` into the message's derived identifiers, remembered per dictionary for 4,096 symbols, a symbol naming no pair remembered as one: a detected pair is derived, so a stated `forex` identifier replaces it. The parse also spells the pair's [cross code](../graph/instrument.md#forex) - `IF:EUR/USD`, or a forward's or a swap's with its settles - as `instcode` and derives its [minted number](../graph/instrument.md#the-minted-number) under `derived:isin`; the walk then learns the pair's instrument and fills it as any other. A row's `forexcode` is a [view](message.md#typed-tags), resolved once as the row is read: the pair its symbol names reads back derived and follows the symbol, and, with no `securityids` column, any other pair the reading does not answer reads back stated under the `forex` base key, whether it was derived lost.

## Snapshots are a grid

The codec's positive `snapshot_ns` / `snapshotNs` gives its lifecycle an epoch-aligned grid; zero, a negative width, `None` / `null`, or omission disables it, which is the default. A codec in hand takes another grid with `with_snapshot_ns` (Rust and Python) / `withSnapshotNs` (JavaScript), which keeps every other setting of it. At every crossed tick the walk yields a separate owned view of every living message: the live message as of that tick. The view is dated at the tick - its `transunix` is the tick, and its `snapunix` the instant the live message it copies was stated at, that message's own `snapunix` where it is itself a view - so its `uuid` is the identity that instant derives, and a view is a row of its own wherever rows are keyed by identity within a time; a view at the live message's own instant derives the live message's identity. Its content (`hashcode`), `seqnum`, `prevuuid` and `crossuuid` are the live message's, and it does not advance the chain: the next message still follows the live one. Source messages keep the snapshot fact they stated and are never stamped backward to a previous tick.

At one instant, finite expirations come first, then every source message at that instant, then its views. Expiration emits one owned `EXPIRED` message at the exact deadline with the live message as predecessor, at the next place of that deadline - `seqnum` 0 for the first - then purges the identity; a replaced or ended generation's stale deadline emits nothing. At EOF the grid reaches the greatest finite deadline still encountered, or the last source instant where none remains, and stops; a nonexpiring message does not make a finite capture infinite. The width is a caller's output choice: the number of rows is proportional to crossed ticks times identities alive at each tick, with no implicit count or span cap.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixRegistry};
    yggdryl_fix::install()?;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let codec = FixCodec::new(registry).with_snapshot_ns(1_000_000_000);
    assert_eq!(codec.snapshot_ns(), Some(1_000_000_000));
    let source = codec.parse_fix_line(
        b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|",
    )?;
    let source_uuid = source.get_uuid();
    let deadline = source.get_exprunix().expect("the stated expiry");
    let walked: Vec<_> = codec.lifecycle([source]).collect::<yggdryl::Result<_>>()?;
    let live = &walked[0];
    assert!(live.get_snapunix().is_none(), "the source comes before its tick's view");

    let snapshots: Vec<_> = walked
        .iter()
        .filter(|message| message.get_snapunix().is_some())
        .collect();
    assert_eq!(snapshots.len(), 2, "the source tick and the crossed second");
    // A view is the live message as of its tick: dated at it, its snapshot
    // instant the one the live message was stated at, with the live message's
    // content, place and chain.
    assert!(snapshots.iter().all(|view| {
        view.get_snapunix() == Some(live.get_transunix())
            && view.get_transunix() < deadline
            && view.get_hashcode() == live.get_hashcode()
            && view.get_crossuuid() == live.get_crossuuid()
            && (view.get_seqnum(), view.get_prevuuid()) == (0, None)
    }));
    // Its identity is the one its tick derives: the source's at the source's
    // own instant, one of its own a second later.
    assert_eq!(snapshots[0].get_uuid(), source_uuid);
    assert_ne!(snapshots[1].get_uuid(), source_uuid);
    let expired = walked.last().expect("the deadline event");
    assert_eq!((expired.get_transunix(), expired.get_state().as_str()), (deadline, "EXPIRED"));
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
    source_uuid = source.uuid
    deadline = source.exprunix
    assert deadline is not None
    walked = list(codec.lifecycle([source]))
    live = walked[0]
    assert live.snapunix is None  # the source comes before its tick's view

    snapshots = [message for message in walked if message.snapunix is not None]
    assert len(snapshots) == 2
    # A view is the live message as of its tick: dated at it, its snapshot
    # instant the one the live message was stated at, with the live message's
    # content, place and chain.
    assert all(
        view.snapunix == live.transunix
        and view.transunix < deadline
        and view.hashcode == live.hashcode
        and view.crossuuid == live.crossuuid
        and (view.seqnum, view.prevuuid) == (0, None)
        for view in snapshots
    )
    # Its identity is the one its tick derives: the source's at the source's
    # own instant, one of its own a second later.
    assert snapshots[0].uuid == source_uuid
    assert snapshots[1].uuid != source_uuid
    expired = walked[-1]
    assert (expired.transunix, expired.state) == (deadline, State.EXPIRED)
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
    const sourceUuid = source.uuid
    const deadline = source.exprunix
    const walked = [...codec.lifecycle([source])]
    const live = walked[0]
    assert.equal(live.snapunix, null, 'the source comes before its tick\'s view')

    const snapshots = walked.filter((message) => message.snapunix !== null)
    assert.equal(snapshots.length, 2)
    // A view is the live message as of its tick: dated at it, its snapshot
    // instant the one the live message was stated at, with the live message's
    // content, place and chain.
    assert.ok(snapshots.every((view) =>
      view.snapunix === live.transunix
      && view.transunix < deadline
      && view.hashcode === live.hashcode
      && view.crossuuid === live.crossuuid
      && view.seqnum === 0
      && view.prevuuid === null,
    ))
    // Its identity is the one its tick derives: the source's at the source's
    // own instant, one of its own a second later.
    assert.equal(snapshots[0].uuid, sourceUuid)
    assert.notEqual(snapshots[1].uuid, sourceUuid)
    const expired = walked.at(-1)
    assert.deepEqual([expired.transunix, expired.state], [deadline, 'EXPIRED'])
    assert.deepEqual([expired.prevuuid, expired.seqnum], [sourceUuid, 0])
    ```

## A sorted source is walked one hour at a time

A codec's `sorted_lifecycle` pin - `with_sorted_lifecycle(true)` (Rust), the `sorted_lifecycle=True` keyword or `with_sorted_lifecycle(True)` (Python), the `sortedLifecycle: true` option or `withSortedLifecycle(true)` (JavaScript), each read back by `sorted_lifecycle` / `sortedLifecycle` - states that the messages `lifecycle` is handed arrive in instant order: a table read hour partition by hour partition, sorted by `transunix`. The walk then reads them as they come instead of collecting the capture, and holds one epoch hour of them at a time. Each message waits in the bucket of its hour - an observation of a delivery already held joins the others of it, whatever its own hour - and a bucket is walked, its deliveries folded and its messages sorted exactly as a whole capture is, once the stream has read a message two hours past it. That hour of grace is what a message the walk dates by its transaction rather than by where it was stored needs to land in its own hour, and what the hops of one delivery logged across an hour's end need to meet. A message dated before an hour already walked is walked where it arrives, as any late message is. What is held is the messages of the hours not yet walked - two of them for a source in order - never the stream, and a source failure is yielded once every hour still held is walked.

The pin is off by default, because nothing says a capture is in order: a capture is collected and sorted whole. On a source that is in instant order both answer the same walk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event};
    use yggdryl::local::LocalFolder;
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry};
    yggdryl_fix::install()?;

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
            .map(|held| held.map(|message| (message.get_uuid(), message.get_seqnum(), message.get_prevuuid())))
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
        return [(held.uuid, held.seqnum, held.prevuuid) for held in codec.lifecycle(messages)]


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
      .map((held) => [held.uuid, held.seqnum, held.prevuuid])

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
    use yggdryl::{ArrowCastOptions, Scalar, Serie};
    use yggdryl_fix::{FixCodec, FixMsg, FixRegistry, fix_schema};
    yggdryl_fix::install()?;

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
    // the report it was split off: a first place is zero, never absent.
    assert_eq!(column("seqnum").scalar(0)?, Scalar::from(0_u64), "a first message stands at zero");
    assert_eq!(column("seqnum").scalar(1)?, Scalar::from(0_u64));
    assert_eq!(column("seqnum").scalar(2)?, Scalar::from(1_u64));
    ```

## Edges

- Two messages of one chain at the same instant are neither after nor before one another: the walk keeps them in arrival order, and the second follows the first at the higher of its own place and one past the first's.
- A message that happened before the live one it would follow - out of order on a walk a caller opened as sorted - follows nothing and changes nothing on the live one, but is yielded re-keyed onto its chain, the chain's side written where it states none; `lifecycle` sorts first, so nothing arrives out of order there. A sorted lifecycle sorts within each hour it holds; a message dated before an hour already walked is walked where it arrives.
- A message the reading refuses - its own predecessor, one following changes nothing on - is yielded under its chain's side and cross code, beside a [warning](capture.md#warnings), and still stands as the live one.
- A terminal state ends the chain once the message is yielded; a message arriving under the retired identity starts a chain afresh, sharing the `crossuuid` and nothing else. A report of a fill the ended chain counted, within the window, starts nothing from a count: it states what it states, and an execution split off it restates the one that counted the fill.
- A count only ends a chain the walk saw fill: an overfill, a repeated fill and a stated `LeavesQty(151)` of nothing below the order quantity end nothing, and a `FILLED` an `ExecType(150)` `F` report reads off its remainder over a chain the walk counted short reads `PARTIALLY_FILLED` - that report's row keeps the wire's `CumQty(14)`, `LeavesQty(151)` and `OrdStatus(39)`, while the message's `cumqty`, `leavesqty` and `state` are the count's ([An order's fills are counted once](#an-orders-fills-are-counted-once)).
- A message whose expiry - `ExpireTime(126)`, else `ValidUntilTime(62)`, else the end of the day `ExpireDate(432)` names, the next midnight - is at or before its `transunix` is not alive and opens no chain; an order good until today is alive all of it. The instrument's `MaturityDate(541)` is no deadline: a venue that ends orders on a maturing instrument says so in a message of its own. A later deadline is scheduled even where no later source arrives: one `EXPIRED` message is emitted at the deadline and the identity is purged.
- The state is the furthest along the two know, as `State::merge_with` reads it, so a chain never moves backward: a `New` after a `Filled` under a live chain would be yielded `Filled`; it is not, because the fill retired the chain first. A `NEW` after a live `NEW` reads `UPDATED`; after a live `PENDING_NEW` it is the first acknowledgement and stays `NEW`.
- A cleared or absent cross code keeps a message in a chain of one; nothing derives one from the identifiers alone, and a message in no chain joins one only by a name a live message goes by.
- A buy and a sell under one `ClOrdID` are two chains, `10:1:A1` and `10:2:A1`; a cancel reject stating no side under that `ClOrdID` cites both while both are alive - a [conflict](#a-message-citing-two-chains-is-a-conflict), standing under its own identity - and joins the one that is where only one is.
- `restating` is never refused: the walk gives the word only for an arrival under the identity the live message arrived under, which a later statement of the same instant and content has and a successor never does.
- The grid begins at the first aligned tick at or after the first source instant; it never stamps a source with a tick before that source existed.
- Errors: sorting consumes the finite source first; a source failure ends that intake, the messages read before it are walked and yielded, then the failure, once, and a sorted lifecycle walks every hour still held before it yields the failure. An item the source refused for what it states is left out with a [warning](capture.md#warnings). No error advances the walk.
- A view at a later tick is a row of its own: dated at the tick, its `uuid` differs from the live message's, so a monitor keyed by `uuid` within a time holds one row per tick and chain.
- The deduplication window is inclusive: a repeat yielded exactly the window after the latest instant the walk yielded is still a repeat. It is measured on event time, so a sorted lifecycle that walks a late message where it arrives - after the hours it yielded since - yields it again once the walk has moved further than the window past it.
- A repeat the window drops still moved the walk: a twin restated the live message with its earliest instants, and a message following the live one follows the restated one.
- A bridge's `PARENTORDERID` reads as `parentorderid`, the previous-value slot: it names no chain, so two child orders of one parent stay two, but a child stating no `OrderID(37)` is filled from it and so cites the parent's chain where that is alive. A bridge's `PARENTCLORDID` is the word `parentclordid`, a type of its own that names nothing.
- A chain's code is where the capture starts: `10:1:<ClOrdID>` when it holds the order, `10:1:<OrderID>` when it starts at the venue's first report - stable within one walk; runs join on `identifiers`.
- Unmet, and said so: a quote re-issued under a new `QuoteID(117)` starts a chain, since nothing on the wire names the one it replaces and `QuoteReqID(131)` is answered by every dealer on a request; a leg's or a party's restatement keeps no lineage - a party or an account names many orders, so the chain's party ids carry where a message names none and a restated one replaces them; a restated side is two chains, a sell short `54=5` stored `10:5:O1` and the venue's `54=2` `10:2:O1`; a cancel reject of a refused replace (`35=9`) states the refused request's `ClOrdID(11)`, which the chain reads as its newest, so the reports after it name that request their `origclordid`; and nothing merges the two chains a conflict found.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl-market --test graph
    cargo test -p yggdryl-fix --test root batch::
    cargo test -p yggdryl-fix --test root market::
    cargo test -p yggdryl-fix --test root schema::
    cargo test -p yggdryl-fix --test root enrich::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_fix.py -k lifecycle
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js node/tests/fix/catalog.test.js
    ```
