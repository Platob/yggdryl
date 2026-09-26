# yggdryl-market-data in JavaScript

`const { graph } = require('yggdryl')` - every leaf is built from a plain object
of named facts (`undefined` skips one, `null` clears it) and is finalized and
immutable on construction. Decimals read back as exact text (`'189.5'`),
instants and 64-bit hashes as `bigint` nanoseconds since the epoch, UTC. The
replay service is `require('yggdryl/replay')`; the browser component is the ES
module `yggdryl/web/book-timeline.js`.

## Build an order event from named facts

The constructor takes the instant as a `bigint` and every other fact by its
column name; the identity and what the facts imply are derived on the spot.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

// A Date is milliseconds: scale to nanoseconds as a bigint.
const T = BigInt(Date.parse('2023-11-14T22:13:20Z')) * 1_000_000n
assert.equal(T, 1_700_000_000_000_000_000n)

const order = new graph.OrderEvent(T, {
  crosscode: 'O-1001',
  side: 'BUY',
  price: '189.50',
  quantity: 100,
  currency: 'USD',
  ticker: 'AAPL',
  securityids: { ISIN: 'US0378331005' },
  altids: { ORDERID: 'O-1001' },
})
// A dated identity is a UUIDv7: its millisecond leads.
assert.ok(order.curruuid.startsWith('018bcfe5-6800-7'))
assert.notEqual(order.crossuuid, order.curruuid, 'the cross code names a chain')
// Derived on construction: the CUSIP inside the ISIN, and the bid lane of a priced buy.
assert.deepEqual(order.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
assert.equal(order.bid.price, '189.5')
assert.equal(order.lastpx, null, 'a price is never a last execution')
assert.equal(order.state, '00UNKNOWN')
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote names its side
through the one lane it states, and a market-data entry carries a `BookRef`.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const order = new graph.Order({ crosscode: 'O-1001', side: 'BUY', price: '189.50' })
assert.equal(order.kind, 'order')
const event = order.at(1_700_000_000_000_000_000n)
assert.ok(event instanceof graph.OrderEvent)
assert.ok(event.intoElement().equals(order))

// One lane, no side of its own: the quote takes the lane's side.
const offer = new graph.QuoteEvent(1_700_000_000_000_000_000n, {
  ticker: 'AAPL',
  ask: new graph.Lane({ price: '189.52', quantity: 100, currency: 'USD' }),
})
assert.equal(offer.side, 'SELL')
assert.equal(offer.price, '189.52')

// The book control digests into the entry.
const entry = offer.withBook(new graph.BookRef({ action: 'new', position: 1 }))
assert.deepEqual([entry.action, entry.book.position], ['0', 1])
assert.notEqual(entry.curruuid, offer.curruuid)
```

## Chain two events and merge two statements of one

`withPrevious` states an event as the one after its predecessor; `mergeWith`
folds another statement of the same event (the later recording leads, sources
union). Both answer a new value, or `null` when nothing moved.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const event = (unix, state) => new graph.OrderEvent(unix, {
  crosscode: 'O-1001', state, side: 'BUY', price: '189.50', quantity: 100,
})

const placed = event(T, 'NEW')
const filled = event(T + 1_000_000_000n, 'PARTIALLY_FILLED').withPrevious(placed)
assert.deepEqual([filled.prevuuid, filled.prevunix, filled.seqnum], [placed.curruuid, T, 1])
assert.equal(filled.crossuuid, placed.crossuuid, 'one chain')
assert.equal(filled.prevpx, '189.5')
// Never itself, never one that happened after it.
assert.equal(placed.withPrevious(filled), null)

// One report recorded by two hops: recording clocks and sources are not content.
const LINE_1 = '018bcfe5-6800-7000-8000-000000000001'
const LINE_2 = '018bcfe5-6800-7000-8000-000000000002'
const hop = (recorded, line) => new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'O-1001', recdunix: recorded, srcuuids: [line] })
const gateway = hop(T + 1_002_000_000n, LINE_1)
const oms = hop(T + 1_005_000_000n, LINE_2)
assert.equal(gateway.curruuid, oms.curruuid)
const merged = oms.mergeWith(gateway)
assert.equal(merged.recdunix, T + 1_002_000_000n)
assert.deepEqual(merged.srcuuids, [LINE_1, LINE_2])
```

## Walk a stream into chains

`graph.EventIterator` chains a stream by cross identity (and by a live
element's `altids`), yields a twin as a restatement rather than a successor,
retires a chain at a terminal state and emits one `95EXPIRED` at a deadline.

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
  event(2n, 'O-1001', 'PARTIALLY_FILLED'),
  event(2n, 'O-1001', 'PARTIALLY_FILLED'),
  event(4n, 'O-1001', 'NEW'),
]
const chained = [...new graph.EventIterator(arrived, false)].map((value) => value.asOrderEvent())
assert.deepEqual(chained.map((held) => held.seqnum), [0, 1, 1, 2, 0])
assert.equal(chained[1].curruuid, chained[2].curruuid, 'a twin, not a successor')
assert.equal(chained[4].prevuuid, null, 'the fill ended the chain')

// A 10 ms grid: a view of the living order per tick, then its deadline.
const MS = 1_000_000n
const expiring = new graph.OrderEvent(T + 50n * MS, { crosscode: 'O-3003', exprtime: T + 70n * MS })
const timed = [...new graph.EventIterator([expiring], true, 10n * MS)].map((value) => value.asOrderEvent())
assert.ok(timed.some((held) => held.snapunix === T + 60n * MS))
const expired = timed[timed.length - 1]
assert.deepEqual([expired.currunix, expired.state], [T + 70n * MS, '95EXPIRED'])
```

## Build a composite trade

`graph.TradeEvent.fromParts` is the one door: a root event and its sided
executions, canonicalized so input order never changes the trade.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const fill = (code, side, unix = T) => new graph.ExecutionEvent(unix, {
  crosscode: code, side, lastpx: '189.50', lastqty: 100,
})
const root = new graph.OrderEvent(T, { crosscode: 'T-1', ticker: 'AAPL' })
const trade = graph.TradeEvent.fromParts(root, [fill('E-SELL', 'SELL'), fill('E-BUY', 'BUY')])
assert.deepEqual(trade.executions.map((execution) => execution.crosscode), ['E-BUY', 'E-SELL'])
assert.equal(trade.isExecution, true)
const again = graph.TradeEvent.fromParts(root, [fill('E-BUY', 'BUY'), fill('E-SELL', 'SELL')])
assert.equal(again.curruuid, trade.curruuid)
// Each execution at the trade's instant; none at all is refused.
assert.throws(() => graph.TradeEvent.fromParts(root, [fill('E-LATE', 'BUY', T + 1n)]), /\$\.executions\[0\]\.currunix/)
assert.throws(() => graph.TradeEvent.fromParts(root, []), /at least one execution/)
```

## Carry any leaf as one value

`graph.MarketData` holds any leaf and answers the element and market facts it
holds; `asOrderEvent()` and its siblings borrow it back, `intoLeaf()` answers
it as its own class.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'BUY' })
const value = new graph.MarketData(order)
assert.equal(value.kind, 'order_event')
assert.ok(graph.MarketData.kinds().includes('trade_event'))
assert.equal(value.isEvent, true)
assert.ok(value.asOrderEvent().equals(order))
assert.equal(value.asQuoteEvent(), null, 'another kind is none of this value')
assert.deepEqual([value.crosscode, value.side], ['O-1001', 'BUY'])
assert.ok(value.intoLeaf() instanceof graph.OrderEvent)
```

## Write and read the marketdata row

`MarketData.arrowReader` streams leaves into bounded batches of the lifted row;
`fromArrowReader` reads any source `BatchReader.from` accepts back, tolerant of
a subset of columns in any order.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, graph } = require('yggdryl')

const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
const values = [new graph.Order(), order, new graph.BookEvent(1_700_000_001_000_000_000n, 'AAPL')]

// 59 columns: kind, 16 event, 19 market, 8 operation, 5 book control, 3 book facts, 7 nested.
assert.equal(graph.MarketData.field().fieldLen, 59)
const table = graph.MarketData.arrowReader(values, 1_000).intoTable()
assert.deepEqual([...table.getChild('kind')], ['order', 'order_event', 'book_event'])
const read = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(values))]
read.forEach((held, at) => assert.ok(held.intoLeaf().equals(values[at])))

// A foreign table: three columns, one the row does not name.
const foreign = new arrow.Table({
  kind: arrow.vectorFromArray(['order_event'], new arrow.Utf8()),
  currunix: arrow.vectorFromArray([1_700_000_000_000_000_000n], new arrow.Int64()),
  crosscode: arrow.vectorFromArray(['O-1001'], new arrow.Utf8()),
  msgtype: arrow.vectorFromArray(['D'], new arrow.Utf8()),
})
const [lifted] = graph.MarketData.fromArrowReader(BatchReader.from(foreign))
assert.deepEqual([lifted.asOrderEvent().crosscode, lifted.asOrderEvent().currunix], ['O-1001', 1_700_000_000_000_000_000n])
```

## Persist a marketdata stream and read it back

Any record medium stores the batches as they stream - here Parquet by suffix -
and reading leaves back is the same `fromArrowReader`.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, graph } = require('yggdryl')

const values = [0n, 1n, 2n].map((at) => new graph.OrderEvent(1_700_000_000_000_000_000n + at, { crosscode: `O-${at}` }))

const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
const file = path.join(directory, 'marketdata.parquet')
new IOBase(file).overwriteArrowReader(graph.MarketData.arrowReader(values))
const read = [...graph.MarketData.fromArrowReader(new IOBase(file).readArrowReader())]
assert.deepEqual(read.map((value) => value.crosscode), ['O-0', 'O-1', 'O-2'])
read.forEach((value, at) => assert.ok(value.intoLeaf().equals(values[at])))
fs.rmSync(directory, { recursive: true, force: true })
```

## Fold a sorted stream into books

`new graph.BookIterator(items, snapshotMillis, global)` folds sorted operations
into one `BookEvent` per touched instant and symbol; depth persists, deltas and
executions are each book's own.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const SECOND = 1_000_000_000n
const bid = (unix, code, price, quantity) => new graph.OrderEvent(unix, {
  crosscode: code, ticker: 'AAPL', side: 'BUY', price, quantity,
})
const fill = new graph.ExecutionEvent(T + SECOND, {
  crosscode: 'E-1', ticker: 'AAPL', side: 'BUY', lastpx: '189.52', lastqty: 100,
})
const stream = [bid(T, 'B-1', '189.48', 300), bid(T + SECOND, 'B-2', '189.49', 200), fill]

const books = [...new graph.BookIterator(stream)]
assert.equal(books.length, 2, 'one book per touched instant')
const last = books[1]
assert.deepEqual([last.currunix, last.bid.length], [T + SECOND, 2], 'depth persists')
assert.equal(last.bid.deltas.length, 1)
assert.deepEqual(last.executions.map((execution) => execution.crosscode), ['E-1'])

// A 500 ms grid adds the living book at each crossed tick; global consolidates.
assert.equal([...new graph.BookIterator(stream, 500)].length, 3)
assert.ok([...new graph.BookIterator(stream, 0, true)].every((book) => book.crosscode === 'GLOBAL'))
// Out of order is refused.
assert.throws(() => [...new graph.BookIterator([...stream].reverse())], /sorted operation timestamp/)
```

## Read a book

A side answers its bests, its `limits` (one per price, best first, the unpriced
market level last) and `depth`; a book its spread, midpoint and imbalance, all
as exact decimal text.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const entry = (code, side, price, quantity) => new graph.OrderEvent(T, {
  crosscode: code, ticker: 'AAPL', side, price, quantity,
})
const book = new graph.BookEvent(T, 'AAPL').withOperations([
  entry('B-1', 'BUY', '189.48', 300),
  entry('B-2', 'BUY', '189.47', 500),
  entry('A-1', 'SELL', '189.52', 100),
  entry('MKT', 'BUY', undefined, 50),
])

assert.equal(book.bid.bestPrice, '189.48')
assert.equal(book.spread, '0.04')
assert.equal(book.bboMidpoint, '189.5')
assert.equal(book.price, book.bboMidpoint)
assert.equal(book.isLocked || book.isCrossed, false)
assert.equal(book.imbalance(1), '0.5')
assert.deepEqual(book.bid.limits.map((limit) => limit.price), ['189.48', '189.47', null])
assert.equal(book.bid.depth(2), '800')
assert.equal(book.bid.depth(3), '850')
```

## Replace a scope with a snapshot

A `SnapshotEvent` clears its `(symbol, scope)` partition on both sides - an empty
FIX `W` is one - and the book records what it replaced.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const order = (code, side, price) => new graph.OrderEvent(T, {
  crosscode: code, ticker: 'AAPL', side, price, quantity: 100,
})
const book = new graph.BookEvent(T, 'AAPL').withOperations([order('B-1', 'BUY', '189.48'), order('A-1', 'SELL', '189.52')])
assert.equal(book.bid.length + book.ask.length, 2)

const control = graph.SnapshotEvent.snapshot(new graph.OrderEvent(T + 1_000_000_000n, { ticker: 'AAPL' }))
assert.equal(control.book.action, 'snapshot')
const after = book.withOperations([control])
assert.ok(after.bid.isEmpty && after.ask.isEmpty)
assert.equal(after.price, null)
assert.deepEqual(after.snapshotPartitions.map((partition) => [partition.symbol, partition.scope]), [['AAPL', '']])
```

## Read the stream through named views

A view is one `Plan` over the `marketdata` row, applied by the expression
engine and bound once per reader; lifts turn nested facts into columns.

```javascript
const assert = require('node:assert/strict')
const { Plan, enums, graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const order = new graph.OrderEvent(T, { crosscode: 'O-1001', side: 'BUY', securityids: { ISIN: 'US0378331005' } })
const root = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'T-1' })
const trade = graph.TradeEvent.fromParts(root, [
  new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-1', side: 'BUY' }),
  new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-2', side: 'SELL' }),
])
const stream = () => graph.MarketData.arrowReader([order, trade])

assert.ok(['orders', 'trades', 'books', 'lifecycle'].every((view) => enums.marketViews.includes(view)))
const orders = graph.MarketData.applyView('orders', stream(), ["securityids['ISIN'] as isin"]).intoTable()
assert.equal(orders.schema.fields[orders.schema.fields.length - 1].name, 'isin')
assert.deepEqual([...orders.getChild('isin')], ['US0378331005'])

// One row per execution, beside the trade's own columns.
const trades = graph.MarketData.applyView('trades', stream()).intoTable()
assert.deepEqual([...trades.getChild('execution.crosscode')].sort(), ['E-1', 'E-2'])

// A view is a plan whose text reads back as the same plan.
const plan = graph.MarketData.plan('trades')
assert.ok(new Plan(plan.toString()).equals(plan))
const chain = graph.MarketData.applyView('lifecycle', stream(), undefined, 'O-1001').intoTable()
assert.deepEqual([...chain.getChild('crosscode')], ['O-1001'])
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `bookArrowReader` folds sorted messages into `book_event` rows, and
`MarketData.fromArrowReader` reads the books back.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')
const { fix, graph } = require('yggdryl')

// `config/fix` of a yggdryl checkout (see the yggdryl-fix skill).
const codec = new fix.FixCodec(fix.FixRegistry.fromHandle(path.resolve('config', 'fix')))
const lines = [
  '8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|',
  '8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|',
]
const capture = [...codec.parseLines(lines)]

const rows = codec.bookArrowReader(codec.lifecycle(capture))
const books = [...graph.MarketData.fromArrowReader(rows)].map((value) => value.asBookEvent())
assert.equal(books.length, 2)
assert.equal(books[1].bid.bestPrice, '101')
assert.equal(books[1].executions.length, 1)
```

## Serve a book replay

`yggdryl/replay` loads a source - `'synthetic'`, a `.parquet`/`.arrow` file of
`marketdata` operations, or a FIX capture - walks it with the native
`BookIterator`, renders each book with `bookJson` and serves the sources and a
server-sent book stream beside the page.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, graph } = require('yggdryl')
const replay = require('yggdryl/replay')

;(async () => {
  // Your own operations, stored as marketdata rows, load like any source.
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
  const file = path.join(directory, 'ops.parquet')
  new IOBase(file).overwriteArrowReader(graph.MarketData.arrowReader(replay.synthetic()))
  const source = replay.loadSource(file)
  assert.deepEqual([source.kind, source.operations.length, source.symbols], ['parquet', 23, ['ALPHA', 'BETA']])

  // The walk, indexed by symbol and instant; a book as the JSON the page reads.
  const books = replay.walk(source.operations)
  const index = replay.indexBooks(books)
  const alpha = replay.bookAt(index, 'ALPHA')
  const json = replay.bookJson(alpha)
  assert.equal(json.kind, 'book_event')
  assert.equal(json.currunix, alpha.currunix.toString(), 'instants cross as decimal text')
  assert.deepEqual(Object.keys(json.bid.limits[0]), ['price', 'quantity', 'uuids'])

  // The service: sources, then a server-sent stream of `book` events and an `end`.
  const service = replay.createReplayServer({ sources: [source] })
  const url = await service.listen(0)
  try {
    const listing = await (await fetch(new URL('/api/sources', url))).json()
    assert.equal(listing.sources[0].id, source.id)
    const stream = await (await fetch(new URL(`/api/sources/${source.id}/books?symbol=ALPHA`, url))).text()
    assert.ok(stream.startsWith('event: book'))
    assert.match(stream, /event: end\ndata: \{"count":6\}/)
  } finally {
    await service.close()
    fs.rmSync(directory, { recursive: true, force: true })
  }
})().catch((error) => { console.error(error); process.exit(1) })
```

From a shell, the same service with its page at `/web/app/`:

```bash
node node_modules/yggdryl/replay.js synthetic
node node_modules/yggdryl/replay.js marketdata.parquet --global --port 8080
node node_modules/yggdryl/replay.js session.log --registry path/to/config/fix --snapshot-millis 1000
```

## Show a book timeline in a page

`BookTimeline` is a dependency-free ES module for the browser: give it the
`bookJson` books of one walk, in walk order, and mount it; it emits
`ygg:book-select` as the instant moves. It needs a DOM, so this block is not
run under Node.

```html
<link rel="stylesheet" href="/node_modules/yggdryl/web/theme.css">
<div id="book"></div>
<script type="module">
  import { BookTimeline } from '/node_modules/yggdryl/web/book-timeline.js'

  // Books rendered server-side with `replay.booksJson(replay.walk(operations))`.
  const books = await (await fetch('/books.json')).json()
  const timeline = new BookTimeline({ books, title: 'ALPHA' }).mount(document.getElementById('book'))
  timeline.el.addEventListener('ygg:book-select', (event) => console.log(event.detail.currunix))
  timeline.select(0)
</script>
```

## Gotchas in JavaScript

- Instants are `bigint`: `T + 1_000_000_000n`, never `T + 1e9`; a `Date` is
  milliseconds - multiply `BigInt(date.getTime())` by `1_000_000n`.
- Decimals are exact text: pass and compare `'189.5'` (canonical, no trailing
  zero), never a `Number` - `{ price: 189.5 }` is refused at `$.price` (`got f64`).
- `new graph.EventIterator(items)` defaults `sorted` to `true` and trusts the
  order: an unsorted array is not refused, it yields broken chains (every
  `seqnum` 0); pass `false` for one you have not sorted.
- Iterators (`BookIterator`, `EventIterator`, `fromArrowReader`) and
  `BatchReader`s are one-shot: spread once, or rebuild the reader.
- Arrow JS interop is copied IPC, never zero copy: keep bulk work in the
  native readers and cross into Arrow JS once (`intoTable()`).
- `withOperations`, `withPrevious`, `mergeWith` answer a new value; the one
  you called is unchanged. Only `withPrevious`/`mergeWith` answer `null` when
  nothing moved; `withOperations` refuses an undated `Order` at
  `$.operations[i].kind` (`BookIterator` at `$.operation.kind`).
- The replay renders what the package answered and nothing else: to change a
  book, change the operations and walk again.
