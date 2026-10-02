---
name: yggdryl-market-data
description: Models and streams market data with yggdryl's graph layer in Rust, Python and Node.js - orders, quotes, executions and trades as dated events (OrderEvent, QuoteEvent, ExecutionEvent, TradeEvent), identities and side-keyed chains (curruuid, crossuuid, with_previous / withPrevious, EventIterator), order books (BookIterator, BookEvent, limits, best_price / bestPrice, spread, depth, SnapshotEvent), OHLC candles of the best bid and ask per zone-aligned bucket (CandleIterator, CandleOptions, graph.candles, Candle.field), the book display served over a marketdata table (yggdryl market serve, BookService, node/book.js), the Side and MarketDataKind enums and the lifted marketdata Arrow row (MarketData.arrow_reader / arrowReader, from_arrow_reader / fromArrowReader, apply_view / applyView). Use when building market events, folding them into books or candles, persisting or querying marketdata batches, or serving a table of books as a display.
---

# yggdryl market data

The graph layer is market data as **elements that name each other by
identity**, never by reference. Four Rust traits say what an element answers -
`Element` (identity, cross code, digest, sources), `Event` (instant, state,
place at its instant, clocks), `Market` (thirty-four facts: the `marketdatatype` of its kind, price, stop price,
quantity and its shown and hidden parts, currency, unit, side, the security's identifiers `securityids` and the `isincode` they hold, classification, market,
the `execunix` it last executed at, last-trade, progress, FX parts, the stated bid and ask - `bidpx`, `bidqty`,
`bidccy`, `askpx`, `askqty`, `askccy` - the `fxrates`, ticker, metadata) and
`Operation` (five more: the `ordqty` it asked for, the `TimeInForce` member it stands for, whether it trades, the `identifiers`, the
`partyids` it names - the account included) - and
typed leaves answer them: `Order`/`OrderEvent`, `Quote`/`QuoteEvent`,
`Execution`/`ExecutionEvent`, the composite `TradeEvent`, and the book types
`BookEvent` and `SnapshotEvent`. `MarketData` is the one value over every
leaf, and the lifted **`marketdata` Arrow row** (60 columns: the element,
event, market and operation columns every generated row opens with, then the
book - [Row schemas](https://platob.github.io/yggdryl/graph/schemas/)) is how
any of them crosses a boundary.

Hold these facts:

- **Instants are `i64` nanoseconds since the Unix epoch, UTC** - `currunix`,
  `creaunix`, `recdunix`, `exprunix`, `prevunix`, `snapunix`, and the market's
  `execunix`, which an undated element states too.
  Python ints, JavaScript `bigint`s, Arrow `datetime64(ns, UTC)`.
- **Identity is derived, not assigned.** A leaf is finalized on construction:
  `currhashcode` is the XXH3-64 of its content, `curruuid` a UUIDv7 of
  millisecond + place for a dated leaf (UUIDv8 of content otherwise),
  `crossuuid` the chain every incarnation shares, from the cross code.
- **A side is never null, and it keys the chain.** `Side` and
  `MarketDataKind` are `uint8` enums; an element stating no side holds
  `UNKN` (code 0). A cross code is stored as `{kind}:{side}:{base}` - a buy
  order `O-1001` is `10:1:O-1001` - so the two sides of one identifier are two
  chains (`MarketDataKind::is_sided`, Rust-only); `UNKN` stores side `0`, and
  so does every trade, book and snapshot control whatever side it states
  (`21:0:T-1`, `3:0:AAPL`).
- **Chains are walked, not rebuilt.** An event names only its predecessor
  (`prevuuid`); `seqnum` is its place among the events of its instant, which
  orders the identities of one millisecond, and a step keeps its own unless
  its predecessor shares or passes its instant. A follower takes what its
  chain states and it does not - every `metadata` key, every `identifiers` key but
  `mdentryrefid` (a book entry's reference to its predecessor, which names one
  step) and every `partyids` source and role, its own values standing, each
  identifier a follower states anew keeping its own value - and its identity
  digests what it took. Parents move along a chain: a
  changed `orderid` leaves its previous value as `parentorderid` and the
  chain's first as `origorderid`, a changed `clordid` leaves `origclordid`
  (`FIX:parents` states the list a FIX field has). A FIX lifecycle message takes the `metadata` keys and
  only the ids its dictionary follows, each with its parents. `EventIterator` joins a stream by cross
  identity and by the type and value of an `identifiers` identifier a live element
  went by (or the parent identifier it replaced, joined under its base),
  within one `marketdatakind` (an order and an execution under one cross code
  are two chains, so a fill never restates, follows or ends its order),
  folds twins, emits expiries, and leaves every element stating `creaunix`. A grid view is the live element
  as of its tick: dated at it (`currunix` = the tick), its `snapunix` the
  instant the element it copies was stated at (never the tick, never a
  predecessor's, and in no digest), so its `curruuid` is the tick's own
  while content, `seqnum`, `prevuuid` and `crossuuid` are the live
  element's; it advances nothing. `srcuuids` is provenance only - the line a
  message was read from and the message a parse split it off - and travels
  along no chain.
- **A ticker of an identifier's shape names it.** `US0378331005`, a
  `BBG` FIGI, a CUSIP, a SEDOL, a detailed CFI code, `AAPL.OQ`, `HOLN SW
  Equity` or an instrument key `CH0012214059_XSWX_CHF` derive the identifier
  (and the key's market and currency) where none is stated; a currency pair
  with no unit takes the currency dealt as its unit.
- **Identifiers are typed maps.** `securityids`, `identifiers` and `partyids` are
  each an `Identifiers` map keyed `src:type` of `Identifier { src, type, value }`
  (`yggdryl::Identifier`, Python `from yggdryl import Identifier`, JavaScript
  `require('yggdryl').Identifier`): one value per source and type, sorted by
  that key, displayed `base:isin=US0378331005`, every word lower case; `key` is
  `src:type`. `base` is no source stated, `derived` what the crate derived (an
  ISIN's CUSIP, a ticker's shape, a FX pair). Build one with `Identifier(src,
  type, value)` - the value is checked by its type, so `Identifier("base",
  "isin", code)` closes on its check digit - or `Identifier.from_key(key, value)`,
  which reads a full `src:type` key or the identifier name a bridge's own
  spelling ends with (`OMS_InstrumentID` is `oms:instrumentid`) - another
  instrument's word before a security type, after any namespace, names none
  (`OMS_UnderlyingISIN`, `FIX.LegISIN`); Rust takes the
  `IdSource` and `IdType` enums. Parentage is a relation between types, never a
  field of a value: when an `orderid` changes along a chain, a follower keeps
  the value it held as `parentorderid` and the chain's first as `origorderid`
  (a `clordid` keeps only `origclordid`), and one stating only a parent takes
  its base from it.
- **A setter fills, or overwrites when told.** Every Rust `Market` and
  `Operation` setter takes a trailing `overwrite: bool`: `false` lands only
  where the fact is unstated, `true` states it. A change carries what it
  implies - a buyer's price and quantity are its bid, a side moves the quote
  and a sided cross code, `leavesqty` is the quantity, an order's `ordqty`,
  `cumqty` and `leavesqty` fill one another by its state (`LeavesQty = OrderQty
  - CumQty` while it works, `0` once done, the rest `cxlqty` when canceled) -
  and the binding constructors run the same fills. Every fill is a column, so
  a row read back answers it unchanged; never recompute one by hand.
- **Books are folded, not replayed by hand.** `BookIterator` folds a sorted
  stream into one `BookEvent` per book and instant; read books back from
  `marketdata` rows, never by re-applying their deltas. Sorted books fold on
  into `Candle`s - one OHLC of the best bid, the best ask, the mid and the
  spread per cross code and bucket - and `yggdryl market serve` serves a
  table of them as candles, books and audits behind the Node.js display.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| a dated order / quote / execution | `OrderEvent::at(unix)`, `set_*`, `finalize()` | `graph.OrderEvent(unix, **facts)` | `new graph.OrderEvent(unix, facts)` |
| an undated leaf, dated later | `Order::new()`, `order.at(unix)`, `event.into_element()` | `graph.Order(**facts)`, `.at(unix)`, `.into_element()` | `new graph.Order(facts)`, `.at(unix)`, `.intoElement()` |
| a two-sided quote | `set_bidpx`, `set_askpx`, `set_bidqty` ... | `graph.QuoteEvent(unix, bidpx=..., askpx=...)` | `new graph.QuoteEvent(unix, { bidpx, askpx })` |
| a market-data entry's book control | `event.with_book(BookRef { .. })` | `event.with_book(graph.BookRef(action="new", position=1))` | `event.withBook(new graph.BookRef({ action: 'new', position: 1 }))` |
| an identifier | `Identifier::new(IdSource::Base, IdType::Isin, value)?`, `Identifiers` | `Identifier(src, type, value)`, `Identifiers([...])` | `new Identifier(src, type, value)`, `new Identifiers([...])` |
| security, own and party identifiers | `insert_securityid(id)?`, `insert_identifier(id)?`, `insert_partyid(id)?` (Rust-only verbs) | `securityids=[Identifier("base", "isin", ...)]`, `identifiers=[...]`, `partyids=[...]` at build | `securityids: [new Identifier('base', 'isin', ...)]`, `identifiers: [...]`, `partyids: [...]` at build |
| read an identifier map | `get_securityids().get(&IdType::Isin)`, `get_from(&src, &kind)` | `order.securityids.get("isin")`, `get_from(src, type)`, iterate `Identifier`s | `order.securityids.get('isin')`, `getFrom(src, type)`, `toArray()` |
| FX rates (nothing fills them) | `insert_fxrate(ccy, rate)`, `set_fxrates(map)` | `fxrates={"EUR": Decimal("1.1")}` at build | `fxrates: { EUR: '1.1' }` at build |
| a composite trade | `TradeEvent::from_parts(&root, executions)?` | `graph.TradeEvent.from_parts(root, executions)` | `graph.TradeEvent.fromParts(root, executions)` |
| follow a predecessor | `event.with_previous(&prev)` | `event.with_previous(prev)` | `event.withPrevious(prev)` |
| merge two statements of one event | `event.merge_with(&other)` | `event.merge_with(other)` | `event.mergeWith(other)` |
| walk a stream into chains | `EventIterator::new(items, sorted)`, `.with_snapshot_ns(ns)` | `graph.EventIterator(items, sorted=True, snapshot_ns=None)` | `new graph.EventIterator(items, sorted, snapshotNs)` (sorted defaults to `true`) |
| any leaf as one value | `MarketData::from(leaf)`, `kind()`, `marketdatakind()`, `as_order_event()`, `TryFrom` | `graph.MarketData(leaf)`, `.kind`, `.marketdatakind`, `.as_order_event()`, `.into_leaf()` | `new graph.MarketData(leaf)`, `.kind`, `.marketdatakind`, `.asOrderEvent()`, `.intoLeaf()` |
| a FIX message held whole | `MarketData::from(msg)` (kind `fix`, its `msgcat`), `as_fix()`, `FixMsg::try_from(value)?`; written and folded as the leaves it splits into | `graph.MarketData(msg)`, `.as_fix()` | `new graph.MarketData(msg)`, `.asFix()` |
| the `marketdata` row schema | `MarketData::field()?` | `graph.MarketData.field()` | `graph.MarketData.field()` |
| leaves to Arrow batches | `MarketData::arrow_reader(values, None, None)?` | `graph.MarketData.arrow_reader(values)` | `graph.MarketData.arrowReader(values)` |
| Arrow batches to leaves | `MarketData::from_arrow_reader(reader)?` | `graph.MarketData.from_arrow_reader(source)` | `graph.MarketData.fromArrowReader(reader)` |
| fold a sorted stream into books | `BookIterator::new(items, snapshot_millis)?` | `graph.BookIterator(items, snapshot_millis=0)` | `new graph.BookIterator(items, snapshotMillis = 0)` |
| one book by hand | `BookEvent::new(unix, symbol)`, `add_operations(..)?` | `graph.BookEvent(unix, symbol).with_operations([...])` | `new graph.BookEvent(unix, symbol).withOperations([...])` |
| a book's entries | `alive()`, `deltas()`, `executions()` | `book.alive`, `book.deltas`, `book.executions` | `book.alive()`, `book.deltas()`, `book.executions()` |
| read a side | `limits(Side::Buy)`, `best_price(Side::Buy)`, `best_quantity(..)`, `depth(Side::Buy, n)` | `book.limits(Side.BUYS)`, `book.best_price(Side.BUYS)`, `book.depth(Side.BUYS, n)` | `book.limits('BUYS')`, `book.bestPrice('BUYS')`, `book.depth('BUYS', n)` |
| read both sides | `get_bidpx()`, `get_askpx()`, `spread()`, `is_crossed()`, `imbalance(n)` | `book.bidpx`, `book.askpx`, `book.spread`, `book.is_crossed`, `book.imbalance(n)` | `book.bidpx`, `book.askpx`, `book.spread`, `book.isCrossed`, `book.imbalance(n)` |
| clear a scope with a snapshot | `SnapshotEvent::snapshot(&event, scope)` | `graph.SnapshotEvent.snapshot(event, scope=None)` | `graph.SnapshotEvent.snapshot(event, scope)` |
| a named view of a stream | `MarketData::apply_view(&MarketView::Orders, &lifts, reader)?` | `graph.MarketData.apply_view("orders", source, lifts)` | `graph.MarketData.applyView('orders', reader, lifts)` |
| a view as a plan | `MarketData::plan(&view, &lifts)?` | `graph.MarketData.plan("trades")` | `graph.MarketData.plan('trades')` |
| FIX to sorted market data / books | `codec.market_data(codec.lifecycle(msgs))`, `codec.book_arrow_reader(msgs, 0)?` | `codec.market_data(...)`, `codec.book_arrow_reader(msgs, snapshot_millis=0)` | `codec.marketData(..)`, `codec.bookArrowReader(msgs, 0)` |
| fold sorted books into candles | `CandleIterator::new(books, CandleOptions::from_spelling("1m")?.with_timezone(zone))` | `graph.candles(books, "1m", timezone=None)`, `graph.CandleIterator(books, graph.CandleOptions("1m", zone))` | `graph.candles(books, '1m', zone)`, `new graph.CandleIterator(books, new graph.CandleOptions('1m', zone))` |
| a candle's row, and candles as Arrow | `Candle::field()?`, `Candle::arrow_reader(candles, None)?`, `candle.into_scalar()`, `Candle::from_scalar(&value)?` | `graph.Candle.field()`, `candle.into_scalar()`, `candle.as_py()`, `graph.Candle.from_scalar(value)` | `graph.Candle.field()`, `candle.intoScalar()`, `candle.toJSON()`, `graph.Candle.fromScalar(value)` |
| serve a table of books as the display | `BookService::new(options).with_table(name, holder)`, `Arc::new(service).route(&server, "/")?` (the `http` feature); `yggdryl market serve books=/data/books` | `yggdryl market serve books=/data/books`, the wheel's own command | `book.serve({ tables: 'books=/data/books' })` over the package's `book.js`; `yggdryl market serve` |
| a served table's readings without HTTP | `service.tickers("books")?`, `service.candles(&query)?`, `service.book(table, ticker, at)?`, `service.events(&query)?` | Rust-only | Rust-only |

## Rules for fast, correct use

1. Stay in the `marketdata` row for bulk. `MarketData.arrow_reader` writes
   column by column with no per-row `Scalar`, closing batches on a row and a
   byte bound; `from_arrow_reader` lands each batch once and resolves columns
   by name once per stream. Persist and exchange that row (Parquet, IPC)
   rather than bespoke per-leaf tables.
2. `from_arrow_reader` is tolerant of shape: any subset of columns, any order,
   any case, extra columns ignored, castable columns cast by one plan compiled
   before the first batch. A `marketdatakind` column and, for a dated leaf,
   `currunix` are the minimum: an undated `ORDR`, `QUOT` or `EXEC` row is an
   order, a quote or an execution, a dated one the event; `TRAD` and `BOOK`
   must be dated, and a `BOOK` row is a book where its `alive` cell is not
   null, a snapshot control where it is.
3. Views are `Plan`s run by the expression engine: `apply_view` binds once
   against the reader's schema and streams; `plan()` shows the text. Add a
   nested fact as a column with a lift (`identifiers['fix:clordid'].value as clordid`,
   a map read by its `src:type` key) instead of post-processing rows. The `lifecycle` view collects (it orders).
4. Feed `BookIterator` a **sorted** stream (by `snapunix`, else `currunix`); it
   leaves an operation dated before its book out with a warning, so an unsorted
   stream loses operations without an error. `FixCodec.market_data` is the
   sorted door for a FIX capture; `EventIterator(sorted=false)` sorts a finite
   stream itself.
5. One book per touched instant and book key - the input's ticker, else its
   category `{miccode}:{cficode}` (`XXXX`, `XXXXXX` for what it does not
   state). Depth persists, `deltas` and `executions` carry only that instant's
   changes. A positive `snapshot_millis` adds the complete live book at every
   crossed epoch-aligned tick.
6. A book row nests `alive`, `deltas`, `executions` (operation rows) and
   `bidlimits`, `asklimits` (one `Limit` per price level, best first: `price`,
   `quantity`, `uuids`, `tradable`). A level trades unless every entry at it
   states `tradable = false`; `best_price`, the book's `bidpx`/`askpx`, the
   spread and the crossed and locked readings read the first level that trades.
7. Join and chain by identity: `crossuuid` is one chain whatever identifier an
   event used; a later event joins a live one of its side and its
   `marketdatakind` through the type and value of an `identifiers` identifier
   (`orderid`, `clordid`, `mdentryid`..., whatever its source), and one stating
   no side joins the single side alive under its code. Name identifiers there,
   as `Identifier`s, rather than inventing a column.
8. Leaves are immutable in the bindings: `with_previous`, `merge_with`,
   `restating`, `with_book`, `with_operations` answer a new value; only
   `with_previous` and `merge_with` answer `None`/`null` when nothing moved.
   Build with named facts: a fact given as `...`/`undefined` is skipped,
   `None`/`null` clears it; a derived identity (`curruuid`, `crossuuid`,
   `currhashcode`, `crosshashcode`) is refused.
9. Sources are provenance: `srcuuids` never changes identity, never travels
   along a chain, and merges as a sorted union - use it to point back at the
   lines an event was read from.
10. Numbers are exact decimals: pass `Decimal('189.5')` in Python and the text
    `'189.5'` in JavaScript - a float price (`189.5`) is refused at `$.price`
    (`got f64`). Python answers `Scalar` (`.as_py()` -> `Decimal`), JavaScript
    exact text (`'189.5'`), Rust `Decimal`.
11. Candles bucket by a zone's wall clock. `CandleOptions` carries the interval
    (`30s`, `1m`, `5m`, `1h`, `1d`, `1w`, sub-second `ms`/`us`/`ns`) and a
    `Timezone`, UTC by default, so a daily candle opens at local midnight and
    hourly candles follow a saving-time change: the hour a spring-forward skips
    yields no candle, the hour a fall-back repeats is one two-hour candle, the
    day is 23 or 25 hours. Feed `CandleIterator` books sorted by `currunix`
    (a `BookIterator`'s are); a regression is refused at `$.book.currunix`.
    One candle per cross code and bucket: `bid`, `ask`, `mid` and `spread`
    each `{open, high, low, close}` over the books that stated one, `bidqty`/
    `askqty` the last book's touch, `volume` what traded - each trade counted
    once within the bucket, at the largest `lastqty` any of its executions
    states (never the order's `quantity`), a trade named by the `tradeid`,
    `tradereportid`, `tvtic` and `execid` identifiers its executions state, else
    by the cross code's base, so a trade's two sides and a fill delivered
    twice count once, and one stated again in the next bucket adds only what
    it states past what was counted - and an empty bucket yields no candle.

## Pitfalls

- A millisecond or second timestamp where nanoseconds are expected lands in
  1970: `graph.OrderEvent(1_700_000_000_000, ...)` is 28 minutes after the
  epoch. Multiply to nanoseconds first.
- `crosscode` answers the stored code: a buy order set to `O-1` reads
  `10:1:O-1`, and the lifecycle view and any lookup name it that way. A trade
  built on that order reads `21:0:O-1`: it is not sided.
- Python enum facts are `IntEnum` members (`order.side is Side.BUYS`); JavaScript
  getters answer the name (`'BUYS'`) while an Arrow column stores the code
  (`Side.BUYS === 1`). Compare against the one you hold.
- `BookIterator` over an unsorted list is no error: each operation dated
  before the book it would fold into is left out with a deduplicated warning,
  and the books lack it; sort first, or take the operations from
  `FixCodec.market_data`. The walk leaves out, never fails on, an order or a
  quote resting on no side and a group the book refuses; only a source's own
  failure ends it, and so does a value no book folds (the next bullet).
- A book folds only dated operations, trades and snapshot controls: an
  undated `Order` or a `BookEvent` is refused - by `BookIterator` at
  `$.operation.kind`, by `with_operations`/`add_operations` at
  `$.operations[i].kind`. An order or a quote whose side is neither the bid
  nor the ask cannot be placed on a side: `with_operations`/`add_operations`
  refuse it at `$.operation.side` and `BookIterator` leaves it out with a
  warning. An execution is never placed on a side and may state any,
  `UNKN` included.
- `EventIterator` defaults to `sorted=True` / `true` and trusts the order: an
  unsorted stream is not refused, it silently yields broken chains (an
  element before the live one is yielded as it came and joins nothing). Pass `sorted=False` / `false` for a stream you have not
  sorted (it collects to sort); only `BookIterator` notices a regression, and
  leaves it out with a warning.
- A trade is built only through `TradeEvent.from_parts`: at least one
  execution, each on any side - `UNKN` included - at the root's instant,
  one ticker, distinct cross codes.
- A lift names an identifier by its key, and keys are lower case:
  `identifiers['fix:clordid'].value`, never `['FIX:ClOrdID']` (that reads null).
  `securityids['base:isin'].value` reads the ISIN a leaf took without a source.
- An identifier map is no dict: compare `str(id)` / `id.toString()`, or read
  `get(type)` - the wire's `fix` first, then another named source, then
  `base`, then `derived` - and `get_from(src, type)`. An `Identifier` is its
  source, type and value: `key` is `src:type`, and
  `Identifier.from_key("fix:clordid", value)` reads a full key.
- `MarketData.kind` is `order_event` for a dated order; the leaf's own `kind`
  is `order`; both stand under `marketdatakind` `ORDR`.
- An order's `price` is what it states, never its last execution and never a
  bid: read `lastpx`/`lastqty` for fills, `bidpx`/`askpx` for the bid and ask
  a quote or a book states.
- `fxrates` maps a target currency to the rate an amount in the element's
  `currency` is divided by; nothing fills it, and a merge unions the targets.
- A leaf read from a FIX message keeps its parties and `Account(1)` in
  `partyids`, not `metadata`, and its `identifiers` can hold more than the
  message's: an identifier-like `metadata` key (`marketorderid`) is lifted into
  it (`base:marketorderid`); see the `yggdryl-fix` skill.
- The lifecycle view needs its chain: `apply_view("lifecycle", source,
  crosscode="10:1:O-1")`; every other view refuses a `crosscode`.
- Rust's `Limit` value type, the column enums' verbs (`EventColumn::fact`,
  `record`) and the `insert_`/`remove_`/`derive_` identifier verbs are
  Rust-only; the bindings answer a limit as a struct `Scalar` (Python) or a
  plain object (JavaScript) and state identifiers when they build a leaf.
- `graph.candles` and `CandleIterator` take `BookEvent`s, or `MarketData`
  holding one: a bare order is refused (`$.kind: expected book_event, got
  order_event`). A one-sided book states no `mid` and no `spread`, and a
  bucket whose ask side empties keeps the ask readings its earlier books made
  while `askqty` reads `None`/`null`, because the touch is the last book's.
- The display's routes render instants as RFC 9557 text with a bracketed zone,
  `2026-08-14T14:00:00.000000000+02:00[Europe/Zurich]`, which `Date.parse`
  does not read: hand a candle's `start`/`end` back as the next question's
  `from`, `to` or `at` - percent-encoded, as `URLSearchParams` does - rather
  than re-parsing them. A naive `from`/`to` is a wall clock in `tz`, and `to`
  is exclusive. An `events` row's `currhashcode`/`crosshashcode` are JSON
  integers up to 2^64: `JSON.parse` rounds them past 2^53, the CSV audit
  does not.
- A route's `tz` reads only the zones this build has rules for, and a zone
  it lacks is `400` at `$.tz`: offer the list `GET <prefix>/api/timezones`
  answers (`UTC` first) rather than the runtime's own zone list. A table's
  `url` and every refusal are stated without the location's user
  information and query, so a credential in a location reaches no client.
- `yggdryl market serve --capture` appends the capture's books to the first
  table every time it runs: prepare the table once, then serve it without the
  capture. The Iceberg table it makes of an absent folder needs the `iceberg`
  feature, which the wheel's command has. A capture is read with
  `--registry`, default `config/fix` under the working directory - a yggdryl
  checkout's committed dictionary; the wheel and the npm package ship none,
  so an installed command passes `--registry <folder>` or refuses
  (`expected a FIX dictionary at "file:///.../config/fix", got nothing`).
- The command prints a refusal on stdout, one `✗` line, and exits `1`; only
  the argument parser's own refusals go to stderr (exit `2`). `book.serve`
  rejects with that `✗` line, then any stderr, as the error's message.

## Language references

- Rust: [references/rust.md](references/rust.md) - `yggdryl::graph::*` leaves, traits, candles and the book service, `Decimal`, `Side`, `MarketDataKind`, `Ccy`.
- Python: [references/python.md](references/python.md) - `from yggdryl import graph`, `Side`, `MarketDataKind`, pyarrow readers, `graph.candles`.
- JavaScript: [references/javascript.md](references/javascript.md) - `graph`, `Side`, `MarketDataKind`, `graph.candles`, the package's `book.js`.

Read the one for the language you write; recipes appear in the same order in each.

## Deeper

- Graph overview and traits: https://platob.github.io/yggdryl/graph/
- Element (identity, cross code, sources): https://platob.github.io/yggdryl/graph/element/
- Event (instants, following, merging, the walk): https://platob.github.io/yggdryl/graph/event/
- Market facts, fill or overwrite, order quantities, security identifiers: https://platob.github.io/yggdryl/graph/market/
- `Identifier` and `Identifiers`, the `IdType`/`IdSource` vocabularies, parentage, FIX party naming: https://platob.github.io/yggdryl/graph/identifier/
- The three row schemas, column by column: https://platob.github.io/yggdryl/graph/schemas/
- Operation facts, identifiers and party ids: https://platob.github.io/yggdryl/graph/operation/
- Order, quote, execution leaves and book control: https://platob.github.io/yggdryl/graph/order/
- Trade: https://platob.github.io/yggdryl/graph/trade/
- Book, limits, snapshots, the fold: https://platob.github.io/yggdryl/graph/book/
- `MarketData`, columns, Arrow row, views: https://platob.github.io/yggdryl/graph/market-data/
- Candles, buckets and zones, the candle row: https://platob.github.io/yggdryl/graph/candle/
- The book display, `yggdryl market serve`, the routes, the components: https://platob.github.io/yggdryl/graph/serve/
- `Side`, `MarketDataKind`, `MarketDataType` and `TimeInForce`: https://platob.github.io/yggdryl/types/enum/
- Sibling skills: `yggdryl-fix` (FIX captures into market data and books),
  `yggdryl-expressions` (the `Plan` a view is), `yggdryl-records` (persisting
  `marketdata` batches), `yggdryl-arrow` (`BatchReader`, casts),
  `yggdryl-hashing` (the digests and UUIDs identities are built from),
  `yggdryl-types` (`Decimal`, the codes `ccy` and `forex`, the enums `side`,
  `marketdatakind`, `state`).
