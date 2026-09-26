# Message

`FixMsg` is a market event with a FIX body around it: the [event](../graph/index.md) the message is, the standard header typed, what the line's own bridge row header said about the capture it was written for, the free text and a bridge's namespaced keys - each held once, as a typed fact - and the content row, every other field the message states, typed by the registry's field for it. One message is one frame, one bridge row or one document - never the line, which can carry [several of them or none](decode.md#a-line-yields-none-one-or-many-messages).

## Contract

| item | contract |
| --- | --- |
| Holders | the event the message is, read through the four [graph traits](../graph/index.md#traits) - `Element`, `Event`, `Market` and `Operation`: the identities, the codes, the instants, the place in the chain, and the market reading *derived* from the FIX fields the message stated: the price, the quantity, the state, the side, the lanes, the security identifiers and the three [identifier maps](#the-identifier-maps); `lifted() -> &FixLifted`, the FIX numbers and identifiers the message lifted out of its row, exactly as it stated them - a fact the traits answer but this holder lacks is derived, and a derived fact reaches neither the wire, the entries nor the code; `header() -> &FixHeader`, the frame: tags 8, 35, 49, 56, 34, 52, 43, 385 and the trailer 93, 89, 10 typed; `capture() -> &FixCapture`, `msgpluginid`, `msgctxid` and `msgsessionid` - what a bridge's own row header stated, read off the line's own bytes, and never what a *reader* said about the line - the `msgsesseventid` the message derives from them, and the `msgoriginator` and `conversationid` a bridge's log line names; `text() -> Option<&str>`, `Text(58)`; `metadata() -> &BTreeMap<SmolStr, SmolStr>`, what a bridge stated under its own namespaces - `TECH.CLIENTID`, `firm.acronym` - each under the key as the bridge spelled it, folded |
| Row | `as_field()` and `as_value()`: a root Struct [`Field`](../types/field.md) and the `Scalar::Serie` it declares, holding only what no holder owns - the dictionary's fields, a group as a Serie of Struct occurrences beside its `int32` counter, a component as a Struct, a key no dictionary explains under its own spelling; a [typed tag](#typed-tags) is never in it |
| Entries | `entries() -> &[FixEntry]`, the row read as a tree, derived on the first ask and dropped by every write: one entry per non-null child, each carrying the tag the dictionary resolved - `0` for a key it does not explain - the canonical name and the value as the wire spells it; a group is one entry under its counter valued the count, with one valueless entry per occurrence heading the members; a component a valueless entry heading its members; the typed facts are not entries |
| Wire | `into_bytes(separator)` and `into_text(separator)` re-emit what the message *stated*: the header tags 8, 35, 49, 56, 34, 43 and 52 - the last only where the message stated it - then the fields it lifted in tag order - 6, 11, 14, 17, 31, 32, 37, 38, 41, 44, 53, 117, 131, 151, 198, 262 and 1003 - then 58, then the entries pre-order, then the trailer 93, 89 and 10, which closes the frame whatever the body's tags are. A fact the message *derived* - the price it is about, the state it reached, a lane it never quoted - is emitted nowhere. A coded fact spells as its wire code, `54=1`, and a lane number at the decimal's full scale. `digest() -> u128` is the XXH3-128 of what `into_bytes` emits, whatever separator |
| Constructors | `FixMsg::new` links `FixRegistry::global()`; `FixMsg::with_registry` keeps the `Arc` it is given, lifts every typed fact out of the children that state it, settles the clocks and derives the identity, and runs no derivation - a [parse](capture.md#a-reader-is-the-whole-parse-surface) does; `FixMsg::from_row` reads a [fixed row](#a-row-is-a-message-again) back, entries included |
| Lookups | every one answers an owned `Scalar`: a typed tag its holder's fact, or nothing where the holder states none; any other key the row child it reaches |
| Writes | `set`, `set_many`, `with_value` and `remove`: a key reaching a typed fact writes its holder, a `Null` clearing it; a key reaching [the capture's own column](#a-row-is-a-message-again) - `sourceurl` - is refused, naming the column, because a message holds no fact for it; any other key [writes the row](#written-into-the-row), typed through the registry's field; every write settles the identity again; a refusal leaves the message unchanged. `set_many` and `with_value` are Rust-only |
| Settled | `currunix` is a stated `currunix`, else the [official clock](capture.md#the-official-clock-dates-the-message) standing within the codec's `official_time_delay_ms` of `SendingTime(52)` - the `TransactTime(60)` the message states, else the `TrdRegTimestamp(769)` its `TrdRegTimestampType(770)` says is about the event or a hop - else that `SendingTime`; `SendingTime` is the message's own, else a row cell or line capture reaching tag 52, else the `currunix` of the [text line](../media/index.md#plain-text) it was read out of, else the codec's `default_sending_time`, else one UTC-now read at intake, and only a stated one is a fact of the message - it goes back on the wire and into a row, while a stand-in intake supplied does neither; `creaunix` is a stated one, else `currunix`; `execunix` is the most precise execution clock the message states, else `currunix` where `FixMsg::is_execution` accepts the report and it follows nothing; the [identity](../hashing.md) - `crosscode`, `crosshashcode`, `currhashcode`, `curruuid`, `crossuuid` - is derived from what the message *states* but the standard header and trailer, less `MsgType(35)`, and never the chain it is in: the event facts, text, metadata, lifted FIX fields and canonical entry tree. A complete nonempty `msgtype`, capture `msgsessionid` and capture `msgctxid` with a present `msgseqnum` are also settled as the capture's `msgsesseventid`, the four values joined by `:` - `"<msgtype>:<msgsessionid>:<msgctxid>:<msgseqnum>"`, as `8:e7256476:9effef3e6a:1094`; the sequence is canonical `u64`, an absent part removes it, and this delivery identity is excluded from the FIX content identity. An explicit nonempty `crosscode` wins; otherwise the first nonempty `OrderID(37)`, `ClOrdID(11)`, `OrigClOrdID(41)`, `QuoteID(117)`, `QuoteReqID(131)` or `MDReqID(262)` names the chain. |
| Identity | a field is its tag and its name; a message speaks no dialect and carries no membership, so a bare tag or name resolves in the registry's [one namespace](#one-namespace) |
| Graph | `FixMsg` implements `Element`, `Event`, `Market` and `Operation` through its event: `is_after` is the instant, `finalize` settles the identity again, which re-derives every market fact from the FIX fields the message states unless a caller or a walk wrote one through the traits. `with_previous` first compares the complete `msgsesseventid`; equality forces a full content-and-graph merge instead of following, with the latest recorded observation as reference and the earliest `recdunix` and `execunix` retained. Otherwise it is `Market::following_market`, which records the predecessor's `curruuid` and `currunix` as `prevuuid` and `prevunix`, puts the message one place after it, `seqnum`, and carries the market facts the chain is about - not the operation's: the time in force, the accounts, the users and the followed alternate identifiers stay this message's own. `merge_with` is `Market::merging_market_event` under the same recording-clock rule, and `restating` the operation restatement, `graph::market::restating_operation`; import the traits to call them |
| Market operations | `market_operations(&self) -> Result<Vec<MarketData>>` reads this message as the [values a book folds](../graph/index.md#book-fold) - an `OrderEvent`, a `QuoteEvent`, an `ExecutionEvent`, a `TradeEvent` or a `SnapshotEvent`; `into_market_operations(self)` moves a direct message's held root event; `FixMarketIterator` streams sorted messages without allocating a vector for direct messages; `FixCodec::book_arrow_reader` composes that iterator with `BookIterator` and bounded Arrow output; `TryFrom<FixMsg> for MarketData` requires exactly one result. Order and quote categories are direct, an actual `EXEC` is one execution, and only an initial TradeCaptureReport `35=AE` whose `TradeReportTransType(487)` is absent/New and whose `ExecType(150)` is absent or execution-like becomes one composite trade built from `NoSides(552)`; non-New/cancel/correct/reverse/status AE are refused. The composed book reader skips administration, requests, acknowledgements and non-executing reports; standalone conversion remains strict. `W` / `X` expand `NoMDEntries(268)` in nondecreasing effective time, retaining source order for ties. Every leaf carries in its metadata what its message states that no typed column reads - [the rule](#market-operations) - and the map is part of the leaf's digest; `FixCodec::market_operations` is the [sorted door](arrow.md#fix-market-books) over a whole capture. The operation conversion is Rust-owned and the composed Arrow readers are exposed by Python and JavaScript |
| Serialization | inherited: `as_field().clone().into_json()` renders the row's schema, [`into_json_scalar`](../media/index.md#json) its value, `from_json_scalar_with_field` reads it back typed, ordered and canonicalized against the same root; `into_row` is the whole message as one fixed row |
| Equality | over the holders, the row and the registry - the same `Arc`, or registries holding the same fields; `Hash` over the hashcode, the row's schema and its value |
| Bindings | The message is Rust, Python and JavaScript; Python `FixCodec.book_arrow_reader` and JavaScript `FixCodec.bookArrowReader` redirect into the complete Rust FIX-to-book Arrow pipeline, and `market_operations`, `market_arrow_reader`, `market_operations_arrow_reader` - `marketOperations`, `marketArrowReader`, `marketOperationsArrowReader` - into the sorted market door, the metadata switch being Python's `market_metadata=` and JavaScript's `{ marketMetadata }` |

## Market operations

`market_operations()` is the borrowed door and clones the one base event; `into_market_operations()` is the consuming door and moves that root holder without cloning for one direct or composite operation. The projected generic event is finalized at that boundary, so its identity derives only from the facts the operation schema carries and it can pass directly through the graph Arrow round trip; the FIX message keeps its richer FIX-content identity before conversion. Its `marketoperationid` is the `int32` code read from `msgcatcodeset`: `10` (`ORDR`) becomes an `OrderEvent`, `14` (`QUOT`) a `QuoteEvent`, and `8` (`EXEC`) an `ExecutionEvent` only where `Event::is_execution()` says this observation itself is an execution. Category `21` (`TRAD`) becomes a composite `TradeEvent` only for an initial TradeCaptureReport `35=AE`: `TradeReportTransType(487)` must be absent or New (`0`, with legacy `N` or `New`), and `ExecType(150)` must be absent or execution-like (`F`, with legacy `1` or `2`). A non-New or cancel/correct/reverse/status AE, every `AD`, `AQ` and `AR`, another category, a non-executing execution message, or a `BOOK` message other than `W` or `X` is an `InvalidRecord` at `MsgType(35)`. `TryFrom<FixMsg> for MarketData` uses the consuming door and refuses unless it yields exactly one value.

`FixCodec::book_arrow_reader` admits order and quote categories, actual executions, `W`/`X` and every `AE`, ignoring the remaining records before strict projection. An unsupported correction, cancellation or status `AE` still raises its named refusal, as does any malformed admitted message; source errors are preserved. Ignored records neither advance book time nor trigger ordering errors. Lifecycle enrichment remains explicit.

An accepted initial `35=AE` must state exactly one nonempty `NoSides(552)` group. Each occurrence becomes one `ExecutionEvent` and must state `Side(54)` as bid or ask; absence or another value is refused at `$.NoSides(552)[index].Side(54)`. `SideLastQty(1009)` supplies that child's `lastqty` and quantity, otherwise root `LastQty(32)` supplies the quantity where present. Root `LastPx(31)` remains the shared price; `SideAvgPx(1852)` records the child's average price and supplies its price only when the root stated no last price. `SideCurrency(1154)` may replace the root currency. The child keeps the root's [identifier maps](#the-identifier-maps) and adds its side plus any `SideExecID(1427)`, `SideTradeReportID(1005)`, `SideTradeID(1506)`, `SideOrigTradeID(1507)`, `OrderID(37)`, `SecondaryOrderID(198)`, `ClOrdID(11)`, `SecondaryClOrdID(526)` and `OrigClOrdID(41)` it states, each an alternate identifier under the upper-cased field name - `SIDEEXECID`, `SECONDARYCLORDID` - replacing the root's entry under that key. No `TradeSideIndex` identifier is emitted. A child cross code uses its order/client chain where available and a tag-qualified first `SideExecID`, `SideTradeID`, `SideTradeReportID`, `OrderID` or `ClOrdID`; when none is stated it uses the canonical 128-bit digest of that occurrence's content. The source group index never participates in the identity. [`TradeEvent::from_parts`](../graph/index.md#trades) finalizes and canonically sorts the children, folds their temporal bounds into the trade root, and feeds each canonical child UUID once into that root's identity. The graph [Arrow row](../graph/index.md#arrow) carries those children in its nullable `executions` list; a [book](../graph/index.md#book-sides-and-books) flattens them into its own execution list and never treats the composite as resting depth.

For `W` and `X`, the converter derives `entries()` once, requires exactly one `NoMDEntries(268)` group, walks its occurrences once, stably sorts the projected operations by effective instant, and returns one operation per occurrence. An empty `W` returns one scoped `SnapshotEvent` - built by `SnapshotEvent::snapshot(&event, scope)` - so an authoritative empty book can clear stale depth; an empty `X` states no changes and returns none. Root context is inherited and an occurrence's stated value overrides it. Root `MDReqID(262)` is taken from its lifted message field because lifting deliberately removes it from the residual entry tree. The entry's own facts are written per occurrence - its `MDENTRYID`, `MDENTRYREFID` and `ORDERID` alternate identifiers replaced under their keys, its book control built afresh - so a missing `MDEntryID`, reference or coordinate cannot leak from one occurrence into the next while unrelated caller identifiers survive. The entry type decides the concrete operation:

| `MDEntryType(269)` | Operation | Side |
| --- | --- | --- |
| `0` bid | an `OrderEvent` where that occurrence states `OrderID(37)`, otherwise a `QuoteEvent` | `BUY` |
| `1` offer | an `OrderEvent` where that occurrence states `OrderID(37)`, otherwise a `QuoteEvent` | `SELL` |
| `2` trade | an `ExecutionEvent` | the occurrence's `Side(54)`, otherwise `UNKNOWN` |

`MDEntryPx(270)` and `MDEntrySize(271)` are read from the already validated typed group values into exact [`Decimal`](../types/numeric/decimal.md#decimal) values, and an absent one states none - an entry stating no price rests at its side's [unpriced level](../graph/index.md#limits); an explicitly typed value outside that range is a located refusal, never a fabricated zero. `MDEntryDate(272)` and `MDEntryTime(273)` use the same typed path rather than reparsing rendered text: an `X` occurrence is dated at that exact nanosecond, while a `W` group remains atomic at the message's `currunix` and retains the entry clock as `creaunix`, or as `execunix` for a trade. A missing date uses the UTC day containing the message timestamp. A missing or unsupported entry type, a malformed decimal, an invalid typed entry clock, a missing incremental action or an action outside `0` through `5` is a located `InvalidRecord` naming the group index and tag. The update action is retained as the operation's book control - `OperationEvent::book()`, a `BookRef` whose `action` is the `MdUpdateAction` - and determines its lifecycle state:

| Message/action | Reading | State and depth effect |
| --- | --- | --- |
| `W` | `MdUpdateAction::Snapshot` | `New`; before the group is applied, a [book](../graph/index.md#book-sides-and-books) clears only the same scope on both sides; zero entries is one control and no invented depth |
| `X`, `0` | new | `New`, live |
| `X`, `1` | change | `Replaced`, live |
| `X`, `2` | delete | `Canceled`, terminal |
| `X`, `3` | delete through | `Canceled`, terminal; for a bid or offer, removes same-symbol, same-scope live entries through a required positive, in-range one-based `MDEntryPositionNo(290)` |
| `X`, `4` | delete from | `Canceled`, terminal; for a bid or offer, removes same-symbol, same-scope live entries from that required position |
| `X`, `5` | overlay | `Replaced`, live |

The book scope - `BookRef::scope`, what `OperationEvent::scope()` answers - is deterministic: `Symbol=<Symbol(55)>`, or the event's ticker, or `GLOBAL`, followed where stated by `MDBookType(1021)`, `MDSubBookType(1173)`, `MDFeedType(1022)`, `MDStreamID(1500)`, `MarketID(1301)`, `MarketSegmentID(1300)`, `MDReqID(262)` and `MarketDepth(264)`, in that order. `%`, `|` and `=` in external values are percent-escaped before those separators are written, so distinct tuples cannot concatenate alike. The occurrence inherits those facts from the message root and may replace them. Its cross code is that complete scope plus `MDEntryID(278)`, or the same qualified ID named by `MDEntryRefID(280)`. Without either ID it is the scope, `MDEntryType`, and the stated position and/or price level; a new or snapshot entry with neither coordinate adds its price, while a change, delete or overlay without any stable ID/position/level is refused as ambiguous. IDs reused by two feeds or symbols therefore never collide, and an anonymous price change retains the position/level identity it updates.

The book control retains the effective action (`Snapshot` for `W`, the wire code for `X`), the scope, `MDEntryPositionNo(290)` as `position`, and `MDEntryPx(270)` and `MDEntrySize(271)` as the entry stated them - `entry_px` and `entry_size`, stated only where the occurrence stated them, so presence is distinct from a numeric zero: change and overlay actions inherit either value the occurrence omitted from the matched live entry, and refuse an omitted value where no predecessor exists. The entry's own and referenced identifiers and the order it names are alternate identifiers - `MDENTRYID` and `MDENTRYREFID` (`graph::book::ENTRY_ID`, `ENTRY_REF_ID`) and `ORDERID`; `MDPriceLevel(1023)` takes part in the cross code of an entry naming no identifier and in nothing else, and `RptSeq(83)` and the `ApplID` sequences are no fact of the operation. A stated `MDEntryRefID` is resolved before the incoming `MDEntryID`; two distinct live entries at those identities are ambiguous and refused. This gives [the book](../graph/index.md#book-sides-and-books) its matching, ordering and scope facts without a second FIX schema. Trade occurrences become executions beside the book and never decrement resting order or quote depth.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Market, MarketKind, Operation};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // A new order is one order event, its facts the message's own.
    let order = codec
        .parse_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30|11=A|55=IBM|54=1|38=10|44=101.5|15=USD|10=0|")?
        .next()
        .expect("one frame")?;
    let operations = order.market_operations()?;
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].kind(), MarketKind::OrderEvent);
    let leaf = operations[0].as_order_event().expect("an order event");
    assert_eq!(leaf.get_price(), order.get_price());
    assert_eq!(leaf.get_bid(), order.get_bid());
    assert_eq!(leaf.get_crossuuid(), order.get_crossuuid());

    // A full snapshot is one operation per NoMDEntries(268) occurrence.
    let book = codec
        .parse_line(b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|")?
        .next()
        .expect("one frame")?;
    let kinds: Vec<MarketKind> = book.market_operations()?.iter().map(|value| value.kind()).collect();
    assert_eq!(kinds, [MarketKind::QuoteEvent, MarketKind::QuoteEvent]);
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl import graph
    from yggdryl.fix import FixCodec, FixRegistry

    codec = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))

    # A new order is one order event, its facts the message's own.
    order = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30|11=A|55=IBM|54=1|38=10|44=101.5|15=USD|10=0|")
    [operation] = order.market_operations()
    assert isinstance(operation, graph.MarketData) and operation.kind == "order_event"
    leaf = operation.as_order_event()
    assert leaf is not None
    assert leaf.price == order.price
    assert isinstance(order.bid, graph.Lane) and leaf.bid == order.bid
    assert leaf.crossuuid == order.crossuuid

    # A full snapshot is one operation per NoMDEntries(268) occurrence.
    book = codec.parse_fix_line(
        b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|"
    )
    assert [value.kind for value in book.market_operations()] == ["quote_event", "quote_event"]
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
    const [operation] = order.marketOperations()
    assert.ok(operation instanceof graph.MarketData)
    assert.equal(operation.kind, 'order_event')
    const leaf = operation.asOrderEvent()
    assert.equal(leaf.price, order.price)
    assert.ok(order.bid instanceof graph.Lane && leaf.bid.equals(order.bid))
    assert.equal(leaf.crossuuid, order.crossuuid)

    // A full snapshot is one operation per NoMDEntries(268) occurrence.
    const book = codec.parseFixLine(Buffer.from(
      '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
    ))
    assert.deepEqual(book.marketOperations().map((value) => value.kind), ['quote_event', 'quote_event'])
    ```

### What a leaf's metadata holds

Every leaf carries, in its [`Market::get_metadata`](../graph/index.md#market), what its message states that no typed column reads, each value the canonical text it spells - a decimal at the scale it is stored at: the bridge's namespaced keys as they are (`tech.clientid`), then every other top-level field under its folded name (`ordtype`), and a group or component member by member under its [`FieldPath`](../types/paths.md) text, the group spelled by its column - `parties[0].partyid`. Left out are the envelope a message's code leaves out, so two hops of one message are one leaf; every tag a typed column reads; an identifier-map source, and a `parties` occurrence whose role an identifier map reads, consumed whole; the fields read by name - `eventtimestamp`, the detailed CFI, the security-type names; a group's counter; and the group a message expands into its leaves, `NoMDEntries(268)` for `W` and `X`, `NoSides(552)` for `AE`. A book entry and a trade side add their own occurrence's members keyed bare, each leading a message field of the same name, and never a sibling's.

The map is computed where the leaf is built and never stored on the `FixMsg`, so it feeds the leaf's digest and never the message's code. `market_operations` and `into_market_operations` always carry it; a codec's `with_market_metadata(false)` turns it off for the codec's own doors - `market_operations`, `market_arrow_reader`, `book_arrow_reader` - which moves the identity of every leaf whose message states such a field.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Market};
    use yggdryl::local::LocalFolder;
    use yggdryl::{FixCodec, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?));

    // OrdType(40) and ExecInst(18) are no typed column, and a PartyRole(452)
    // of 11 names no identifier map; the rest of the order is typed.
    let line: &[u8] = b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|453=1|448=TRADER1|447=D|452=11|10=0|";
    let message = codec.parse_line(line)?.next().expect("one frame")?;
    let leaves = message.market_operations()?;
    let keys: Vec<&str> = leaves[0].get_metadata().keys().map(|key| key.as_str()).collect();
    assert_eq!(
        keys,
        ["execinst", "ordtype", "parties[0].partyid", "parties[0].partyidsource", "parties[0].partyrole"]
    );
    assert_eq!(leaves[0].get_metadata().get("ordtype").map(|value| value.as_str()), Some("2"));

    // The codec's switch leaves the map empty, and the leaf's identity says so.
    let bare = codec.clone().with_market_metadata(false);
    assert!(!bare.market_metadata());
    let plain = bare
        .market_operations([bare.parse_line(line)?.next().expect("one frame")?])
        .next()
        .expect("one leaf")?;
    assert!(plain.get_metadata().is_empty());
    assert_ne!(plain.get_curruuid(), leaves[0].get_curruuid());
    assert_eq!(plain.get_crosscode(), leaves[0].get_crosscode(), "and never its chain");
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    codec = FixCodec(registry)

    # OrdType(40) and ExecInst(18) are no typed column, and a PartyRole(452)
    # of 11 names no identifier map; the rest of the order is typed.
    line = b"8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|453=1|448=TRADER1|447=D|452=11|10=0|"
    [leaf] = codec.parse_fix_line(line).market_operations()
    order = leaf.as_order_event()
    assert order is not None
    assert order.metadata == {
        "execinst": "G",
        "ordtype": "2",
        "parties[0].partyid": "TRADER1",
        "parties[0].partyidsource": "D",
        "parties[0].partyrole": "11",
    }

    # The codec's switch leaves the map empty, and the leaf's identity says so.
    bare = FixCodec(registry, market_metadata=False)
    assert not bare.market_metadata
    [plain] = bare.market_operations([bare.parse_fix_line(line)])
    held = plain.as_order_event()
    assert held is not None and held.metadata == {}
    assert plain.curruuid != leaf.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const codec = new fix.FixCodec(registry)

    // OrdType(40) and ExecInst(18) are no typed column, and a PartyRole(452)
    // of 11 names no identifier map; the rest of the order is typed.
    const line = Buffer.from(
      '8=FIX.4.4|35=D|52=20260921-10:00:00|11=C1|55=AAPL|54=1|44=100.5|38=5|40=2|18=G|453=1|448=TRADER1|447=D|452=11|10=0|',
    )
    const [leaf] = codec.parseFixLine(line).marketOperations()
    const metadata = leaf.asOrderEvent().metadata
    assert.deepEqual(Object.keys(metadata), [
      'execinst', 'ordtype', 'parties[0].partyid', 'parties[0].partyidsource', 'parties[0].partyrole',
    ])
    assert.equal(metadata.ordtype, '2')

    // The codec's switch leaves the map empty, and the leaf's identity says so.
    const bare = new fix.FixCodec(registry, { marketMetadata: false })
    assert.equal(bare.marketMetadata, false)
    const [plain] = [...bare.marketOperations([bare.parseFixLine(line)])]
    assert.equal(Object.keys(plain.asOrderEvent().metadata).length, 0)
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
    assert_eq!(msg.get_side().as_str(), "BUY");
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
    assert message.side.as_py() == "BUY"
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
    assert.equal(message.side, 'BUY')
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
| every [crate tag](capture.md#the-crates-own-columns), 65000 to 65099, but `sourceurl` (65026) | the [event](../graph/index.md#event) getters and `capture()` | the identities, the codes, the instants, the place in the chain, the state it reached and when it expires on the event; `msgpluginid`, `msgctxid`, `msgsessionid`, the `msgsesseventid` they join to, `msgoriginator` and `conversationid` on the capture; `metadata` is the bridge's namespaced keys; the names a message goes by are its [identifier maps](#the-identifier-maps), read off the FIX fields that state them and no column of their own. The one exception is [the capture's own column](#a-row-is-a-message-again): no holder answers it, so `get_by_tag(SOURCEURL_TAG_NAME.0)` is a miss on every message |
| 58 | `text()` | the free text |

Every market fact is *not* here. `Currency(15)`, `Side(54)`, `CFICode(461)`, the lanes 132 to 135, `SecurityID(48)` under its source, the `secaltids` group and the market are ordinary children of the row, and what a [`Market` or `Operation`](../graph/index.md) getter answers is derived from them and from the lifted numbers - `get_price` is `Price(44)`, else the price its own side's lane quotes, and `None` otherwise - never `LastPx(31)` or `AvgPx(6)`, which are the last executed and the average price and answer as `get_lastpx` and `get_avgpx`; `get_spotrate` and `get_forwardpoints` are `LastSpotRate(194)` and `LastForwardPoints(195)`, and each lane's are its own four; `get_securityids` is built at every settle, each source filling only the keys the ones before it left absent: `SecurityID(48)` under `SecurityIDSource(22)`, each `secaltids` occurrence read through `SecType::read`, the crated `isincode`, `bloombergcode` and `figicode` views a row stated, then every top-level field no dictionary maps whose name names a source through `SecType::from_field_name` - `ISIN`, `cusip_code`, `#SEDOLCODE` - trimmed, validated and left on the wire as it arrived; a value that is empty or null-like states nothing, and an invalid code, or a different code under a filled key, states nothing and is kept as an [anomaly](#anomalies). The first identifier under a key ending `INSTRUMENTID` whose value after its last `;` is shaped `{ISIN}_{MIC}_{CCY}` - twelve, four and three ASCII alphanumerics, `dbi;CH0012214059_XSWX_CHF` - is a bridge's instrument key: it fills the `ISIN` and `get_currency` only where nothing stated them and names the market `get_miccode` ranks after `LastMkt(30)` and `ExDestination(100)` and before `SecurityExchange(207)`, a part its own type refuses skipped, and nothing it fills reaches the wire; `get_bid` and `get_ask` are the two `Lane`s the lanes 132 to 135 state - and a quote stating one lane, no `Side(54)` and no price, `LastPx` or `AvgPx` of its own takes that lane's side, `BUY` for `BidPx(132)`/`BidSize(134)` alone and `SELL` for `OfferPx(133)`/`OfferSize(135)` alone, so its price, quantity and currency read off that lane. A derived market fact is the traits' to answer and nobody's to emit: it reaches no column, no entry, no byte on the wire and no input to the code the message digests to. Four readings are event facts with [columns of the crate's own](capture.md#the-crates-own-columns): `state` (65052) and `exprtime` (65053), which a walk folds forward; `execunix` (65062), the lifecycle's latest precise execution clock, which a report `FixMsg::is_execution` accepts takes from its own `currunix` when it settles stating none and follows nothing; and the observation's own `recdunix` (65063). Duplicate observations of one event merge `execunix` and `recdunix` to their earliest precise values before lifecycle following carries the latest execution forward, and the later `recdunix` of the two decides which one is the reference. A row stating one is the row's word; none reaches the wire or the entries.

A typed fact answers as its column types it: `by_tag(35)` is text, `by_tag(52)` a `datetime64(ns, UTC)`, `by_tag(54)` the side's name - `BUY`, `SELL` - `by_tag(CURRHASHCODE_TAG_NAME.0)` a `UInt64`, `by_tag(CURRUUID_TAG_NAME.0)` a `Uuid`, `by_tag(METADATA_TAG_NAME.0)` a sorted map; a holder stating nothing - a side of `UNKNOWN`, a place of zero in no chain - answers nothing, and a state of `00UNKNOWN` answers as it is, the state stated as none, so `by_tag(STATE_TAG_NAME.0)` is never a miss and the `state` column never null on a row a message wrote.

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
| `header`, `capture`, `text`, `metadata` | the holders themselves, borrowed without a lookup; the event's facts are the trait getters, and the leaves a message expands to are [`market_operations`](#market-operations) |

Every lookup answers an owned `Scalar`: a holder's fact is rendered into the column's type on the way out, and a row child cloned.

## Anomalies

A value that will not type is null in the row rather than a failure, a counter that disagrees with the group it counts is kept as it arrived, and a settle drops a stated identifier that conflicts with a stated one - a security identifier, or a second value under one [identifier-map](#the-identifier-maps) key. A bridge's line adds its own: a `#`-marked twin stating another value than the bare key beside it, a `CONVERSATIONID` its `{conversationId: ...}` contradicts, a MIC alias that is no ISO 10383 code, and - when two observations of one delivery merge - a later one naming another `msgoriginator` or `conversationid` than the earlier, which keeps its own. Each is a fact about the message worth more than a null nobody can explain. `anomalies()` reads them off the message beside the row, in arrival order - the parse's first, then what the last settle dropped - each a `FixAnomaly` of the field it was stated under and the reason. Never a column, never part of the code the message digests to; two statements of one message merge them as a union, the reference's first.

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

The security-identifier verbs write FIX's own fields through one writer, `sync_security_id`: `set_securityids`, `insert_securityid` and `remove_securityid` write `SecurityID(48)` in place where `SecurityIDSource(22)` already names the key, else the `secaltids` occurrence under the key's source code - `4` for `ISIN`, `1` for `CUSIP`, `2` for `SEDOL`, `A` for `BLOOMBERG`, `S` for `FIGI`, and a key with no code under its own upper-cased name. The group's canonical name is `secaltids` and its display is `SecAltIDs`; setting a value replaces that source's occurrence or appends one, removing a key removes only that source, occurrences under every other source remain in place, and `NoSecurityAltID(454)` stays synchronized with the resulting group rather than becoming a second count. `derive_securityid` fills the event alone and never the wire. The [identifier-map verbs](#the-identifier-maps) write the field a key is read from - `insert_altid("ORDERID", ..)` writes `OrderID(37)` - and a key no field states is a located refusal.

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

A written value is then [restated](#restated-under-the-dictionary) exactly as a read one is: `set(47, 'A')` writes the `OrderCapacity(528)` that replaced `Rule80A`, and `set(76, 'BRKR')` makes the `Parties` occurrence `ExecBroker` became, from the [specification's retirements](registry.md#what-the-specification-retired) or from the rule the registry states on the field. What runs is the rules of the tags written, so a write of a tag no rule speaks for is the write and nothing more. The pass never overwrites a stated value and is idempotent, so writing one value twice writes its replacement once, and a caller who stated the replacement keeps what they stated.

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
    assert_eq!(message.remove(54)?.as_ref().and_then(Scalar::as_str), Some("BUY"));
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
    assert message.remove(54).as_py() == "BUY"
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
    assert.equal(message.remove(54).asJs(), 'BUY')
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

A message goes by the names the fields of its dictionary state, and the three [`IdMap`](../graph/index.md#operation)s `Operation` answers - `get_accountids()`, `get_userids()` and `get_altids()` - are rebuilt from those fields at every settle: a view of the row, never a store of its own. Which field states which key is a fact about the field, so it travels on the field as its [`FIX:idmap`](registry.md#a-field-names-a-message-by-its-identifiers) document, and `FixRegistry::idmap_sources` compiles every field's once into the table a settle reads. The dictionary ships this one:

| Map | Key | Source |
| --- | --- | --- |
| `accountids` | `ACCOUNT`, `OMSDEALERACCOUNT` | `Account(1)`, the crate's `omsdealeraccount` (65068) |
| `accountids` | `CUSTOMERACCOUNT` | the `PartyID(448)` of every `Parties` occurrence whose `PartyRole(452)` is `24` |
| `userids` | `SENDERSUBID`, `ONBEHALFOFSUBID`, `OMSUSERID` | `SenderSubID(50)`, `OnBehalfOfSubID(116)`, the crate's `omsuserid` (65069) |
| `userids` | `ENTERINGTRADER`, `EXECUTINGTRADER` | the `PartyID(448)` of every `Parties` occurrence whose `PartyRole(452)` is `36` or `12` |
| `altids`, followed | `ORDERID`, `SECONDARYORDERID`, `PARENTORDERID`, `PARENTCLORDID`, `OMSDEALERPARENTORDERID`, `EXCHANGECLIENTORDERID`, `TRANSVERSALKEY` | `OrderID(37)`, `SecondaryOrderID(198)`, the crate's 65070 to 65074 |
| `altids` | `CLORDID`, `ORIGCLORDID`, `EXECID`, `TRDMATCHID`, `QUOTEID`, `QUOTEREQID`, `MDREQID`, `TRADEID`, `ULTRADERCLORDID` | `ClOrdID(11)`, `OrigClOrdID(41)`, `ExecID(17)`, `TrdMatchID(880)`, `QuoteID(117)`, `QuoteReqID(131)`, `MDReqID(262)`, `TradeID(1003)`, the crate's `ultraderclordid` (65075) |

The eight crate fields are a bridge's own identifiers - `OMSDEALERACCOUNT=`, `PARENTORDERID=` - content the row keeps under its [entries](capture.md#the-crates-own-columns) rather than a column of the fixed row. The first value a key is stated with fills it; a later different one - a second `Parties` occurrence under one role naming another trader - states nothing and is kept as an [anomaly](#anomalies), `parties: states ENTERINGTRADER:t2 where ENTERINGTRADER:t1 is already stated`.

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
    assert_eq!((*tag, parent.map(), parent.follows()), (65_070, FixIdMapKind::Alts, true));

    let reader = FixCodec::new(Arc::clone(&registry));
    let held = reader.parse_fix_line(
        b"8=FIX.4.4|35=8|17=E1|37=O1|OMSDEALERACCOUNT=ACC1|OMSUSERID=trader1|PARENTORDERID=P1|10=0|",
    )?;
    assert_eq!(held.get_accountids().get("OMSDEALERACCOUNT"), Some("ACC1"));
    assert_eq!(held.get_userids().get("OMSUSERID"), Some("trader1"));
    assert_eq!(held.get_altids().get("PARENTORDERID"), Some("P1"));
    assert_eq!(held.get_altids().get("ORDERID"), Some("O1"));
    ```

=== "Python"

    ```python
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    assert {
        "tag": 65070, "map": "altids", "key": "PARENTORDERID", "follow": True, "role": None
    } in registry.idmap_sources()

    held = FixCodec(registry).parse_fix_line(
        b"8=FIX.4.4|35=8|17=E1|37=O1|OMSDEALERACCOUNT=ACC1|OMSUSERID=trader1|PARENTORDERID=P1|10=0|"
    )
    assert held.accountids == {"OMSDEALERACCOUNT": "ACC1"}
    assert held.userids == {"OMSUSERID": "trader1"}
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
      { tag: 65070, map: 'altids', key: 'PARENTORDERID', follow: true },
    )

    const held = new fix.FixCodec(registry).parseFixLine(Buffer.from(
      '8=FIX.4.4|35=8|17=E1|37=O1|OMSDEALERACCOUNT=ACC1|OMSUSERID=trader1|PARENTORDERID=P1|10=0|',
    ))
    assert.deepEqual(held.accountids, { OMSDEALERACCOUNT: 'ACC1' })
    assert.deepEqual(held.userids, { OMSUSERID: 'trader1' })
    assert.deepEqual(held.altids, { EXECID: 'E1', ORDERID: 'O1', PARENTORDERID: 'P1' })
    ```

A value that states nothing - empty, `null`, `none`, `n/a`, `[n/a]` - adds nothing. A write goes through the field a key is read from: `insert_accountid`, `insert_userid` and `insert_altid` fill only an absent key, writing its field and answering `false` for a held one; `remove_*` nulls the field; `set_accountids`, `set_userids` and `set_altids` write every entry that moved and null every key the new map lacks; each settles the identity again, and the map is rebuilt from the row on the way out, so what a map answers is always what the row states. A key no top-level field states - a party-role key such as `CUSTOMERACCOUNT`, or a name the table lacks - is a located `InvalidRecord` at `$.altids.<KEY>` (`$.accountids.<KEY>`, `$.userids.<KEY>`) on every write that would have to land it or null it, because a map that is a view of the row cannot hold what the row cannot say; a graph leaf - an `OrderEvent` - accepts any key. The [lifecycle](lifecycle.md#a-chain-is-named-by-its-cross-code) joins a chain by `altids`, and an operation that follows another carries its accounts, its users and the alternate identifiers whose `FIX:idmap` entry follows - the order's own and its parents', `graph::FOLLOWED_ALTIDS` by default and `Operation::is_followed_altid` for a message - never an execution's or a quote's. The maps are no column of the [fixed row](capture.md#the-crates-own-columns) - the fields that state them are - and the `identifiers` Map (65020) that once carried a message's names is retired, its tag never reused.

## A row is a message again

`from_row` is the inverse of [`into_row`](capture.md#a-column-is-filled-by-the-tag-its-field-carries): typed facts are read from the columns that own them, and `fixentries` supplies only the residual arrival content those columns do not represent. The two are rebuilt under the dictionary into one semantic message. The row's `beginstring`, `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid` and `crossuuid` columns must be stated, and every other one may be null - `sendingtime` among them, because a row states tag 52 only where the message did. The six recorded identity cells remain recorded while market getters refill from reconstructed content. Every one of the capture's own columns is carried: the one the crate tags, `sourceurl`, and every column no tag and no counter names - the body the line was cut from, its place in the object, its media type, what a bound dropped - each non-null cell under its column's name, answered by `carried`. A message is what parsing one line answered, and what a *reader* said about that line is not it, so none of them is content: none reaches an entry, the code the message answers to, or a `body=` at a counterparty, and a namespaced column lands in the [metadata](#typed-tags) as a parsed line's does. `into_row` states each carried cell again at its column, so `from_row` then `into_row` returns the same canonical row. Nothing is parsed again, which is what makes a [batch of rows a stream of messages](arrow.md#rows-are-messages-again-and-messages-rows) at the cost of the values it already holds. A row without `fixentries` rebuilds from its projected facts alone.

The fixed-row round trip is semantic: it preserves the canonical row, reconstructed content identity, and typed facts, while original wire and arrival order are not a row contract. Reconstructed sibling fields are canonicalized for the event hash; repeated occurrences retain their order. The public wire digest remains arrival-ordered. A group no dictionary declares - a bridge packing `NOTRADINGSESSIONS[0]=...` under a counter's own name - rebuilds from the row as the serie it is. The [example above](#written-into-the-row) ends with the round trip.

## Restated under the dictionary

A capture holds what each session spoke: a FIX 4.2 report states its fill as `LastShares`, its broker as `ExecBroker(76)`, its capacity as `Rule80A(47)` and a partial fill as `ExecType(150)` `1` - four things the newest specification spells as `LastQty`, a `Parties` occurrence, `OrderCapacity(528)` and `Trade`. A [parse](capture.md#a-reader-is-the-whole-parse-surface) restates the message once as it builds it, from what the dictionary itself says - the registry's field for every tag and the aliases that reach it - and from what the specification says of a retired field or value: [the crate's table of retirements](registry.md#what-the-specification-retired), applied as the message is built, with nothing parsed, bound or evaluated per message. A registry that states a rule of its own on a field, as [`FIX:replacements`](registry.md#a-field-carries-what-replaced-it), restates that field by its document alone. The parse then [fills what the restated row implies](capture.md#what-a-message-implied-is-filled-in) by the [`FIX:derivation`](registry.md#a-field-carries-how-it-is-derived) each derived field carries. There is no door of its own: a parsed message is a restated message, and so is a written one - [`set`](#written-into-the-row) restates what it writes, so a value a caller states and a value a line states are restated the same way.

| item | contract |
| --- | --- |
| Canonicalizes | every child the registry knows - by its `FIX:tag`, else its name or alias, else the decimal tag its name spells - is re-expressed under the registry's field: canonical name, datatype, tag, in the position it held; a child no dictionary knows stays exactly as it is |
| Merges | children reaching one field become one: the canonical-named child's value when stated, else the first stated among the rest; a child whose stated value disagrees with the kept one is left in place, so nothing that arrived is lost |
| Restates | each child the specification retired or whose field carries `FIX:replacements`, in ascending tag order, by the first entry whose condition holds at the level - the held value, the message type and the enclosing group - the field's own document before the specification's table; a group occurrence a rule states makes or completes one occurrence and sets its counter |
| Deprecated | a field the dictionary marks [`FIX:deprecated`](registry.md#the-dictionary-holds-one-reading-and-filters-by-no-version) - one FIX Latest removed, `MaxFloor(111)` among them - is restated to the field that replaced it and then nulled, so the row holds the fact once under its latest name while the entries keep the pair as it arrived |
| All or nothing | every target an entry fills is computed and checked before any is written; one target that cannot take its value blocks the whole entry, and no later entry fills in for it |
| Never overwrites | a value the message stated: a target takes a value only when it is absent, null or already equal; the source field itself is the one exception, because it is what is being restated |
| Keeps | a removed field the specification named no replacement for, and the source of every rule that did not write it - the row says what was sent and what it means |
| Levels | the root, then every occurrence of every repeating group to any depth, each canonicalized and restated in turn; `:group` is the occurrence's group, `:msgtype` the root's `MsgType(35)` |
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

A rule's plan reads the row's typed values through the [expression grammar](../expression/grammar.md) and writes wire text, because that is what the specification's appendices are written in. The two meet at the condition and at the target.

| held value | in the `where` | so |
| --- | --- | --- |
| text, ASCII | compared as text | `rule80a = 'A'`; a `MultipleCharValue` holds several codes in one text, so `ExecInst` `G T` is asked with `contains(execinst, 'T')` |
| boolean | `'Y'` coerces to it | `OddLot` `Y` meets `oddlot = 'Y'` |
| integer, float, decimal | a literal coerces to the column's type | `repurchaseterm = '1'`; a join spells an integer with two digits through `substring(concat('0', cast(maturityday as utf8)), -2)`, which is how a day completes a month-year |
| temporal | itself | `onbehalfofsendingtime as hopsendingtime` lands the instant it is |
| a coded value | compared by the code's spelling | `exectype = '1'` meets the `1` a partial fill was read as, as `exectype in ('1', '2')` meets either |

A value written into a target is re-typed for the target's field through the codec's own text-to-typed reading: a constant `'1'` lands in `PartyRole(452)` as an integer, `'F'` in `ExecType(150)` as the text it is, `'A'` in `OrderCapacity(528)` likewise. A constant written over a multi-valued source replaces the token the condition named - `G T` restated at `T` is `G R`. A value the target cannot hold blocks the entry rather than landing as null.

## Edges

- `get_by_tag(9999)`, an unknown tag -> the root child named `9999` exactly, never `09999`; the miss allocates nothing.
- A tag two fields hold under different names (`OrderQty` and `Quantity`, both 38) -> `by_id` tells them apart, each id reaching its own child; a bare tag reaches one child, the one named by the registry's first holder where the row itself does not carry the tag.
- `by_id` with another name or another tag than the field's -> a miss, because an identifier names the pair exactly and never folds a tag onto a name it does not carry.
- `FixId::of(0, ..)` or `FixId::of(-1, ..)` -> refused, because a definition's tag is positive and 0 marks only an unresolved arrival entry; a message root the codec builds carries no `FIX:branches`, because a message is not a dictionary member.
- `by_path("Parties.PartyID")` -> an error; a repeating group is a Serie of Structs, so a member needs the occurrence (`Parties[0].PartyID`), which is the spelling the registry takes too.
- A typed tag stated null at construction, or a holder stating nothing -> the lookup answers nothing: `get_by_tag(54)` on a report stating no side is `None`, `get_by_tag(SEQNUM_TAG_NAME.0)` on a message in no chain is `None`, and `get_by_tag(STATE_TAG_NAME.0)` on an order that reached no state is `None`.
- `set` with a name nothing reaches -> a typed absence naming the key, and the message unchanged; with a value the field refuses -> the value contract's refusal, and the message unchanged; `set_many` refuses all of its writes on the first refusal.
- `set` on a typed tag with a value its type refuses - text into `OrderQty(38)`, a spelling outside the side's set - is silence: the holder keeps what it held. `set` with a `Null` on a row child -> a stated null, the child kept and made nullable; on a typed tag -> the fact cleared. `remove` -> the child gone or the fact cleared and its value answered, `None` for a key that reaches nothing; a cleared `crosscode` keeps the settled one, because the identity is re-settled from what the message states and the code, once named, stands.
- `FixMsg::new` / `with_registry` on a root stating `SendingTime(52)` -> the header's clock, marked stated; on one stating none -> one UTC-now read, marked not stated, so the wire omits it; a stated clock that is not an instant -> a located refusal.
- `set` twice under one key -> one child, the later value; a bare unknown tag written twice -> one decimal-named child.
- `from_row` on a row whose entries column holds something that is not an arrival entry - a folded entry without its members, a negative tag, a name or value that is not text - or a leaf the JSON reader cannot decode -> refused at the arrival path; on a schema without the column -> a message with the typed facts, no content and a wire of the header alone; on a row leaving `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid` or `crossuuid` null -> the schema's refusal, since the fixed row declares them required.
- Two children reaching one field, both stated and different (`lastqty` `50` beside `LastShares` `100`) -> both kept as they arrived; equal once re-typed, or one null -> one child.
- A child named by a tag's digits (`"32"`) that the registry knows -> re-expressed under the registry's field like any other; one it does not know (`"9999"`) -> kept exactly, name, datatype and value.
- A Serie no `FIX:counter` heads, and any nested value that is not a repeating group -> kept exactly; only group occurrences are levels.
- A rule whose target holds a stated current code (`40=A|59=0`, a `TimeInForce` the message chose) -> blocked whole: `OrdType` stays `A` and no later entry answers for it; `59=7`, the rule's own value, is no obstacle.
- A rule that rewrote the source's own value (`ExecInst` `T` -> `R`) -> the new value is restated in turn (`R` is a `PegPriceType`), so one parse reaches what a second would find; a chain is bounded by the rules the field states.
- A `join` with a part unstated (`205=5` and no `200`) or a `from` whose tag is absent -> the entry fills nothing.
- A group fill whose constants match an existing occurrence (`ClearingFirm` made the role-4 party, `ClearingAccount` adds its sub-identifier) -> merged into it and the counter unchanged; a stated occurrence of the same role with another identifier -> the fill is blocked, the occurrence stands.
- A rule scoped by `:msgtype` on a message stating no `MsgType(35)` -> does not apply; one scoped by `:group` at the root -> does not apply.
- A removed field the specification replaced with nothing (`SendingDate(51)`) -> kept as read, and the registry still holds it under its own tag.

## Commands

=== "Rust"

    ```bash
    cargo test --features internals -p yggdryl --test fix -- mod_::internal::a_message
    cargo test -p yggdryl --test fix codec::a_message_re_emits_from_its_entries_and_reads_back_equal
    cargo test -p yggdryl --test fix -- msg
    cargo test -p yggdryl --test fix latest::
    cargo test -p yggdryl --test fix latest::a_fix_42_execution_report_restates_at_the_dictionarys_newest_version
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
