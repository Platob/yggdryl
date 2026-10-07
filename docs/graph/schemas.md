# Row schemas

The crate generates three row shapes: a [plain-text line](../media/text.md), a [FIX message](../fix/capture.md) and a [`marketdata` row](market-data.md). All three open with the same prefix:

- the [element](element.md) columns;
- the [event](event.md) columns;
- for market data only, the [market](market.md) and [operation](operation.md) columns.

A fact has one name and one datatype in every row, so a reader who knows one row knows how the other two start.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | Nothing of its own. The column names come from `ElementColumn::ALL` (6), `EventColumn::ALL` (9), `MarketColumn::ALL` (36) and `OperationColumn::ALL` (5) in `yggdryl::graph`. The rows are built by `MarketData::field`, `fix_schema` and the text record reader |
| Order | Element, event, market, operation - the order of the traits that answer them (`Element`, `Event`, `Market`, `Operation`) - then the row's own columns |
| Names | Every fact has one name and one datatype in every row. Every generated column carries a display name in its `display` metadata |
| Text line | The 15 element and event columns, then `body`, then one column per row-header capture |
| FIX row | The 56 prefix columns, then the message's own bands and `fixentries`: 153 fields and 152 tags under the committed dictionary. A fact FIX states in a field of its own is that field, typed as the dictionary types it (`price` is `Price(44)`, `timeinforce` the `TimeInForce(59)` wire text the message's [`TimeInForce`](../types/enum/timeinforce.md) member is read from) |
| `marketdata` row | The 56 prefix columns, the three book controls `bookscope`, `bookaction` and `bookposition`, then the nested `alive`, `delta`, `events`, `executions`, `bidlimits` and `asklimits`: 65 columns |
| Identifiers | `securityids`, `identifiers` and `partyids` are each a sorted `map<utf8, utf8>` from the key's text to its value ([Identifier](identifier.md#arrow)): on the FIX row every key the map holds - `src:type`, the type alone for the base source; on a market-data row the base keys alone, one per type, every other key [side information](market-data.md#side-information) in `metadata` under its map's name and its `src:type` spelling, `securityids.ullink:isin`; read raw and closed once, so every type held has its base key |
| Cross code | `crosscode` is the code as an element stores it - `{kind}:{side}:{base}` on a `marketdata` row and on a FIX row, the side stated by an order or an execution alone (`10:1:O-1001`, `14:0:Q-1`, `3:0:AAPL`) - and as given on a text line, which is no market element ([Market](market.md#sides-and-cross-codes)) |
| Persisted | Every [market fill](market.md#setting-fill-or-overwrite) a leaf answered is stored as a column value. A row read back through `MarketData::from_arrow_reader` or `FixMsg::from_row` answers the same facts without running the fills again |

=== "Rust"

    ```rust
    use yggdryl::graph::{ElementColumn, EventColumn, MarketColumn, MarketData, OperationColumn};

    let prefix: Vec<&str> = ElementColumn::ALL
        .iter()
        .map(|column| column.name())
        .chain(EventColumn::ALL.iter().map(|column| column.name()))
        .chain(MarketColumn::ALL.iter().map(|column| column.name()))
        .chain(OperationColumn::ALL.iter().map(|column| column.name()))
        .collect();
    assert_eq!(prefix.len(), 56);
    assert_eq!(&prefix[..3], ["curruuid", "crossuuid", "crosscode"]);
    assert_eq!(&prefix[15..17], ["marketdatakind", "marketdatatype"]);

    let row = MarketData::field()?;
    let names: Vec<&str> = row.fields().iter().map(|field| field.name()).collect();
    assert_eq!(names.len(), 65);
    assert_eq!(&names[..56], prefix.as_slice());
    assert_eq!(&names[56..59], ["bookscope", "bookaction", "bookposition"]);
    assert_eq!(&names[59..62], ["alive", "delta", "events"]);
    ```

=== "Python"

    ```python
    from yggdryl import enums, graph

    prefix = [*enums.ELEMENT_COLUMNS, *enums.EVENT_COLUMNS, *enums.MARKET_COLUMNS, *enums.OPERATION_COLUMNS]
    assert len(prefix) == 56
    assert prefix[15:17] == ["marketdatakind", "marketdatatype"]

    names = graph.MarketData.field().into_arrow_schema().names
    assert len(names) == 65
    assert names[:56] == prefix and names[56:59] == ["bookscope", "bookaction", "bookposition"]
    assert names[59:62] == ["alive", "delta", "events"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { enums, graph } = require('yggdryl')

    const prefix = [...enums.elementColumns, ...enums.eventColumns, ...enums.marketColumns, ...enums.operationColumns]
    assert.equal(prefix.length, 56)
    assert.deepEqual(prefix.slice(15, 17), ['marketdatakind', 'marketdatatype'])

    const row = graph.MarketData.field()
    assert.equal(row.fieldLen, 65)
    assert.deepEqual([56, 57, 58].map((at) => row.getFieldAt(at).name), ['bookscope', 'bookaction', 'bookposition'])
    assert.deepEqual([59, 60, 61].map((at) => row.getFieldAt(at).name), ['alive', 'delta', 'events'])
    ```

## The text line

A line read as records is an event:

- the element and event columns;
- `body`, the part of the line after what the row header matched;
- one column per capture.

A line states no market fact, so this row has no market or operation columns.

| # | Column | Datatype | Display |
| ---: | --- | --- | --- |
| 0 | `curruuid` | `uuid` | Current UUID |
| 1 | `crossuuid` | `uuid` | Cross UUID |
| 2 | `crosscode` | `utf8` | Cross Code |
| 3 | `currhashcode` | `uint64` | Current Hash Code |
| 4 | `crosshashcode` | `uint64` | Cross Hash Code |
| 5 | `srcuuids` | `serie<uuid>` | Source UUIDs |
| 6 | `currunix` | `datetime64(ns,"UTC")` | Current Time |
| 7 | `creaunix` | `datetime64(ns,"UTC")` | Creation Time |
| 8 | `recdunix` | `datetime64(ns,"UTC")` | Recording Time |
| 9 | `exprunix` | `datetime64(ns,"UTC")` | Expiry Time |
| 10 | `prevunix` | `datetime64(ns,"UTC")` | Previous Time |
| 11 | `snapunix` | `datetime64(ns,"UTC")` | Snapshot Time |
| 12 | `prevuuid` | `uuid` | Previous UUID |
| 13 | `seqnum` | `uint64` | Sequence Number |
| 14 | `state` | `state` | State |
| 15 | `body` | `utf8`, required | |
| 16... | one per row-header capture | as the capture declares | |

## The FIX row

This is `fix_schema(registry, name)` under the committed dictionary in `config/fix`.

When FIX has a field of its own for a market fact - `Price(44)`, `Side(54)`, `CxlQty(84)`, `BidPx(132)` - the prefix column is that FIX field, with its FIX tag and its FIX name as display. Every other prefix column is a [crate field](../fix/capture.md#the-crates-own-columns), tagged from `65001` up.

Some wire values also keep a column of their own:

- `OrdType(40)`, `QuoteType(537)` and `TrdType(828)` sit in the values band, next to the `marketdatatype` read from them.
- `OrderQty(38)` sits there too, next to `ordqty`.

| # | Column | Datatype | Required | Display | Tag |
| ---: | --- | --- | --- | --- | ---: |
| 0 | `curruuid` | `uuid` | yes | Current UUID | 65001 |
| 1 | `crossuuid` | `uuid` | yes | Cross UUID | 65002 |
| 2 | `crosscode` | `utf8` |  | Cross Code | 65003 |
| 3 | `currhashcode` | `uint64` | yes | Current Hash Code | 65004 |
| 4 | `crosshashcode` | `uint64` | yes | Cross Hash Code | 65005 |
| 5 | `srcuuids` | `serie<uuid>` |  | Source UUIDs | 65006 |
| 6 | `currunix` | `datetime64(ns,"UTC")` | yes | Current Time | 65007 |
| 7 | `creaunix` | `datetime64(ns,"UTC")` | yes | Creation Time | 65008 |
| 8 | `recdunix` | `datetime64(ns,"UTC")` |  | Recording Time | 65009 |
| 9 | `exprunix` | `datetime64(ns,"UTC")` |  | Expiry Time | 65010 |
| 10 | `prevunix` | `datetime64(ns,"UTC")` |  | Previous Time | 65011 |
| 11 | `snapunix` | `datetime64(ns,"UTC")` |  | Snapshot Time | 65012 |
| 12 | `prevuuid` | `uuid` |  | Previous UUID | 65013 |
| 13 | `seqnum` | `uint64` |  | Sequence Number | 65014 |
| 14 | `state` | `state` |  | State | 65015 |
| 15 | `marketdatakind` | `marketdatakind` |  | Market Data Kind | 65016 |
| 16 | `marketdatatype` | `marketdatatype` |  | Market Data Type | 65017 |
| 17 | `price` | `decimal128(38,18)` |  | Price | 44 |
| 18 | `stoppx` | `decimal128(38,18)` |  | StopPx | 99 |
| 19 | `currency` | `ccy` |  | Currency | 15 |
| 20 | `origccy` | `ccy` |  | Origin Currency | 65018 |
| 21 | `quantity` | `decimal128(38,18)` |  | Quantity | 53 |
| 22 | `displayqty` | `decimal128(38,18)` |  | DisplayQty | 1138 |
| 23 | `hiddenqty` | `decimal` |  | Hidden Quantity | 65019 |
| 24 | `unit` | `unit` |  | Unit | 65020 |
| 25 | `side` | `side` |  | Side | 54 |
| 26 | `securityids` | `map(...)` |  | Security IDs | 65021 |
| 27 | `isincode` | `isin` |  | ISIN Code | 65022 |
| 28 | `cficode` | `utf8` |  | CFICode | 461 |
| 29 | `miccode` | `mic` |  | MIC Code | 65023 |
| 30 | `execunix` | `datetime64(ns,"UTC")` |  | Execution Time | 65024 |
| 31 | `lastpx` | `decimal128(38,18)` |  | LastPx | 31 |
| 32 | `lastqty` | `decimal128(38,18)` |  | LastQty | 32 |
| 33 | `avgpx` | `decimal128(38,18)` |  | AvgPx | 6 |
| 34 | `cumqty` | `decimal128(38,18)` |  | CumQty | 14 |
| 35 | `leavesqty` | `decimal128(38,18)` |  | LeavesQty | 151 |
| 36 | `cxlqty` | `decimal128(38,18)` |  | CxlQty | 84 |
| 37 | `prevpx` | `decimal` |  | Previous Price | 65025 |
| 38 | `prevqty` | `decimal` |  | Previous Quantity | 65026 |
| 39 | `spotrate` | `decimal` |  | Spot Rate | 65027 |
| 40 | `forwardpoints` | `decimal` |  | Forward Points | 65028 |
| 41 | `bidpx` | `decimal128(38,18)` |  | BidPx | 132 |
| 42 | `bidqty` | `decimal` |  | Bid Quantity | 65029 |
| 43 | `bidccy` | `ccy` |  | Bid Currency | 65030 |
| 44 | `askpx` | `decimal` |  | Ask Price | 65031 |
| 45 | `askqty` | `decimal` |  | Ask Quantity | 65032 |
| 46 | `askccy` | `ccy` |  | Ask Currency | 65033 |
| 47 | `fxrates` | `map(...)` |  | FX Rates | 65034 |
| 48 | `ticker` | `utf8` |  | Ticker | 65035 |
| 49 | `strikepx` | `decimal` |  | Strike Price | 65036 |
| 50 | `metadata` | `map(...)` |  | Metadata | 65037 |
| 51 | `ordqty` | `decimal` |  | Order Quantity | 65038 |
| 52 | `timeinforce` | `utf8` |  | TimeInForce | 59 |
| 53 | `tradable` | `boolean` |  | Tradable | 65039 |
| 54 | `identifiers` | `map(...)` |  | Identifiers | 65040 |
| 55 | `partyids` | `map(...)` |  | Party IDs | 65041 |
| 56 | `sendingtime` | `datetime64(ns,"UTC")` |  | SendingTime | 52 |
| 57 | `origsendingtime` | `datetime64(ns,"UTC")` |  | OrigSendingTime | 122 |
| 58 | `transacttime` | `datetime64(ns,"UTC")` |  | TransactTime | 60 |
| 59 | `settldate` | `datetime64(ns)` |  | SettlDate | 64 |
| 60 | `tradedate` | `datetime64(ns)` |  | TradeDate | 75 |
| 61 | `expiretime` | `datetime64(ns,"UTC")` |  | ExpireTime | 126 |
| 62 | `validuntiltime` | `datetime64(ns,"UTC")` |  | ValidUntilTime | 62 |
| 63 | `expiredate` | `datetime64(ns)` |  | ExpireDate | 432 |
| 64 | `beginstring` | `utf8` | yes | BeginString | 8 |
| 65 | `msgtype` | `utf8` |  | MsgType | 35 |
| 66 | `msgseqnum` | `int64` |  | MsgSeqNum | 34 |
| 67 | `sendercompid` | `utf8` |  | SenderCompID | 49 |
| 68 | `targetcompid` | `utf8` |  | TargetCompID | 56 |
| 69 | `possdupflag` | `boolean` |  | PossDupFlag | 43 |
| 70 | `msgdirection` | `utf8` |  | MsgDirection | 385 |
| 71 | `msgpluginid` | `utf8` |  | Message Plugin ID | 65042 |
| 72 | `msgpluginside` | `pluginside` | yes | Message Plugin Side | 65043 |
| 73 | `msgoriginator` | `utf8` |  | Message Originator | 65044 |
| 74 | `msgctxid` | `utf8` |  | Message Context ID | 65045 |
| 75 | `msgsessionid` | `utf8` |  | Message Session ID | 65046 |
| 76 | `msgsesseventid` | `utf8` |  | Message Session Event ID | 65047 |
| 77 | `conversationid` | `utf8` |  | Conversation ID | 65048 |
| 78 | `symbol` | `utf8` |  | Symbol | 55 |
| 79 | `forexcode` | `forex` |  | Forex Code | 65049 |
| 80 | `bloombergcode` | `bbg` |  | Bloomberg Code | 65050 |
| 81 | `figicode` | `figi` |  | FIGI Code | 65051 |
| 82 | `strikeprice` | `decimal128(38,18)` |  | StrikePrice | 202 |
| 83 | `securitytype` | `utf8` |  | SecurityType | 167 |
| 84 | `securitysubtype` | `utf8` |  | SecuritySubType | 762 |
| 85 | `securityexchange` | `mic` |  | SecurityExchange | 207 |
| 86 | `exdestination` | `mic` |  | ExDestination | 100 |
| 87 | `lastmkt` | `mic` |  | LastMkt | 30 |
| 88 | `maturitydate` | `datetime64(ns)` |  | MaturityDate | 541 |
| 89 | `product` | `int32` |  | Product | 460 |
| 90 | `securitytradingstatus` | `int32` |  | SecurityTradingStatus | 326 |
| 91 | `tradsesstatus` | `int32` |  | TradSesStatus | 340 |
| 92 | `securitystatus` | `utf8` |  | SecurityStatus | 965 |
| 93 | `account` | `utf8` |  | Account | 1 |
| 94 | `clordid` | `utf8` |  | ClOrdID | 11 |
| 95 | `origclordid` | `utf8` |  | OrigClOrdID | 41 |
| 96 | `secondaryclordid` | `utf8` |  | SecondaryClOrdID | 526 |
| 97 | `orderid` | `utf8` |  | OrderID | 37 |
| 98 | `secondaryorderid` | `utf8` |  | SecondaryOrderID | 198 |
| 99 | `execid` | `utf8` |  | ExecID | 17 |
| 100 | `tradeid` | `utf8` |  | TradeID | 1003 |
| 101 | `quotereqid` | `utf8` |  | QuoteReqID | 131 |
| 102 | `quoteid` | `utf8` |  | QuoteID | 117 |
| 103 | `mdreqid` | `utf8` |  | MDReqID | 262 |
| 104 | `quoterespid` | `utf8` |  | QuoteRespID | 693 |
| 105 | `orderqty` | `decimal128(38,18)` |  | OrderQty | 38 |
| 106 | `maxfloor` | `decimal128(38,18)` |  | MaxFloor | 111 |
| 107 | `prevclosepx` | `decimal128(38,18)` |  | PrevClosePx | 140 |
| 108 | `unitofmeasure` | `utf8` |  | UnitOfMeasure | 996 |
| 109 | `settlcurrency` | `ccy` |  | SettlCurrency | 120 |
| 110 | `qtytype` | `int32` |  | QtyType | 854 |
| 111 | `ordtype` | `utf8` |  | OrdType | 40 |
| 112 | `quotetype` | `int32` |  | QuoteType | 537 |
| 113 | `trdtype` | `int32` |  | TrdType | 828 |
| 114 | `offerpx` | `decimal128(38,18)` |  | OfferPx | 133 |
| 115 | `bidsize` | `decimal128(38,18)` |  | BidSize | 134 |
| 116 | `offersize` | `decimal128(38,18)` |  | OfferSize | 135 |
| 117 | `lastspotrate` | `decimal128(38,18)` |  | LastSpotRate | 194 |
| 118 | `lastforwardpoints` | `decimal128(38,18)` |  | LastForwardPoints | 195 |
| 119 | `bidspotrate` | `decimal128(38,18)` |  | BidSpotRate | 188 |
| 120 | `bidforwardpoints` | `decimal128(38,18)` |  | BidForwardPoints | 189 |
| 121 | `offerspotrate` | `decimal128(38,18)` |  | OfferSpotRate | 190 |
| 122 | `offerforwardpoints` | `decimal128(38,18)` |  | OfferForwardPoints | 191 |
| 123 | `ordstatus` | `utf8` |  | OrdStatus | 39 |
| 124 | `exectype` | `utf8` |  | ExecType | 150 |
| 125 | `quotestatus` | `int32` |  | QuoteStatus | 297 |
| 126 | `quoteresponselevel` | `int32` |  | QuoteResponseLevel | 301 |
| 127 | `quoteentryrejectreason` | `int32` |  | QuoteEntryRejectReason | 368 |
| 128 | `ordrejreason` | `int32` |  | OrdRejReason | 103 |
| 129 | `cxlrejreason` | `int32` |  | CxlRejReason | 102 |
| 130 | `text` | `utf8` |  | Text | 58 |
| 131 | `trdregtimestamps` | `serie(...)` |  | TrdRegTimestamps | 763375 |
| 132 | `regulatorytradeids` | `serie(...)` |  | RegulatoryTradeIDs | 497401 |
| 133 | `bodylength` | `int32` |  | BodyLength | 9 |
| 134 | `onbehalfofcompid` | `utf8` |  | OnBehalfOfCompID | 115 |
| 135 | `delivertocompid` | `utf8` |  | DeliverToCompID | 128 |
| 136 | `securedatalen` | `int32` |  | SecureDataLen | 90 |
| 137 | `securedata` | `binary` |  | SecureData | 91 |
| 138 | `sendersubid` | `utf8` |  | SenderSubID | 50 |
| 139 | `senderlocationid` | `utf8` |  | SenderLocationID | 142 |
| 140 | `targetsubid` | `utf8` |  | TargetSubID | 57 |
| 141 | `targetlocationid` | `utf8` |  | TargetLocationID | 143 |
| 142 | `onbehalfofsubid` | `utf8` |  | OnBehalfOfSubID | 116 |
| 143 | `onbehalfoflocationid` | `utf8` |  | OnBehalfOfLocationID | 144 |
| 144 | `delivertosubid` | `utf8` |  | DeliverToSubID | 129 |
| 145 | `delivertolocationid` | `utf8` |  | DeliverToLocationID | 145 |
| 146 | `possresend` | `boolean` |  | PossResend | 97 |
| 147 | `xmldatalen` | `int32` |  | XmlDataLen | 212 |
| 148 | `xmldata` | `binary` |  | XmlData | 213 |
| 149 | `signaturelength` | `int32` |  | SignatureLength | 93 |
| 150 | `signature` | `binary` |  | Signature | 89 |
| 151 | `checksum` | `utf8` |  | CheckSum | 10 |
| 152 | `fixentries` | `map(...)` |  | FixEntries |  |

The capture-only `sourceurl` has crate tag 65052; the persisted `fixmsg` component has tag 65053. Neither is an extra column in this fixed row.

A group column (`trdregtimestamps`, `regulatorytradeids`) is a `serie` of the group's struct, found by its counter's tag (`NoTrdRegTimestamps(768)`, `NoRegulatoryTradeIDs(1907)`); no counter column stands beside it, the group's length being the count. `SecurityID(48)`, `SecurityIDSource(22)` and the groups `Parties(453)` and `SecAltIDGrp(454)` are no columns: the prefix's [`securityids` and `partyids`](identifier.md) state the identifiers they name, as does an unmapped entry whose key names one - `OMS_InstrumentID`, `OMS_UserID`, a security alias such as `ISINCODE` or `OMS_RICCODE` - read into the map its type belongs to ([Identifier](identifier.md#where-identifiers-come-from)). A message stating them keeps them in `fixentries` as sent - `453:parties` holding the group as JSON - and an unmapped entry an identifier map holds leaves the `metadata` cell and rides `fixentries` under `0:<key>`, the key as it arrived, so `metadata` holds only what nothing resolved. `fixentries` is the sorted residual `map<utf8, utf8>`, keyed `tag:name`.

## The `marketdata` row

This is `MarketData::field()`: the prefix, then the book.

- `securityids`, `identifiers` and `partyids` are sorted `map<utf8, utf8>`s from a type's base key - the type alone - to its value, one per type; every other key a map holds, `src:type`, is [side information](market-data.md#side-information) in `metadata` under the map's name, `securityids.ullink:isin`.
- `bookscope`, `bookaction` and `bookposition` are the [book control](order.md#book-control) a market-data entry states, which a book's `delta` replays by.
- `alive`, `delta`, `events` and `executions` are series of the prefix and the three book controls (`operationevent`); `alive` is a [complete](book.md#complete-books-and-delta-books) book's live orders and quotes, `delta` every book's orders and quotes its instant applied, `events` every other event that instant recorded - its executions and snapshot controls - and `executions` a trade's alone, null on a book row.
- `bidlimits` and `asklimits` are series of [`Limit`](book.md), best level first.
- `srcuuids` is null on a book row, which states no sources, and on every entry nested in its `alive`, whose sources the `delta` row of the book that applied it writes; `delta`, `events` and `executions` entries write theirs ([Sources](market-data.md#arrow)).

| # | Column | Datatype | Required | Display | Band |
| ---: | --- | --- | --- | --- | --- |
| 0 | `curruuid` | `uuid` |  | Current UUID | element |
| 1 | `crossuuid` | `uuid` |  | Cross UUID | element |
| 2 | `crosscode` | `utf8` |  | Cross Code | element |
| 3 | `currhashcode` | `uint64` |  | Current Hash Code | element |
| 4 | `crosshashcode` | `uint64` |  | Cross Hash Code | element |
| 5 | `srcuuids` | `serie<uuid>` |  | Source UUIDs | element |
| 6 | `currunix` | `datetime64(ns,"UTC")` |  | Current Time | event |
| 7 | `creaunix` | `datetime64(ns,"UTC")` |  | Creation Time | event |
| 8 | `recdunix` | `datetime64(ns,"UTC")` |  | Recording Time | event |
| 9 | `exprunix` | `datetime64(ns,"UTC")` |  | Expiry Time | event |
| 10 | `prevunix` | `datetime64(ns,"UTC")` |  | Previous Time | event |
| 11 | `snapunix` | `datetime64(ns,"UTC")` |  | Snapshot Time | event |
| 12 | `prevuuid` | `uuid` |  | Previous UUID | event |
| 13 | `seqnum` | `uint64` |  | Sequence Number | event |
| 14 | `state` | `state` |  | State | event |
| 15 | `marketdatakind` | `marketdatakind` | yes | Market Data Kind | market |
| 16 | `marketdatatype` | `marketdatatype` | yes | Market Data Type | market |
| 17 | `price` | `decimal` |  | Price | market |
| 18 | `stoppx` | `decimal` |  | Stop Price | market |
| 19 | `currency` | `ccy` | yes | Currency | market |
| 20 | `origccy` | `ccy` |  | Origin Currency | market |
| 21 | `quantity` | `decimal` |  | Quantity | market |
| 22 | `displayqty` | `decimal` |  | Display Quantity | market |
| 23 | `hiddenqty` | `decimal` |  | Hidden Quantity | market |
| 24 | `unit` | `unit` | yes | Unit | market |
| 25 | `side` | `side` | yes | Side | market |
| 26 | `securityids` | `map<utf8, utf8>` |  | Security IDs | market |
| 27 | `isincode` | `isin` |  | ISIN Code | market |
| 28 | `cficode` | `cfi` |  | CFI Code | market |
| 29 | `miccode` | `mic` |  | MIC Code | market |
| 30 | `execunix` | `datetime64(ns,"UTC")` |  | Execution Time | market |
| 31 | `lastpx` | `decimal` |  | Last Price | market |
| 32 | `lastqty` | `decimal` |  | Last Quantity | market |
| 33 | `avgpx` | `decimal` |  | Average Price | market |
| 34 | `cumqty` | `decimal` |  | Cumulative Quantity | market |
| 35 | `leavesqty` | `decimal` |  | Leaves Quantity | market |
| 36 | `cxlqty` | `decimal` |  | Canceled Quantity | market |
| 37 | `prevpx` | `decimal` |  | Previous Price | market |
| 38 | `prevqty` | `decimal` |  | Previous Quantity | market |
| 39 | `spotrate` | `decimal` |  | Spot Rate | market |
| 40 | `forwardpoints` | `decimal` |  | Forward Points | market |
| 41 | `bidpx` | `decimal` |  | Bid Price | market |
| 42 | `bidqty` | `decimal` |  | Bid Quantity | market |
| 43 | `bidccy` | `ccy` |  | Bid Currency | market |
| 44 | `askpx` | `decimal` |  | Ask Price | market |
| 45 | `askqty` | `decimal` |  | Ask Quantity | market |
| 46 | `askccy` | `ccy` |  | Ask Currency | market |
| 47 | `fxrates` | `map<ccy, decimal>` |  | FX Rates | market |
| 48 | `ticker` | `utf8` |  | Ticker | market |
| 49 | `strikepx` | `decimal` |  | Strike Price | market |
| 50 | `metadata` | `map<utf8, utf8?>` |  | Metadata | market |
| 51 | `ordqty` | `decimal` |  | Order Quantity | operation |
| 52 | `timeinforce` | `timeinforce` |  | Time In Force | operation |
| 53 | `tradable` | `boolean` |  | Tradable | operation |
| 54 | `identifiers` | `map<utf8, utf8>` |  | Identifiers | operation |
| 55 | `partyids` | `map<utf8, utf8>` |  | Party IDs | operation |
| 56 | `bookscope` | `utf8` |  | Book Scope | book control |
| 57 | `bookaction` | `utf8` |  | Book Action | book control |
| 58 | `bookposition` | `uint32` |  | Book Position | book control |
| 59 | `alive` | `serie<operationevent>` |  |  | book |
| 60 | `delta` | `serie<operationevent>` |  |  | book |
| 61 | `events` | `serie<operationevent>` |  |  | book |
| 62 | `executions` | `serie<operationevent>` |  |  | trade |
| 63 | `bidlimits` | `serie<limit>` |  |  | book |
| 64 | `asklimits` | `serie<limit>` |  |  | book |

## Edges

- Read a column by name, never by position. A dictionary with other fields moves the FIX row's own bands, but never the prefix.
- `marketdatakind`, `marketdatatype`, `currency`, `unit` and `side` are required in a `marketdata` row, because their unstated members (`UKNW`, `XXX`, none) are values. The FIX row leaves them nullable, because a message may state none of them.
- An [enum](../types/enum/index.md) column crosses Arrow as the codes of its members under its own extension name: `state` and `marketdatatype` as `uint16`, `marketdatakind`, `side`, the `marketdata` row's `timeinforce` and the FIX row's required `msgpluginside` as `uint8`.
- A bridge key spelled `ordqty` lands in the crate's `ordqty` column, not in the message's metadata.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test graph -- column
    cargo test -p yggdryl --test fix -- schema
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/graph -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/graph/index.test.js
    ```
