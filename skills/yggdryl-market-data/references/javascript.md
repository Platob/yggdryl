# yggdryl-market-data in JavaScript

`const { graph } = require('yggdryl')` - every leaf is built from a plain object
of named facts (`undefined` skips one, `null` clears it) and is finalized and
immutable on construction. Decimals read back as exact text (`'189.5'`), the
enum facts - `side`, `state`, `marketdatakind` - as the member's stored name
(`'BUY'`, `'NEW'`, `'ORDR'`), instants and 64-bit hashes as `bigint`
nanoseconds since the epoch, UTC. `Side` and `MarketDataKind` at the package
root are frozen name-to-code objects.

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
// A sided element stores its cross code under its side: one chain per side.
assert.equal(order.crosscode, 'BUY:O-1001')
assert.notEqual(order.crossuuid, order.curruuid, 'the cross code names a chain')
// Derived on construction: the CUSIP inside the ISIN; the ISIN itself reads as `isincode`.
assert.deepEqual(order.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
assert.equal(order.isincode, 'US0378331005')
assert.equal(order.lastpx, null, 'a price is never a last execution')
assert.equal(order.bidpx, null, 'nor the bid an order states')
assert.deepEqual(order.fxrates, {}, 'nothing fills the rates')
assert.deepEqual([order.side, order.state, order.marketdatakind], ['BUY', 'UNKNOWN', 'ORDR'])
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote states its
own side and price, or the bid and ask it quotes as the six `bid*`/`ask*`
facts; a market-data entry carries a `BookRef`.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const order = new graph.Order({ crosscode: 'O-1001', side: 'BUY', price: '189.50' })
assert.deepEqual([order.kind, order.marketdatakind], ['order', 'ORDR'])
const event = order.at(T)
assert.ok(event instanceof graph.OrderEvent)
assert.ok(event.intoElement().equals(order))

// A two-sided quote: its bid and ask are facts, and it takes no side.
const quote = new graph.QuoteEvent(T, {
  crosscode: 'Q-7',
  ticker: 'AAPL',
  bidpx: '189.48',
  bidqty: 300,
  bidccy: 'USD',
  askpx: '189.52',
  askqty: 100,
  askccy: 'USD',
})
assert.deepEqual([quote.side, quote.crosscode], ['UNKNOWN', 'Q-7'])
assert.deepEqual([quote.askpx, quote.askqty, quote.marketdatakind], ['189.52', '100', 'QUOT'])

// A sided offer, placed in a book by its control; the scope is a fact, the rest walk-time.
const offer = new graph.QuoteEvent(T, { crosscode: 'Q-8', ticker: 'AAPL', side: 'SELL', price: '189.52', quantity: 100 })
const entry = offer.withBook(new graph.BookRef({ action: 'new', position: 1, scope: 'L2' }))
assert.deepEqual([entry.action, entry.book.position, entry.scope], ['0', 1, 'L2'])
assert.notEqual(entry.curruuid, offer.curruuid, 'the scope digests')
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
retires a chain at a terminal state and emits one `EXPIRED` at a deadline.
Chains are keyed by side, and every walked element leaves stating `creaunix`.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const SECOND = 1_000_000_000n
const event = (second, order, state, side = 'UNKNOWN') =>
  new graph.OrderEvent(T + second * SECOND, { crosscode: order, state, side })
const walk = (items, sorted = true) => [...new graph.EventIterator(items, sorted)].map((value) => value.asOrderEvent())

// Unsorted: one report logged twice, and O-1001 reopened after its fill.
const chained = walk([
  event(3n, 'O-1001', 'FILLED'),
  event(0n, 'O-1001', 'NEW'),
  event(2n, 'O-1001', 'PARTIALLY_FILLED'),
  event(2n, 'O-1001', 'PARTIALLY_FILLED'),
  event(4n, 'O-1001', 'NEW'),
], false)
assert.deepEqual(chained.map((held) => held.seqnum), [0, 1, 1, 2, 0])
assert.equal(chained[1].curruuid, chained[2].curruuid, 'a twin, not a successor')
assert.equal(chained[4].prevuuid, null, 'the fill ended the chain')
// A chain's creation instant is its first element's, carried along it.
assert.ok(chained.slice(0, 4).every((held) => held.creaunix === T))

// One identifier, two sides: two chains. A report stating no side joins the
// one side alive under its code, and a NEW over a live NEW reads UPDATED.
const walked = walk([event(0n, 'O-2002', 'NEW', 'BUY'), event(1n, 'O-2002', 'NEW', 'SELL'), event(2n, 'O-2002', 'NEW', 'BUY')])
assert.deepEqual([walked[1].crosscode, walked[1].seqnum], ['SELL:O-2002', 0])
assert.deepEqual([walked[2].prevuuid, walked[2].state], [walked[0].curruuid, 'UPDATED'])
const joined = walk([event(0n, 'O-3003', 'NEW', 'BUY'), event(1n, 'O-3003', 'CANCELED')])
assert.equal(joined[1].prevuuid, joined[0].curruuid)
assert.deepEqual([joined[1].side, joined[1].crosscode], ['BUY', 'BUY:O-3003'])

// A 10 ms grid: a view of the living order per tick, then its deadline.
const MS = 1_000_000n
const expiring = new graph.OrderEvent(T + 50n * MS, { crosscode: 'O-4004', exprunix: T + 70n * MS })
const timed = [...new graph.EventIterator([expiring], true, 10n * MS)].map((value) => value.asOrderEvent())
const view = timed.find((held) => held.snapunix === T + 60n * MS)
// Dated at its tick: the identity is the tick's, the content the order's.
assert.deepEqual([view.currunix, view.seqnum], [T + 60n * MS, 0])
assert.equal(view.currhashcode, expiring.currhashcode)
const expired = timed[timed.length - 1]
assert.deepEqual([expired.currunix, expired.state], [T + 70n * MS, 'EXPIRED'])
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
assert.deepEqual(trade.executions.map((execution) => execution.crosscode), ['BUY:E-BUY', 'SELL:E-SELL'])
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
assert.deepEqual([value.kind, value.marketdatakind], ['order_event', 'ORDR'])
assert.ok(graph.MarketData.kinds().includes('trade_event'))
assert.equal(value.isEvent, true)
assert.ok(value.asOrderEvent().equals(order))
assert.equal(value.asQuoteEvent(), null, 'another kind is none of this value')
assert.deepEqual([value.crosscode, value.side], ['BUY:O-1001', 'BUY'])
assert.ok(value.intoLeaf() instanceof graph.OrderEvent)
```

## Write and read the marketdata row

`MarketData.arrowReader` streams leaves into bounded batches of the lifted row;
`fromArrowReader` reads any source `BatchReader.from` accepts back, tolerant of
a subset of columns in any order. `marketdatakind` and, for a dated leaf,
`currunix` are the minimum.

```javascript
const assert = require('node:assert/strict')
const arrow = require('apache-arrow')
const { BatchReader, MarketDataKind, graph } = require('yggdryl')

const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
const values = [new graph.Order(), order, new graph.BookEvent(1_700_000_001_000_000_000n, 'AAPL')]

// 53 columns: marketdatakind, 16 event, 27 market, 3 operation, bookscope, 5 nested.
const field = graph.MarketData.field()
assert.equal(field.fieldLen, 53)
assert.equal(field.fieldAt(0).name, 'marketdatakind')
const table = graph.MarketData.arrowReader(values, 1_000).intoTable()
// The column stores each member's code.
assert.deepEqual([...table.getChild('marketdatakind')], [MarketDataKind.ORDR, MarketDataKind.ORDR, MarketDataKind.BOOK])
const read = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(values))]
read.forEach((held, at) => assert.ok(held.intoLeaf().equals(values[at])))

// A foreign table: three columns, one the row does not name.
const foreign = new arrow.Table({
  marketdatakind: arrow.vectorFromArray([MarketDataKind.ORDR], new arrow.Int32()),
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

`new graph.BookIterator(items, snapshotMillis = 0)` folds sorted operations
into one `BookEvent` per touched instant and book: an input's ticker, else its
category `MIC:CFI`. Depth persists; deltas and executions are each book's own.

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
assert.deepEqual([last.currunix, last.alive().length], [T + SECOND, 2], 'depth persists')
assert.equal(last.deltas().length, 1)
assert.deepEqual(last.executions().map((execution) => execution.crosscode), ['BUY:E-1'])

// A 500 ms grid adds the living book at each crossed tick.
assert.equal([...new graph.BookIterator(stream, 500)].length, 3)
// No ticker: the book is the category, `XXXX` or `XXXXXX` for what is unstated.
const [book] = new graph.BookIterator([new graph.OrderEvent(T, { crosscode: 'L-1', side: 'SELL', miccode: 'XNAS' })])
assert.deepEqual([book.crosscode, book.ticker], ['XNAS:XXXXXX', null])
// Out of order is refused.
assert.throws(() => [...new graph.BookIterator([...stream].reverse())], /sorted operation timestamp/)
```

## Read a book

A book answers each side as its `limits` (one per price, best first, the
unpriced market level last) and the readings of the first level that can
trade: `bestPrice`, `bestQuantity`, the `bidpx`/`askpx` it states, `spread`,
`depth`, `imbalance`, all as exact decimal text. A side is its stored name,
any spelling `Side` reads, or its code.

```javascript
const assert = require('node:assert/strict')
const { Side, graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const entry = (code, side, price, quantity, tradable) => new graph.OrderEvent(T, {
  crosscode: code, ticker: 'AAPL', side, price, quantity, tradable,
})
const book = new graph.BookEvent(T, 'AAPL').withOperations([
  entry('B-0', 'BUY', '189.49', 10, false),
  entry('B-1', 'BUY', '189.48', 300),
  entry('B-2', 'BUY', '189.47', 500),
  entry('A-1', 'SELL', '189.52', 100),
  entry('MKT', 'BUY', undefined, 50),
])

// The level at 189.49 cannot trade: the best bid is the first that can.
const limits = book.limits('BUY')
assert.deepEqual(limits.map((limit) => [limit.price, limit.tradable]), [
  ['189.49', false], ['189.48', true], ['189.47', true], [null, true],
])
assert.equal(book.bestPrice(Side.BUY), '189.48')
assert.deepEqual([book.bidpx, book.bidqty, book.askpx], ['189.48', '300', '189.52'])
assert.equal(book.spread, '0.04')
assert.equal(book.bboMidpoint, '189.5')
assert.equal(book.price, book.bboMidpoint)
assert.equal(book.isLocked || book.isCrossed, false)
assert.equal(book.depth('BUY', 2), '310')
assert.equal(book.alive().length, 5)
```

## Replace a scope with a snapshot

A `SnapshotEvent` clears its `(book, scope)` partition on both sides - an
empty FIX `W` is one.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const order = (code, side, price) => new graph.OrderEvent(T, {
  crosscode: code, ticker: 'AAPL', side, price, quantity: 100,
})
const book = new graph.BookEvent(T, 'AAPL').withOperations([order('B-1', 'BUY', '189.48'), order('A-1', 'SELL', '189.52')])
assert.equal(book.alive().length, 2)

const control = graph.SnapshotEvent.snapshot(new graph.OrderEvent(T + 1_000_000_000n, { ticker: 'AAPL' }))
assert.equal(control.book.action, 'snapshot')
const after = book.withOperations([control])
assert.deepEqual(after.alive(), [])
assert.deepEqual([after.price, after.bidpx, after.askpx], [null, null, null])
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

assert.deepEqual([...enums.marketViews].sort(), ['books', 'executions', 'lifecycle', 'orders', 'quotes', 'trades'])
const orders = graph.MarketData.applyView('orders', stream(), ["securityids['ISIN'] as isin"]).intoTable()
assert.equal(orders.schema.fields[orders.schema.fields.length - 1].name, 'isin')
assert.deepEqual([...orders.getChild('isin')], ['US0378331005'])

// One row per execution, beside the trade's own columns.
const trades = graph.MarketData.applyView('trades', stream()).intoTable()
assert.deepEqual([...trades.getChild('execution.crosscode')].sort(), ['BUY:E-1', 'SELL:E-2'])

// A view is a plan whose text reads back as the same plan.
const plan = graph.MarketData.plan('trades')
assert.ok(new Plan(plan.toString()).equals(plan))
// A lifecycle follows the cross code as stored: side included.
const chain = graph.MarketData.applyView('lifecycle', stream(), undefined, 'BUY:O-1001').intoTable()
assert.deepEqual([...chain.getChild('crosscode')], ['BUY:O-1001'])
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `bookArrowReader` folds sorted messages into book rows, and
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

const rows = codec.bookArrowReader(codec.lifecycle(capture), 0)
const values = [...graph.MarketData.fromArrowReader(rows)]
assert.deepEqual(values.map((value) => value.marketdatakind), ['BOOK', 'BOOK'])
const last = values[1].asBookEvent()
assert.equal(last.bestPrice('BUY'), '101')
assert.equal(last.executions().length, 1)
```

## Gotchas in JavaScript

- Instants are `bigint`: `T + 1_000_000_000n`, never `T + 1e9`; a `Date` is
  milliseconds - multiply `BigInt(date.getTime())` by `1_000_000n`.
- Decimals are exact text: pass and compare `'189.5'` (canonical, no trailing
  zero), never a `Number` - `{ price: 189.5 }` is refused at `$.price` (`got f64`).
  `fxrates` takes and answers `{ EUR: '1.1' }`.
- `crosscode` answers the stored code: `'BUY:O-1001'` for a buy, the bare code
  for `'UNKNOWN'`. `side` is never `null`; an Arrow column stores the code
  (`Side.BUY`, `MarketDataKind.ORDR`), a getter answers the name.
- `new graph.EventIterator(items)` defaults `sorted` to `true` and trusts the
  order: an unsorted array is not refused, it yields broken chains (every
  `seqnum` 0); pass `false` for one you have not sorted.
- Iterators (`BookIterator`, `EventIterator`, `fromArrowReader`) and
  `BatchReader`s are one-shot: spread once, or rebuild the reader.
- A book's `alive()`, `deltas()`, `executions()`, `limits(side)`,
  `bestPrice(side)`, `depth(side, n)` are methods; `spread`, `isCrossed`,
  `isLocked`, `bboMidpoint` are getters.
- Arrow JS interop is copied IPC, never zero copy: keep bulk work in the
  native readers and cross into Arrow JS once (`intoTable()`).
- `withOperations`, `withPrevious`, `mergeWith` answer a new value; the one
  you called is unchanged. Only `withPrevious`/`mergeWith` answer `null` when
  nothing moved; `withOperations` refuses an undated `Order` at
  `$.operations[i].kind` (`BookIterator` at `$.operation.kind`).
