# Message

`FixMsg` is a market event with a FIX body around it: the [event](../graph/event.md) the message is, the standard header typed, what the line's own bridge row header said about the capture it was written for, the free text and a bridge's namespaced keys - each held once, as a typed fact - and the content row, every other field the message states, typed by the registry's field for it. One message is one frame, one bridge row or one document - never the line, which can carry [several of them or none](decode.md#a-line-yields-none-one-or-many-messages).

## Contract

| item | contract |
| --- | --- |
| Holders | the event the message is, read through the four [graph traits](../graph/index.md#traits) - `Element`, `Event`, `Market` and `Operation`: the identities, the codes, the instants, the place at the instant, and the market reading *derived* from the FIX fields the message stated: the price, the quantity, the bid and the ask, the state, the side, the security identifiers and the [alternate identifiers](#the-identifier-maps); `lifted() -> &FixLifted`, the FIX numbers and identifiers the message lifted out of its row, exactly as it stated them - a fact the traits answer but this holder lacks is derived, and a derived fact reaches neither the wire, the entries nor the code; `header() -> &FixHeader`, the frame: tags 8, 35, 49, 56, 34, 52, 43, 385 and the trailer 93, 89, 10 typed; `capture() -> &FixCapture`, `msgpluginid`, `msgctxid` and `msgsessionid` - what a bridge's own row header stated, read off the line's own bytes, and never what a *reader* said about the line - the `msgsesseventid` the message derives from them, and the `msgoriginator` and `conversationid` a bridge's log line names; `text() -> Option<&str>`, `Text(58)`; `metadata() -> &BTreeMap<SmolStr, SmolStr>`, what a bridge stated under its own namespaces - `TECH.CLIENTID`, `firm.acronym` - each under the key as the bridge spelled it, folded |
| Row | `as_field()` and `as_value()`: a root Struct [`Field`](../types/field.md) and the `Scalar::Serie` it declares, holding only what no holder owns - the dictionary's fields, a group as a Serie of Struct occurrences beside its `int32` counter, a component as a Struct, a key no dictionary explains under its own spelling; a [typed tag](#typed-tags) is never in it |
| Entries | `entries() -> &[FixEntry]`, the row read as a tree, derived on the first ask and dropped by every write: one entry per non-null child, each carrying the tag the dictionary resolved - `0` for a key it does not explain - the canonical name and the value as the wire spells it; a group is one entry under its counter valued the count, with one valueless entry per occurrence heading the members; a component a valueless entry heading its members; the typed facts are not entries |
| Wire | `into_bytes(separator)` and `into_text(separator)` re-emit what the message *stated*: the header tags 8, 35, 49, 56, 34, 43 and 52 - the last only where the message stated it - then the fields it lifted in tag order - 6, 11, 14, 17, 31, 32, 37, 38, 41, 44, 53, 117, 131, 151, 198, 262 and 1003 - then 58, then the entries pre-order, then the trailer 93, 89 and 10, which closes the frame whatever the body's tags are. A fact the message *derived* - the price it is about, the state it reached, the category it is filed under - is emitted nowhere. A coded fact spells as its wire code, `54=1`. `digest() -> u128` is the XXH3-128 of what `into_bytes` emits, whatever separator |
| Constructors | `FixMsg::new` links `FixRegistry::from_env()`; `FixMsg::with_registry` keeps the `Arc` it is given, lifts every typed fact out of the children that state it, settles the clocks and derives the identity, and runs no derivation - a [parse](capture.md#a-reader-is-the-whole-parse-surface) does; `FixMsg::from_row` reads a [fixed row](#a-row-is-a-message-again) back, entries included |
| Lookups | every one answers an owned `Scalar`: a typed tag its holder's fact, or nothing where the holder states none; any other key the row child it reaches |
| Writes | `set`, `set_many`, `with_value` and `remove`: a key reaching a typed fact writes its holder, a `Null` clearing it; a key reaching [the capture's own column](#a-row-is-a-message-again) - `sourceurl` - is refused, naming the column, because a message holds no fact for it; any other key [writes the row](#written-into-the-row), typed through the registry's field; every write settles the identity again; a refusal leaves the message unchanged. `set_many` and `with_value` are Rust-only |
| Settled | `currunix` is a stated `currunix`, else the [official clock](capture.md#the-official-clock-dates-the-message) standing within the codec's `official_time_delay_ms` of `SendingTime(52)` - the `TransactTime(60)` the message states, else the `TrdRegTimestamp(769)` its `TrdRegTimestampType(770)` says is about the event or a hop - else that `SendingTime`; `SendingTime` is the message's own, else a row cell or line capture reaching tag 52, else the `currunix` of the [text line](../media/index.md#plain-text) it was read out of, else the codec's `default_sending_time`, else one UTC-now read at intake, and only a stated one is a fact of the message - it goes back on the wire and into a row, while a stand-in intake supplied does neither; `creaunix` is a stated one, else `currunix`; `execunix` is the most precise execution clock the message states, else `currunix` where `FixMsg::is_execution` accepts the report and it follows nothing; the [identity](../hashing.md) - `crosscode`, `crosshashcode`, `currhashcode`, `curruuid`, `crossuuid` - is derived from what the message *states* but the standard header and trailer, less `MsgType(35)`, and never the chain it is in: the event facts, text, metadata, lifted FIX fields and canonical entry tree. A complete nonempty `msgtype`, capture `msgsessionid` and capture `msgctxid` with a present `msgseqnum` are also settled as the capture's `msgsesseventid`, the four values joined by `:` - `"<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>"`, as `8:e7256476:9effef3e6a:1094`; the sequence is canonical `u64`, an absent part removes it, and this delivery identity is excluded from the FIX content identity. An explicit nonempty `crosscode` wins; otherwise the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)` names the chain, and a message filed under `ORDR`, `QUOT` or `EXEC` - [`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches) - stores it under the name of the side it takes - `BUYS:A1` - so a buy and a sell under one `ClOrdID` are two chains; a side of `UNKN` leaves it as spelled, and so does a message of any other category, whatever side it states. `Market::sided_crosscode` is the one place the prefix is spelled, and `crosshashcode` and `crossuuid` read the stored code |
| Identity | a field is its tag and its name; a message speaks no dialect and carries no membership, so a bare tag or name resolves in the registry's [one namespace](#one-namespace) |
| Graph | `FixMsg` implements `Element`, `Event`, `Market` and `Operation` through its event: `is_after` is the instant, `finalize` settles the identity again, which re-derives every market fact from the FIX fields the message states unless a caller or a walk wrote one through the traits. `with_previous` first compares the complete `msgsesseventid`; equality forces a full content-and-graph merge instead of following, with the latest recorded observation as reference and the earliest `recdunix` and `execunix` retained. Otherwise it is `Operation::following_operation`, which records the predecessor's `curruuid` and `currunix` as `prevuuid` and `prevunix`, keeps the message's own [place](lifecycle.md#a-place-counts-one-instant), `seqnum` - the higher of its own and one past the predecessor's where that happened at the same instant or later - and carries the market facts the chain is about and the operation's: the time in force and whether it can trade where this message states neither, each alternate identifier whose [`FIX:idmap`](registry.md#a-field-names-a-message-by-its-identifiers) entry follows, written into the field it is read from, and every `metadata` key of the chain it does not state, its own values standing, so a followed message's row carries the chain's keys; it takes no account, its accounts being its fields'. `merge_with` is `Operation::merging_operation_event` under the same recording-clock rule, and `restating` the operation restatement, `graph::market::restating_operation`; import the traits to call them |
| Split | a [stream door of the parse](#a-parse-splits-what-a-message-reports) answers, beside a message reporting an execution, quoting both sides or stating a batch, the messages it reports - an execution per fill, a sided quote per side, a message per batch entry - each its own identity, its `srcuuids` its source's |
| Market data | `market_data(&self) -> Result<Vec<MarketData>>` reads this message as the [values a book folds](../graph/book.md#book-fold), one leaf per message: an `OrderEvent` for the category `ORDR`, a `QuoteEvent` for `QUOT`, an `ExecutionEvent` for an `EXEC` that reports an execution, and one leaf per `NoMDEntries(268)` occurrence - or a scoped `SnapshotEvent` - for a `W` or `X`; `into_market_data(self)` moves a direct message's held root event; `FixMarketIterator` streams sorted messages without allocating a vector for direct messages; `FixCodec::book_arrow_reader` composes that iterator with `BookIterator` and bounded Arrow output; `TryFrom<FixMsg> for MarketData` requires exactly one result. A trade `35=AE` is no leaf: its fills are the executions its parse split off; nor is a batch, whose entries are the messages its parse split off. The composed book reader skips administration, requests, acknowledgements, non-executing reports, trades, batches and two-sided quotes. Nothing a message states fails the conversion: an entry or a message that cannot stand is left out and a fact that cannot be read takes its default, each beside a [warning](capture.md#warnings), so `market_data` answers `Ok` for every message. `W` / `X` expand `NoMDEntries(268)` in nondecreasing effective time, retaining source order for ties. Every leaf carries in its metadata what its message states that no typed column reads and no identifier map of the leaf holds - [the rule](#what-a-leafs-metadata-holds), which also lifts some of it into the leaf's `altids` - and the map is part of the leaf's digest; `FixCodec::market_data` is the [sorted door](arrow.md#fix-market-books) over a whole capture. The conversion is Rust-owned and reached from Python and JavaScript |
| Serialization | inherited: `as_field().clone().into_json()` renders the row's schema, [`into_json_scalar`](../media/index.md#json) its value, `from_json_scalar_with_field` reads it back typed, ordered and canonicalized against the same root; `into_row` is the whole message as one fixed row |
| Equality | over the holders, the row and the registry - the same `Arc`, or registries holding the same fields; `Hash` over the hashcode, the row's schema and its value |
| Bindings | The message is Rust, Python and JavaScript: `FixMsg.market_data()` / `marketData()` and `FixMsg.msgcat` - Python the `MarketDataKind` member, JavaScript its name; Python `FixCodec.book_arrow_reader` and JavaScript `FixCodec.bookArrowReader` redirect into the complete Rust FIX-to-book Arrow pipeline, and `market_data`, `market_arrow_reader`, `market_data_arrow_reader` - `marketData`, `marketArrowReader`, `marketDataArrowReader` - into the sorted market door, the metadata switch being Python's `market_metadata=` and JavaScript's `{ marketMetadata }` |

## Market data

`market_data()` is the borrowed door and clones the one base event; `into_market_data()` is the consuming door and moves that root holder without cloning for a direct message. The projected generic event is finalized at that boundary, so its identity derives only from the facts the [market data row](../graph/market-data.md#arrow) carries and it passes directly through the graph Arrow round trip; the FIX message keeps its richer FIX-content identity. One message is one leaf, chosen by `msgcat` - the [`MarketDataKind`](../types/enum/marketdatakind.md) member the dictionary files the message's type under, which the leaf states again as its `marketdatakind`: `ORDR` (`10`) becomes an `OrderEvent`, `QUOT` (`14`) a `QuoteEvent`, `EXEC` (`8`) an `ExecutionEvent` only where `Event::is_execution()` says the message reports an execution, and `BOOK` (`3`) one leaf per entry of a `W` or `X`. A trade (`TRAD`, `21`) is no leaf: what it reports are the sided executions [its parse split off](#a-parse-splits-what-a-message-reports), so a fill is never stated twice; nor is a batch (`ORDB`, `QUOB`, `EXEB`, `TRDB`), whose entries its parse split off as orders, quotes, executions and trades of their own. Any other category, an execution report of no fill - its order's report in a [lifecycle](lifecycle.md), filed `ORDR` or `QUOT` - and a `BOOK` message other than `W` or `X` answers no leaf, beside a [warning](capture.md#warnings); a trade and a batch answer none in silence, their leaves being the messages their parse split off. `TryFrom<FixMsg> for MarketData` uses the consuming door and refuses unless it yields exactly one value.

`FixCodec::book_arrow_reader` admits orders, one-sided quotes, actual executions and `W`/`X`, ignoring every other record before projection - a trade, a batch and a two-sided quote among them, because the messages their parse split off are what the book reads. A source failure follows the completed book prefix and ends the reader, ignored records advance no book time, and what an admitted message states that cannot stand is passed over with a [warning](capture.md#warnings). Lifecycle enrichment remains explicit.

For `W` and `X`, the converter derives `entries()` once, reads its one `NoMDEntries(268)` group, walks its occurrences once, stably sorts the projected operations by effective instant, and returns one operation per occurrence. An empty `W` - or one stating no `NoMDEntries(268)` group at all - returns one scoped `SnapshotEvent` - built by `SnapshotEvent::snapshot(&event, scope)` - so an authoritative empty book can clear stale depth; an empty `X` states no changes and returns none. Root context - `Symbol(55)`, the book scope tags below and `MDEntryDate(272)` / `MDEntryTime(273)` - is inherited and an occurrence's stated value overrides it; any other entry tag a root states is read by no entry and rides every leaf's [metadata](#what-a-leafs-metadata-holds). Root `MDReqID(262)` is taken from its lifted message field because lifting deliberately removes it from the residual entry tree. The entry's own facts are written per occurrence - its `MDENTRYID`, `MDENTRYREFID` and `ORDERID` alternate identifiers replaced under their keys, its book control built afresh - so a missing `MDEntryID`, reference or coordinate cannot leak from one occurrence into the next while unrelated caller identifiers survive. The entry type decides the concrete operation:

| `MDEntryType(269)` | Operation | Side |
| --- | --- | --- |
| `0` bid | an `OrderEvent` where that occurrence states `OrderID(37)`, otherwise a `QuoteEvent` | `BUYS` |
| `1` offer | an `OrderEvent` where that occurrence states `OrderID(37)`, otherwise a `QuoteEvent` | `SELL` |
| `2` trade | an `ExecutionEvent` | the occurrence's `Side(54)`, otherwise `UNKN` |

`MDEntryPx(270)` and `MDEntrySize(271)` are read from the already validated typed group values into exact [`Decimal`](../types/numeric/decimal.md#decimal) values, and an absent one states none - an entry stating no price rests at its side's [unpriced level](../graph/book.md#limits); a value outside that range is never a fabricated zero: a new or snapshot entry, which would rest at it, is excluded, and an update keeps the live entry's price as one stating none does. `MDEntryDate(272)` and `MDEntryTime(273)` use the same typed path rather than reparsing rendered text: an `X` occurrence is dated at that exact nanosecond, while a `W` group remains atomic at the message's `currunix` and retains the entry clock as `creaunix`, or as `execunix` for a trade. A missing date uses the UTC day containing the message timestamp. What an entry states never fails the message: it is read at the smallest part that can stand, and what is passed over is [warned about](capture.md#warnings), naming the group index and tag. An entry that cannot stand is excluded and the others stand - an `MDEntryType(269)` that is absent or none of `0`, `1` and `2`, an `X` entry stating no `MDUpdateAction(279)` or one outside `0` through `5`, an anonymous incremental update naming no entry, and a new or snapshot entry whose `MDEntryPx(270)` is no exact decimal. A fact that does not decide the entry takes its default: a size or an FX part that is no exact decimal is null, an entry clock naming no instant is the message's, and an unreadable `Side(54)` on a trade entry is `UNKN`. A `W` or `X` stating several `NoMDEntries(268)` groups answers no leaf, since which one is the book's cannot be chosen. The update action is retained as the operation's book control - `OperationEvent::book()`, a `BookRef` whose `action` is the `MdUpdateAction` - and determines its lifecycle state:

| Message/action | Reading | State and depth effect |
| --- | --- | --- |
| `W` | `MdUpdateAction::Snapshot` | `New`; before the group is applied, a [book](../graph/book.md#books) clears only the same scope on both sides; zero entries is one control and no invented depth |
| `X`, `0` | new | `New`, live |
| `X`, `1` | change | `Replaced`, live |
| `X`, `2` | delete | `Canceled`, terminal |
| `X`, `3` | delete through | `Canceled`, terminal; for a bid or offer, removes same-symbol, same-scope live entries through a required positive, in-range one-based `MDEntryPositionNo(290)` |
| `X`, `4` | delete from | `Canceled`, terminal; for a bid or offer, removes same-symbol, same-scope live entries from that required position |
| `X`, `5` | overlay | `Replaced`, live |

An execution leaf - a trade entry, or an `EXEC` message - is one fill, complete in itself: it reads `FILLED` whatever its report's state, and only a trade entry a book message deletes reads `CANCELED`.

The book scope - `BookRef::scope`, what `OperationEvent::scope()` answers - is deterministic: `Symbol=` the entry's `Symbol(55)`, else the message's ticker, else the instrument's `ISIN`, else its currency pair, else the book it keys to, [`Market::book_crosscode`](../graph/market.md#sides-and-cross-codes); then, where stated, `MDBookType(1021)`, `MDSubBookType(1173)`, `MDFeedType(1022)`, `MDStreamID(1500)`, `MarketID(1301)`, `MarketSegmentID(1300)`, `MDReqID(262)` and `MarketDepth(264)`, in that order. `%`, `|` and `=` in external values are percent-escaped before those separators are written, so distinct tuples cannot concatenate alike. The occurrence inherits those facts from the message root and may replace them. Its cross code is that complete scope plus `MDEntryID(278)`, or the same qualified ID named by `MDEntryRefID(280)`, stored under the entry's side as every order's and quote's is - `BUYS:Symbol=AAPL|MDEntryID=B1`. Without either ID it is the scope, `MDEntryType`, and the stated position and/or price level; a new or snapshot entry with neither coordinate adds its price, while a change, delete or overlay without any stable ID/position/level names no entry and is excluded, with a warning. IDs reused by two feeds or symbols therefore never collide, and an anonymous price change retains the position/level identity it updates. Two instruments stating neither ticker nor identifier under one market and classification share a scope.

The book control retains the effective action (`Snapshot` for `W`, the wire code for `X`), the scope, `MDEntryPositionNo(290)` as `position`, and `MDEntryPx(270)` and `MDEntrySize(271)` as the entry stated them - `entry_px` and `entry_size`, stated only where the occurrence stated them, so presence is distinct from a numeric zero: change and overlay actions inherit either value the occurrence omitted from the matched live entry, and the book refuses an omitted value where no predecessor exists - its walk leaves that group out with a warning ([book fold](../graph/book.md#book-fold)). The book control is the walk's and no column of the market data row. The entry's own and referenced identifiers and the order it names are alternate identifiers - `MDENTRYID` and `MDENTRYREFID` (`graph::book::ENTRY_ID`, `ENTRY_REF_ID`) and `ORDERID`; `MDPriceLevel(1023)` takes part in the cross code of an entry naming no identifier and in nothing else, and `RptSeq(83)` and the `ApplID` sequences are no fact of the operation. A stated `MDEntryRefID` is resolved before the incoming `MDEntryID`; two distinct live entries at those identities are ambiguous and refused. This gives [the book](../graph/book.md#book-fold) its matching, ordering and scope facts without a second FIX schema. Trade occurrences become executions beside the book and never decrement resting order or quote depth.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Market, MarketKind};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry, MarketDataKind};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // A new order is one order event, its facts the message's own.
    let order = codec
        .parse_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30|11=A|55=IBM|54=1|38=10|44=101.5|15=USD|10=0|")?
        .next()
        .expect("one frame")?;
    assert_eq!(order.msgcat(), MarketDataKind::Order);
    let operations = order.market_data()?;
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].kind(), MarketKind::OrderEvent);
    assert_eq!(operations[0].marketdatakind(), MarketDataKind::Order);
    let leaf = operations[0].as_order_event().expect("an order event");
    assert_eq!(leaf.get_price(), order.get_price());
    // The chain is the ClOrdID under the side the order takes.
    assert_eq!(leaf.get_crosscode(), "BUYS:A");
    assert_eq!(leaf.get_crossuuid(), order.get_crossuuid());

    // A full snapshot is one operation per NoMDEntries(268) occurrence.
    let book = codec
        .parse_line(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|")?
        .next()
        .expect("one frame")?;
    let entries = book.market_data()?;
    let kinds: Vec<MarketKind> = entries.iter().map(|value| value.kind()).collect();
    assert_eq!(kinds, [MarketKind::QuoteEvent, MarketKind::QuoteEvent]);
    assert_eq!(entries[0].get_crosscode(), "BUYS:Symbol=AAPL|MDEntryID=B1");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import MarketDataKind, graph
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A new order is one order event, its facts the message's own.
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30|11=A|55=IBM|54=1|38=10|44=101.5|15=USD|10=0|")
    assert order.msgcat == MarketDataKind.ORDR
    [operation] = order.market_data()
    assert isinstance(operation, graph.MarketData) and operation.kind == "order_event"
    assert operation.marketdatakind == MarketDataKind.ORDR
    leaf = operation.as_order_event()
    assert leaf is not None
    assert leaf.price == order.price
    # The chain is the ClOrdID under the side the order takes.
    assert leaf.crosscode == "BUYS:A"
    assert leaf.crossuuid == order.crossuuid

    # A full snapshot is one operation per NoMDEntries(268) occurrence.
    book = codec.parse_fix_line(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|"
    )
    entries = book.market_data()
    assert [value.kind for value in entries] == ["quote_event", "quote_event"]
    assert entries[0].crosscode == "BUYS:Symbol=AAPL|MDEntryID=B1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, graph } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A new order is one order event, its facts the message's own.
    const order = codec.parseFixLine(
      Buffer.from('8=FIX.4.4|35=D|52=20240102-10:15:30|11=A|55=IBM|54=1|38=10|44=101.5|15=USD|10=0|'),
    )
    assert.equal(order.msgcat, 'ORDR')
    const [operation] = order.marketData()
    assert.ok(operation instanceof graph.MarketData)
    assert.equal(operation.kind, 'order_event')
    assert.equal(operation.marketdatakind, 'ORDR')
    const leaf = operation.asOrderEvent()
    assert.equal(leaf.price, order.price)
    // The chain is the ClOrdID under the side the order takes.
    assert.equal(leaf.crosscode, 'BUYS:A')
    assert.equal(leaf.crossuuid, order.crossuuid)

    // A full snapshot is one operation per NoMDEntries(268) occurrence.
    const book = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
    ))
    const entries = book.marketData()
    assert.deepEqual(entries.map((value) => value.kind), ['quote_event', 'quote_event'])
    assert.equal(entries[0].crosscode, 'BUYS:Symbol=AAPL|MDEntryID=B1')
    ```

### A parse splits what a message reports

A message its type files under `EXEC` - an execution report - that reports no fill is its order's report, `ORDR`, or its quote's, `QUOT`, where it names a `QuoteID(117)`, from its parse, so an acknowledgement and a cancel walk in their order's [lifecycle](lifecycle.md), which chains within one category; one that reports a fill states that execution until a stream door splits it. A message may report more than itself: an execution report stating a fill reports the fill, a trade capture report reports each side that traded, a quote stating a bid and an offer quotes two sides, and a batch - an order list, a mass order, a cross, a mass quote, a match report - states many orders, quotes or trades at once. Each of those is an event of its own, with its own chain, so every stream door of the parse - `parse_line`, `parse_lines`, `parse_text_line`, `parse_text_lines` and `parse_text_arrow_reader` - answers, right after the message, the messages it reports. The split runs once, there, so a fill, a quoted side or a batch entry is one message wherever a capture is read, and the [book](../graph/book.md#book-fold) and the [lifecycle](lifecycle.md) read each once. The one-body doors - `parse_fix_line`, `parse_fixml_line`, `parse_ullink_line` and `parse_pairs` - answer the message as stated and split nothing.

| Message | Reports | Split off |
| --- | --- | --- |
| an order's, a quote's or an execution report stating an execution - `ExecType(150)` `F`, or FIX 4.2's `1` or `2`, else a state that reports one | one fill | one execution: the report's content under the category `EXEC`, its cross code its own - the `ExecID(17)` as given, else `TradeID=<TradeID(1003)>`, else the report's code and content digest. The execution report is then its order's report, `ORDR` - `QUOT` where it names a `QuoteID(117)` - as a report of no fill is from its parse, so the fill is stated by the execution alone |
| a TradeCaptureReport `35=AE` whose `TradeReportTransType(487)` is absent or New and whose `ExecType(150)` is absent or execution-like | each side that traded | one execution per `NoSides(552)` occurrence: the trade's content with that occurrence alone in its group and the side's facts at the root where an `ExecutionReport` states them - `Side(54)`, `SideExecID(1427)` as `ExecID(17)`, `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `SecondaryOrderID(198)`, `SecondaryClOrdID(526)`, `SideLastQty(1009)` as `LastQty(32)`, `SideAvgPx(1852)` as `AvgPx(6)`, `SideCurrency(1154)` as `Currency(15)`. Its cross code is the side's order - its `OrderID`, `ClOrdID` or `OrigClOrdID`, else the trade's - and the first stable identifier it states: `SideExecID`, `SideTradeID(1506)`, `SideTradeReportID(1005)`, `OrderID`, `ClOrdID`, else the occurrence's own content digest; the group index never enters it. An occurrence stating no `Side(54)`, or one no side reads, splits off an execution of side `UNKN` with a [warning](capture.md#warnings) - an unreadable one also kept on the trade as an [anomaly](#anomalies) - so a fill is never lost; an occurrence whose facts make no execution splits off none and is an anomaly of the trade |
| a quote stating a bid and an offer and no `Side(54)` | two sides | a `BUYS` quote and a `SELL` quote, the source's content stating that `Side(54)`, so each reads its price, quantity and FX parts off its own side's `BidPx(132)`/`BidSize(134)`/`BidSpotRate(188)`/`BidForwardPoints(189)` or `OfferPx(133)`/`OfferSize(135)`/`OfferSpotRate(190)`/`OfferForwardPoints(191)`; both keep the whole [bid and ask](../graph/market.md#bid-and-ask) the source stated |
| a batch: a type filed under `ORDB`, `QUOB`, `EXEB` or `TRDB` - [`MarketDataKind::is_batch`](../types/enum/marketdatakind.md#sided-kinds-and-batches) | each entry it states | one message per entry of its entry group ([below](#a-batch-splits-per-entry)), filed under the batch's single category - [`MarketDataKind::item`](../types/enum/marketdatakind.md#sided-kinds-and-batches): `ORDR`, `QUOT`, `EXEC` or `TRAD` - the batch's content without that group and the entry's own members at its root, a component's members read through and a nested group kept whole; each is then split as a message of its category is, so a two-sided mass-quote entry is a `BUYS` and a `SELL` quote |

Every message split off has an identity of its own, stands after its source at a later [place](lifecycle.md#a-place-counts-one-instant) of their instant, reads `FILLED` where it is an execution and otherwise the state its content states - its source's, for a quoted side - and names as its `srcuuids` its source's `curruuid` - the identity its source was placed under - beside its source's own sources. The source keeps what it states, its own state included - a partial fill's report stays `PARTIALLY_FILLED` while its execution reads `FILLED`. An order, a quote or an execution split off stores its cross code under the side it states, `BUYS:E1` - the `ExecID(17)` as given after the side -, as [every sided element](../graph/market.md#sides-and-cross-codes) does; a trade split off a match report keeps its code as given. A message split off is an ordinary message: it lands as a row of its own in the [fixed row](capture.md#the-columns-are-the-folded-names), and the lifecycle walks it under its own cross code and market data kind - a batch entry naming an order joins that order's chain, and an execution, being of the `EXEC` kind, never restates, follows or ends its order, so a fill cannot end it.



=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, Market};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // A partial fill: the order's report, then the fill it reports.
    let line: &[u8] = b"8=FIX.4.4|35=8|52=20260921-10:00:01|37=O1|11=C1|17=E1|150=F|39=1|55=AAPL|54=1|31=10|32=5|38=10|14=5|151=5|10=0|";
    let messages: Vec<FixMsg> = codec.parse_line(line)?.collect::<yggdryl::Result<_>>()?;
    let [report, fill] = messages.as_slice() else { panic!("two messages") };
    assert_eq!((report.msgcat(), report.get_state().as_str()), (MarketDataKind::Order, "PARTIALLY_FILLED"));
    assert_eq!((fill.msgcat(), fill.get_state().as_str()), (MarketDataKind::Execution, "FILLED"));
    assert_eq!(report.get_crosscode(), "BUYS:O1");
    assert_eq!(fill.get_crosscode(), "BUYS:E1");
    assert_eq!(fill.get_srcuuids(), [report.get_curruuid()]);
    // The one-body door answers the message as stated, split nothing.
    assert_eq!(codec.parse_fix_line(line)?.msgcat(), MarketDataKind::Execution);

    // A quote stating both sides is two sided quotes beside it.
    let quote = b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|132=100.5|133=101|134=500|135=700|15=USD|10=0|";
    let quotes: Vec<FixMsg> = codec.parse_line(quote)?.collect::<yggdryl::Result<_>>()?;
    let codes: Vec<&str> = quotes.iter().map(|held| held.get_crosscode()).collect();
    assert_eq!(codes, ["Q1", "BUYS:Q1", "SELL:Q1"]);
    assert_eq!(quotes[1].get_price(), quotes[1].get_bidpx());
    assert_eq!(quotes[2].get_price(), quotes[2].get_askpx());
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import MarketDataKind, State
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A partial fill: the order's report, then the fill it reports.
    line = b"8=FIX.4.4|35=8|52=20260921-10:00:01|37=O1|11=C1|17=E1|150=F|39=1|55=AAPL|54=1|31=10|32=5|38=10|14=5|151=5|10=0|"
    report, fill = codec.parse_line(line)
    assert (report.msgcat, report.state) == (MarketDataKind.ORDR, State.PARTIALLY_FILLED)
    assert (fill.msgcat, fill.state) == (MarketDataKind.EXEC, State.FILLED)
    assert report.crosscode == "BUYS:O1"
    assert fill.crosscode == "BUYS:E1"
    assert fill.srcuuids == [report.curruuid]
    # The one-body door answers the message as stated, split nothing.
    assert codec.parse_fix_line(line).msgcat == MarketDataKind.EXEC

    # A quote stating both sides is two sided quotes beside it.
    quote = b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|132=100.5|133=101|134=500|135=700|15=USD|10=0|"
    quotes = list(codec.parse_line(quote))
    assert [held.crosscode for held in quotes] == ["Q1", "BUYS:Q1", "SELL:Q1"]
    assert quotes[1].price == quotes[1].bidpx
    assert quotes[2].price == quotes[2].askpx
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    // A partial fill: the order's report, then the fill it reports.
    const line = Buffer.from('8=FIX.4.4|35=8|52=20260921-10:00:01|37=O1|11=C1|17=E1|150=F|39=1|55=AAPL|54=1|31=10|32=5|38=10|14=5|151=5|10=0|')
    const [report, fill] = [...codec.parseLine(line)]
    assert.deepEqual([report.msgcat, report.state], ['ORDR', 'PARTIALLY_FILLED'])
    assert.deepEqual([fill.msgcat, fill.state], ['EXEC', 'FILLED'])
    assert.equal(report.crosscode, 'BUYS:O1')
    assert.equal(fill.crosscode, 'BUYS:E1')
    assert.deepEqual(fill.srcuuids, [report.curruuid])
    // The one-body door answers the message as stated, split nothing.
    assert.equal(codec.parseFixLine(line).msgcat, 'EXEC')

    // A quote stating both sides is two sided quotes beside it.
    const quote = Buffer.from('8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|132=100.5|133=101|134=500|135=700|15=USD|10=0|')
    const quotes = [...codec.parseLine(quote)]
    assert.deepEqual(quotes.map((held) => held.crosscode), ['Q1', 'BUYS:Q1', 'SELL:Q1'])
    assert.equal(quotes[1].price, quotes[1].bidpx)
    assert.equal(quotes[2].price, quotes[2].askpx)
    ```

#### A batch splits per entry

A batch states its entries in one repeating group, read by its `MsgType(35)`; a mass quote's are the quotes of each quote set, each carrying its set's members beside its own. An entry is chained by the order it names - its `OrderID(37)`, `ClOrdID(11)` or `OrigClOrdID(41)`, as a single order message names it, so it joins that order's chain - else by `QuoteSetID=<set>|QuoteEntryID=<entry>` (`QuoteEntryID=<entry>` outside a set), else `OrderEntryID=<OrderEntryID(2430)>`, else `<batch code or MsgType>|<counter>:<index>`. A batch type stating its entries in no group splits into nothing, and the batch itself stays a message of its own.

| `MsgType(35)` | Category | Entries |
| --- | --- | --- |
| `E` NewOrderList, `N` ListStatus | `ORDB` | one order per `NoOrders(73)` occurrence |
| `DJ` MassOrder, `DK` MassOrderAck | `ORDB` | one order per `NoOrderEntries(2428)` occurrence |
| `s` NewOrderCross, `t` CrossOrderCancelReplaceRequest, `u` CrossOrderCancelRequest | `ORDB` | one order per `NoSides(552)` occurrence |
| `r` OrderMassCancelReport, `BZ` OrderMassActionReport | `ORDB` | one order per `NoAffectedOrders(534)` occurrence, `AffectedOrderID(535)` read as `OrderID(37)`, `AffectedOrigClOrdID(1824)` as `OrigClOrdID(41)` and `AffectedSecondaryOrderID(536)` as `SecondaryOrderID(198)` |
| `i` MassQuote, `b` MassQuoteAck | `QUOB` | one quote per `NoQuoteEntries(295)` occurrence of each `NoQuoteSets(296)` occurrence |
| `k` BidRequest, `l` BidResponse | `QUOB` | one quote per `NoBidComponents(420)` occurrence |
| `m` ListStrikePrice | `QUOB` | one quote per `NoStrikes(428)` occurrence |
| `DC` TradeMatchReport | `TRDB` | one trade per `NoInstrmtMatchSides(1889)` occurrence |
| `K`, `L`, `M`, `q`, `AF`, `CA`, `DS`, `DT` | `ORDB` | none: a list's cancel, execute or status request, a mass cancel, status or action request, and a cross request and its acknowledgement state no entry group |
| `DD` TradeMatchReportAck | `TRDB` | none |

An order list with a buy and a sell is the list, then one order per entry, each chained by its own `ClOrdID` on its side.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Market};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind, Side};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    let line: &[u8] = b"8=FIX.4.4|35=E|52=20260921-10:00:00|66=L1|394=3|68=2|73=2|11=C1|67=1|55=AAPL|54=1|38=5|40=2|44=100.5|11=C2|67=2|55=MSFT|54=2|38=7|40=2|44=300.25|10=0|";
    let messages: Vec<FixMsg> = codec.parse_line(line)?.collect::<yggdryl::Result<_>>()?;
    let [list, buy, sell] = messages.as_slice() else { panic!("the list and its two orders") };
    assert_eq!(list.msgcat(), MarketDataKind::OrderBatch);
    assert_eq!((buy.msgcat(), sell.msgcat()), (MarketDataKind::Order, MarketDataKind::Order));
    assert_eq!((buy.get_side(), sell.get_side()), (Side::Buy, Side::Sell));
    assert_eq!((buy.get_crosscode(), sell.get_crosscode()), ("BUYS:C1", "SELL:C2"));
    assert_eq!((buy.get_ticker(), sell.get_ticker()), (Some("AAPL"), Some("MSFT")));
    assert!(buy.get_srcuuids().contains(&list.get_curruuid()));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import MarketDataKind, Side
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    line = b"8=FIX.4.4|35=E|52=20260921-10:00:00|66=L1|394=3|68=2|73=2|11=C1|67=1|55=AAPL|54=1|38=5|40=2|44=100.5|11=C2|67=2|55=MSFT|54=2|38=7|40=2|44=300.25|10=0|"
    order_list, buy, sell = codec.parse_line(line)
    assert order_list.msgcat == MarketDataKind.ORDB
    assert buy.msgcat == sell.msgcat == MarketDataKind.ORDR
    assert (buy.side, sell.side) == (Side.BUYS, Side.SELL)
    assert (buy.crosscode, sell.crosscode) == ("BUYS:C1", "SELL:C2")
    assert order_list.curruuid in buy.srcuuids
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))

    const line = Buffer.from('8=FIX.4.4|35=E|52=20260921-10:00:00|66=L1|394=3|68=2|73=2|11=C1|67=1|55=AAPL|54=1|38=5|40=2|44=100.5|11=C2|67=2|55=MSFT|54=2|38=7|40=2|44=300.25|10=0|')
    const [orderList, buy, sell] = [...codec.parseLine(line)]
    assert.equal(orderList.msgcat, 'ORDB')
    assert.deepEqual([buy.msgcat, sell.msgcat], ['ORDR', 'ORDR'])
    assert.deepEqual([buy.side, sell.side], ['BUYS', 'SELL'])
    assert.deepEqual([buy.crosscode, sell.crosscode], ['BUYS:C1', 'SELL:C2'])
    assert.ok(buy.srcuuids.includes(orderList.curruuid))
    ```

### What a leaf's metadata holds

Every leaf carries, in its [`Market::get_metadata`](../graph/market.md#contract), what its message states that no typed column reads and no identifier map of the leaf holds, each value the canonical text it spells - a decimal at the scale it is stored at: the bridge's namespaced keys as they are (`tech.clientid`), then every other top-level field under its folded name (`ordtype`, `execinst`), and a group, a component or a map as one key under its folded name holding JSON - a group the array of one object per occurrence, a component or a map one object, each member under its folded name and nested groups and components recursing, every leaf the canonical text a root field spells, so no value is a JSON number and the keys are in name order: `miscfees` holds `[{"miscfeeamt":"1.500000000000000000","miscfeecurr":"EUR","miscfeetype":"4"}]`. A key no dictionary resolved is a top-level field like any other, `9999` under its own spelling. Left out are the envelope a message's code leaves out, so two hops of one message are one leaf; every tag a typed column reads; an alternate identifier's source field; what the leaf's [identifier maps](#the-identifier-maps) hold, below; the fields read by name - `eventtimestamp`, the detailed CFI, `BidCurrency` and `AskCurrency`, the security-type names; null members and a group's counter; and the group a message expands into its leaves, `NoMDEntries(268)` for `W` and `X`, `NoSides(552)` for an execution a trade split off. A book entry and a trade side add their own occurrence's scalar members keyed bare, each leading a message field of the same name, a nested member of that occurrence as JSON under its bare name, and never a sibling's.

What the leaf's identifier maps hold is no metadata: each `Parties(453)` or `RootParties(1116)` occurrence whose `PartyID(448)` its `accountids` hold under its role's key, the `Account(1)` they hold as `ACCOUNT`, and each `RegulatoryTradeIDGrp` or `SideRegulatoryTradeIDGrp` occurrence whose identifier its `altids` hold under its type's key (`REGTRADEID`, `TVTIC`, ...). A book entry's own `Parties(453)` are its leaf's accounts, leading the message's, as a trade side's are its execution's, and leave the entry leaf's metadata. What no map holds stays: a second party of one role, whose occurrence alone stays in `parties`, and a value a map refuses. An empty `W` snapshot's control holds no maps, so its metadata keeps its parties and its account.

A scalar of the metadata whose key ends with one of the identifiers its message's type declares under [`FIX:identifiers`](registry.md#component-identifiers) - letters and digits compared, whatever the case - is lifted into the leaf's `altids` under its own key as stated, `IdMap` upper-casing it, and leaves the metadata: an execution report's `marketorderid` and `omsdealerorderid`, the two losing aliases of `OrderID(37)` in a bridge's capture, become `MARKETORDERID` and `OMSDEALERORDERID`; `RefOrderID(1080)` becomes `REFORDERID` and `SecondaryClOrdID(526)` `SECONDARYCLORDID`; a bridge's `venue.x.parentorderid` becomes `VENUE.X.PARENTORDERID` on an execution report and stays on a new order, which declares no `orderid`. A trade side's or a book entry's own member is checked against the identifiers its component declares, so a trade report's `TradeReportID(571)` becomes `TRADEREPORTID` on the executions its sides split into. A key fills only where the leaf holds it free or already with the same value; a key over 32 bytes, a value over 64 bytes or not ASCII, and a key holding another value stay in the metadata. So a leaf's `altids` can hold more than its message's, which are what its fields state.

The map is computed where the leaf is built and never stored on the `FixMsg`, so it feeds the leaf's digest, with what it lifts, and never the message's code. `market_data` and `into_market_data` always carry it; a codec's `with_market_metadata(false)` turns it off for the codec's own doors - `market_data`, `market_arrow_reader`, `market_data_arrow_reader`, `book_arrow_reader` - which leaves the map empty, lifts nothing, and moves the identity of every leaf whose message states such a field. The parties and the `Account(1)` stay the leaf's accounts either way, since its fields state them.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Market, Operation};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // OrdType(40) and ExecInst(18) are no typed column, so they are the leaf's
    // metadata. The party is its account, and RefOrderID(1080), an identifier
    // a new order declares, is lifted into its altids; the rest is typed.
    let line: &[u8] = b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|1080=R-1|453=1|448=TRADER1|447=D|452=11|10=0|";
    let message = codec.parse_line(line)?.next().expect("one frame")?;
    let leaves = message.market_data()?;
    let metadata = leaves[0].get_metadata();
    let keys: Vec<&str> = metadata.keys().map(|key| key.as_str()).collect();
    assert_eq!(keys, ["execinst", "ordtype"]);
    assert_eq!(metadata.get("ordtype").map(|value| value.as_str()), Some("2"));
    let order = leaves[0].as_order_event().expect("an order event");
    assert_eq!(order.get_accountids().get("ORDERORIGINATIONTRADER"), Some("TRADER1"));
    assert_eq!(order.get_altids().get("REFORDERID"), Some("R-1"));

    // The codec's switch leaves the map empty and lifts nothing, and the leaf's
    // identity says so.
    let bare = codec.clone().with_market_metadata(false);
    assert!(!bare.market_metadata());
    let plain = bare
        .market_data([bare.parse_line(line)?.next().expect("one frame")?])
        .next()
        .expect("one leaf")?;
    assert!(plain.get_metadata().is_empty());
    let unlifted = plain.as_order_event().expect("an order event");
    assert_eq!(unlifted.get_accountids(), order.get_accountids(), "its fields state its parties");
    assert!(unlifted.get_altids().get("REFORDERID").is_none());
    assert_ne!(plain.get_curruuid(), leaves[0].get_curruuid());
    assert_eq!(plain.get_crosscode(), leaves[0].get_crosscode(), "and never its chain");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)

    # OrdType(40) and ExecInst(18) are no typed column, so they are the leaf's
    # metadata. The party is its account, and RefOrderID(1080), an identifier
    # a new order declares, is lifted into its altids; the rest is typed.
    line = b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|1080=R-1|453=1|448=TRADER1|447=D|452=11|10=0|"
    [leaf] = codec.parse_fix_line(line).market_data()
    order = leaf.as_order_event()
    assert order is not None
    assert order.metadata == {"execinst": "G", "ordtype": "2"}
    assert order.accountids == {"ORDERORIGINATIONTRADER": "TRADER1"}
    assert order.altids == {"CLORDID": "C1", "REFORDERID": "R-1"}

    # The codec's switch leaves the map empty and lifts nothing, and the leaf's
    # identity says so.
    bare = FixCodec(registry, market_metadata=False)
    assert not bare.market_metadata
    [plain] = bare.market_data([bare.parse_fix_line(line)])
    held = plain.as_order_event()
    assert held is not None and held.metadata == {}
    assert held.accountids == order.accountids, "its fields state its parties"
    assert held.altids == {"CLORDID": "C1"}
    assert plain.curruuid != leaf.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)

    // OrdType(40) and ExecInst(18) are no typed column, so they are the leaf's
    // metadata. The party is its account, and RefOrderID(1080), an identifier
    // a new order declares, is lifted into its altids; the rest is typed.
    const line = Buffer.from(
      '8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|1080=R-1|453=1|448=TRADER1|447=D|452=11|10=0|',
    )
    const [leaf] = codec.parseFixLine(line).marketData()
    const order = leaf.asOrderEvent()
    assert.deepEqual(Object.keys(order.metadata), ['execinst', 'ordtype'])
    assert.equal(order.metadata.ordtype, '2')
    assert.deepEqual(order.accountids, { ORDERORIGINATIONTRADER: 'TRADER1' })
    assert.deepEqual(order.altids, { CLORDID: 'C1', REFORDERID: 'R-1' })

    // The codec's switch leaves the map empty and lifts nothing, and the leaf's
    // identity says so.
    const bare = new fix.FixCodec(registry, { marketMetadata: false })
    assert.equal(bare.marketMetadata, false)
    const [plain] = [...bare.marketData([bare.parseFixLine(line)])]
    const unlifted = plain.asOrderEvent()
    assert.equal(Object.keys(unlifted.metadata).length, 0)
    assert.deepEqual(unlifted.accountids, order.accountids, 'its fields state its parties')
    assert.deepEqual(unlifted.altids, { CLORDID: 'C1' })
    assert.notEqual(plain.curruuid, leaf.curruuid)
    ```

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, Market};
    use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, FieldPath, StructType, from_json_scalar_with_field, into_json_scalar};

    let mut msgtype = DataType::utf8().nullable_field("MsgType");
    msgtype.as_fix_mut().set_tag(35)?;
    let mut side = DataType::utf8().nullable_field("Side");
    side.as_fix_mut().set_tag(54)?;
    let mut symbol = DataType::utf8().required_field("Symbol");
    symbol.as_fix_mut().set_tag(55)?;
    symbol.as_fix_mut().set_names(["Ticker"])?;
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38)?;
    let mut party_id = DataType::utf8().nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448)?;
    let mut count = DataType::Int32.required_field("NoPartyIDs");
    count.as_fix_mut().set_tag(453)?;
    let party = DataType::from(StructType::from_fields([party_id.clone()])?).required_field("Party");
    let mut parties = DataType::serie(party.clone()).nullable_field("Parties");
    parties.as_fix_mut().set_counter(453)?;
    parties.as_fix_mut().set_component("Party")?;
    let mut registry = FixRegistry::from_fields([msgtype.clone(), side.clone(), symbol.clone(), qty.clone(), count.clone(), party_id])?;
    registry.insert(party)?;
    registry.insert(parties.clone())?;
    let registry = Arc::new(registry);

    // The root carries two typed tags and a tag no dictionary explains.
    let root = DataType::from(StructType::from_fields([msgtype, side, qty, symbol, count, parties, DataType::utf8().nullable_field("9999")])?)
        .required_field("NewOrderSingle");
    let value = Scalar::from_struct([
        ("MsgType", Scalar::from("D")),
        ("Side", Scalar::from("1")),
        ("Symbol", Scalar::from("AAPL")),
        ("OrderQty", Scalar::from(100_i64)),
        ("NoPartyIDs", Scalar::from(1_i32)),
        ("Parties", Scalar::from_sequence([
            Scalar::from_struct([("PartyID", Scalar::from("BROKER"))])?,
        ])),
        ("9999", Scalar::from("custom")),
    ])?;
    let msg = FixMsg::with_registry(Arc::clone(&registry), root, value)?;

    // The typed facts left the row for their holders: the type is the
    // header's and the quantity the message lifts. The side is an ordinary
    // child, so it stays in the row beside the four others.
    assert_eq!(msg.header().msgtype(), "D");
    assert_eq!(msg.get_side().as_str(), "BUYS");
    let children: Vec<&str> = msg.as_field().fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(children, ["Side", "Symbol", "NoPartyIDs", "Parties", "9999"]);
    // A lookup answers the holder for a typed tag and the row for the rest.
    // `OrderQty` is the quantity the message lifts, so it answers exact;
    // `Side(54)` is a row child, so it answers the code the row holds and
    // `get_side` reads the side off it.
    let hundred = Scalar::from(yggdryl::Decimal::from_int(100));
    assert_eq!(msg.by_tag(35)?, Scalar::from("D"));
    assert_eq!(msg.by_tag(54)?, Scalar::from("1"));
    assert_eq!(msg.by_tag(38)?, hundred);
    assert_eq!(msg.get_quantity(), Some(yggdryl::Decimal::from_int(100)));
    assert_eq!(msg.by_name("ticker")?, Scalar::from("AAPL"));
    assert_eq!(msg.by_path(&FieldPath::from_str("Parties[0].PartyID")?)?, Scalar::from("BROKER"));
    assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"), "an unknown tag is retained");
    assert_eq!(msg.get(55), msg.get_by_tag(55));
    assert!(msg.value("Parties.PartyID").is_err(), "a group member needs its index");

    // The identity is settled from what the message states.
    assert_ne!(msg.get_currhashcode(), 0);
    assert_eq!(msg.get_curruuid(), msg.time_uuid()?);
    assert_eq!(msg.get_currunix(), msg.header().sendingtime(), "undated, so the sending clock stands in");
    assert_eq!(msg.get_crosscode(), "", "no OrderID or ClOrdID names a chain");
    assert_eq!(msg.get_crossuuid(), msg.get_curruuid(), "so the message is a chain of one");

    // An identifier is the tag and the name together, under the one fold, and exact.
    let id = registry.field_by_tag(38)?.as_fix().id()?.expect("a tagged field");
    assert_eq!(id, yggdryl::FixId::of(38, "order_qty")?);
    assert_eq!(msg.by_id(id)?, hundred);
    assert!(msg.get_by_id(yggdryl::FixId::of(38, "Quantity")?).is_none(), "another name is another field");

    // The row serializes through the paths every field and value share, and
    // a message rebuilt from them holds the same content; its typed facts
    // are its own to state again.
    let root = msg.as_field().clone();
    let schema = root.clone().into_json()?;
    assert!(schema.contains("\"FIX:tag\":\"55\""), "{schema}");
    let text = into_json_scalar(msg.as_value())?;
    let read = from_json_scalar_with_field(&text, &root)?;
    assert_eq!(&read, msg.as_value());
    let again = FixMsg::with_registry(registry, root, read)?;
    assert_eq!(again.entries(), msg.entries());
    assert_eq!(again.header().msgtype(), "", "the type was the header's, not the row's");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    import pytest

    import yggdryl

    from yggdryl import DataType, Field
    from yggdryl.fix import FixMsg, FixRegistry

    msgtype = Field("MsgType", "utf8")
    msgtype.fix.tag = 35
    side = Field("Side", "utf8")
    side.fix.tag = 54
    symbol = Field("Symbol", "utf8", nullable=False)
    symbol.fix.tag = 55
    symbol.fix.names = ["Ticker"]
    qty = Field("OrderQty", "int64", nullable=False)
    qty.fix.tag = 38
    party_id = Field("PartyID", "utf8")
    party_id.fix.tag = 448
    count = Field("NoPartyIDs", "int32", nullable=False)
    count.fix.tag = 453
    party = Field("Party", DataType.from_fields([party_id]), nullable=False)
    parties = yggdryl.serie("Parties", party)
    parties.fix.counter = 453
    parties.fix.component = "Party"
    registry = FixRegistry.from_fields([msgtype, side, symbol, qty, count, party_id])
    registry.insert(party)
    registry.insert(parties)

    # The root carries two typed tags and a tag no dictionary explains.
    root = Field(
        "NewOrderSingle",
        DataType.from_fields([msgtype, side, qty, symbol, count, parties, Field("9999", "utf8")]),
        nullable=False,
    )
    message = FixMsg(
        root,
        {
            "MsgType": "D",
            "Side": "1",
            "Symbol": "AAPL",
            "OrderQty": 100,
            "NoPartyIDs": 1,
            "Parties": [{"PartyID": "BROKER"}],
            "9999": "custom",
        },
        registry,
    )

    # The typed facts left the row for their holders: the type is the header's,
    # the side and the quantity the event's, and the row holds the four others.
    assert message.header().msgtype == "D"
    assert message.side == yggdryl.Side.BUYS
    assert [name for name, _ in message] == ["Side", "Symbol", "NoPartyIDs", "Parties", "9999"]
    assert len(message) == 5

    # A lookup answers the holder for a typed tag and the row for the rest.
    # `OrderQty` is the quantity the event is about, so it answers exact.
    assert message.by_tag(35).as_py() == "D"
    assert message.by_tag(54).as_py() == "1"
    assert message.by_tag(38).as_py() == Decimal(100)
    assert message.by_name("ticker").as_py() == "AAPL"
    assert message.by_path("Parties[0].PartyID").as_py() == "BROKER"
    assert message.by_tag(9999).as_py() == "custom", "an unknown tag is retained"
    assert message[55] == message.get_by_tag(55)
    with pytest.raises(KeyError):
        message.by_path("Parties.PartyID")  # a group member needs its index

    # The entries are the row read as a tree: the group is its counter valued
    # the count, over one valueless entry per occurrence.
    assert message.entries() == [
        (54, "Side", "1", []),
        (55, "Symbol", "AAPL", []),
        (453, "Parties", "1", [(0, "Party", None, [(448, "PartyID", "BROKER", [])])]),
        (0, "9999", "custom", []),
    ]

    # The identity is settled from what the message states.
    assert message.currhashcode != 0
    assert message.currunix == message.header().sendingtime, "undated, so the sending clock stands in"
    assert message.crosscode == "", "no OrderID or ClOrdID names a chain"
    assert message.crossuuid == message.curruuid, "so the message is a chain of one"

    # An identifier is the tag and the name together, under the one fold, and exact.
    folded = Field("order_qty", "int64")
    folded.fix.tag = 38
    assert folded.fix.id == qty.fix.id
    assert message.by_id(qty.fix.id).as_py() == 100
    renamed = Field("Quantity", "int64")
    renamed.fix.tag = 38
    assert message.get_by_id(renamed.fix.id) is None, "another name is another field"

    # The row serializes through the paths every field and value share, and a
    # message rebuilt from them holds the same content; its typed facts are its
    # own to state again.
    assert '"FIX:tag":"55"' in message.field.into_json()
    again = FixMsg(message.field, message.value, registry)
    assert again.entries() == message.entries()
    assert again.header().msgtype == "", "the type was the header's, not the row's"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields, fix } = require('yggdryl')

    const msgtype = Field.from('MsgType: utf8')
    msgtype.fix.tag = 35
    const side = Field.from('Side: utf8')
    side.fix.tag = 54
    const symbol = Field.from('Symbol: utf8 not null')
    symbol.fix.tag = 55
    symbol.fix.names = ['Ticker']
    const qty = Field.from('OrderQty: int64 not null')
    qty.fix.tag = 38
    const partyId = Field.from('PartyID: utf8')
    partyId.fix.tag = 448
    const count = fields.int32('NoPartyIDs', { nullable: false })
    count.fix.tag = 453
    const party = fields.struct('Party', [partyId], { nullable: false })
    const parties = fields.serie('Parties', party)
    parties.fix.counter = 453
    parties.fix.component = 'Party'
    const registry = fix.FixRegistry.fromFields([msgtype, side, symbol, qty, count, partyId])
    registry.insert(party)
    registry.insert(parties)

    // The root carries two typed tags and a tag no dictionary explains.
    const root = fields.struct(
      'NewOrderSingle',
      [msgtype, side, qty, symbol, count, parties, Field.from('9999: utf8')],
      { nullable: false },
    )
    const message = new fix.FixMsg(
      root,
      {
        MsgType: 'D',
        Side: '1',
        OrderQty: 100n,
        Symbol: 'AAPL',
        NoPartyIDs: 1,
        Parties: [{ PartyID: 'BROKER' }],
        9999: 'custom',
      },
      registry,
    )

    // The typed facts left the row for their holders: the type is the header's,
    // and the quantity the message lifts. The side is an ordinary child, so it
    // stays in the row beside the four others.
    assert.equal(message.header().msgtype, 'D')
    assert.equal(message.side, 'BUYS')
    assert.equal(message.size, 5)

    // A lookup answers the holder for a typed tag and the row for the rest.
    // `OrderQty` is the quantity the message lifts, so it answers exact - a
    // decimal at the crate's own scale, which `quantity` renders as text in
    // JavaScript so no decimal precision is lost;
    // `Side(54)` is a row child, so it answers the code the row holds and
    // `side` reads the side off it.
    assert.equal(message.byTag(35).asJs(), 'D')
    assert.equal(message.byTag(54).asJs(), '1')
    assert.equal(message.quantity, '100')
    assert.equal(message.byName('ticker').asJs(), 'AAPL')
    assert.equal(message.byPath('Parties[0].PartyID').asJs(), 'BROKER')
    assert.equal(message.byTag(9999).asJs(), 'custom', 'an unknown tag is retained')
    assert.ok(message.get(55).equals(message.getByTag(55)))
    assert.throws(() => message.at('Parties.PartyID'), /fix/)

    // Iterating a message walks its entries: the row read as a tree, the group
    // its counter valued the count over one valueless entry per occurrence.
    assert.deepEqual(
      [...message].map(entry => [entry.tag, entry.name, entry.value]),
      [[54, 'Side', '1'], [55, 'Symbol', 'AAPL'], [453, 'Parties', '1'], [0, '9999', 'custom']],
    )
    const group = message.entries().find(entry => entry.tag === 453)
    const [occurrence] = group.entries
    assert.deepEqual([occurrence.tag, occurrence.name, occurrence.value], [0, 'Party', null])
    assert.equal(occurrence.entries[0].name, 'PartyID')

    // The identity is settled from what the message states.
    assert.notEqual(message.currhashcode, 0n)
    assert.equal(message.currunix, message.header().sendingtime, 'undated, so the sending clock stands in')
    assert.equal(message.crosscode, '', 'no OrderID or ClOrdID names a chain')
    assert.equal(message.crossuuid, message.curruuid, 'so the message is a chain of one')

    // An identifier is the tag and the name together, under the one fold, and exact.
    const folded = Field.from('order_qty: int64')
    folded.fix.tag = 38
    assert.equal(folded.fix.id, qty.fix.id)
    assert.ok(message.byId(qty.fix.id).equals(message.byTag(38)))
    const renamed = Field.from('Quantity: int64')
    renamed.fix.tag = 38
    assert.equal(message.getById(renamed.fix.id), null, 'another name is another field')

    // Schema and value serialize through the paths every field and value share,
    // and a message rebuilt from them holds the same content; its typed facts
    // are its own to state again.
    const document = message.toJSON()
    const tags = document.field.dtype.fields.map((field) => field.metadata['FIX:tag'])
    assert.ok(tags.includes('55'), 'every row field carries its own tag')
    const again = new fix.FixMsg(message.field, message.value, registry)
    assert.deepEqual(again.entries(), message.entries())
    assert.equal(again.header().msgtype, '', 'the type was the header\'s, not the row\'s')
    ```

## Typed tags

A message holds each fact once. The tags below are the holders' and are never in the row: a child stating one at construction fills its holder and leaves the row, a lookup by one of them answers the holder, a write to one of them writes the holder. Everything else - `ClOrdID(11)`, `OrderQty(38)`, a `Parties` group, a `9999` no dictionary explains - is the row's.

| tags | holder | facts |
| --- | --- | --- |
| 8, 35, 49, 56, 34, 52, 43, 385, 93, 89, 10 | `header()` | the frame: `beginstring`, `msgtype`, `sendercompid`, `targetcompid`, `msgseqnum`, `sendingtime` with `stated_sendingtime`, `possdupflag`, `msgdirection`, and the trailer `signaturelength`, `signature`, `checksum` |
| 6, 11, 14, 17, 31, 32, 37, 38, 41, 44, 53, 117, 131, 151, 188, 189, 190, 191, 194, 195, 198, 262, 1003 | `lifted()` | the numbers a consumer reads first and the identifiers one message of a chain shares with the next: `Price`, `OrderQty`, `Quantity`, `LastPx`, `LastQty`, `AvgPx`, `CumQty`, `LeavesQty`, `ClOrdID`, `OrigClOrdID`, `OrderID`, `SecondaryOrderID`, `ExecID`, `QuoteID`, `QuoteReqID`, `MDReqID`, `TradeID`, and the six FX parts of a price behind one pointer - `LastSpotRate(194)`, `LastForwardPoints(195)`, `BidSpotRate(188)`, `BidForwardPoints(189)`, `OfferSpotRate(190)`, `OfferForwardPoints(191)`, read as `lifted().lastspotrate()` and its five siblings - each exactly as the message stated it |
| every [crate tag](capture.md#the-crates-own-columns), 65001 to 65030 | the [event](../graph/event.md) getters, the message and `capture()` | the identities, the codes, the instants, the place at the instant, the state it reached and when it expires on the event; the category `msgcat()` - a [`MarketDataKind`](../types/enum/marketdatakind.md) member - and the strike `strikepx()` on the message; the instrument codes `isincode`, `forexcode`, `bloombergcode`, `figicode` and `miccode`, views of the market facts below; `msgpluginid`, `msgctxid`, `msgsessionid`, the `msgsesseventid` they join to, `msgoriginator` and `conversationid` on the capture; `metadata` is the bridge's namespaced keys; the names a message goes by are its [alternate identifiers](#the-identifier-maps), read off the FIX fields that state them and no column of their own. [The capture's own column](#a-row-is-a-message-again), `sourceurl` (65031), is no fact: no holder answers it, so `get_by_tag(SOURCEURL_TAG_NAME.0)` is a miss on every message; and the ten a bridge names in its own words, 65032 to 65041, are content the row keeps |
| 58 | `text()` | the free text |

Every market fact is *not* here. `Currency(15)`, `Side(54)`, `CFICode(461)`, the bid and offer `BidPx(132)`, `BidSize(134)`, `OfferPx(133)` and `OfferSize(135)`, `SecurityID(48)` under its source, the `secaltids` group and the market are ordinary children of the row, and what a [`Market` or `Operation`](../graph/market.md) getter answers is derived from them and from the lifted numbers - `get_price` is `Price(44)`, else on a quote taking a side that side's `BidPx(132)` or `OfferPx(133)`, and `None` otherwise - never `LastPx(31)` or `AvgPx(6)`, which are the last executed and the average price and answer as `get_lastpx` and `get_avgpx`; `get_quantity` is `OrderQty(38)`, else `Quantity(53)`, else on a sided quote its side's size; `get_spotrate` and `get_forwardpoints` are `LastSpotRate(194)` and `LastForwardPoints(195)`, else on a sided quote its side's `BidSpotRate(188)`/`BidForwardPoints(189)` or `OfferSpotRate(190)`/`OfferForwardPoints(191)`, and where two of `LastPx(31)`, `LastSpotRate(194)` and `LastForwardPoints(195)` are stated the third is their sum or difference; the [bid and ask](../graph/market.md#bid-and-ask) - `get_bidpx`, `get_bidqty`, `get_askpx`, `get_askqty` - are `BidPx(132)`, `BidSize(134)`, `OfferPx(133)` and `OfferSize(135)`, and `get_bidccy` a stated `BidCurrency` field, `get_askccy` a stated `AskCurrency` or `OfferCurrency` field, each read by name, else the message's currency where that side states a price or a size; `get_fxrates` is empty, because no FIX field states a rate a message divides by; `get_side` is the `Side(54)` stated, else `UNKN`, and never read off which of the bid or the offer a quote states; `get_securityids` is built at every settle, each source filling only the keys the ones before it left absent: `SecurityID(48)` under `SecurityIDSource(22)`, each `secaltids` occurrence read through `SecType::read`, the crated `isincode`, `forexcode`, `bloombergcode` and `figicode` views a row stated, then every top-level field no dictionary maps whose name names a source through `SecType::from_field_name` - `ISIN`, `cusip_code`, `#SEDOLCODE` - trimmed, validated and left on the wire as it arrived; a value that is empty or null-like states nothing, and an invalid code, or a different code under a filled key, states nothing and is kept as an [anomaly](#anomalies); a currency pair `Symbol(55)` names where the message states no other class is [derived](capture.md#a-currency-pair-is-read-off-the-symbol) under `FOREX`, and `get_isincode` borrows the `ISIN` entry. The first identifier under a key ending `INSTRUMENTID` whose value after its last `;` is shaped `{ISIN}_{MIC}_{CCY}` - twelve, four and three ASCII alphanumerics, `dbi;CH0012214059_XSWX_CHF` - is a bridge's instrument key: it fills the `ISIN` and `get_currency` only where nothing stated them and names the market `get_miccode` ranks after `LastMkt(30)` and `ExDestination(100)` and before `SecurityExchange(207)`, a part its own type refuses skipped, and nothing it fills reaches the wire; a currency pair trades on no one market, so where none of them names one `get_miccode` is ISO 10383's `XXXX`. A derived market fact is the traits' to answer and nobody's to emit: it reaches no column, no entry, no byte on the wire and no input to the code the message digests to. Six readings have [columns of the crate's own](capture.md#the-crates-own-columns): the event facts `state` (65029) and `exprunix` (65007), which a walk folds forward; `execunix` (65002), the lifecycle's latest precise execution clock, which a report `FixMsg::is_execution` accepts takes from its own `currunix` when it settles stating none and follows nothing; the observation's own `recdunix` (65003); and the message's `msgcat` (65016) - the dictionary's `FIX:msgcat` for the type through `msgcatcodeset`, `UNKN` where it files none - and `strikepx` (65028), `StrikePrice(202)` as the decimal leaf. Duplicate observations of one event merge `execunix` and `recdunix` to their earliest precise values before lifecycle following carries the latest execution forward, and the later `recdunix` of the two decides which one is the reference. A row stating one is the row's word; none reaches the wire or the entries.

A typed fact answers as its column types it: `by_tag(35)` is text, `by_tag(52)` a `datetime64(ns, UTC)`, `by_tag(MSGCAT_TAG_NAME.0)` a `MarketDataKind` member, `by_tag(CURRHASHCODE_TAG_NAME.0)` a `UInt64`, `by_tag(CURRUUID_TAG_NAME.0)` a `Uuid`, `by_tag(METADATA_TAG_NAME.0)` a sorted map; a row child answers as the dictionary types it, so under the shipped dictionary `by_tag(54)` is a `Side` member whose `as_str` is `BUYS`, and nothing on a message stating no `Side(54)`, whose `get_side` is `UNKN`; a holder stating nothing - a place of zero, the first at its instant - answers nothing, and a state of `UNKNOWN` answers as it is, the state stated as none, so `by_tag(STATE_TAG_NAME.0)` is never a miss and the `state` column never null on a row a message wrote.

## One namespace

A message speaks no dialect of its own: the registry is one namespace, and a bare tag or name resolves in it directly, the way the [registry](registry.md) resolves it.

| key | answers |
| --- | --- |
| a tag | the canonical holder of the tag, then a field holding it as an alternate |
| a name | the canonical fold, then an alias fold - `ticker` reaches `Symbol` through its alias. An alias fills its field only where neither the canonical name nor the tag stated it, and among aliases the one `FIX:names` lists first wins whatever the arrival order; one that loses is a `utf8` child of its own spelling whose `FIX:alias` names the field it lost to - no restatement folds it back, a row rebuilds it so, it re-emits as it arrived and it is no anomaly |
| an id | exactly one field: `FixId::of(tag, name)`, the signed XXH32 of the tag's bytes and the folded name, so `OrderQty`, `order_qty` and `orderqty` under 38 are one id and `Quantity` under 38 is another |

`get_by_tag(5001)` finds a venue's own field and `get_by_tag(35)` finds the message type from the same message, because both live in the one namespace. Which dictionaries a field belongs to is the field's own `FIX:branches`, a membership a reader may ask about and nothing here resolves through; a message root the codec builds carries none.

## Accessors

| accessor | resolution |
| --- | --- |
| `get_by_tag` / `by_tag` | a [typed tag](#typed-tags) answers its holder; else the root child carrying the tag, else the tag through the registry to its canonical name and the root child of that name, else a root child named by the tag's decimal text |
| `get_by_id` / `by_id` | takes a `FixId` (an `int` in Python, a `number` in JavaScript) and names one field exactly: a typed field's holder, else the child under that field's name, and a miss for any other tag or name |
| `get_by_name` / `by_name` | folds through the registry to the canonical spelling - a typed field answers its holder - then matches a root child exactly |
| `get_by_path` / `by_path` | the first segment as a name, then segment by segment: into a Struct child by name, into a Serie entry by a decimal index |
| `get` / `value` | takes a `FixKey` and redirects; a name that reaches nothing and spells more than one segment is read as a path |
| `header`, `capture`, `text`, `metadata` | the holders themselves, borrowed without a lookup; the event's facts are the trait getters, and the leaves a message expands to are [`market_data`](#market-data) |

Every lookup answers an owned `Scalar`: a holder's fact is rendered into the column's type on the way out, and a row child cloned.

## Anomalies

A value that will not type is null in the row rather than a failure - a clock, `SendingTime(52)` or `TransactTime(60)`, naming no instant included, which leaves the message dated as one stating none is - a counter that disagrees with the group it counts is kept as it arrived, and a settle drops a stated identifier that conflicts with a stated one - a security identifier, or a second value under one [identifier-map](#the-identifier-maps) key. A bridge's line adds its own: a `#`-marked twin stating another value than the bare key beside it, a `CONVERSATIONID` its `{conversationId: ...}` contradicts, a MIC alias that is no ISO 10383 code, and - when two observations of one delivery merge - a later one naming another `msgoriginator` or `conversationid` than the earlier, which keeps its own. Each is a fact about the message worth more than a null nobody can explain, and what the parse defaults is also said once as a deduplicated [warning](capture.md#warnings). `anomalies()` reads them off the message beside the row, in arrival order - the parse's first, then what the last settle dropped - each a `FixAnomaly` of the field it was stated under and the reason. Never a column, never part of the code the message digests to; two statements of one message merge them as a union, the reference's first.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    // `StopPx(99)` is a decimal and `abc` is not one: the row types null, the
    // refusal names the field and what arrived.
    let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=abc|10=0|")?;
    assert!(held.get_by_tag(99).is_none_or(|value| value.is_null()));
    let [refused] = held.anomalies() else { panic!("one anomaly") };
    assert_eq!(refused.field(), "stoppx");
    assert!(refused.reason().ends_with(", got \"abc\""), "{}", refused.reason());

    // A counter that disagrees with its group: the group is the row, the
    // disagreement is kept.
    let held = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=2|448=BROKER|447=D|452=1|10=0|")?;
    assert_eq!(held.anomalies()[0].to_string(), "nopartyids: states 2, the group holds 1");

    // A line that types whole has none.
    let clean = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=10.5|10=0|")?;
    assert!(clean.anomalies().is_empty());
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)

    held = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=abc|10=0|")
    [(field, reason)] = held.anomalies
    assert field == "stoppx"
    assert reason.endswith(', got "abc"'), reason

    held = reader.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|453=2|448=BROKER|447=D|452=1|10=0|")
    assert held.anomalies == [("nopartyids", "states 2, the group holds 1")]

    assert reader.parse_fix_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|99=10.5|10=0|").anomalies == []
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    const reader = new fix.FixCodec(registry)

    const held = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A1|55=AAPL|99=abc|10=0|'))
    assert.equal(held.anomalies.length, 1)
    assert.equal(held.anomalies[0].field, 'stoppx')
    assert.ok(held.anomalies[0].reason.endsWith(', got "abc"'), held.anomalies[0].reason)

    const counted = reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A1|55=AAPL|453=2|448=BROKER|447=D|452=1|10=0|'))
    assert.deepEqual(counted.anomalies, [{ field: 'nopartyids', reason: 'states 2, the group holds 1' }])

    assert.deepEqual(reader.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=A1|55=AAPL|99=10.5|10=0|')).anomalies, [])
    ```

## Written into the row

A message is read once and then written to: a [walk](lifecycle.md) stamps what the stream implied, a caller corrects a value. All of it goes through one door. `set` writes one value, `set_many` lands several with one rebuild, `with_value` is the consuming twin, and `remove` takes a value out and answers what it held. A key reaching a [typed tag](#typed-tags) writes its holder - `set(34, ..)` is the header's sequence number, `set(PX_TAG_NAME.0, ..)` the event's price - and a `Null` clears it; any other key writes the row, typed by the field the key reaches. Every write settles the [identity](../hashing.md) again, so a content write moves `currhashcode` and `curruuid` - a write to the frame, `set(34, ..)`, does not, because the standard header and trailer are outside the code - while `crossuuid` stays with the cross code; and every write drops the derived entries, so `into_bytes` re-emits the message as it now stands.

The security-identifier verbs write FIX's own fields through one writer, `sync_security_id`: `set_securityids`, `insert_securityid` and `remove_securityid` write `SecurityID(48)` in place where `SecurityIDSource(22)` already names the key, else the `secaltids` occurrence under the key's source code - `4` for `ISIN`, `1` for `CUSIP`, `2` for `SEDOL`, `A` for `BLOOMBERG`, `S` for `FIGI`, and a key with no code under its own upper-cased name. The group's canonical name is `secaltids` and its display is `SecAltIDs`; setting a value replaces that source's occurrence or appends one, removing a key removes only that source, occurrences under every other source remain in place, and `NoSecurityAltID(454)` stays synchronized with the resulting group rather than becoming a second count. `derive_securityid` fills the event alone and never the wire: a derived identifier - the national number the ISIN carries, what the lifecycle's registry learned - is replaced by a stated one under its key, which `insert_securityid` then writes, and removing the ISIN takes every derived identifier back. The [identifier-map verbs](#the-identifier-maps) write the field a key is read from - `insert_altid("ORDERID", ..)` writes `OrderID(37)` - and a key no field states is a located refusal.

| Key | Reaches |
| --- | --- |
| [the capture's own column](#a-row-is-a-message-again) - `sourceurl` | refused, naming the column: a message holds no fact for it, and a row child would put `sourceurl=` on the wire. `remove` answers nothing, because there is nothing to reach |
| a typed tag | the holder that owns it; a value the fact's type refuses is silence, a `Null` clears the fact |
| a tag the dictionary knows | the child carrying it, replaced where it stands; else appended under the dictionary's field - its canonical name, its datatype, its `FIX:tag` - so a written child is indistinguishable from a stated one |
| a name the dictionary knows | the same field, through the [one namespace](#one-namespace) every lookup resolves in |
| a name it does not know | the child spelled that way, exactly or under the fold every name resolves by, keeping that child's own field; nothing reached is refused, and the message stands |
| a tag it does not know | the child named by its decimal, else a nullable `utf8` child appended under it - what the builder does with an unknown tag |
| a `Null` value | a stated null: the child stays, nullable, holding nothing |

A written child keeps its position, so every reader already holding the row addresses it as before, and the tag index follows the one child that changed rather than being reread. A value the field refuses - text into `OrderQty(38)` - is refused whole, and with `set_many` one refused write refuses them all.

A written value is then [restated](#restated-under-the-dictionary) exactly as a read one is: `set(47, 'A')` writes the `OrderCapacity(528)` that replaced `Rule80A`, and `set(76, 'BRKR')` makes the `Parties` occurrence `ExecBroker` became, from the [specification's retirements](registry.md#what-the-specification-retired). What runs is the rules of the tags written, so a write of a tag no rule speaks for is the write and nothing more. The pass never overwrites a stated value and is idempotent, so writing one value twice writes its replacement once, and a caller who stated the replacement keeps what they stated.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Element;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixMsg, FixRegistry, Scalar, fix_schema};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    let line = b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|9999=x|10=0|";
    let mut message = reader.parse_fix_line(line)?;
    let children = message.as_field().fields().len();
    let at = message.as_field().index_of("symbol").expect("the symbol child");
    let (hashcode, crossuuid) = (message.get_currhashcode(), message.get_crossuuid());

    // A typed tag lands on its holder and never in the row.
    message.set(34, Scalar::from(7_i32))?;
    assert_eq!(message.header().msgseqnum(), Some(7));
    assert_eq!(message.by_name("MsgSeqNum")?.as_u64(), Some(7));
    assert_eq!(message.as_field().fields().len(), children);
    // A header fact is the session's, outside the content identity: neither moved.
    assert_eq!(message.get_currhashcode(), hashcode);
    assert_eq!(message.get_crossuuid(), crossuuid);

    // Replaced where it stands: the position is kept, the value changes, and
    // the content identity moves with the content; the chain's is the cross
    // code's alone.
    message.set("Symbol", Scalar::from("MSFT"))?;
    assert_eq!(message.as_field().index_of("symbol"), Some(at));
    assert_eq!(message.by_tag(55)?.as_str(), Some("MSFT"));
    assert_ne!(message.get_currhashcode(), hashcode);
    assert_eq!(message.get_crossuuid(), crossuuid);

    // A tag no dictionary explains is kept under its decimal spelling, and
    // appended to the row.
    message.set(7777, Scalar::from("custom"))?;
    assert_eq!(message.by_tag(7777)?.as_str(), Some("custom"));
    assert_eq!(message.as_field().fields().len(), children + 1);

    // A name nothing reaches is refused, and the row stands as it was.
    assert!(message.set("nosuchfield", Scalar::from("x")).is_err());
    assert_eq!(message.as_field().fields().len(), children + 1);

    // Removed, and the value answered: a typed fact is cleared on its
    // holder, a row child taken out, and the other tags still reach theirs.
    assert_eq!(message.remove(54)?.as_ref().and_then(Scalar::as_str), Some("BUYS"));
    assert_eq!(message.get_by_tag(54), None);
    assert_eq!(message.remove("nosuchfield")?, None);
    assert_eq!(message.by_tag(11)?.as_str(), Some("A1"));

    // The wire is the message as it now stands: the header from its holder,
    // then the row, the written pairs where they landed.
    assert_eq!(
        message.into_text('|')?,
        "8=FIX.4.4|35=D|34=7|52=20260102-10:15:30|11=A1|55=MSFT|9999=x|59=0|7777=custom|10=0|",
    );

    // The written message is a fixed row, and the row a message again:
    // the same canonical row and content identity. Residual entries rebuild before
    // projected columns, so entry and wire order are not row contracts.
    let schema = fix_schema(&registry, "fix")?;
    let row = message.into_row(&schema)?;
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row)?;
    assert_eq!(held.by_tag(55)?, message.by_tag(55)?);
    assert_eq!(held.by_tag(7777)?.as_str(), Some("custom"));
    assert_eq!(held.get_currhashcode(), message.get_currhashcode());
    assert_eq!(held.into_row(&schema)?, row);
    ```

=== "Python"

    ```python
    from pathlib import Path

    import pytest

    from yggdryl import Side
    from yggdryl.fix import FixCodec, FixMsg, FixRegistry, fix_schema

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)

    line = b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|9999=x|10=0|"
    message = reader.parse_fix_line(line)
    children = len(message)
    names = [name for name, _ in message]
    hashcode, crossuuid = message.currhashcode, message.crossuuid

    # A typed tag lands on its holder and never in the row.
    message.set(34, 7)
    assert message.header().msgseqnum == 7
    assert message.by_name("MsgSeqNum").as_py() == 7
    assert len(message) == children
    # A header fact is the session's, outside the content identity: neither moved.
    assert message.currhashcode == hashcode
    assert message.crossuuid == crossuuid

    # Replaced where it stands: the position is kept, the value changes, and the
    # content identity moves with the content; the chain's is the cross code's alone.
    message.set("Symbol", "MSFT")
    assert [name for name, _ in message].index("symbol") == names.index("symbol")
    assert message.currhashcode != hashcode
    assert message.crossuuid == crossuuid
    assert message.by_tag(55).as_py() == "MSFT"

    # A tag no dictionary explains is kept under its decimal spelling, and
    # appended to the row.
    message.set(7777, "custom")
    assert message.by_tag(7777).as_py() == "custom"
    assert len(message) == children + 1

    # A name nothing reaches is refused, and the row stands as it was.
    with pytest.raises(KeyError):
        message.set("nosuchfield", "x")
    assert len(message) == children + 1

    # Removed, and the value answered: a typed fact is cleared on its holder, a
    # row child taken out, and the other tags still reach theirs.
    assert message.remove(54).as_py() == Side.BUYS
    assert message.get_by_tag(54) is None
    assert message.remove("nosuchfield") is None
    assert message.by_tag(11).as_py() == "A1"

    # The wire is the message as it now stands: the header from its holder, then
    # the row, the written pairs where they landed.
    assert message.into_text("|") == "8=FIX.4.4|35=D|34=7|52=20260102-10:15:30|11=A1|55=MSFT|9999=x|59=0|7777=custom|10=0|"

    # The written message is a fixed row, and the row a message again: the same
    # canonical row and content identity. Residual entries rebuild before projected
    # columns, so entry and wire order are not row contracts.
    schema = fix_schema(registry, "fix")
    row = message.into_row(schema)
    held = FixMsg.from_row(schema, row, registry)
    assert held.by_tag(55) == message.by_tag(55)
    assert held.by_tag(7777).as_py() == "custom"
    assert held.currhashcode == message.currhashcode
    assert held.into_row(schema) == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    const reader = new fix.FixCodec(registry)

    const line = '8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|9999=x|10=0|'
    const message = reader.parseFixLine(Buffer.from(line))
    const children = message.size
    const names = [...message].map(entry => entry.name)
    const hashcode = message.currhashcode
    const crossuuid = message.crossuuid

    // A typed tag lands on its holder and never in the row.
    message.set(34, 7)
    assert.equal(message.header().msgseqnum, 7)
    assert.equal(message.byName('MsgSeqNum').asJs(), 7)
    assert.equal(message.size, children)
    // A header fact is the session's, outside the content identity: neither moved.
    assert.equal(message.currhashcode, hashcode)
    assert.equal(message.crossuuid, crossuuid)

    // Replaced where it stands: the position is kept, the value changes, and the
    // content identity moves with the content; the chain's is the cross code's alone.
    message.set('Symbol', 'MSFT')
    assert.equal([...message].map(entry => entry.name).indexOf('symbol'), names.indexOf('symbol'))
    assert.equal(message.byTag(55).asJs(), 'MSFT')
    assert.notEqual(message.currhashcode, hashcode)
    assert.equal(message.crossuuid, crossuuid)

    // A tag no dictionary explains is kept under its decimal spelling, and
    // appended to the row.
    message.set(7777, 'custom')
    assert.equal(message.byTag(7777).asJs(), 'custom')
    assert.equal(message.size, children + 1)

    // A name nothing reaches is refused, and the row stands as it was.
    assert.throws(() => message.set('nosuchfield', 'x'), /nosuchfield/)
    assert.equal(message.size, children + 1)

    // Removed, and the value answered: a typed fact is cleared on its holder, a
    // row child taken out, and the other tags still reach theirs.
    assert.equal(message.remove(54).asJs(), 'BUYS')
    assert.equal(message.getByTag(54), null)
    assert.equal(message.remove('nosuchfield'), null)
    assert.equal(message.byTag(11).asJs(), 'A1')

    // The wire is the message as it now stands: the header from its holder, then
    // the row, the written pairs where they landed.
    assert.equal(
      message.intoText('|'),
      '8=FIX.4.4|35=D|34=7|52=20260102-10:15:30|11=A1|55=MSFT|9999=x|59=0|7777=custom|10=0|',
    )

    // A fixed row preserves the semantic message: projected facts and residual
    // entries rebuild to the same row and content identity; their order is not
    // a row contract.
    const schema = fix.schema(registry, 'fix')
    const row = message.intoRow(schema)
    const held = fix.FixMsg.fromRow(schema, row, registry)
    assert.ok(held.byTag(55).equals(message.byTag(55)))
    assert.equal(held.byTag(7777).asJs(), 'custom')
    assert.equal(held.currhashcode, message.currhashcode)
    assert.ok(held.intoRow(schema).equals(row))
    ```

## The identifier maps

A message goes by the names the fields of its dictionary state, and the [`IdMap`](../graph/operation.md#alternate-identifiers) of alternate identifiers `Operation` answers - `get_altids()` - is rebuilt from those fields at every settle: a view of the row, never a store of its own. Which field states which key is a fact about the field, so it travels on the field as its [`FIX:idmap`](registry.md#a-field-names-a-message-by-its-identifiers) document, and `FixRegistry::idmap_sources` compiles every field's once into the table a settle reads. `altids` is the map of the identifiers a message goes by; the parties it names and the account it is booked to (`Account(1)`) are its [accounts](#accounts-and-regulatory-trade-identifiers), and the user who entered it is neither, and stays in its leaves' [metadata](#what-a-leafs-metadata-holds). The dictionary ships this table:

| Key | Followed | Source |
| --- | --- | --- |
| `ORDERID`, `SECONDARYORDERID`, `PARENTORDERID`, `PARENTCLORDID`, `OMSDEALERPARENTORDERID`, `EXCHANGECLIENTORDERID`, `TRANSVERSALKEY` | yes | `OrderID(37)`, `SecondaryOrderID(198)`, the crate's `parentorderid` (65034) to `transversalkey` (65038) |
| `CLORDID`, `ORIGCLORDID`, `EXECID`, `TRDMATCHID`, `QUOTEID`, `QUOTEREQID`, `MDREQID`, `TRADEID`, `ULTRADERCLORDID` | no | `ClOrdID(11)`, `OrigClOrdID(41)`, `ExecID(17)`, `TrdMatchID(880)`, `QuoteID(117)`, `QuoteReqID(131)`, `MDReqID(262)`, `TradeID(1003)`, the crate's `ultraderclordid` (65039) |

The six crate fields are a bridge's own identifiers - `PARENTORDERID=`, `TRANSVERSALKEY=` - content the row keeps under its [entries](capture.md#the-crates-own-columns) rather than a column of the fixed row, and so are `omsdealeraccount` (65032) and `omsuserid` (65033), which name no identifier. The first value a key is stated with fills it; a later different one states nothing and is kept as an [anomaly](#anomalies).

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::fix::FixIdMapKind;
    use yggdryl::graph::Operation;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    // The field states the key; the registry compiles every field's once.
    let (tag, parent) = registry
        .idmap_sources()
        .iter()
        .find(|(_, source)| source.key() == "PARENTORDERID")
        .expect("a bridge's parent order");
    assert_eq!((*tag, parent.map(), parent.follows()), (65_034, FixIdMapKind::Alts, true));

    let reader = FixCodec::new(Arc::clone(&registry));
    let held = reader.parse_fix_line(
        b"8=FIX.4.4|35=8|17=E1|37=O1|OMSDEALERACCOUNT=ACC1|OMSUSERID=trader1|PARENTORDERID=P1|10=0|",
    )?;
    let keys: Vec<&str> = held.get_altids().iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["EXECID", "ORDERID", "PARENTORDERID"]);
    assert_eq!(held.get_altids().get("PARENTORDERID"), Some("P1"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert {
        "tag": 65034, "map": "altids", "key": "PARENTORDERID", "follow": True, "role": None
    } in registry.idmap_sources()

    held = FixCodec(registry).parse_fix_line(
        b"8=FIX.4.4|35=8|17=E1|37=O1|OMSDEALERACCOUNT=ACC1|OMSUSERID=trader1|PARENTORDERID=P1|10=0|"
    )
    assert held.altids == {"EXECID": "E1", "ORDERID": "O1", "PARENTORDERID": "P1"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    assert.deepEqual(
      registry.idmapSources().find((source) => source.key === 'PARENTORDERID'),
      { tag: 65034, map: 'altids', key: 'PARENTORDERID', follow: true },
    )

    const held = new fix.FixCodec(registry).parseFixLine(Buffer.from(
      '8=FIX.4.4|35=8|17=E1|37=O1|OMSDEALERACCOUNT=ACC1|OMSUSERID=trader1|PARENTORDERID=P1|10=0|',
    ))
    assert.deepEqual(held.altids, { EXECID: 'E1', ORDERID: 'O1', PARENTORDERID: 'P1' })
    ```

A value that states nothing - empty, `null`, `none`, `n/a`, `[n/a]` - adds nothing. A write goes through the field a key is read from: `insert_altid` fills only an absent key, writing its field and answering `false` for a held one; `remove_altid` nulls the field; `set_altids` writes every entry that moved and nulls every key the new map lacks; each settles the identity again, and the map is rebuilt from the row on the way out, so what it answers is always what the row states. A key no top-level field states - a party-role key, or a name the table lacks - is a located `InvalidRecord` at `$.altids.<KEY>` on every write that would have to land it or null it, because a map that is a view of the row cannot hold what the row cannot say; a graph leaf - an `OrderEvent` - accepts any key. The [lifecycle](lifecycle.md#a-chain-is-named-by-its-cross-code) joins a chain by `altids`, and a message that follows another carries the alternate identifiers whose `FIX:idmap` entry follows - the order's own and its parents', what `Operation::is_followed_altid` answers for a message - never an execution's or a quote's, where a graph leaf follows every key it lacks but `MDENTRYREFID`. `altids` is what a message's fields state: a leaf built from it can hold more, since it [lifts](#what-a-leafs-metadata-holds) the metadata keys ending with one of its message's identifiers. The map is no column of the [fixed row](capture.md#the-crates-own-columns) - the fields that state it are.

### Accounts and regulatory trade identifiers

Two readings are the crate's own rather than a field's `FIX:idmap`, and a settle rebuilds both from the message's repeating groups and its `Account(1)`. A side's own group and `Account(1)` are read first, so an execution a trade's parse split off states its side's, and a book entry's own parties lead the message's on that entry's leaf.

| Reading | Rule |
| --- | --- |
| Accounts | `Operation::get_accountids()`: each `PartyID(448)` of `NoPartyIDs(453)` under its `PartyRole(452)`, and each `RootPartyID(1117)` of `NoRootPartyIDs(1116)` under its `RootPartyRole(1119)`, keyed by the role's name in the code set, upper-cased - `EXECUTINGTRADER`, `CUSTOMERACCOUNT`, `CLIENTID`; `PARTYROLE{code}` where the role has no name or its name is wider than a key holds (32 bytes), `PARTY` where an occurrence states no role |
| One party a role | the first party stated under a role stands; a second of the same role - two contra firms - is skipped without an anomaly, and its occurrence alone stays in the leaf's [metadata](#what-a-leafs-metadata-holds) |
| The account | `Account(1)` under the key `ACCOUNT`; the leaf's metadata leaves it out |
| Regulatory trade ids | `RegulatoryTradeID(1903)` of `NoRegulatoryTradeIDs(1907)`, and a side's `SideRegulatoryTradeID(1972)` of `NoSideRegulatoryTradeIDs(1971)`, in `altids` under the key of the `RegulatoryTradeIDType(1906)` or `SideRegulatoryTradeIDType(1975)`: `0` or none `REGTRADEID`, `1` `PREVREGTRADEID`, `2` `BLOCKREGTRADEID`, `3` `RELATEDREGTRADEID`, `4` `CLEAREDREGTRADEID`, `5` `TVTIC`, `6` `REPORTTRACKINGNUMBER`, another `REGTRADEID{n}`; the first value a key is stated with fills it, and a later different one is an [anomaly](#anomalies) |
| Writing | a message's accounts are its parties and its `Account(1)`, so `set_accountids`, `insert_accountid` and `remove_accountid` refuse a change with an `InvalidRecord` naming `accountids` and expecting the accounts a message's `Parties(453)` and `Account(1)` state - write the `Parties(453)` occurrence or the `Account(1)` instead; a call that changes nothing answers `Ok` |
| Following | a message states its own accounts and carries none from its predecessor; a graph leaf's [accounts follow](../graph/operation.md#account-identifiers) |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Operation;
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let held = FixCodec::new(registry).parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|1=ACCT-7|55=AAPL|54=1|38=5|40=2|44=100|453=3|448=TRADER1|447=D|452=12|448=ACC-9|447=D|452=24|448=NOROLE|1907=2|1903=UTI-1|1906=0|1903=TVT-1|1906=5|10=0|",
    )?;
    // Each party under its role's name, the one stating no role under PARTY,
    // and Account(1) under ACCOUNT.
    let accounts = held.get_accountids();
    assert_eq!(accounts.get("EXECUTINGTRADER"), Some("TRADER1"));
    assert_eq!(accounts.get("CUSTOMERACCOUNT"), Some("ACC-9"));
    assert_eq!(accounts.get("PARTY"), Some("NOROLE"));
    assert_eq!(accounts.get("ACCOUNT"), Some("ACCT-7"));
    // The regulatory trade ids are alternate identifiers by their type.
    assert_eq!(held.get_altids().get("REGTRADEID"), Some("UTI-1"));
    assert_eq!(held.get_altids().get("TVTIC"), Some("TVT-1"));
    // The accounts are the parties' and the account's: a change is refused, a
    // no-op is not.
    let mut written = held.clone();
    assert!(written.insert_accountid("CLIENTID", "C-2").is_err());
    assert!(!written.insert_accountid("EXECUTINGTRADER", "OTHER")?);
    assert!(!written.insert_accountid("ACCOUNT", "OTHER")?);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    held = FixCodec(registry).parse_fix_line(
        b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|1=ACCT-7|55=AAPL|54=1|38=5|40=2|44=100|453=3|448=TRADER1|447=D|452=12|448=ACC-9|447=D|452=24|448=NOROLE|1907=2|1903=UTI-1|1906=0|1903=TVT-1|1906=5|10=0|"
    )
    # Each party under its role's name, the one stating no role under PARTY,
    # and Account(1) under ACCOUNT.
    assert held.accountids == {
        "ACCOUNT": "ACCT-7", "CUSTOMERACCOUNT": "ACC-9", "EXECUTINGTRADER": "TRADER1", "PARTY": "NOROLE"
    }
    # The regulatory trade ids are alternate identifiers by their type.
    assert held.altids == {"CLORDID": "C1", "REGTRADEID": "UTI-1", "TVTIC": "TVT-1"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config/fix'))
    const held = new fix.FixCodec(registry).parseFixLine(Buffer.from(
      '8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|1=ACCT-7|55=AAPL|54=1|38=5|40=2|44=100|453=3|448=TRADER1|447=D|452=12|448=ACC-9|447=D|452=24|448=NOROLE|1907=2|1903=UTI-1|1906=0|1903=TVT-1|1906=5|10=0|',
    ))
    // Each party under its role's name, the one stating no role under PARTY,
    // and Account(1) under ACCOUNT.
    assert.deepEqual(held.accountids, {
      ACCOUNT: 'ACCT-7', CUSTOMERACCOUNT: 'ACC-9', EXECUTINGTRADER: 'TRADER1', PARTY: 'NOROLE',
    })
    // The regulatory trade ids are alternate identifiers by their type.
    assert.deepEqual(held.altids, { CLORDID: 'C1', REGTRADEID: 'UTI-1', TVTIC: 'TVT-1' })
    ```

## A row is a message again

`from_row` is the inverse of [`into_row`](capture.md#a-column-is-filled-by-the-tag-its-field-carries): typed facts are read from the columns that own them, and `fixentries` supplies only the residual arrival content those columns do not represent. The two are rebuilt under the dictionary into one semantic message. The row's `beginstring`, `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid` and `crossuuid` columns must be stated, and every other one may be null - `sendingtime` among them, because a row states tag 52 only where the message did. The six recorded identity cells remain recorded while market getters refill from reconstructed content. Every one of the capture's own columns is carried: the one the crate tags, `sourceurl`, and every column no tag and no counter names - the body the line was cut from, its place in the object, its media type, what a bound dropped - each non-null cell under its column's name, answered by `carried`. A message is what parsing one line answered, and what a *reader* said about that line is not it, so none of them is content: none reaches an entry, the code the message answers to, or a `body=` at a counterparty, and a namespaced column lands in the [metadata](#typed-tags) as a parsed line's does. `into_row` states each carried cell again at its column, so `from_row` then `into_row` returns the same canonical row. Nothing is parsed again, which is what makes a [batch of rows a stream of messages](arrow.md#rows-are-messages-again-and-messages-rows) at the cost of the values it already holds. A row without `fixentries` rebuilds from its projected facts alone.

A key no dictionary resolved is no `fixentries` key: `into_row` writes it into the row's `metadata`, [as that column holds it](capture.md#nothing-is-lost-at-the-end), beside the bridge's namespaced keys, and `from_row` reads every key of that cell that holds no `.` back as the message's own entry: tag `0` under that spelling, a JSON value as the entries it holds. The namespaced keys stay the message's `metadata`. So the wire re-emits the key, a read by name reaches it and the code the message digests to is the parse's, while the row and its round trip keep one place for it.

The fixed-row round trip is semantic: it preserves the canonical row, reconstructed content identity, and typed facts, while original wire and arrival order are not a row contract. Reconstructed sibling fields are canonicalized for the event hash; repeated occurrences retain their order. The public wire digest remains arrival-ordered. A group no dictionary declares - a bridge packing `NOTRADINGSESSIONS[0]=...` under a counter's own name - rebuilds from the row as the serie it is. The [example above](#written-into-the-row) ends with the round trip.

## Restated under the dictionary

A capture holds what each session spoke: a FIX 4.2 report states its fill as `LastShares`, its broker as `ExecBroker(76)`, its capacity as `Rule80A(47)` and a partial fill as `ExecType(150)` `1` - four things the newest specification spells as `LastQty`, a `Parties` occurrence, `OrderCapacity(528)` and `Trade`. A [parse](capture.md#a-reader-is-the-whole-parse-surface) restates the message once as it builds it, from what the dictionary itself says - the registry's field for every tag and the aliases that reach it - and from what the specification says of a retired field or value: [the crate's table of retirements](registry.md#what-the-specification-retired), applied as the message is built, with nothing parsed, bound or evaluated per message. A registry states no rule of its own: the table is native code, the same for every dictionary that declares the tags. The parse then [fills what the restated row implies](capture.md#what-a-message-implied-is-filled-in), by the crate's native derivations. There is no door of its own: a parsed message is a restated message, and so is a written one - [`set`](#written-into-the-row) restates what it writes, so a value a caller states and a value a line states are restated the same way.

| item | contract |
| --- | --- |
| Canonicalizes | every child the registry knows - by its `FIX:tag`, else its name or alias, else the decimal tag its name spells - is re-expressed under the registry's field: canonical name, datatype, tag, in the position it held; a child no dictionary knows stays exactly as it is |
| Merges | children reaching one field become one: the canonical-named child's value when stated, else the first stated among the rest; a child whose stated value disagrees with the kept one is left in place, so nothing that arrived is lost |
| Restates | each child the specification retired, in ascending tag order, by the first entry of [the table](registry.md#what-the-specification-retired) whose condition holds at the level - the held value, the message type and the enclosing group; a group occurrence a rule states makes or completes one occurrence and sets its counter |
| Deprecated | a field the dictionary marks [`FIX:deprecated`](registry.md#the-dictionary-holds-one-reading-and-filters-by-no-version) - one FIX Latest removed, `MaxFloor(111)` among them - is restated to the field that replaced it and then nulled, so the row holds the fact once under its latest name while the entries keep the pair as it arrived |
| All or nothing | every target an entry fills is computed and checked before any is written; one target that cannot take its value blocks the whole entry, and no later entry fills in for it |
| Never overwrites | a value the message stated: a target takes a value only when it is absent, null or already equal; the source field itself is the one exception, because it is what is being restated |
| Keeps | a removed field the specification named no replacement for, and the source of every rule that did not write it - the row says what was sent and what it means |
| Levels | the root, then every occurrence of every repeating group to any depth, each canonicalized and restated in turn; a rule scoped to a group applies inside an occurrence of it, a rule scoped to message types reads the root's `MsgType(35)` |
| Wire | `into_bytes` emits the restated row, so a FIX 4.2 report re-emits with the `OrderCapacity`, the `Parties` and the derived pairs behind what it stated; the entries carry both the source and its restatement |
| Idempotent | a message read back from its row is the same message: what one parse wrote is what the row states |
| Batch | inside the parse, so `parse_text_arrow_reader` lands restated rows and the [lifecycle](lifecycle.md) walks them |
| Bindings | reached through every `parse_*` door in all three languages |

A FIX 4.2 execution report, read as it was sent, which restates it as it builds it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry, Scalar, FieldPath};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));

    // A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    // naming its broker (76) and client (109), the fill as LastShares (32).
    let line: &[u8] = b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|";
    let latest = reader.parse_line(line)?.next().expect("one frame")?;

    // ExecTransType Cancel states TradeCancel, but ExecType already stated
    // PartiallyFilled and a stated value stands - so the rule that answers
    // 150 is ExecType's own, folding the partial fill into Trade. The source
    // stays as it arrived.
    assert_eq!(latest.by_tag(150)?.as_str(), Some("F"));
    assert_eq!(latest.by_tag(20)?.as_str(), Some("1"));
    // Rule80A A is an agency order.
    assert_eq!(latest.by_tag(528)?.as_str(), Some("A"));
    assert_eq!(latest.by_tag(47)?.as_str(), Some("A"), "the source stays as read");
    // ExecBroker and ClientID are two parties, in tag order, and the
    // counter states the count.
    assert_eq!(latest.by_tag(453)?.as_i128(), Some(2));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[0].partyid")?)?, Scalar::from("BRKR"));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[0].partyrole")?)?, Scalar::from(1));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[1].partyid")?)?, Scalar::from("CLIENT1"));
    assert_eq!(latest.by_path(&FieldPath::from_str("parties[1].partyrole")?)?, Scalar::from(3));
    // LastShares is LastQty, reachable by either spelling.
    assert_eq!(latest.by_tag(32)?, Scalar::from(yggdryl::Decimal::from_int(100)));
    assert_eq!(latest.by_name("LastShares")?, latest.by_tag(32)?);

    // What the message said of itself is its header's; the wire opens with
    // the facts the event holds - the price and quantity ladders, the side,
    // how long it stands - and the restated row follows them.
    assert_eq!(latest.header().beginstring(), "FIX.4.2");
    let wire = latest.into_text('|')?;
    assert!(wire.starts_with("8=FIX.4.2|35=8|6=10.5|14=100|17=E1|31=10.5|32=100|37=O1|"), "{wire}");
    assert!(wire.contains("|528=A|453=2|448=BRKR|452=1|448=CLIENT1|452=3|"), "{wire}");
    ```

=== "Python"

    ```python
    import decimal
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)

    # A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    # naming its broker (76) and client (109), the fill as LastShares (32).
    line = b"8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|"
    latest = next(reader.parse_line(line))

    # ExecTransType Cancel states TradeCancel, but ExecType already stated
    # PartiallyFilled and a stated value stands - so the rule that answers 150 is
    # ExecType's own, folding the partial fill into Trade. The source stays as it
    # arrived.
    assert latest.by_tag(150).as_py() == "F"
    assert latest.by_tag(20).as_py() == "1"
    # Rule80A A is an agency order.
    assert latest.by_tag(528).as_py() == "A"
    assert latest.by_tag(47).as_py() == "A", "the source stays as read"
    # ExecBroker and ClientID are two parties, in tag order, and the counter
    # states the count.
    assert latest.by_tag(453).as_py() == 2
    assert latest.by_path("parties[0].partyid").as_py() == "BRKR"
    assert latest.by_path("parties[0].partyrole").as_py() == 1
    assert latest.by_path("parties[1].partyid").as_py() == "CLIENT1"
    assert latest.by_path("parties[1].partyrole").as_py() == 3
    # LastShares is LastQty, reachable by either spelling.
    assert latest.by_tag(32).as_py() == decimal.Decimal(100)
    assert latest.by_name("LastShares") == latest.by_tag(32)

    # What the message said of itself is its header's; the wire opens with
    # the facts the event holds - the price and quantity ladders, the side,
    # how long it stands - and the restated row follows them.
    assert latest.header().beginstring == "FIX.4.2"
    wire = latest.into_text("|")
    assert wire.startswith("8=FIX.4.2|35=8|6=10.5|14=100|17=E1|31=10.5|32=100|37=O1|")
    assert "|528=A|453=2|448=BRKR|452=1|448=CLIENT1|452=3|" in wire
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { Scalar, fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)

    // A cancelled partial fill (20=1, 150=1) of an agency order (47=A),
    // naming its broker (76) and client (109), the fill as LastShares (32).
    const line = '8=FIX.4.2|35=8|37=O1|17=E1|20=1|150=1|39=1|55=AAPL|54=1|32=100|31=10.5|14=100|151=0|47=A|109=CLIENT1|76=BRKR|10=0|'
    const latest = reader.parseLine(Buffer.from(line)).next().value

    // ExecTransType Cancel states TradeCancel, but ExecType already stated
    // PartiallyFilled and a stated value stands - so the rule that answers 150 is
    // ExecType's own, folding the partial fill into Trade. The source stays as it
    // arrived.
    assert.equal(latest.byTag(150).asJs(), 'F')
    assert.equal(latest.byTag(20).asJs(), '1')
    // Rule80A A is an agency order.
    assert.equal(latest.byTag(528).asJs(), 'A')
    assert.equal(latest.byTag(47).asJs(), 'A', 'the source stays as read')
    // ExecBroker and ClientID are two parties, in tag order, and the counter
    // states the count.
    assert.equal(latest.byTag(453).asJs(), 2)
    assert.equal(latest.byPath('parties[0].partyid').asJs(), 'BRKR')
    assert.equal(latest.byPath('parties[0].partyrole').asJs(), 1)
    assert.equal(latest.byPath('parties[1].partyid').asJs(), 'CLIENT1')
    assert.equal(latest.byPath('parties[1].partyrole').asJs(), 3)
    // LastShares is LastQty, reachable by either spelling.
    assert.ok(latest.byTag(32).equals(Scalar.decimal(100n)))
    assert.ok(latest.byName('LastShares').equals(latest.byTag(32)))

    // What the message said of itself is its header's; the wire opens with
    // the facts the event holds - the price and quantity ladders, the side,
    // how long it stands - and the restated row follows them.
    assert.equal(latest.header().beginstring, 'FIX.4.2')
    const wire = latest.intoText('|')
    assert.ok(wire.startsWith('8=FIX.4.2|35=8|6=10.5|14=100|17=E1|31=10.5|32=100|37=O1|'), wire)
    assert.ok(wire.includes('|528=A|453=2|448=BRKR|452=1|448=CLIENT1|452=3|'), wire)
    ```

### What a held value is, to a rule

A rule reads the held value as the wire spells it and writes wire text, because that is what the specification's appendices are written in. The two meet at the condition and at the target.

| The rule says | Holds when | So |
| --- | --- | --- |
| every value | always: the field itself was retired | `MaxFloor(111)` is `DisplayQty(1138)` whatever it states |
| a code | the held value spells it | `Rule80A(47)` `A` is `OrderCapacity(528)` `A`; a coded value is compared by its wire code, so `ExecType(150)` `1` meets the `1` a partial fill was read as |
| one code among several | a `MultipleCharValue` holds it among its codes | `ExecInst(18)` `G T` meets the rule for `T` |
| a width | the value is no longer than an identifier source may hold | a bridge's `OmsInstrumentId` becomes a `secaltids` occurrence under its own source only where the source can hold it |
| message types, a group | the root's `MsgType(35)` is one of them, the level an occurrence of that group | `AllocTransType(71)` on `J`; `SettlCurrAmt(119)` inside `AllocGrp` |

A boolean value spells no code, so only a rule about every value applies to it. What a rule fills is a constant, the source's own value, another field's value stated at the same level, the wire texts of several fields joined - every part stated, a day spelled with two digits, which is how it completes a month-year - or one occurrence of a repeating group at this level, a member per fill. A value written into a target is re-typed for the target's field through the codec's own text-to-typed reading: a constant `'1'` lands in `PartyRole(452)` as an integer, `'F'` in `ExecType(150)` as the text it is, `'A'` in `OrderCapacity(528)` likewise. A constant written over a multi-valued source replaces the code the condition named - `G T` restated at `T` is `G R`. A value the target cannot hold blocks the entry rather than landing as null.

## Edges

- `get_by_tag(9999)`, an unknown tag -> the root child named `9999` exactly, never `09999`; the miss allocates nothing.
- A tag two fields hold under different names (`OrderQty` and `Quantity`, both 38) -> `by_id` tells them apart, each id reaching its own child; a bare tag reaches one child, the one named by the registry's first holder where the row itself does not carry the tag.
- `by_id` with another name or another tag than the field's -> a miss, because an identifier names the pair exactly and never folds a tag onto a name it does not carry.
- `FixId::of(0, ..)` or `FixId::of(-1, ..)` -> refused, because a definition's tag is positive and 0 marks only an unresolved arrival entry; a message root the codec builds carries no `FIX:branches`, because a message is not a dictionary member.
- A quote stating one side's `BidPx(132)`/`BidSize(134)` or `OfferPx(133)`/`OfferSize(135)` and no `Side(54)` -> `get_side` is `UNKN`, because a side is never read off which of the two a quote states; `book_arrow_reader` reads the sided quotes its parse split off instead, and a quote stating neither side's facts nor a `Side(54)` rests on no side, so the book walk leaves it out with a [warning](capture.md#warnings).
- `by_path("Parties.PartyID")` -> an error; a repeating group is a Serie of Structs, so a member needs the occurrence (`Parties[0].PartyID`), which is the spelling the registry takes too.
- A typed tag stated null at construction, or a holder stating nothing -> the lookup answers nothing: `get_by_tag(54)` on a report stating no side is `None`, `get_by_tag(SEQNUM_TAG_NAME.0)` on a message first at its instant is `None`, and `get_by_tag(STATE_TAG_NAME.0)` on an order that reached no state is `None`.
- `set` with a name nothing reaches -> a typed absence naming the key, and the message unchanged; with a value the field refuses -> the value contract's refusal, and the message unchanged; `set_many` refuses all of its writes on the first refusal.
- `set` on a typed tag with a value its type refuses - text into `OrderQty(38)`, a spelling outside the side's set - is silence: the holder keeps what it held. `set` with a `Null` on a row child -> a stated null, the child kept and made nullable; on a typed tag -> the fact cleared. `remove` -> the child gone or the fact cleared and its value answered, `None` for a key that reaches nothing; a cleared `crosscode` keeps the settled one, because the identity is re-settled from what the message states and the code, once named, stands.
- `FixMsg::new` / `with_registry` on a root stating `SendingTime(52)` -> the header's clock, marked stated; on one stating none -> one UTC-now read, marked not stated, so the wire omits it; a stated clock that is not an instant -> a located refusal from these constructors, where a parse leaves it unstated beside an anomaly.
- `set` twice under one key -> one child, the later value; a bare unknown tag written twice -> one decimal-named child.
- `from_row` on a row whose `fixentries` holds a key that is not `tag:name` - a tag of `0` among them, because a key no dictionary resolved lives in `metadata` - or a value opening as JSON that the JSON reader cannot decode -> refused at `$.fixentries.<key>`; a `metadata` key holding JSON that does not decode -> refused at `$.metadata.<key>`; on a schema without the column -> a message with the typed facts, no content and a wire of the header alone; on a row leaving `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid` or `crossuuid` null -> the schema's refusal, since the fixed row declares them required.
- Two children reaching one field, both stated and different (`lastqty` `50` beside `LastShares` `100`) -> both kept as they arrived; equal once re-typed, or one null -> one child.
- A child named by a tag's digits (`"32"`) that the registry knows -> re-expressed under the registry's field like any other; one it does not know (`"9999"`) -> kept exactly, name, datatype and value.
- A Serie no `FIX:counter` heads, and any nested value that is not a repeating group -> kept exactly; only group occurrences are levels.
- A rule whose target holds a stated current code (`40=A|59=0`, a `TimeInForce` the message chose) -> blocked whole: `OrdType` stays `A` and no later entry answers for it; `59=7`, the rule's own value, is no obstacle.
- A rule that rewrote the source's own value (`ExecInst` `T` -> `R`) -> the new value is restated in turn (`R` is a `PegPriceType`), so one parse reaches what a second would find; a chain is bounded by the rules the field states.
- A `join` with a part unstated (`205=5` and no `200`) or a `from` whose tag is absent -> the entry fills nothing.
- A group fill whose constants match an existing occurrence (`ClearingFirm` made the role-4 party, `ClearingAccount` adds its sub-identifier) -> merged into it and the counter unchanged; a stated occurrence of the same role with another identifier -> the fill is blocked, the occurrence stands.
- A rule scoped to message types on a message stating no `MsgType(35)` -> does not apply; one scoped to a group at the root -> does not apply.
- A removed field the specification replaced with nothing (`SendingDate(51)`) -> kept as read, and the registry still holds it under its own tag.

## Commands

=== "Rust"

    ```bash
    cargo test --features internals -p yggdryl --test fix -- mod_::internal::a_message
    cargo test -p yggdryl --test fix codec::a_message_re_emits_from_its_entries_and_reads_back_equal
    cargo test -p yggdryl --test fix -- msg
    cargo test -p yggdryl --test fix latest::
    cargo test -p yggdryl --test fix latest::a_fix_42_execution_report_restates_at_the_dictionarys_newest_version
    cargo test -p yggdryl --test fix market::
    cargo test -p yggdryl --test fix retired::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_fix.py
    python/.venv/bin/python -m pytest python/tests/test_fix.py -k "message or scalar_value_and_field"
    python/.venv/bin/python -m pytest python/tests/test_fix.py -k latest
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/fix.test.js
    node --test --test-name-pattern="message" node/tests/fix.test.js
    node --test --test-name-pattern="latest" node/tests/fix.test.js
    ```
