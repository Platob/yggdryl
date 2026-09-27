---
name: yggdryl-market-data
description: Models and streams market data with yggdryl's graph layer in Rust, Python and Node.js - orders, quotes, executions and trades as dated events (OrderEvent, QuoteEvent, ExecutionEvent, TradeEvent), identities and side-keyed chains (curruuid, crossuuid, with_previous / withPrevious, EventIterator), order books (BookIterator, BookEvent, limits, best_price / bestPrice, spread, depth, SnapshotEvent), the Side and MarketDataKind enums and the lifted marketdata Arrow row (MarketData.arrow_reader / arrowReader, from_arrow_reader / fromArrowReader, apply_view / applyView). Use when building market events, folding them into books, or persisting or querying marketdata batches.
---

# yggdryl market data

The graph layer is market data as **elements that name each other by
identity**, never by reference. Four Rust traits say what an element answers -
`Element` (identity, cross code, digest, sources), `Event` (instant, state,
chain place, clocks), `Market` (twenty-seven facts: price, quantity, currency,
unit, side, security ids and the `isincode` they hold, classification, market,
last-trade, progress, FX parts, the stated bid and ask - `bidpx`, `bidqty`,
`bidccy`, `askpx`, `askqty`, `askccy` - the `fxrates`, ticker, metadata) and
`Operation` (three more: time in force, whether it trades, the `altids`) - and
typed leaves answer them: `Order`/`OrderEvent`, `Quote`/`QuoteEvent`,
`Execution`/`ExecutionEvent`, the composite `TradeEvent`, and the book types
`BookEvent` and `SnapshotEvent`. `MarketData` is the one value over every
leaf, and the lifted **`marketdata` Arrow row** (53 columns, led by the
`marketdatakind` its leaf stands under) is how any of them crosses a boundary.

Hold five facts:

- **Instants are `i64` nanoseconds since the Unix epoch, UTC** - `currunix`,
  `creaunix`, `execunix`, `recdunix`, `exprunix`, `prevunix`, `snapunix`.
  Python ints, JavaScript `bigint`s, Arrow `datetime64(ns, UTC)`.
- **Identity is derived, not assigned.** A leaf is finalized on construction:
  `currhashcode` is the XXH3-64 of its content, `curruuid` a UUIDv7 of
  instant + chain place for a dated leaf (UUIDv8 of content otherwise),
  `crossuuid` the chain every incarnation shares, from the cross code.
- **A side is never null, and it keys the chain.** `Side` and
  `MarketDataKind` are `int32` enums; an element stating no side holds
  `UNKNOWN` (code 0). A sided element stores its cross code under its side -
  `BUY:O-1001` - so the two sides of one identifier are two chains; `UNKNOWN`
  and books keep the bare code.
- **Chains are walked, not rebuilt.** An event names only its predecessor
  (`prevuuid`, `seqnum`); `EventIterator` joins a stream by cross identity and
  by the `altids` a live element went by, folds twins, emits expiries, and
  leaves every element stating `creaunix`. A grid view is the live element
  as of its tick: dated at it (`currunix` = `snapunix`), so its `curruuid`
  is the tick's own while content, `seqnum`, `prevuuid` and `crossuuid` are
  the live element's; it advances nothing.
- **Books are folded, not replayed by hand.** `BookIterator` folds a sorted
  stream into one `BookEvent` per book and instant; read books back from
  `marketdata` rows, never by re-applying their deltas.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| a dated order / quote / execution | `OrderEvent::at(unix)`, `set_*`, `finalize()` | `graph.OrderEvent(unix, **facts)` | `new graph.OrderEvent(unix, facts)` |
| an undated leaf, dated later | `Order::new()`, `order.at(unix)`, `event.into_element()` | `graph.Order(**facts)`, `.at(unix)`, `.into_element()` | `new graph.Order(facts)`, `.at(unix)`, `.intoElement()` |
| a two-sided quote | `set_bidpx`, `set_askpx`, `set_bidqty` ... | `graph.QuoteEvent(unix, bidpx=..., askpx=...)` | `new graph.QuoteEvent(unix, { bidpx, askpx })` |
| a market-data entry's book control | `event.with_book(BookRef { .. })` | `event.with_book(graph.BookRef(action="new", position=1))` | `event.withBook(new graph.BookRef({ action: 'new', position: 1 }))` |
| security identifiers | `insert_securityid(SecurityId::new(..)?)?` (Rust-only verbs) | `securityids={"ISIN": ...}` at build | `securityids: { ISIN: ... }` at build |
| FX rates (nothing fills them) | `insert_fxrate(ccy, rate)`, `set_fxrates(map)` | `fxrates={"EUR": Decimal("1.1")}` at build | `fxrates: { EUR: '1.1' }` at build |
| a composite trade | `TradeEvent::from_parts(&root, executions)?` | `graph.TradeEvent.from_parts(root, executions)` | `graph.TradeEvent.fromParts(root, executions)` |
| follow a predecessor | `event.with_previous(&prev)` | `event.with_previous(prev)` | `event.withPrevious(prev)` |
| merge two statements of one event | `event.merge_with(&other)` | `event.merge_with(other)` | `event.mergeWith(other)` |
| walk a stream into chains | `EventIterator::new(items, sorted)`, `.with_snapshot_ns(ns)` | `graph.EventIterator(items, sorted=True, snapshot_ns=None)` | `new graph.EventIterator(items, sorted, snapshotNs)` (sorted defaults to `true`) |
| any leaf as one value | `MarketData::from(leaf)`, `kind()`, `marketdatakind()`, `as_order_event()`, `TryFrom` | `graph.MarketData(leaf)`, `.kind`, `.marketdatakind`, `.as_order_event()`, `.into_leaf()` | `new graph.MarketData(leaf)`, `.kind`, `.marketdatakind`, `.asOrderEvent()`, `.intoLeaf()` |
| the `marketdata` row schema | `MarketData::field()?` | `graph.MarketData.field()` | `graph.MarketData.field()` |
| leaves to Arrow batches | `MarketData::arrow_reader(values, None, None)?` | `graph.MarketData.arrow_reader(values)` | `graph.MarketData.arrowReader(values)` |
| Arrow batches to leaves | `MarketData::from_arrow_reader(reader)?` | `graph.MarketData.from_arrow_reader(source)` | `graph.MarketData.fromArrowReader(reader)` |
| fold a sorted stream into books | `BookIterator::new(items, snapshot_millis)?` | `graph.BookIterator(items, snapshot_millis=0)` | `new graph.BookIterator(items, snapshotMillis = 0)` |
| one book by hand | `BookEvent::new(unix, symbol)`, `add_operations(..)?` | `graph.BookEvent(unix, symbol).with_operations([...])` | `new graph.BookEvent(unix, symbol).withOperations([...])` |
| a book's entries | `alive()`, `deltas()`, `executions()` | `book.alive`, `book.deltas`, `book.executions` | `book.alive()`, `book.deltas()`, `book.executions()` |
| read a side | `limits(Side::Buy)`, `best_price(Side::Buy)`, `best_quantity(..)`, `depth(Side::Buy, n)` | `book.limits(Side.BUY)`, `book.best_price(Side.BUY)`, `book.depth(Side.BUY, n)` | `book.limits('BUY')`, `book.bestPrice('BUY')`, `book.depth('BUY', n)` |
| read both sides | `get_bidpx()`, `get_askpx()`, `spread()`, `is_crossed()`, `imbalance(n)` | `book.bidpx`, `book.askpx`, `book.spread`, `book.is_crossed`, `book.imbalance(n)` | `book.bidpx`, `book.askpx`, `book.spread`, `book.isCrossed`, `book.imbalance(n)` |
| clear a scope with a snapshot | `SnapshotEvent::snapshot(&event, scope)` | `graph.SnapshotEvent.snapshot(event, scope=None)` | `graph.SnapshotEvent.snapshot(event, scope)` |
| a named view of a stream | `MarketData::apply_view(&MarketView::Orders, &lifts, reader)?` | `graph.MarketData.apply_view("orders", source, lifts)` | `graph.MarketData.applyView('orders', reader, lifts)` |
| a view as a plan | `MarketData::plan(&view, &lifts)?` | `graph.MarketData.plan("trades")` | `graph.MarketData.plan('trades')` |
| FIX to sorted market data / books | `codec.market_data(codec.lifecycle(msgs))`, `codec.book_arrow_reader(msgs, 0)?` | `codec.market_data(...)`, `codec.book_arrow_reader(msgs, snapshot_millis=0)` | `codec.marketData(..)`, `codec.bookArrowReader(msgs, 0)` |

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
   nested fact as a column with a lift (`securityids['ISIN'] as isin`) instead
   of post-processing rows. The `lifecycle` view collects (it orders).
4. Feed `BookIterator` a **sorted** stream (by `snapunix`, else `currunix`); it
   refuses a timestamp regression. `FixCodec.market_data` is the sorted door
   for a FIX capture; `EventIterator(sorted=false)` sorts a finite stream
   itself.
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
   event used; a later event joins a live one of its side through an `altids`
   pair (`ORDERID`, `CLORDID`, `MDENTRYID`...), and one stating no side joins
   the single side alive under its code. Name identifiers there, upper-cased,
   rather than inventing a column.
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

## Pitfalls

- A millisecond or second timestamp where nanoseconds are expected lands in
  1970: `graph.OrderEvent(1_700_000_000_000, ...)` is 28 minutes after the
  epoch. Multiply to nanoseconds first.
- `crosscode` answers the stored, side-prefixed code: a buy set to `O-1`
  reads `BUY:O-1`, and the lifecycle view and any lookup name it that way.
- Python enum facts are `IntEnum` members (`order.side is Side.BUY`); JavaScript
  getters answer the name (`'BUY'`) while an Arrow column stores the code
  (`Side.BUY === 1`). Compare against the one you hold.
- `BookIterator` over an unsorted list refuses the first regression at
  `$.operations` ("expected a sorted operation timestamp at or after ...");
  sort first, or take the operations from `FixCodec.market_data`.
- A book folds only dated operations, trades and snapshot controls: an
  undated `Order` or a `BookEvent` is refused - by `BookIterator` at
  `$.operation.kind`, by `with_operations`/`add_operations` at
  `$.operations[i].kind`. A live entry or an execution whose side is
  `UNKNOWN` cannot be placed on a side.
- `EventIterator` defaults to `sorted=True` / `true` and trusts the order: an
  unsorted stream is not refused, it silently yields broken chains (every
  event `seqnum` 0). Pass `sorted=False` / `false` for a stream you have not
  sorted (it collects to sort); only `BookIterator` refuses a regression.
- A trade is built only through `TradeEvent.from_parts`: at least one
  execution, each bid- or ask-sided, at the root's instant, one ticker,
  distinct cross codes.
- A lift key is matched exactly and stored upper case:
  `securityids['ISIN']`, never `securityids['isin']` (that reads null).
- `MarketData.kind` is `order_event` for a dated order; the leaf's own `kind`
  is `order`; both stand under `marketdatakind` `ORDR`.
- An order's `price` is what it states, never its last execution and never a
  bid: read `lastpx`/`lastqty` for fills, `bidpx`/`askpx` for the bid and ask
  a quote or a book states.
- `fxrates` maps a target currency to the rate an amount in the element's
  `currency` is divided by; nothing fills it, and a merge unions the targets.
- The lifecycle view needs its chain: `apply_view("lifecycle", source,
  crosscode="BUY:O-1")`; every other view refuses a `crosscode`.
- Rust's `Limit` value type, the column enums' verbs (`EventColumn::fact`,
  `record`) and the `insert_`/`remove_`/`derive_` identifier verbs are
  Rust-only; the bindings answer a limit as a struct `Scalar` (Python) or a
  plain object (JavaScript) and state identifiers when they build a leaf.

## Language references

- Rust: [references/rust.md](references/rust.md) - `yggdryl::graph::*` leaves and traits, `Decimal`, `Side`, `MarketDataKind`, `Ccy`.
- Python: [references/python.md](references/python.md) - `from yggdryl import graph`, `Side`, `MarketDataKind`, pyarrow readers.
- JavaScript: [references/javascript.md](references/javascript.md) - `graph`, `Side`, `MarketDataKind`.

Read the one for the language you write; recipes appear in the same order in each.

## Deeper

- Graph overview and traits: https://platob.github.io/yggdryl/graph/
- Element (identity, cross code, sources): https://platob.github.io/yggdryl/graph/element/
- Event (instants, following, merging, the walk): https://platob.github.io/yggdryl/graph/event/
- Market facts and security identifiers: https://platob.github.io/yggdryl/graph/market/
- Operation facts and identifier maps: https://platob.github.io/yggdryl/graph/operation/
- Order, quote, execution leaves and book control: https://platob.github.io/yggdryl/graph/order/
- Trade: https://platob.github.io/yggdryl/graph/trade/
- Book, limits, snapshots, the fold: https://platob.github.io/yggdryl/graph/book/
- `MarketData`, columns, Arrow row, views: https://platob.github.io/yggdryl/graph/market-data/
- `Side` and `MarketDataKind`: https://platob.github.io/yggdryl/types/enum/
- Sibling skills: `yggdryl-fix` (FIX captures into market data and books),
  `yggdryl-expressions` (the `Plan` a view is), `yggdryl-records` (persisting
  `marketdata` batches), `yggdryl-arrow` (`BatchReader`, casts),
  `yggdryl-hashing` (the digests and UUIDs identities are built from),
  `yggdryl-types` (`Decimal`, the codes `ccy` and `forex`, the enums `side`,
  `marketdatakind`, `state`).
