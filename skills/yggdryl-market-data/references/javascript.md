# yggdryl-market-data in JavaScript

`const { graph } = require('yggdryl')` - every leaf is built from a plain object
of named facts (`undefined` skips one, `null` clears it) and is finalized and
immutable on construction. Decimals read back as exact text (`'189.5'`), the
enum facts - `side`, `state`, `marketdatakind` - as the member's stored name
(`'BUYS'`, `'NEW'`, `'ORDR'`), instants and 64-bit hashes as `bigint`
nanoseconds since the epoch, UTC. `Side` and `MarketDataKind` at the package
root are frozen name-to-code objects. `securityids`, `identifiers` and `partyids` take an
array of `Identifier` (`require('yggdryl').Identifier`) and read back as an
`Identifiers` map keyed `src:type`.

## Build an order event from named facts

The constructor takes the instant as a `bigint` and every other fact by its
column name; the identity and what the facts imply are derived on the spot.

```javascript
const assert = require('node:assert/strict')
const { Identifier, graph } = require('yggdryl')

// A Date is milliseconds: scale to nanoseconds as a bigint.
const T = BigInt(Date.parse('2023-11-14T22:13:20Z')) * 1_000_000n
assert.equal(T, 1_700_000_000_000_000_000n)

const order = new graph.OrderEvent(T, {
  crosscode: 'O-1001',
  side: 'BUYS',
  price: '189.50',
  quantity: 100,
  currency: 'USD',
  ticker: 'AAPL',
  // An identifier is a source, a type and a value, unique by `src:type`; a code is held to its type's shape.
  securityids: [new Identifier('isin', 'US0378331005')],
  identifiers: [new Identifier('orderid', 'O-1001')],
})
// A dated identity is a UUIDv7: its millisecond leads.
assert.ok(order.curruuid.startsWith('018bcfe5-6800-7'))
// A cross code is stored as `{kind}:{side}:{base}`: one chain per side.
assert.equal(order.crosscode, '10:1:O-1001')
assert.notEqual(order.crossuuid, order.curruuid, 'the cross code names a chain')
// Derived on construction: the CUSIP inside the ISIN; the ISIN itself reads as `isincode`.
assert.equal(order.securityids.toString(), '[cusip=037833100, derived:cusip=037833100, isin=US0378331005]')
assert.equal(order.securityids.get('cusip'), '037833100')
assert.equal(order.isincode, 'US0378331005')
assert.equal(order.lastpx, null, 'a price is never a last execution')
assert.equal(order.bidpx, order.price, "a buy's price is its bid")
assert.deepEqual(order.fxrates, {}, 'nothing fills the rates')
assert.deepEqual([order.side, order.state, order.marketdatakind], ['BUYS', 'UNKNOWN', 'ORDR'])
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote is one
element holding a bid and an ask leg - the six `bid*`/`ask*` facts - stored
under side `0`; its `side` is a tag, so a quote stating a side and a price
states the leg that side takes. A market-data entry carries a `BookRef`.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const order = new graph.Order({ crosscode: 'O-1001', side: 'BUYS', price: '189.50' })
assert.deepEqual([order.kind, order.marketdatakind], ['order', 'ORDR'])
const event = order.at(T)
assert.ok(event instanceof graph.OrderEvent)
assert.ok(event.intoElement().equals(order))

// A two-sided quote: its bid and ask are its two legs, and it tags no side.
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
assert.deepEqual([quote.side, quote.crosscode], ['UNKN', '14:0:Q-7'])
assert.deepEqual([quote.askpx, quote.askqty, quote.marketdatakind], ['189.52', '100', 'QUOT'])

// An offer: tagged `SELL`, its price is its ask leg; a quote's code stays under side 0.
const offer = new graph.QuoteEvent(T, { crosscode: 'Q-8', ticker: 'AAPL', side: 'SELL', price: '189.52', quantity: 100 })
assert.deepEqual([offer.crosscode, offer.askpx], ['14:0:Q-8', '189.52'])
// Placed in a book by its control; the scope is a fact, the rest walk-time.
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
  crosscode: 'O-1001', state, side: 'BUYS', price: '189.50', quantity: 100,
})

const placed = event(T, 'NEW')
const filled = event(T + 1_000_000_000n, 'PARTIALLY_FILLED').withPrevious(placed)
// A later instant keeps its own place: the first there.
assert.deepEqual([filled.prevuuid, filled.prevunix, filled.seqnum], [placed.curruuid, T, 0])
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
element's `identifiers`), yields a twin as a restatement rather than a successor,
retires a chain at a terminal state and emits one `EXPIRED` at a deadline.
An order's or an execution's chain is keyed by side - a quote's is one chain
whatever side it tags - a chain lives within one `marketdatakind` (an order and
an execution under one cross code are two chains), and every walked element
leaves stating `creaunix`.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const SECOND = 1_000_000_000n
const event = (second, order, state, side = 'UNKN') =>
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
// Each step is at an instant of its own, so each is the first there; the
// chain is in prevuuid.
assert.deepEqual(chained.map((held) => held.seqnum), [0, 0, 0, 0, 0])
assert.equal(chained[3].prevuuid, chained[1].curruuid)
assert.equal(chained[1].curruuid, chained[2].curruuid, 'a twin, not a successor')
assert.equal(chained[4].prevuuid, null, 'the fill ended the chain')
// A chain's creation instant is its first element's, carried along it.
assert.ok(chained.slice(0, 4).every((held) => held.creaunix === T))

// One identifier, two sides: two chains. A report stating no side joins the
// one side alive under its code, and a NEW over a live NEW reads UPDATED.
const walked = walk([event(0n, 'O-2002', 'NEW', 'BUYS'), event(1n, 'O-2002', 'NEW', 'SELL'), event(2n, 'O-2002', 'NEW', 'BUYS')])
assert.deepEqual([walked[1].crosscode, walked[1].seqnum], ['10:2:O-2002', 0])
assert.deepEqual([walked[2].prevuuid, walked[2].state], [walked[0].curruuid, 'UPDATED'])
const joined = walk([event(0n, 'O-3003', 'NEW', 'BUYS'), event(1n, 'O-3003', 'CANCELED')])
assert.equal(joined[1].prevuuid, joined[0].curruuid)
assert.deepEqual([joined[1].side, joined[1].crosscode], ['BUYS', '10:1:O-3003'])

// A 10 ms grid: a view of the living order per tick, then its deadline.
const MS = 1_000_000n
const expiring = new graph.OrderEvent(T + 50n * MS, { crosscode: 'O-4004', exprunix: T + 70n * MS })
const timed = [...new graph.EventIterator([expiring], true, 10n * MS)].map((value) => value.asOrderEvent())
const view = timed.find((held) => held.snapunix !== null && held.currunix === T + 60n * MS)
// Dated at its tick: the identity is the tick's, the content the order's,
// and its snapshot instant the one the order was stated at.
assert.equal(view.snapunix, T + 50n * MS)
assert.deepEqual([view.currunix, view.seqnum], [T + 60n * MS, 0])
assert.equal(view.currhashcode, expiring.currhashcode)
const expired = timed[timed.length - 1]
assert.deepEqual([expired.currunix, expired.state], [T + 70n * MS, 'EXPIRED'])
```

## Build a composite trade

`graph.TradeEvent.fromParts` is the one door: a root event and its sided
executions, canonicalized so input order never changes the trade. The trade
itself is not sided: it keeps the root's cross code, a sided root's base code.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const fill = (code, side, unix = T) => new graph.ExecutionEvent(unix, {
  crosscode: code, side, lastpx: '189.50', lastqty: 100,
})
const root = new graph.OrderEvent(T, { crosscode: 'T-1', ticker: 'AAPL' })
const trade = graph.TradeEvent.fromParts(root, [fill('E-SELL', 'SELL'), fill('E-BUYS', 'BUYS')])
assert.deepEqual(trade.executions.map((execution) => execution.crosscode), ['8:1:E-BUYS', '8:2:E-SELL'])
assert.equal(trade.isExecution, true)
const again = graph.TradeEvent.fromParts(root, [fill('E-BUYS', 'BUYS'), fill('E-SELL', 'SELL')])
assert.equal(again.curruuid, trade.curruuid)
// Each execution at the trade's instant; none at all is refused.
assert.throws(() => graph.TradeEvent.fromParts(root, [fill('E-LATE', 'BUYS', T + 1n)]), /\$\.executions\[0\]\.currunix/)
assert.throws(() => graph.TradeEvent.fromParts(root, []), /at least one execution/)
```

## Carry any leaf as one value

`graph.MarketData` holds any leaf and answers the element and market facts it
holds; `asOrderEvent()` and its siblings borrow it back, `intoLeaf()` answers
it as its own class.

```javascript
const assert = require('node:assert/strict')
const { graph } = require('yggdryl')

const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'BUYS' })
const value = new graph.MarketData(order)
assert.deepEqual([value.kind, value.marketdatakind], ['order_event', 'ORDR'])
assert.ok(graph.MarketData.kinds().includes('trade_event'))
assert.equal(value.isEvent, true)
assert.ok(value.asOrderEvent().equals(order))
assert.equal(value.asQuoteEvent(), null, 'another kind is none of this value')
assert.deepEqual([value.crosscode, value.side], ['10:1:O-1001', 'BUYS'])
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

// 62 columns: 6 element, 9 event, 34 market (marketdatakind first), 5 operation,
// the book controls bookscope, bookaction and bookposition, 5 nested.
const field = graph.MarketData.field()
assert.equal(field.fieldLen, 62)
assert.equal(field.fieldAt(15).name, 'marketdatakind')
const table = graph.MarketData.arrowReader(values, 1_000).intoTable()
// The column stores each member's code.
assert.deepEqual([...table.getChild('marketdatakind')], [MarketDataKind.ORDR, MarketDataKind.ORDR, MarketDataKind.BOOK])
const read = [...graph.MarketData.fromArrowReader(graph.MarketData.arrowReader(values))]
read.forEach((held, at) => assert.ok(held.intoLeaf().equals(values[at])))

// A foreign table: three columns, one the row does not name.
const foreign = new arrow.Table({
  marketdatakind: arrow.vectorFromArray([MarketDataKind.ORDR], new arrow.Int32()),
  currunix: arrow.vectorFromArray([1_700_000_000_000_000_000n], new arrow.Int64()),
  crosscode: arrow.vectorFromArray(['10:0:O-1001'], new arrow.Utf8()),
  msgtype: arrow.vectorFromArray(['D'], new arrow.Utf8()),
})
const [lifted] = graph.MarketData.fromArrowReader(BatchReader.from(foreign))
assert.deepEqual([lifted.asOrderEvent().crosscode, lifted.asOrderEvent().currunix], ['10:0:O-1001', 1_700_000_000_000_000_000n])
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
assert.deepEqual(read.map((value) => value.crosscode), ['10:0:O-0', '10:0:O-1', '10:0:O-2'])
read.forEach((value, at) => assert.ok(value.intoLeaf().equals(values[at])))
fs.rmSync(directory, { recursive: true, force: true })
```

## Fold a sorted stream into books

`new graph.BookIterator(items, snapshotMillis = 0, filter = undefined)` folds
sorted orders and quotes into one `BookEvent` per instant and book key that
moved it - the instrument's ISIN, else its ticker, else `XX0000000000` -
recording each execution among its instant's deltas, moving no side, and
pruning every trade. A book is complete (`isComplete`) only at a
snapshot tick; every other book states its deltas alone beside the top of book
they settled on, and `withPrevious` over the complete book before it rebuilds
it whole. `filter` - a predicate over the `marketdata` row - narrows what folds.

```javascript
const assert = require('node:assert/strict')
const { Identifier, graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const SECOND = 1_000_000_000n
const bid = (unix, code, price, quantity) => new graph.OrderEvent(unix, {
  crosscode: code, ticker: 'AAPL', side: 'BUYS', price, quantity, state: 'NEW',
})
const fill = new graph.ExecutionEvent(T + SECOND, {
  crosscode: 'E-1', ticker: 'AAPL', side: 'BUYS', lastpx: '189.52', lastqty: 100,
})
const stream = [bid(T, 'B-1', '189.48', 300), bid(T + SECOND, 'B-2', '189.49', 200), fill]

const books = [...new graph.BookIterator(stream)]
assert.equal(books.length, 2, 'one book per instant that moved it; the execution is recorded beside the better bid')
// No grid and no snapshot input: each book states its deltas alone and its top of book.
const last = books[1]
assert.equal(last.isComplete, false)
assert.deepEqual([last.currunix, last.deltas().length, last.alive().length], [T + SECOND, 2, 0])
assert.equal(last.bestPrice('BUYS'), '189.49')
// Rebuilt whole: the first over the empty book its key starts from, the next over it.
assert.equal(books[0].prevuuid, null)
const first = books[0].withPrevious(graph.BookEvent.keyed(T, 'AAPL'))
const whole = last.withPrevious(first)
assert.equal(whole.isComplete, true)
assert.deepEqual([whole.alive().length, whole.curruuid], [2, last.curruuid], 'depth persists')

// A 500 ms grid states the whole living book at each crossed tick.
const gridded = [...new graph.BookIterator(stream, 500)]
assert.equal(gridded.length, 3)
assert.ok(gridded.every((book) => book.isComplete))
// A filter narrows what folds: one keeping the execution alone folds its book.
assert.equal([...new graph.BookIterator(stream, 0, "marketdatakind = 'EXEC'")].length, 1)
// The book key: the instrument's ISIN, else the ticker, else `XX0000000000`.
const [keyless] = new graph.BookIterator([new graph.OrderEvent(T, { crosscode: 'L-1', side: 'SELL' })])
assert.deepEqual([keyless.crosscode, keyless.ticker], ['3:0:XX0000000000', null])
const listed = new graph.OrderEvent(T, {
  crosscode: 'L-2', side: 'SELL', ticker: 'AAPL', securityids: [new Identifier('isin', 'US0378331005')],
})
const [book] = new graph.BookIterator([listed])
assert.deepEqual([book.crosscode, book.isincode, book.ticker], ['3:0:US0378331005', 'US0378331005', 'AAPL'])
// Out of order is no error: the operation dated before its book is left out,
// with a warning on standard error (unless a `logging` handler takes it).
assert.equal([...new graph.BookIterator([...stream].reverse())].length, 1)
```

## Read a book

A complete book answers each side as its `limits` (one per price, best
first, the unpriced market level last) and its entries as `aliveOn(side)`;
every book answers the readings of the first level that can trade:
`bestPrice`, `bestQuantity`, the `bidpx`/`askpx` it states, `spread`; a
complete one `depth` and `imbalance` too - all as exact decimal text. A book
built by hand is complete. A side is its stored name, any spelling `Side`
reads, or its code.

```javascript
const assert = require('node:assert/strict')
const { Side, graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const entry = (code, side, price, quantity, tradable) => new graph.OrderEvent(T, {
  crosscode: code, ticker: 'AAPL', side, price, quantity, tradable,
})
const book = new graph.BookEvent(T, 'AAPL').withOperations([
  entry('B-0', 'BUYS', '189.49', 10, false),
  entry('B-1', 'BUYS', '189.48', 300),
  entry('B-2', 'BUYS', '189.47', 500),
  entry('A-1', 'SELL', '189.52', 100),
  entry('MKT', 'BUYS', undefined, 50),
])

// The level at 189.49 cannot trade: the best bid is the first that can.
const limits = book.limits('BUYS')
assert.deepEqual(limits.map((limit) => [limit.price, limit.tradable]), [
  ['189.49', false], ['189.48', true], ['189.47', true], [null, true],
])
assert.equal(book.bestPrice(Side.BUYS), '189.48')
assert.deepEqual([book.bidpx, book.bidqty, book.askpx], ['189.48', '300', '189.52'])
assert.equal(book.spread, '0.04')
assert.equal(book.bboMidpoint, '189.5')
assert.equal(book.price, book.bboMidpoint)
assert.equal(book.isLocked || book.isCrossed, false)
assert.equal(book.depth('BUYS', 2), '310')
assert.equal(book.isComplete, true)
assert.deepEqual([book.alive().length, book.aliveOn('BUYS').length, book.aliveOn(Side.SELL).length], [5, 4, 1])
// The deltas are the five orders, in the order applied.
assert.deepEqual(book.deltas().map((delta) => delta.crosscode), ['10:1:B-0', '10:1:B-1', '10:1:B-2', '10:2:A-1', '10:1:MKT'])
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
const book = new graph.BookEvent(T, 'AAPL').withOperations([order('B-1', 'BUYS', '189.48'), order('A-1', 'SELL', '189.52')])
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
const { Identifier, Plan, enums, graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const order = new graph.OrderEvent(T, {
  crosscode: 'O-1001', side: 'BUYS', identifiers: [new Identifier('clordid', 'C-1')],
})
const root = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'T-1' })
const trade = graph.TradeEvent.fromParts(root, [
  new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-1', side: 'BUYS' }),
  new graph.ExecutionEvent(T + 1_000_000_000n, { crosscode: 'E-2', side: 'SELL' }),
])
const stream = () => graph.MarketData.arrowReader([order, trade])

assert.deepEqual([...enums.marketViews].sort(), ['books', 'executions', 'lifecycle', 'orders', 'quotes', 'trades'])
// A lift reaches one identifier of the map by its key.
const orders = graph.MarketData.applyView('orders', stream(), ["identifiers['clordid'] as clordid"]).intoTable()
assert.equal(orders.schema.fields[orders.schema.fields.length - 1].name, 'clordid')
assert.deepEqual([...orders.getChild('clordid')], ['C-1'])

// One row per execution, beside the trade's own columns.
const trades = graph.MarketData.applyView('trades', stream()).intoTable()
assert.deepEqual([...trades.getChild('execution.crosscode')].sort(), ['8:1:E-1', '8:2:E-2'])

// A view is a plan whose text reads back as the same plan.
const plan = graph.MarketData.plan('trades')
assert.ok(new Plan(plan.toString()).equals(plan))
// A lifecycle follows the cross code as stored: side included.
const chain = graph.MarketData.applyView('lifecycle', stream(), undefined, '10:1:O-1001').intoTable()
assert.deepEqual([...chain.getChild('crosscode')], ['10:1:O-1001'])
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `bookArrowReader` folds sorted messages into book rows - orders,
quotes and `W`/`X` entries, a trade entry recorded as the execution it is - and
`MarketData.fromArrowReader` reads the books back. A `W` full refresh is a
snapshot input, so its book is complete; the `X` after it states its delta.

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
assert.equal(values[0].asBookEvent().isComplete, true)
const last = values[1].asBookEvent()
assert.equal(last.bestPrice('BUYS'), '101')
// The bid's change and the trade entry (`269=2`), recorded as the execution it is, are the deltas.
assert.deepEqual([last.isComplete, last.deltas().length], [false, 2])
```

## Read a table of books back as its deltas

`graph.MarketData.deltasSerie(source, kind?)` lays the deltas of every book a
`Serie`, a `ChunkedSerie` or a `SerieReader` (consumed) holds back out as
`marketdata` rows in book order - every kind, or the one `kind` names - as a
`SerieReader` pulled with the books: how a stage reads a window's orders,
quotes or executions out of a table of books.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase, graph } = require('yggdryl')

const T = 1_700_000_000_000_000_000n
const bid = new graph.OrderEvent(T, { crosscode: 'B-1', ticker: 'AAPL', side: 'BUYS', price: '189', quantity: 100 })
const fill = new graph.ExecutionEvent(T + 1n, { crosscode: 'E-1', ticker: 'AAPL', side: 'BUYS', lastqty: 100 })

const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
// A table of books: one per instant, the execution recorded in the second.
const table = new IOBase(path.join(directory, 'books.arrows'))
table.overwriteArrowReader(graph.MarketData.arrowReader([...new graph.BookIterator([bid, fill])]))

const deltas = (kind) => {
  const rows = graph.MarketData.deltasSerie(table.readSerie(), kind)
  return [...graph.MarketData.fromArrowReader(rows.intoArrowReader())].map((value) => value.crosscode)
}
assert.deepEqual(deltas(), ['10:1:B-1', '8:1:E-1'])
assert.deepEqual(deltas('EXEC'), ['8:1:E-1'])
fs.rmSync(directory, { recursive: true, force: true })
```

## Fold books into candles

`graph.candles(books, interval, timezone)` folds books sorted by their instant
into one `Candle` per book cross code and bucket - the best bid, the best ask,
the mid and the spread each `{ open, high, low, close }` of decimal text, the
touch when the bucket closed and how many books folded - and
`new graph.CandleIterator(books, options)` is the same walk, lazy. It reads a
book's top of book alone, so delta books fold as complete ones do. The
interval is a spelling or a `bigint` of nanoseconds; the zone is what the
buckets' wall clock aligns to.

```javascript
const assert = require('node:assert/strict')
const { Serie, graph } = require('yggdryl')

// 2023-11-14T22:13:20Z.
const T = 1_700_000_000_000_000_000n
const SECOND = 1_000_000_000n
const quote = (unix, code, side, price, quantity) => new graph.QuoteEvent(unix, {
  crosscode: code, ticker: 'AAPL', side, price, quantity, state: 'NEW',
})
const stream = [
  quote(T, 'B1', 'BUYS', '189.48', 300),
  quote(T, 'A1', 'SELL', '189.52', 100),
  quote(T + 20n * SECOND, 'B2', 'BUYS', '189.50', 200),
  quote(T + 70n * SECOND, 'A2', 'SELL', '189.51', 50),
]
// Three books: the two quotes at T share one instant.
const books = [...new graph.BookIterator(stream)]

// Minute candles in UTC: the 22:13 and 22:14 buckets.
const [first, second] = graph.candles(books, '1m')
assert.deepEqual([first.crosscode, first.start, first.end], ['3:0:AAPL', 1_699_999_980n * SECOND, 1_700_000_040n * SECOND])
assert.deepEqual(first.bid, { open: '189.48', high: '189.5', low: '189.48', close: '189.5' })
assert.deepEqual(first.spread, { open: '0.04', high: '0.04', low: '0.02', close: '0.02' })
assert.deepEqual([first.bidqty, first.askqty, first.books], ['200', '100', 2])
// A2 undercuts A1: the second bucket's ask opens at 189.51 and the spread narrows.
assert.equal(second.ask.open, '189.51')
assert.equal(second.mid.close, '189.505')
const [walked] = new graph.CandleIterator(books, new graph.CandleOptions('1m'))
assert.ok(walked.equals(first))

// Buckets align to the zone's wall clock: 22:13:20Z is 23:13:20 in Zurich,
// so its daily candle opens at Zurich midnight, 23:00Z the day before.
const [daily] = graph.candles(books, '1d', 'Europe/Zurich')
assert.deepEqual([daily.start, daily.books], [1_699_916_400n * SECOND, 3])
const [utc] = graph.candles(books, new graph.CandleOptions('1d'))
assert.equal(utc.start, 1_699_920_000n * SECOND)

// Candles cross as rows of Candle.field(), as JSON, and read back as the same values.
const field = graph.Candle.field()
assert.equal(field.fieldLen, 23)
const rows = Serie.fromScalars(field, [first.intoScalar(), second.intoScalar()])
assert.ok(graph.Candle.fromScalar(rows.scalar(1)).equals(second))
assert.equal(JSON.parse(JSON.stringify(second)).askopen, '189.51')
assert.ok(graph.Candle.fromJSON(JSON.stringify(second)).equals(second))
// Books must arrive sorted; a bare quote is not a book.
assert.throws(() => graph.candles([...books].reverse(), '1m'), /\$\.book\.currunix/)
assert.throws(() => graph.candles([stream[0]], '1m'), /expected book_event, got quote_event/)
```

## Start the display beside a Node program

`require('yggdryl/book')` is the package's door to the display: `assets` is the folder of
the display's files, `serveArguments(options)` the argument vector `yggdryl
market serve` takes, and `serve(options)` spawns the command - `bin` from
`YGGDRYL_BIN`, else `yggdryl` on the path - resolving `{ endpoint, process,
close() }` once it has printed its endpoint. A process that exits first
rejects with what it printed as the message: the command's refusal, the `✗`
line it prints on stdout in the endpoint's place (`✗ invalid record value at
$.capture: ...`), then its stderr, where the argument parser refuses; an
aborted `signal` (`AbortSignal.timeout(ms)`) rejects with an `AbortError`
once it has ended the process. A table is
`'name=location'`, a location or `{ name, location }` - an Iceberg folder,
`s3tables://<bucket>/<namespace>/<table>`, a `.arrows`, `.parquet`, `.avro` or
`.csv` leaf, a partitioned folder; `capture` folds FIX bridge logs into the
first table before serving, and every other flag of
[the command](https://platob.github.io/yggdryl/graph/serve/#the-command) -
`--snapshot-millis`, `--registry`, `--timezone`, `--public-url` ... - rides
`args`, verbatim, last.

```javascript
const assert = require('node:assert/strict')
const path = require('node:path')

const book = require('yggdryl/book')

assert.equal(book.assetFiles.length, 8)
assert.equal(book.assetFiles[0], 'index.html')
assert.deepEqual(
  book.serveArguments({ tables: [{ name: 'books', location: '/data/books' }], bind: '127.0.0.1:8080', capture: ['bridge.log'] }),
  ['market', 'serve', 'books=/data/books', '--bind', '127.0.0.1:8080', '--path', '/', '--capture', 'bridge.log'],
)
// Unstated, `bind` takes a free port; the other flags follow in `args`.
assert.deepEqual(
  book.serveArguments({ tables: 'books=/data/books', args: ['--snapshot-millis', '1000', '--registry', '/etc/fix'] }),
  ['market', 'serve', 'books=/data/books', '--bind', '127.0.0.1:0', '--path', '/', '--snapshot-millis', '1000', '--registry', '/etc/fix'],
)
// `serve` runs that vector and hands back the endpoint to open, or rejects with
// the refusal the command printed in its place - its `✗` line, then any stderr:
//   const { endpoint, close } = await book.serve({ tables: 'books=/data/books' })
assert.equal(typeof book.serve, 'function')
```

## Gotchas in JavaScript

- Instants are `bigint`: `T + 1_000_000_000n`, never `T + 1e9`; a `Date` is
  milliseconds - multiply `BigInt(date.getTime())` by `1_000_000n`.
- Decimals are exact text: pass and compare `'189.5'` (canonical, no trailing
  zero), never a `Number` - `{ price: 189.5 }` is refused at `$.price` (`got f64`).
  `fxrates` takes and answers `{ EUR: '1.1' }`.
- `crosscode` answers the stored code `{kind}:{side}:{base}`: `'10:1:O-1001'`
  for a buy order (kind 10, side 1), `'10:0:O-1001'` for `'UNKN'`, and a
  quote, a trade, a book or a snapshot control carries side `0` whatever side
  it states.
  `side` is never `null`; an Arrow column stores the code (`Side.BUYS`,
  `MarketDataKind.ORDR`), a getter answers the name.
- `new graph.EventIterator(items)` defaults `sorted` to `true` and trusts the
  order: an unsorted array is not refused, it yields broken chains (a step
  before its live element yielded as it came, `prevuuid` null); pass `false`
  for one you have not sorted.
- Iterators (`BookIterator`, `EventIterator`, `fromArrowReader`) and
  `BatchReader`s are one-shot: spread once, or rebuild the reader.
- A book's `alive()`, `aliveOn(side)`, `deltas()`, `limits(side)`,
  `bestPrice(side)`, `depth(side, n)` are methods; `isComplete`, `spread`,
  `isCrossed`, `isLocked`, `bboMidpoint` are getters; `BookEvent.keyed(unix,
  key)` is static.
- A book from a walk is complete only at a snapshot tick: test `isComplete`
  before reading `alive()`, `aliveOn`, `limits`, `depth` or `imbalance`, and
  rebuild a delta book with `book.withPrevious(previous)` - over
  `graph.BookEvent.keyed(unix, key)` for a code's first book, whose `prevuuid`
  is `null`. `withOperations` on a delta book is refused at `$.alive`.
- Arrow JS interop is copied IPC, never zero copy: keep bulk work in the
  native readers and cross into Arrow JS once (`intoTable()`).
- `withOperations`, `withPrevious`, `mergeWith` answer a new value; the one
  you called is unchanged. Only `withPrevious`/`mergeWith` answer `null` when
  nothing moved; `withOperations` refuses an undated `Order` at
  `$.operations[i].kind` (`BookIterator` at `$.operation.kind`); an execution
  is recorded among its instant's deltas, moving no side, and a trade is
  pruned, no error and no book. What `BookIterator`
  finds wrong in the data - an operation dated before its book - it leaves
  out, and an order or a quote stating neither side it places nowhere (still
  the book's delta), each with a warning on standard error (unless a handler
  on `logging.getLogger('yggdryl')` takes it), and no error.
